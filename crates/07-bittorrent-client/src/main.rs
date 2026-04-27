use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

#[cfg(test)]
mod tests;

const MAX_BLOCK_LENGTH: u32 = 16 * 1024;

#[derive(Parser)]
#[command(author, version, about = "Minimal BitTorrent client foundations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Read a .torrent file and print its metadata
    Inspect { path: PathBuf },
    /// Contact the torrent's HTTP tracker and print returned peers
    Tracker {
        path: PathBuf,
        /// TCP port this client would listen on for peers
        #[arg(long, default_value_t = 6881)]
        port: u16,
    },
    /// Discover peers and complete one BitTorrent TCP handshake
    Handshake {
        path: PathBuf,
        /// TCP port this client would listen on for peers
        #[arg(long, default_value_t = 6881)]
        port: u16,
        /// Number of tracker peers to try before giving up
        #[arg(long, default_value_t = 20)]
        max_peers: usize,
        /// Per-peer TCP and handshake timeout in milliseconds
        #[arg(long, default_value_t = 5_000)]
        timeout_ms: u64,
        /// Read and print one peer wire message after a successful handshake
        #[arg(long)]
        read_message: bool,
    },
    /// Download one piece into memory from a peer without verifying or writing it
    Piece {
        path: PathBuf,
        /// Piece index to request
        #[arg(long, default_value_t = 0)]
        index: u32,
        /// TCP port this client would listen on for peers
        #[arg(long, default_value_t = 6881)]
        port: u16,
        /// Number of tracker peers to try before giving up
        #[arg(long, default_value_t = 20)]
        max_peers: usize,
        /// Maximum in-flight block requests to one peer
        #[arg(long, default_value_t = 4)]
        pipeline: usize,
        /// Per-peer operation timeout in milliseconds
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
    },
}

#[derive(Debug, Clone)]
enum Bencode<'a> {
    Integer(i64),
    Bytes(&'a [u8]),
    List(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

#[derive(Debug, Clone)]
struct Value<'a> {
    raw: &'a [u8],
    kind: Bencode<'a>,
}

struct BencodeParser<'a> {
    input: &'a [u8],
    pos: usize,
}

#[derive(Debug)]
struct TorrentMeta {
    announce: Option<String>,
    name: String,
    piece_length: i64,
    total_length: Option<i64>,
    file_count: Option<usize>,
    piece_count: usize,
    info_hash: [u8; 20],
    info_hash_hex: String,
}

#[derive(Debug, PartialEq, Eq)]
struct TrackerResponse {
    interval: Option<i64>,
    peers: Vec<SocketAddrV4>,
}

#[derive(Debug, PartialEq, Eq)]
struct PeerHandshake {
    reserved: [u8; 8],
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PeerMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockRequest {
    index: u32,
    begin: u32,
    length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PieceBlock {
    index: u32,
    begin: u32,
    block: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PeerState {
    peer_choking: bool,
    peer_interested: bool,
    client_choking: bool,
    client_interested: bool,
    available_pieces: Vec<bool>,
    requested_blocks: Vec<BlockRequest>,
    peer_requested_blocks: Vec<BlockRequest>,
    received_blocks: Vec<PieceBlock>,
    sent_blocks: Vec<PieceBlock>,
}

impl<'a> BencodeParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(mut self) -> Result<Value<'a>> {
        let value = self.parse_value()?;
        if self.pos != self.input.len() {
            bail!("unexpected trailing data at byte {}", self.pos);
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<Value<'a>> {
        let start = self.pos;
        let byte = *self
            .input
            .get(self.pos)
            .ok_or_else(|| anyhow!("unexpected end of input"))?;

        let kind = match byte {
            b'i' => Bencode::Integer(self.parse_integer()?),
            b'l' => Bencode::List(self.parse_list()?),
            b'd' => Bencode::Dict(self.parse_dict()?),
            b'0'..=b'9' => Bencode::Bytes(self.parse_bytes()?),
            _ => bail!(
                "invalid bencode token '{}' at byte {}",
                byte as char,
                self.pos
            ),
        };

        let end = self.pos;
        Ok(Value {
            raw: &self.input[start..end],
            kind,
        })
    }

    fn parse_integer(&mut self) -> Result<i64> {
        self.expect_byte(b'i')?;
        let start = self.pos;
        while self.peek_byte()? != b'e' {
            self.pos += 1;
            if self.pos >= self.input.len() {
                bail!("unterminated integer");
            }
        }
        let digits = std::str::from_utf8(&self.input[start..self.pos])
            .context("integer was not valid utf-8 digits")?;
        self.expect_byte(b'e')?;
        digits
            .parse::<i64>()
            .with_context(|| format!("invalid integer value '{digits}'"))
    }

    fn parse_bytes(&mut self) -> Result<&'a [u8]> {
        let len_start = self.pos;
        while self.peek_byte()?.is_ascii_digit() {
            self.pos += 1;
            if self.pos >= self.input.len() {
                bail!("unterminated byte string length");
            }
        }
        if self.peek_byte()? != b':' {
            bail!("expected ':' after byte string length");
        }
        let len = std::str::from_utf8(&self.input[len_start..self.pos])
            .context("byte string length was not utf-8 digits")?
            .parse::<usize>()
            .context("invalid byte string length")?;
        self.expect_byte(b':')?;
        let end = self.pos.saturating_add(len);
        let bytes = self
            .input
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("byte string length exceeds input"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn parse_list(&mut self) -> Result<Vec<Value<'a>>> {
        self.expect_byte(b'l')?;
        let mut values = Vec::new();
        while self.peek_byte()? != b'e' {
            values.push(self.parse_value()?);
        }
        self.expect_byte(b'e')?;
        Ok(values)
    }

    fn parse_dict(&mut self) -> Result<Vec<(&'a [u8], Value<'a>)>> {
        self.expect_byte(b'd')?;
        let mut items = Vec::new();
        while self.peek_byte()? != b'e' {
            let key = self.parse_bytes()?;
            let value = self.parse_value()?;
            items.push((key, value));
        }
        self.expect_byte(b'e')?;
        Ok(items)
    }

    fn expect_byte(&mut self, expected: u8) -> Result<()> {
        let actual = *self
            .input
            .get(self.pos)
            .ok_or_else(|| anyhow!("unexpected end of input"))?;
        if actual != expected {
            bail!(
                "expected '{}' at byte {}, found '{}'",
                expected as char,
                self.pos,
                actual as char
            );
        }
        self.pos += 1;
        Ok(())
    }

    fn peek_byte(&self) -> Result<u8> {
        self.input
            .get(self.pos)
            .copied()
            .ok_or_else(|| anyhow!("unexpected end of input"))
    }
}

impl TorrentMeta {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let root = BencodeParser::new(bytes).parse()?;
        let root_dict = root
            .as_dict()
            .ok_or_else(|| anyhow!("torrent root must be a dictionary"))?;

        let announce = find_bytes(root_dict, b"announce")
            .map(bytes_to_string)
            .transpose()?;

        let info = find_value(root_dict, b"info")
            .ok_or_else(|| anyhow!("torrent is missing required 'info' dictionary"))?;
        let info_dict = info
            .as_dict()
            .ok_or_else(|| anyhow!("'info' must be a dictionary"))?;

        let name = bytes_to_string(
            find_bytes(info_dict, b"name")
                .ok_or_else(|| anyhow!("'info.name' must be present and be a byte string"))?,
        )?;

        let piece_length = find_integer(info_dict, b"piece length")
            .ok_or_else(|| anyhow!("'info.piece length' must be present and be an integer"))?;
        let pieces = find_bytes(info_dict, b"pieces")
            .ok_or_else(|| anyhow!("'info.pieces' must be present and be a byte string"))?;

        if pieces.len() % 20 != 0 {
            bail!(
                "'info.pieces' must be a multiple of 20 bytes, found {}",
                pieces.len()
            );
        }

        let total_length = find_integer(info_dict, b"length");
        let file_count = find_list(info_dict, b"files").map(|files| files.len());

        let info_hash = sha1_digest(info.raw);
        let info_hash_hex = hex_bytes(&info_hash);

        Ok(Self {
            announce,
            name,
            piece_length,
            total_length,
            file_count,
            piece_count: pieces.len() / 20,
            info_hash,
            info_hash_hex,
        })
    }

    fn mode_label(&self) -> &'static str {
        if self.file_count.is_some() {
            "multi-file"
        } else {
            "single-file"
        }
    }

    fn piece_length_at(&self, index: u32) -> Result<usize> {
        if index as usize >= self.piece_count {
            bail!("piece index {index} is outside torrent piece count");
        }
        if self.piece_length <= 0 {
            bail!("piece length must be positive");
        }

        let total_length = self
            .total_length
            .ok_or_else(|| anyhow!("piece command currently supports single-file torrents only"))?;
        if total_length < 0 {
            bail!("torrent length cannot be negative");
        }

        let piece_length = self.piece_length as usize;
        let total_length = total_length as usize;
        let start = index as usize * piece_length;
        let remaining = total_length.saturating_sub(start);

        Ok(remaining.min(piece_length))
    }
}

impl<'a> Value<'a> {
    fn as_dict(&self) -> Option<&Vec<(&'a [u8], Value<'a>)>> {
        match &self.kind {
            Bencode::Dict(entries) => Some(entries),
            _ => None,
        }
    }

    fn as_bytes(&self) -> Option<&'a [u8]> {
        match self.kind {
            Bencode::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    fn as_integer(&self) -> Option<i64> {
        match self.kind {
            Bencode::Integer(value) => Some(value),
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&Vec<Value<'a>>> {
        match &self.kind {
            Bencode::List(values) => Some(values),
            _ => None,
        }
    }
}

fn find_value<'a>(dict: &'a [(&'a [u8], Value<'a>)], key: &[u8]) -> Option<&'a Value<'a>> {
    dict.iter().find(|(k, _)| *k == key).map(|(_, value)| value)
}

fn find_bytes<'a>(dict: &'a [(&'a [u8], Value<'a>)], key: &[u8]) -> Option<&'a [u8]> {
    find_value(dict, key).and_then(Value::as_bytes)
}

fn find_integer(dict: &[(&[u8], Value<'_>)], key: &[u8]) -> Option<i64> {
    find_value(dict, key).and_then(Value::as_integer)
}

fn find_list<'a>(dict: &'a [(&'a [u8], Value<'a>)], key: &[u8]) -> Option<&'a Vec<Value<'a>>> {
    find_value(dict, key).and_then(Value::as_list)
}

fn bytes_to_string(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).context("field was not valid UTF-8")
}

#[cfg(test)]
fn sha1_hex(bytes: &[u8]) -> String {
    hex_bytes(&sha1_digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    out
}

fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => unreachable!("nibble must be in 0..=15"),
    }
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut message = input.to_vec();
    let bit_len = (message.len() as u64) * 8;

    message.push(0x80);
    while (message.len() % 64) != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xefcd_ab89;
    let mut h2: u32 = 0x98ba_dcfe;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xc3d2_e1f0;

    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (i, word) in words.iter_mut().take(16).enumerate() {
            let offset = i * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }

        for i in 16..80 {
            words[i] = (words[i - 3] ^ words[i - 8] ^ words[i - 14] ^ words[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, word) in words.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

fn inspect(path: PathBuf) -> Result<()> {
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let meta = TorrentMeta::from_bytes(&bytes)?;

    let announce = meta.announce.as_deref().unwrap_or("(none)");
    println!("announce: {}", announce);
    println!("name: {}", meta.name);
    println!("mode: {}", meta.mode_label());
    println!("piece length: {} bytes", meta.piece_length);
    match meta.total_length {
        Some(length) => println!("total length: {} bytes", length),
        None => println!("total length: (multi-file torrent)"),
    }
    if let Some(file_count) = meta.file_count {
        println!("files: {}", file_count);
    }
    println!("pieces: {}", meta.piece_count);
    println!("info hash: {}", meta.info_hash_hex);

    Ok(())
}

fn build_tracker_url(meta: &TorrentMeta, peer_id: &[u8; 20], port: u16) -> Result<String> {
    let announce = meta
        .announce
        .as_ref()
        .ok_or_else(|| anyhow!("torrent has no announce URL"))?;
    let left = meta
        .total_length
        .ok_or_else(|| anyhow!("tracker command currently supports single-file torrents only"))?;
    if left < 0 {
        bail!("torrent length cannot be negative");
    }

    let separator = if announce.contains('?') { '&' } else { '?' };
    Ok(format!(
        "{announce}{separator}info_hash={}&peer_id={}&port={port}&uploaded=0&downloaded=0&left={left}&compact=1&event=started",
        percent_encode_bytes(&meta.info_hash),
        percent_encode_bytes(peer_id),
    ))
}

fn percent_encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for &byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(nibble_to_hex(byte >> 4).to_ascii_uppercase());
                out.push(nibble_to_hex(byte & 0x0f).to_ascii_uppercase());
            }
        }
    }
    out
}

async fn tracker(path: PathBuf, port: u16) -> Result<()> {
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let meta = TorrentMeta::from_bytes(&bytes)?;

    println!("tracker: {}", meta.announce.as_deref().unwrap_or("(none)"));
    println!("requesting compact peer list...");

    let response = request_tracker_response(&meta, port).await?;
    if let Some(interval) = response.interval {
        println!("interval: {} seconds", interval);
    }
    println!("peers: {}", response.peers.len());
    for peer in response.peers.iter().take(20) {
        println!("- {}", peer);
    }
    if response.peers.len() > 20 {
        println!("- ... {} more", response.peers.len() - 20);
    }

    Ok(())
}

async fn request_tracker_response(meta: &TorrentMeta, port: u16) -> Result<TrackerResponse> {
    let peer_id = default_peer_id();
    let url = build_tracker_url(meta, &peer_id, port)?;
    let response_bytes = reqwest::get(&url)
        .await
        .with_context(|| format!("requesting tracker URL for {}", meta.name))?
        .error_for_status()
        .context("tracker returned an HTTP error")?
        .bytes()
        .await
        .context("reading tracker response body")?;

    TrackerResponse::from_bytes(&response_bytes)
}

fn default_peer_id() -> [u8; 20] {
    *b"-RS0001-000000000001"
}

impl TrackerResponse {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let root = BencodeParser::new(bytes).parse()?;
        let root_dict = root
            .as_dict()
            .ok_or_else(|| anyhow!("tracker response must be a dictionary"))?;

        if let Some(reason) = find_bytes(root_dict, b"failure reason") {
            bail!("tracker failure: {}", String::from_utf8_lossy(reason));
        }

        let interval = find_integer(root_dict, b"interval");
        let peers = find_bytes(root_dict, b"peers")
            .ok_or_else(|| anyhow!("tracker response missing compact 'peers' field"))?;
        let peers = parse_compact_peers(peers)?;

        Ok(Self { interval, peers })
    }
}

fn parse_compact_peers(bytes: &[u8]) -> Result<Vec<SocketAddrV4>> {
    if bytes.len() % 6 != 0 {
        bail!(
            "compact peer list must be a multiple of 6 bytes, found {}",
            bytes.len()
        );
    }

    Ok(bytes
        .chunks_exact(6)
        .map(|chunk| {
            SocketAddrV4::new(
                Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]),
                u16::from_be_bytes([chunk[4], chunk[5]]),
            )
        })
        .collect())
}

async fn handshake(
    path: PathBuf,
    port: u16,
    max_peers: usize,
    timeout_ms: u64,
    read_message: bool,
) -> Result<()> {
    if max_peers == 0 {
        bail!("max-peers must be greater than zero");
    }

    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let meta = TorrentMeta::from_bytes(&bytes)?;
    let peer_id = default_peer_id();
    let timeout = Duration::from_millis(timeout_ms);
    let tracker_response = request_tracker_response(&meta, port).await?;

    println!("tracker peers: {}", tracker_response.peers.len());
    for peer in tracker_response.peers.iter().take(max_peers) {
        println!("trying {}", peer);
        match connect_and_handshake(*peer, &meta.info_hash, &peer_id, timeout).await {
            Ok((mut stream, response)) => {
                println!("connected: {}", peer);
                println!("peer id: {}", String::from_utf8_lossy(&response.peer_id));
                println!("reserved: {}", hex_bytes(&response.reserved));
                if read_message {
                    let message = time::timeout(timeout, read_peer_message(&mut stream))
                        .await
                        .with_context(|| format!("timed out reading peer message from {peer}"))?
                        .with_context(|| format!("reading peer message from {peer}"))?;
                    let mut state = PeerState::new(meta.piece_count);
                    state.apply_inbound(&message)?;
                    println!("message: {}", message.summary());
                    println!("raw: {}", hex_bytes(&message.encode()));
                    println!("state: {}", state.summary());
                }
                return Ok(());
            }
            Err(error) => {
                println!("failed {}: {}", peer, error);
            }
        }
    }

    bail!(
        "no peer completed a BitTorrent handshake after trying {} peers",
        max_peers.min(tracker_response.peers.len())
    )
}

async fn piece(
    path: PathBuf,
    index: u32,
    port: u16,
    max_peers: usize,
    pipeline: usize,
    timeout_ms: u64,
) -> Result<()> {
    if max_peers == 0 {
        bail!("max-peers must be greater than zero");
    }
    if pipeline == 0 {
        bail!("pipeline must be greater than zero");
    }

    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let meta = TorrentMeta::from_bytes(&bytes)?;
    let piece_length = meta.piece_length_at(index)?;
    let peer_id = default_peer_id();
    let timeout = Duration::from_millis(timeout_ms);
    let tracker_response = request_tracker_response(&meta, port).await?;

    println!("tracker peers: {}", tracker_response.peers.len());
    println!(
        "downloading piece {} ({} bytes, pipeline {})",
        index, piece_length, pipeline
    );

    for peer in tracker_response.peers.iter().take(max_peers) {
        println!("trying {}", peer);
        match connect_and_handshake(*peer, &meta.info_hash, &peer_id, timeout).await {
            Ok((mut stream, response)) => {
                println!("connected: {}", peer);
                println!("peer id: {}", String::from_utf8_lossy(&response.peer_id));

                match time::timeout(
                    timeout,
                    download_piece_from_peer(
                        &mut stream,
                        meta.piece_count,
                        index,
                        piece_length,
                        pipeline,
                    ),
                )
                .await
                {
                    Ok(Ok(piece)) => {
                        println!("downloaded piece {}: {} bytes", index, piece.len());
                        println!("note: piece hash verification is not implemented yet");
                        return Ok(());
                    }
                    Ok(Err(error)) => {
                        println!("failed {}: {}", peer, error);
                    }
                    Err(error) => {
                        println!("failed {}: timed out downloading piece: {}", peer, error);
                    }
                }
            }
            Err(error) => {
                println!("failed {}: {}", peer, error);
            }
        }
    }

    bail!(
        "no peer downloaded piece {} after trying {} peers",
        index,
        max_peers.min(tracker_response.peers.len())
    )
}

async fn connect_and_handshake(
    peer: SocketAddrV4,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    timeout: Duration,
) -> Result<(TcpStream, PeerHandshake)> {
    let mut stream = time::timeout(timeout, TcpStream::connect(peer))
        .await
        .with_context(|| format!("timed out connecting to {peer}"))?
        .with_context(|| format!("connecting to {peer}"))?;

    let response = time::timeout(
        timeout,
        perform_peer_handshake(&mut stream, info_hash, peer_id),
    )
    .await
    .with_context(|| format!("timed out during handshake with {peer}"))?
    .with_context(|| format!("handshaking with {peer}"))?;

    if &response.info_hash != info_hash {
        bail!("peer returned a different info_hash");
    }

    Ok((stream, response))
}

async fn perform_peer_handshake<S>(
    stream: &mut S,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
) -> Result<PeerHandshake>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = build_peer_handshake(info_hash, peer_id);
    stream
        .write_all(&request)
        .await
        .context("sending handshake")?;

    let mut response = [0u8; 68];
    stream
        .read_exact(&mut response)
        .await
        .context("reading handshake")?;

    parse_peer_handshake(&response)
}

fn build_peer_handshake(info_hash: &[u8; 20], peer_id: &[u8; 20]) -> [u8; 68] {
    let mut handshake = [0u8; 68];
    handshake[0] = 19;
    handshake[1..20].copy_from_slice(b"BitTorrent protocol");
    handshake[28..48].copy_from_slice(info_hash);
    handshake[48..68].copy_from_slice(peer_id);
    handshake
}

fn parse_peer_handshake(bytes: &[u8]) -> Result<PeerHandshake> {
    if bytes.len() != 68 {
        bail!("peer handshake must be 68 bytes, found {}", bytes.len());
    }
    if bytes[0] != 19 || &bytes[1..20] != b"BitTorrent protocol" {
        bail!("peer did not speak the BitTorrent protocol");
    }

    let mut reserved = [0u8; 8];
    reserved.copy_from_slice(&bytes[20..28]);
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&bytes[28..48]);
    let mut peer_id = [0u8; 20];
    peer_id.copy_from_slice(&bytes[48..68]);

    Ok(PeerHandshake {
        reserved,
        info_hash,
        peer_id,
    })
}

impl PeerMessage {
    fn encode(&self) -> Vec<u8> {
        match self {
            Self::KeepAlive => 0u32.to_be_bytes().to_vec(),
            Self::Choke => encode_message(0, &[]),
            Self::Unchoke => encode_message(1, &[]),
            Self::Interested => encode_message(2, &[]),
            Self::NotInterested => encode_message(3, &[]),
            Self::Have(index) => encode_message(4, &index.to_be_bytes()),
            Self::Bitfield(bitfield) => encode_message(5, bitfield),
            Self::Request {
                index,
                begin,
                length,
            } => encode_message(6, &concat_u32s(&[*index, *begin, *length])),
            Self::Piece {
                index,
                begin,
                block,
            } => {
                let mut payload = Vec::with_capacity(8 + block.len());
                payload.extend_from_slice(&index.to_be_bytes());
                payload.extend_from_slice(&begin.to_be_bytes());
                payload.extend_from_slice(block);
                encode_message(7, &payload)
            }
            Self::Cancel {
                index,
                begin,
                length,
            } => encode_message(8, &concat_u32s(&[*index, *begin, *length])),
            Self::Port(port) => encode_message(9, &port.to_be_bytes()),
        }
    }

    fn decode(frame: &[u8]) -> Result<Self> {
        if frame.len() < 4 {
            bail!("peer message frame must include a 4-byte length prefix");
        }

        let length = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        let payload = frame
            .get(4..4 + length)
            .ok_or_else(|| anyhow!("peer message length exceeds frame"))?;
        if frame.len() != 4 + length {
            bail!("peer message frame has trailing bytes");
        }
        if length == 0 {
            return Ok(Self::KeepAlive);
        }

        let (&message_id, payload) = payload
            .split_first()
            .ok_or_else(|| anyhow!("non-empty peer message missing id"))?;

        match message_id {
            0 => expect_empty_payload(payload, "choke").map(|()| Self::Choke),
            1 => expect_empty_payload(payload, "unchoke").map(|()| Self::Unchoke),
            2 => expect_empty_payload(payload, "interested").map(|()| Self::Interested),
            3 => expect_empty_payload(payload, "not interested").map(|()| Self::NotInterested),
            4 => Ok(Self::Have(read_u32_payload(payload, "have")?)),
            5 => Ok(Self::Bitfield(payload.to_vec())),
            6 => {
                let [index, begin, length] = read_three_u32_payload(payload, "request")?;
                Ok(Self::Request {
                    index,
                    begin,
                    length,
                })
            }
            7 => {
                if payload.len() < 8 {
                    bail!("piece message payload must contain index and begin");
                }
                Ok(Self::Piece {
                    index: read_u32(&payload[0..4]),
                    begin: read_u32(&payload[4..8]),
                    block: payload[8..].to_vec(),
                })
            }
            8 => {
                let [index, begin, length] = read_three_u32_payload(payload, "cancel")?;
                Ok(Self::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            9 => {
                if payload.len() != 2 {
                    bail!("port message payload must be 2 bytes");
                }
                Ok(Self::Port(u16::from_be_bytes([payload[0], payload[1]])))
            }
            _ => bail!("unknown peer message id {}", message_id),
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::KeepAlive => "keep-alive".to_string(),
            Self::Choke => "choke".to_string(),
            Self::Unchoke => "unchoke".to_string(),
            Self::Interested => "interested".to_string(),
            Self::NotInterested => "not interested".to_string(),
            Self::Have(index) => format!("have piece {index}"),
            Self::Bitfield(bytes) => format!("bitfield ({} bytes)", bytes.len()),
            Self::Request {
                index,
                begin,
                length,
            } => format!("request piece {index}, begin {begin}, length {length}"),
            Self::Piece {
                index,
                begin,
                block,
            } => format!(
                "piece data for piece {index}, begin {begin}, block {} bytes",
                block.len()
            ),
            Self::Cancel {
                index,
                begin,
                length,
            } => format!("cancel piece {index}, begin {begin}, length {length}"),
            Self::Port(port) => format!("port {port}"),
        }
    }
}

impl PeerState {
    fn new(piece_count: usize) -> Self {
        Self {
            peer_choking: true,
            peer_interested: false,
            client_choking: true,
            client_interested: false,
            available_pieces: vec![false; piece_count],
            requested_blocks: Vec::new(),
            peer_requested_blocks: Vec::new(),
            received_blocks: Vec::new(),
            sent_blocks: Vec::new(),
        }
    }

    fn apply_inbound(&mut self, message: &PeerMessage) -> Result<()> {
        match message {
            PeerMessage::KeepAlive => {}
            PeerMessage::Choke => self.peer_choking = true,
            PeerMessage::Unchoke => self.peer_choking = false,
            PeerMessage::Interested => self.peer_interested = true,
            PeerMessage::NotInterested => self.peer_interested = false,
            PeerMessage::Have(index) => self.mark_piece_available(*index)?,
            PeerMessage::Bitfield(bitfield) => self.apply_bitfield(bitfield)?,
            PeerMessage::Request {
                index,
                begin,
                length,
            } => self.peer_requested_blocks.push(BlockRequest {
                index: *index,
                begin: *begin,
                length: *length,
            }),
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => self.received_blocks.push(PieceBlock {
                index: *index,
                begin: *begin,
                block: block.clone(),
            }),
            PeerMessage::Cancel {
                index,
                begin,
                length,
            } => self.remove_peer_request(*index, *begin, *length),
            PeerMessage::Port(_) => {}
        }

        Ok(())
    }

    fn apply_outbound(&mut self, message: &PeerMessage) -> Result<()> {
        match message {
            PeerMessage::KeepAlive => {}
            PeerMessage::Choke => self.client_choking = true,
            PeerMessage::Unchoke => self.client_choking = false,
            PeerMessage::Interested => self.client_interested = true,
            PeerMessage::NotInterested => self.client_interested = false,
            PeerMessage::Have(_) => {}
            PeerMessage::Bitfield(_) => {}
            PeerMessage::Request {
                index,
                begin,
                length,
            } => self.requested_blocks.push(BlockRequest {
                index: *index,
                begin: *begin,
                length: *length,
            }),
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => self.sent_blocks.push(PieceBlock {
                index: *index,
                begin: *begin,
                block: block.clone(),
            }),
            PeerMessage::Cancel {
                index,
                begin,
                length,
            } => self.remove_request(*index, *begin, *length),
            PeerMessage::Port(_) => {}
        }

        Ok(())
    }

    fn has_piece(&self, index: usize) -> bool {
        self.available_pieces.get(index).copied().unwrap_or(false)
    }

    fn available_piece_count(&self) -> usize {
        self.available_pieces
            .iter()
            .filter(|&&available| available)
            .count()
    }

    fn summary(&self) -> String {
        format!(
            "peer_choking={}, peer_interested={}, client_choking={}, client_interested={}, available_pieces={}/{}, requested_blocks={}, received_blocks={}",
            self.peer_choking,
            self.peer_interested,
            self.client_choking,
            self.client_interested,
            self.available_piece_count(),
            self.available_pieces.len(),
            self.requested_blocks.len(),
            self.received_blocks.len()
        )
    }

    fn mark_piece_available(&mut self, index: u32) -> Result<()> {
        let piece = self
            .available_pieces
            .get_mut(index as usize)
            .ok_or_else(|| anyhow!("piece index {index} is outside torrent piece count"))?;
        *piece = true;
        Ok(())
    }

    fn apply_bitfield(&mut self, bitfield: &[u8]) -> Result<()> {
        if bitfield.len() * 8 < self.available_pieces.len() {
            bail!("bitfield is too short for torrent piece count");
        }

        for piece_index in 0..self.available_pieces.len() {
            let byte = bitfield[piece_index / 8];
            let mask = 1 << (7 - (piece_index % 8));
            self.available_pieces[piece_index] = byte & mask != 0;
        }

        Ok(())
    }

    fn remove_request(&mut self, index: u32, begin: u32, length: u32) {
        self.requested_blocks.retain(|request| {
            request.index != index || request.begin != begin || request.length != length
        });
    }

    fn remove_peer_request(&mut self, index: u32, begin: u32, length: u32) {
        self.peer_requested_blocks.retain(|request| {
            request.index != index || request.begin != begin || request.length != length
        });
    }
}

async fn read_peer_message<S>(stream: &mut S) -> Result<PeerMessage>
where
    S: AsyncRead + Unpin,
{
    let mut length_prefix = [0u8; 4];
    stream
        .read_exact(&mut length_prefix)
        .await
        .context("reading peer message length")?;

    let length = u32::from_be_bytes(length_prefix) as usize;
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&length_prefix);
    frame.resize(4 + length, 0);
    stream
        .read_exact(&mut frame[4..])
        .await
        .context("reading peer message payload")?;

    PeerMessage::decode(&frame)
}

async fn write_peer_message<S>(stream: &mut S, message: &PeerMessage) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&message.encode())
        .await
        .with_context(|| format!("sending peer message: {}", message.summary()))
}

async fn download_piece_from_peer<S>(
    stream: &mut S,
    piece_count: usize,
    piece_index: u32,
    piece_length: usize,
    pipeline: usize,
) -> Result<Vec<u8>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut state = PeerState::new(piece_count);
    let interested = PeerMessage::Interested;
    write_peer_message(stream, &interested).await?;
    state.apply_outbound(&interested)?;

    wait_until_piece_can_be_requested(stream, &mut state, piece_index).await?;

    let requests = build_piece_requests(piece_index, piece_length)?;
    let mut piece = vec![0u8; piece_length];
    let mut next_request = 0usize;
    let mut completed_blocks = 0usize;

    while completed_blocks < requests.len() {
        while state.requested_blocks.len() < pipeline && next_request < requests.len() {
            let request = requests[next_request].clone();
            let message = PeerMessage::Request {
                index: request.index,
                begin: request.begin,
                length: request.length,
            };
            write_peer_message(stream, &message).await?;
            state.apply_outbound(&message)?;
            next_request += 1;
        }

        let message = read_peer_message(stream).await?;
        match &message {
            PeerMessage::Piece {
                index,
                begin,
                block,
            } if *index == piece_index => {
                let begin_offset = *begin as usize;
                let end = begin_offset + block.len();
                if end > piece.len() {
                    bail!("peer sent piece block beyond requested piece length");
                }

                piece[begin_offset..end].copy_from_slice(block);
                state.remove_request(*index, *begin, block.len() as u32);
                completed_blocks += 1;
                println!(
                    "received block {}/{} (begin {}, {} bytes)",
                    completed_blocks,
                    requests.len(),
                    begin_offset,
                    block.len()
                );
            }
            PeerMessage::Choke => {
                state.apply_inbound(&message)?;
                bail!("peer choked us while downloading");
            }
            _ => {
                state.apply_inbound(&message)?;
            }
        }
    }

    Ok(piece)
}

async fn wait_until_piece_can_be_requested<S>(
    stream: &mut S,
    state: &mut PeerState,
    piece_index: u32,
) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    loop {
        if !state.peer_choking && state.has_piece(piece_index as usize) {
            return Ok(());
        }

        let message = read_peer_message(stream).await?;
        state.apply_inbound(&message)?;
        println!("message: {}", message.summary());
        println!("state: {}", state.summary());
    }
}

fn build_piece_requests(piece_index: u32, piece_length: usize) -> Result<Vec<BlockRequest>> {
    if piece_length == 0 {
        bail!("piece length must be greater than zero");
    }

    let mut requests = Vec::new();
    let mut offset = 0usize;
    while offset < piece_length {
        let length = (piece_length - offset).min(MAX_BLOCK_LENGTH as usize);
        requests.push(BlockRequest {
            index: piece_index,
            begin: offset as u32,
            length: length as u32,
        });
        offset += length;
    }

    Ok(requests)
}

fn encode_message(message_id: u8, payload: &[u8]) -> Vec<u8> {
    let length = 1 + payload.len();
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&(length as u32).to_be_bytes());
    frame.push(message_id);
    frame.extend_from_slice(payload);
    frame
}

fn concat_u32s(values: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes
}

fn expect_empty_payload(payload: &[u8], name: &str) -> Result<()> {
    if !payload.is_empty() {
        bail!("{name} message payload must be empty");
    }
    Ok(())
}

fn read_u32_payload(payload: &[u8], name: &str) -> Result<u32> {
    if payload.len() != 4 {
        bail!("{name} message payload must be 4 bytes");
    }
    Ok(read_u32(payload))
}

fn read_three_u32_payload(payload: &[u8], name: &str) -> Result<[u32; 3]> {
    if payload.len() != 12 {
        bail!("{name} message payload must be 12 bytes");
    }
    Ok([
        read_u32(&payload[0..4]),
        read_u32(&payload[4..8]),
        read_u32(&payload[8..12]),
    ])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect { path } => inspect(path),
        Commands::Tracker { path, port } => tracker(path, port).await,
        Commands::Handshake {
            path,
            port,
            max_peers,
            timeout_ms,
            read_message,
        } => handshake(path, port, max_peers, timeout_ms, read_message).await,
        Commands::Piece {
            path,
            index,
            port,
            max_peers,
            pipeline,
            timeout_ms,
        } => piece(path, index, port, max_peers, pipeline, timeout_ms).await,
    }
}

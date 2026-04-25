use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

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
    info_hash_hex: String,
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

        let info_hash_hex = sha1_hex(info.raw);

        Ok(Self {
            announce,
            name,
            piece_length,
            total_length,
            file_count,
            piece_count: pieces.len() / 20,
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

fn sha1_hex(bytes: &[u8]) -> String {
    let digest = sha1_digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect { path } => inspect(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_file_torrent_metadata() {
        let torrent =
            b"d8:announce32:https://tracker.example/announce4:infod6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

        let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

        assert_eq!(
            meta.announce.as_deref(),
            Some("https://tracker.example/announce")
        );
        assert_eq!(meta.name, "test.txt");
        assert_eq!(meta.piece_length, 16384);
        assert_eq!(meta.total_length, Some(12));
        assert_eq!(meta.file_count, None);
        assert_eq!(meta.piece_count, 1);
    }

    #[test]
    fn hashes_original_info_bytes() {
        let info =
            b"d6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
        let torrent =
            b"d8:announce32:https://tracker.example/announce4:infod6:lengthi12e4:name8:test.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

        let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

        assert_eq!(meta.info_hash_hex, sha1_hex(info));
    }

    #[test]
    fn parses_multi_file_marker() {
        let torrent = b"d4:infod5:filesld6:lengthi5e4:pathl5:a.txteed6:lengthi7e4:pathl5:b.txteee4:name4:pack12:piece lengthi32768e6:pieces20:bbbbbbbbbbbbbbbbbbbbee";

        let meta = TorrentMeta::from_bytes(torrent).expect("torrent should parse");

        assert_eq!(meta.mode_label(), "multi-file");
        assert_eq!(meta.total_length, None);
        assert_eq!(meta.file_count, Some(2));
    }

    #[test]
    fn sha1_matches_known_vector() {
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}

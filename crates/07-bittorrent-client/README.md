# 07 BitTorrent Client

[Back to workspace README](../../README.md)

## Overview

`bittorrent-client` starts with the smallest useful BitTorrent building blocks: reading a `.torrent` file, decoding bencode, extracting metadata, computing the correct `info_hash` from the original raw `info` dictionary bytes, asking an HTTP tracker for peers, completing a peer handshake, encoding peer wire messages, tracking basic peer state, downloading a single piece into memory, and verifying that piece against the torrent metadata.

This version can contact HTTP trackers, parse compact IPv4 peer lists, open TCP connections to peers, verify the BitTorrent handshake, encode/decode the core length-prefixed peer messages, update in-memory state for choke, interest, bitfield, request, and piece messages, request piece blocks with bounded backpressure, and verify downloaded piece bytes with SHA-1. It does not write file data yet.

## What It Demonstrates

- Recursive bencode parsing
- Borrowed byte-slice parsing without copying the whole file structure
- Correct `info_hash` computation from raw bencoded bytes
- HTTP tracker announce URL construction
- Compact tracker peer response parsing
- TCP peer connection attempts with `tokio`
- BitTorrent handshake encoding and validation
- Peer wire message encoding and decoding
- Peer state transitions for choking, interest, availability, requests, and pieces
- Bounded in-flight block requests for downloading one piece into memory
- SHA-1 verification of downloaded pieces before accepting them
- Small CLI structure with focused commands

## Setup Steps

1. Read the code in `src/main.rs` from top to bottom.
2. Start with `BencodeParser` and see how each value returns both its parsed shape and its original byte span.
3. Look at `TorrentMeta::from_bytes` to understand how top-level fields are extracted.
4. Read `build_tracker_url` to see how `info_hash`, `peer_id`, and transfer counters become tracker query parameters.
5. Read `build_peer_handshake` and `parse_peer_handshake` to see the 68-byte peer handshake layout.
6. Read `PeerMessage::encode` and `PeerMessage::decode` to see how peer wire messages use a 4-byte length prefix, a 1-byte message id, and optional payload bytes.
7. Read `PeerState::apply_inbound` and `PeerState::apply_outbound` to see how messages change choking, interest, piece availability, request, and piece-block state.
8. Read `download_piece_from_peer` to see how a bounded pipeline keeps only a limited number of block requests in flight.
9. Read `verify_piece_hash` to see how downloaded piece bytes are checked against the 20-byte hash from `info.pieces`.
10. Check the tests to see why hashing the raw `info` bytes matters and how compact peers, handshakes, peer messages, peer state, bounded piece downloads, and piece verification are decoded.

## Manual Usage

Run from the repository root:

```bash
cargo run -p bittorrent-client -- inspect path/to/file.torrent
```

Ask the torrent's tracker for peers:

```bash
cargo run -p bittorrent-client -- tracker path/to/file.torrent
cargo run -p bittorrent-client -- tracker path/to/file.torrent --port 6881
```

Try a TCP connection and BitTorrent handshake with one discovered peer:

```bash
cargo run -p bittorrent-client -- handshake path/to/file.torrent
cargo run -p bittorrent-client -- handshake path/to/file.torrent --max-peers 10 --timeout-ms 3000
cargo run -p bittorrent-client -- handshake path/to/file.torrent --read-message --max-peers 50 --timeout-ms 10000
```

Download and verify one piece in memory without file writes:

```bash
cargo run -p bittorrent-client -- piece path/to/file.torrent --index 0 --pipeline 4 --max-peers 50 --timeout-ms 10000
```

Example output:

```text
announce: https://tracker.example/announce
name: ubuntu.iso
mode: single-file
piece length: 262144 bytes
total length: 1048576 bytes
pieces: 4
info hash: 0123456789abcdef0123456789abcdef01234567
```

Example tracker output:

```text
tracker: http://bttracker.debian.org:6969/announce
requesting compact peer list...
interval: 1800 seconds
peers: 2
- 127.0.0.1:6881
- 192.0.2.5:51226
```

If the torrent is multi-file, this version will still inspect it, but the tracker and piece commands currently support single-file torrents only. Final file assembly is not implemented yet.

# 07 BitTorrent Client

[Back to workspace README](../../README.md)

## Overview

`bittorrent-client` starts with the smallest useful BitTorrent building blocks: reading a `.torrent` file, decoding bencode, extracting metadata, computing the correct `info_hash` from the original raw `info` dictionary bytes, and asking an HTTP tracker for peers.

This version can contact HTTP trackers and parse compact IPv4 peer lists. It does not connect to peers or download file data yet.

## What It Demonstrates

- Recursive bencode parsing
- Borrowed byte-slice parsing without copying the whole file structure
- Correct `info_hash` computation from raw bencoded bytes
- HTTP tracker announce URL construction
- Compact tracker peer response parsing
- Small CLI structure with focused commands

## Setup Steps

1. Read the code in `src/main.rs` from top to bottom.
2. Start with `BencodeParser` and see how each value returns both its parsed shape and its original byte span.
3. Look at `TorrentMeta::from_bytes` to understand how top-level fields are extracted.
4. Read `build_tracker_url` to see how `info_hash`, `peer_id`, and transfer counters become tracker query parameters.
5. Check the tests to see why hashing the raw `info` bytes matters and how compact peers are decoded.

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

If the torrent is multi-file, this version will still inspect it, but the tracker command currently supports single-file torrents only. Downloading is not implemented yet.

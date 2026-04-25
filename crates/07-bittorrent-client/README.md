# 07 BitTorrent Client

[Back to workspace README](../../README.md)

## Overview

`bittorrent-client` starts with the smallest useful BitTorrent building block: reading a `.torrent` file, decoding bencode, extracting metadata, and computing the correct `info_hash` from the original raw `info` dictionary bytes.

This first version does not contact trackers or peers yet. It exists to make the file format and hashing rules concrete before networking is added.

## What It Demonstrates

- Recursive bencode parsing
- Borrowed byte-slice parsing without copying the whole file structure
- Correct `info_hash` computation from raw bencoded bytes
- Small CLI structure with one focused command

## Setup Steps

1. Read the code in `src/main.rs` from top to bottom.
2. Start with `BencodeParser` and see how each value returns both its parsed shape and its original byte span.
3. Look at `TorrentMeta::from_bytes` to understand how top-level fields are extracted.
4. Check the tests to see why hashing the raw `info` bytes matters.

## Manual Usage

Run from the repository root:

```bash
cargo run -p bittorrent-client -- inspect path/to/file.torrent
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

If the torrent is multi-file, this version will still inspect it, but downloading is not implemented yet.

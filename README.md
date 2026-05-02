# Rust Systems Lab

Rust Systems Lab is a Cargo workspace for learning Rust through small systems projects. Each crate focuses on one practical backend or infrastructure concept, starting with command-line parsing and moving through networking, async work, storage, durable queues, and query execution.

The goal is to understand how real systems are shaped at a smaller scale: how data moves through a program, how state is represented, how processes communicate, how durability works, and how Rust's ownership model affects those designs.

## Repository Structure

All projects are located in the `crates/` directory, following a strict "Foundations to Distributed" order. Each crate has its own README with manual usage commands.

- **[`01-log-analyzer`](crates/01-log-analyzer/README.md)**: CLI, pattern matching, and zero-copy parsing.
- **[`02-mini-http`](crates/02-mini-http/README.md)**: `std::net`, TCP, and thread pool implementation.
- **[`03-async-scraper`](crates/03-async-scraper/README.md)**: `tokio`, async I/O, and backpressure management.
- **[`04-kv-store`](crates/04-kv-store/README.md)**: State management, binary serialization, and WAL recovery.
- **[`05-task-queue`](crates/05-task-queue/README.md)**: Durable queue semantics, idempotency, leasing, and WAL replay.
- **[`06-query-engine`](crates/06-query-engine/README.md)**: B+ tree-style indexing, query parsing, and memory arena allocation.
- **[`07-bittorrent-client`](crates/07-bittorrent-client/README.md)**: Bencode parsing, torrent metadata inspection, and correct `info_hash` extraction.

## How To Use This Repo

Start with the crate README for the project you want to inspect. Each one explains what the crate demonstrates and includes manual commands that can be run from the repository root.

The workspace can also be checked as a whole:

```bash
cargo check --workspace
cargo test --workspace
```

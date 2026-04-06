# 🦀 Rust Systems Lab

A sequenced progression through Rust systems engineering, structured as a Cargo workspace to compound mastery in ownership, concurrency, and distributed architecture.

## 🏗️ Workspace Structure

All projects are located in the `crates/` directory, following a strict "Foundations to Distributed" order.

- **`01-log-analyzer`**: CLI, pattern matching, and zero-copy parsing.
- **`02-mini-http`**: `std::net`, TCP, and thread pool implementation.
- **`03-async-scraper`**: `tokio`, async I/O, and backpressure management.
- **`04-kv-store`**: State management, binary serialization, and WAL recovery.
- **`05-task-queue`**: gRPC, idempotency, and distributed reliability.
- **`06-query-engine`**: B+ Trees, query parsing, and memory arena allocation.

---

## 🚀 The Roadmap

- 🟢 **Phase 1: 01-CLI Log Analyzer** - Ownership, `Result`/`Option`, `clap`, `serde`.
- 🟢 **Phase 2: 02-Minimal HTTP Server** - TCP streams, thread pools, `Arc` + `Mutex`.
- 🟡 **Phase 3: 03-Async Web Scraper** - `tokio`, `reqwest`, `mpsc` channels, `tracing`.
- 🟡 **Phase 4: 04-In-Memory KV Store** - `RwLock`, binary serialization, WAL, snapshotting.
- 🔴 **Phase 5: 05-Distributed Task Queue** - gRPC, leader election, exactly-once semantics.
- 🔴 **Phase 6: 06-Embedded Query Engine** - B+ tree indexing, `nom`, memory arena.

---

## 🛠️ Standards

- **Validation:** `cargo check` and `cargo test` for all crates.
- **Observability:** `tracing` integration for all 🟡 and 🔴 phases.
- **Safety:** Zero `unsafe` code unless strictly required for Phase 6 performance.

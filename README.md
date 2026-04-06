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

### 🟢 Phase 1: 01-CLI Log Analyzer

**Concepts:** Ownership, `Result`/`Option`, pattern matching, `clap`, `serde`, `std::fs`  
**Trade-off:** Low architectural complexity; forces strict borrow checker discipline without async noise.  
**BC/AB Signal:** Proves production-ready CLI/tooling skills; high demand in DevOps and data ops.  
**Takeaway:** Add `criterion` benchmarks + integration tests. Publish to crates.io with CI.

### 🟢 Phase 2: 02-Minimal HTTP Server

**Concepts:** TCP streams, HTTP/1.1 parsing, thread pools, graceful shutdown, `Arc` + `Mutex`  
**Trade-off:** Reinvents routing/async; intentionally avoids frameworks to expose sync concurrency boundaries.  
**BC/AB Signal:** Foundation for proxies, load balancers, and internal API gateways.  
**Takeaway:** Implement keep-alive, connection limits, and document throughput vs `axum`.

### 🟡 Phase 3: 03-Async Web Scraper

**Concepts:** `tokio`, `reqwest`, `mpsc` channels, rate limiting, backpressure, `tracing`  
**Trade-off:** Async introduces lifetime/task complexity; requires explicit cancellation and error recovery design.  
**BC/AB Signal:** Direct mapping to data engineering and monitoring roles in SaaS/startups.  
**Takeaway:** Add retry logic with exponential backoff and expose `/metrics` endpoint.

### 🟡 Phase 4: 04-In-Memory KV Store

**Concepts:** `RwLock`, binary serialization (`bincode`), connection pooling, WAL, snapshotting  
**Trade-off:** Manual concurrency vs framework convenience; teaches when to avoid over-engineering.  
**BC/AB Signal:** Mirrors internal caching/session services in fintech and industrial IoT platforms.  
**Takeaway:** Implement crash recovery via append-only log + add Prometheus metrics for SLO tracking.

### 🔴 Phase 5: 05-Distributed Task Queue

**Concepts:** gRPC/TCP, leader election, idempotency, exactly-once semantics, partition simulation  
**Trade-off:** High complexity; minimal immediate ROI for simple apps, essential for scale-out systems.  
**BC/AB Signal:** Positions you for platform engineering, infra, or distributed systems roles.  
**Takeaway:** Start single-node → add Raft-lite consensus → implement worker heartbeats.

### 🔴 Phase 6: 06-Embedded Query Engine

**Concepts:** B+ tree indexing, query parsing (`nom`), MVCC basics, WAL, memory arena allocation  
**Trade-off:** Steep curve; correctness > performance initially. Overkill unless targeting infra/data track.  
**BC/AB Signal:** High-leverage signal for Staff/Principal track; demonstrates architectural depth.  
**Takeaway:** Focus on property-based testing (`proptest`) first. Optimize after flamegraph validation.

---

## 🛠️ Standards

- **Linter:** `cargo clippy --all-targets -- -D warnings`
- **Formatter:** `cargo fmt --all`
- **Observability:** `tracing` integration for all 🟡 and 🔴 phases.
- **Safety:** Zero `unsafe` code unless strictly required for Phase 6 performance.

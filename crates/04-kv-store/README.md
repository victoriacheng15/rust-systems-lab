# 04 KV Store

`kv-store` is a WAL-backed key-value store with `set`, `get`, and `remove` commands. Each write is appended to a log so later invocations can rebuild the in-memory state.

## What It Demonstrates

- Concurrent state management with `DashMap`
- Binary serialization with `bincode`
- Write-ahead logging and replay
- Async file I/O with `tokio`
- CLI subcommands with `clap`

## Manual Usage

Run from the repository root:

```bash
cargo run -p kv-store -- --wal crates/04-kv-store/kv-demo.wal set user:1 alice
cargo run -p kv-store -- --wal crates/04-kv-store/kv-demo.wal get user:1
cargo run -p kv-store -- --wal crates/04-kv-store/kv-demo.wal remove user:1
cargo run -p kv-store -- --wal crates/04-kv-store/kv-demo.wal get user:1
```

The WAL file lets later commands replay earlier changes.

[Back to main README](../../README.md)

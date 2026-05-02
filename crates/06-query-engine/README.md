# 06 Query Engine

`query-engine` is a small in-memory database core for learning how SQL-style systems parse commands, store rows, maintain an index, and execute primary-key lookups or range scans.

## What It Demonstrates

- Query parsing with `nom`
- Arena-style row storage
- Primary-key indexing with sorted leaf pages
- Insert, select, and range scan execution
- REPL behavior for keeping process-local state alive

## Manual Usage

Run a single command from the repository root:

```bash
cargo run -p query-engine -- query "insert 1 'hello'"
cargo run -p query-engine -- query "select 1"
```

Each `query` run starts with an empty in-memory table. Use the REPL to keep one table alive across multiple commands:

```bash
cargo run -p query-engine -- repl
```

Inside the REPL:

```text
insert 1 'hello'
select 1
insert 5 'world'
scan 1..10
select 99
quit
```

[Back to main README](../../README.md)

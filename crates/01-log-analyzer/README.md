# 01 Log Analyzer

`log-analyzer` is a small CLI for processing newline-delimited JSON logs. It can read from a file or standard input, filter entries by log level, and print a summary of observed levels.

## What It Demonstrates

- CLI argument parsing with `clap`
- Borrowed JSON deserialization with `serde`
- Streaming line-by-line file and stdin processing
- Basic filtering and aggregation

## Manual Usage

Run from the repository root:

```bash
cargo run -p log-analyzer -- --file crates/01-log-analyzer/tests/sample.log
cargo run -p log-analyzer -- --file crates/01-log-analyzer/tests/sample.log --level ERROR
cat crates/01-log-analyzer/tests/sample.log | cargo run -p log-analyzer --
```

[Back to main README](../../README.md)

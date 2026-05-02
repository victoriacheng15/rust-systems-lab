# 03 Async Scraper

`async-scraper` is an asynchronous scraping pipeline that fetches an HTML page and an RSS feed, parses a few results, and prints them as a table. Work is coordinated through bounded channels and worker tasks.

## What It Demonstrates

- Async execution with `tokio`
- HTTP requests with `reqwest`
- Backpressure with bounded `mpsc` channels
- HTML and RSS parsing
- Runtime instrumentation with `tracing`

## Manual Usage

Run from the repository root:

```bash
cargo run -p async-scraper
```

The program fetches the configured targets, parses a few results, prints them as a table, and exits.

The manual run requires network access.

[Back to main README](../../README.md)

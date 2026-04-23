# 02 Mini HTTP

[Back to workspace README](../../README.md)

## Overview

`mini-http` is a synchronous HTTP server built with the Rust standard library. It accepts TCP connections, dispatches requests through a small thread pool, and returns simple HTTP responses.

## What It Demonstrates

- TCP networking with `std::net`
- Request handling with blocking I/O
- Thread pool construction with channels and worker threads
- Shared state with `Arc<Mutex<_>>`

## Manual Usage

Run from the repository root:

```bash
cargo run -p mini-http
```

In another terminal:

```bash
curl -i http://127.0.0.1:7878/
curl -i http://127.0.0.1:7878/sleep
curl -i http://127.0.0.1:7878/missing
```

Stop the server with `Ctrl-C`.

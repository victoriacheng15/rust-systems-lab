# 05 Task Queue

`task-queue` is a durable single-node queue engine. It supports idempotent enqueueing, worker leases, acknowledgement, failure, requeueing, queue stats, and WAL replay.

## What It Demonstrates

- Durable state transitions with a write-ahead log
- Idempotency keys for duplicate-safe enqueueing
- Worker leasing and retry-oriented queue behavior
- Replay-based recovery after process exit
- Operational CLI commands for queue inspection

## Manual Usage

Run from the repository root:

```bash
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal enqueue email-1 "send welcome email"
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal lease worker-a
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal stats
```

Use the printed `id=...` task id from `lease` for ack or fail commands:

```bash
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal ack TASK_ID
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal fail TASK_ID "temporary error" --requeue
cargo run -p task-queue -- --wal crates/05-task-queue/tasks-demo.wal get TASK_ID
```

[Back to main README](../../README.md)

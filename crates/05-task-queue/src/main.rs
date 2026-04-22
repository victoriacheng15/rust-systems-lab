use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum TaskState {
    Queued,
    Leased {
        worker_id: String,
        lease_expires_at_ms: u64,
    },
    Completed,
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Task {
    id: String,
    idempotency_key: String,
    payload: String,
    attempts: u32,
    created_at_ms: u64,
    updated_at_ms: u64,
    state: TaskState,
}

#[derive(Debug, Serialize, Deserialize)]
enum WalCommand {
    Enqueued(Task),
    Leased {
        task_id: String,
        worker_id: String,
        lease_expires_at_ms: u64,
        updated_at_ms: u64,
        attempts: u32,
    },
    Acked {
        task_id: String,
        updated_at_ms: u64,
    },
    Failed {
        task_id: String,
        reason: String,
        requeue: bool,
        updated_at_ms: u64,
    },
}

#[derive(Debug, Default)]
struct QueueStats {
    queued: usize,
    leased: usize,
    completed: usize,
    failed: usize,
}

struct TaskQueue {
    tasks: BTreeMap<String, Task>,
    idempotency_index: BTreeMap<String, String>,
    ready: VecDeque<String>,
    wal: File,
    sequence: u64,
}

#[derive(Parser)]
#[command(author, version, about = "Durable idempotent task queue", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the queue WAL file
    #[arg(short, long, default_value = "task-queue.wal")]
    wal: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a task, returning an existing task for duplicate idempotency keys
    Enqueue {
        idempotency_key: String,
        payload: String,
    },
    /// Lease the next available task for a worker
    Lease {
        worker_id: String,
        #[arg(long, default_value_t = 30_000)]
        lease_ms: u64,
    },
    /// Mark a leased task complete
    Ack { task_id: String },
    /// Mark a task failed, optionally putting it back on the queue
    Fail {
        task_id: String,
        reason: String,
        #[arg(long)]
        requeue: bool,
    },
    /// Print a task by id
    Get { task_id: String },
    /// Print queue state counts
    Stats,
}

impl TaskQueue {
    async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut queue = Self {
            tasks: BTreeMap::new(),
            idempotency_index: BTreeMap::new(),
            ready: VecDeque::new(),
            wal: OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .with_context(|| format!("opening WAL at {}", path.display()))?,
            sequence: 0,
        };

        if path.exists() {
            queue.replay(&path).await?;
        }

        Ok(queue)
    }

    async fn replay(&mut self, path: &Path) -> Result<()> {
        let mut file = File::open(path)
            .await
            .with_context(|| format!("reading WAL at {}", path.display()))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await?;

        let mut cursor = 0;
        while cursor < buffer.len() {
            if cursor + 8 > buffer.len() {
                warn!("Ignoring truncated WAL length header");
                break;
            }

            let len = u64::from_le_bytes(buffer[cursor..cursor + 8].try_into().unwrap()) as usize;
            cursor += 8;

            if cursor + len > buffer.len() {
                warn!("Ignoring incomplete WAL command at end of file");
                break;
            }

            let command = bincode::deserialize(&buffer[cursor..cursor + len])
                .context("deserializing WAL command")?;
            cursor += len;
            self.apply(command);
        }

        self.rebuild_ready_queue();
        Ok(())
    }

    async fn enqueue(&mut self, idempotency_key: String, payload: String) -> Result<Task> {
        if let Some(task_id) = self.idempotency_index.get(&idempotency_key) {
            return self
                .tasks
                .get(task_id)
                .cloned()
                .ok_or_else(|| anyhow!("idempotency index points to missing task"));
        }

        let now = now_ms();
        self.sequence += 1;
        let task = Task {
            id: format!("task-{}-{}", now, self.sequence),
            idempotency_key,
            payload,
            attempts: 0,
            created_at_ms: now,
            updated_at_ms: now,
            state: TaskState::Queued,
        };

        self.append(WalCommand::Enqueued(task.clone())).await?;
        self.apply(WalCommand::Enqueued(task.clone()));
        Ok(task)
    }

    async fn lease_next(&mut self, worker_id: String, lease_ms: u64) -> Result<Option<Task>> {
        self.requeue_expired_leases();

        while let Some(task_id) = self.ready.pop_front() {
            let Some(task) = self.tasks.get(&task_id) else {
                continue;
            };

            if task.state != TaskState::Queued {
                continue;
            }

            let updated_at_ms = now_ms();
            let attempts = task.attempts + 1;
            let lease_expires_at_ms = updated_at_ms.saturating_add(lease_ms);
            let command_worker_id = worker_id.clone();
            let command = WalCommand::Leased {
                task_id: task_id.clone(),
                worker_id: command_worker_id.clone(),
                lease_expires_at_ms,
                updated_at_ms,
                attempts,
            };
            self.append(command).await?;
            self.apply(WalCommand::Leased {
                task_id: task_id.clone(),
                worker_id: command_worker_id,
                lease_expires_at_ms,
                updated_at_ms,
                attempts,
            });
            return Ok(self.tasks.get(&task_id).cloned());
        }

        Ok(None)
    }

    async fn ack(&mut self, task_id: String) -> Result<Task> {
        let task = self
            .tasks
            .get(&task_id)
            .ok_or_else(|| anyhow!("unknown task id: {task_id}"))?;

        if !matches!(task.state, TaskState::Leased { .. }) {
            return Err(anyhow!("task must be leased before ack: {task_id}"));
        }

        let updated_at_ms = now_ms();
        let command = WalCommand::Acked {
            task_id: task_id.clone(),
            updated_at_ms,
        };
        self.append(command).await?;
        self.apply(WalCommand::Acked {
            task_id: task_id.clone(),
            updated_at_ms,
        });
        self.tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown task id after ack: {task_id}"))
    }

    async fn fail(&mut self, task_id: String, reason: String, requeue: bool) -> Result<Task> {
        let task = self
            .tasks
            .get(&task_id)
            .ok_or_else(|| anyhow!("unknown task id: {task_id}"))?;

        if !matches!(task.state, TaskState::Leased { .. }) {
            return Err(anyhow!("task must be leased before fail: {task_id}"));
        }

        let updated_at_ms = now_ms();
        let command = WalCommand::Failed {
            task_id: task_id.clone(),
            reason: reason.clone(),
            requeue,
            updated_at_ms,
        };
        self.append(command).await?;
        self.apply(WalCommand::Failed {
            task_id: task_id.clone(),
            reason,
            requeue,
            updated_at_ms,
        });
        self.tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown task id after fail: {task_id}"))
    }

    fn get(&self, task_id: &str) -> Option<Task> {
        self.tasks.get(task_id).cloned()
    }

    fn stats(&self) -> QueueStats {
        let mut stats = QueueStats::default();
        for task in self.tasks.values() {
            match task.state {
                TaskState::Queued => stats.queued += 1,
                TaskState::Leased { .. } => stats.leased += 1,
                TaskState::Completed => stats.completed += 1,
                TaskState::Failed { .. } => stats.failed += 1,
            }
        }
        stats
    }

    async fn append(&mut self, command: WalCommand) -> Result<()> {
        let encoded = bincode::serialize(&command).context("serializing WAL command")?;
        let len = encoded.len() as u64;
        self.wal.write_all(&len.to_le_bytes()).await?;
        self.wal.write_all(&encoded).await?;
        self.wal.flush().await?;
        Ok(())
    }

    fn apply(&mut self, command: WalCommand) {
        match command {
            WalCommand::Enqueued(task) => {
                self.sequence = self.sequence.max(parse_sequence(&task.id));
                self.idempotency_index
                    .insert(task.idempotency_key.clone(), task.id.clone());
                self.ready.push_back(task.id.clone());
                self.tasks.insert(task.id.clone(), task);
            }
            WalCommand::Leased {
                task_id,
                worker_id,
                lease_expires_at_ms,
                updated_at_ms,
                attempts,
            } => {
                if let Some(task) = self.tasks.get_mut(&task_id) {
                    task.state = TaskState::Leased {
                        worker_id,
                        lease_expires_at_ms,
                    };
                    task.updated_at_ms = updated_at_ms;
                    task.attempts = attempts;
                }
            }
            WalCommand::Acked {
                task_id,
                updated_at_ms,
            } => {
                if let Some(task) = self.tasks.get_mut(&task_id) {
                    task.state = TaskState::Completed;
                    task.updated_at_ms = updated_at_ms;
                }
            }
            WalCommand::Failed {
                task_id,
                reason,
                requeue,
                updated_at_ms,
            } => {
                if let Some(task) = self.tasks.get_mut(&task_id) {
                    task.updated_at_ms = updated_at_ms;
                    if requeue {
                        task.state = TaskState::Queued;
                        self.ready.push_back(task_id);
                    } else {
                        task.state = TaskState::Failed { reason };
                    }
                }
            }
        }
    }

    fn rebuild_ready_queue(&mut self) {
        self.ready = self
            .tasks
            .iter()
            .filter_map(|(task_id, task)| {
                (task.state == TaskState::Queued).then(|| task_id.clone())
            })
            .collect();
    }

    fn requeue_expired_leases(&mut self) {
        let now = now_ms();
        for (task_id, task) in &mut self.tasks {
            if let TaskState::Leased {
                lease_expires_at_ms,
                ..
            } = task.state
            {
                if lease_expires_at_ms <= now {
                    task.state = TaskState::Queued;
                    task.updated_at_ms = now;
                    self.ready.push_back(task_id.clone());
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let mut queue = TaskQueue::open(&cli.wal).await?;

    match cli.command {
        Commands::Enqueue {
            idempotency_key,
            payload,
        } => {
            let task = queue.enqueue(idempotency_key, payload).await?;
            info!("enqueued task {}", task.id);
            print_task(&task);
        }
        Commands::Lease {
            worker_id,
            lease_ms,
        } => match queue.lease_next(worker_id, lease_ms).await? {
            Some(task) => print_task(&task),
            None => {
                println!("No queued tasks available");
            }
        },
        Commands::Ack { task_id } => {
            let task = queue.ack(task_id).await?;
            print_task(&task);
        }
        Commands::Fail {
            task_id,
            reason,
            requeue,
        } => {
            let task = queue.fail(task_id, reason, requeue).await?;
            print_task(&task);
        }
        Commands::Get { task_id } => match queue.get(&task_id) {
            Some(task) => print_task(&task),
            None => return Err(anyhow!("unknown task id: {task_id}")),
        },
        Commands::Stats => {
            let stats = queue.stats();
            println!(
                "queued={} leased={} completed={} failed={}",
                stats.queued, stats.leased, stats.completed, stats.failed
            );
        }
    }

    Ok(())
}

fn print_task(task: &Task) {
    println!(
        "id={} key={} attempts={} state={:?} payload={}",
        task.id, task.idempotency_key, task.attempts, task.state, task.payload
    );
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn parse_sequence(task_id: &str) -> u64 {
    task_id
        .rsplit('-')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enqueue_is_idempotent() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let wal = dir.path().join("queue.wal");
        let mut queue = TaskQueue::open(&wal).await?;

        let first = queue
            .enqueue("invoice-1".to_string(), "send receipt".to_string())
            .await?;
        let duplicate = queue
            .enqueue("invoice-1".to_string(), "different payload".to_string())
            .await?;

        assert_eq!(first.id, duplicate.id);
        assert_eq!(first.payload, duplicate.payload);
        assert_eq!(queue.stats().queued, 1);
        Ok(())
    }

    #[tokio::test]
    async fn lease_ack_and_replay_preserve_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let wal = dir.path().join("queue.wal");
        let task_id = {
            let mut queue = TaskQueue::open(&wal).await?;
            let task = queue
                .enqueue("email-1".to_string(), "send welcome email".to_string())
                .await?;
            let leased = queue
                .lease_next("worker-a".to_string(), 60_000)
                .await?
                .expect("task should lease");

            assert_eq!(task.id, leased.id);
            assert_eq!(leased.attempts, 1);
            queue.ack(leased.id.clone()).await?;
            leased.id
        };

        let queue = TaskQueue::open(&wal).await?;
        let task = queue.get(&task_id).expect("task should replay");
        assert_eq!(task.state, TaskState::Completed);
        assert_eq!(queue.stats().completed, 1);
        Ok(())
    }

    #[tokio::test]
    async fn failed_task_can_be_requeued() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let wal = dir.path().join("queue.wal");
        let mut queue = TaskQueue::open(&wal).await?;
        let task = queue
            .enqueue("job-1".to_string(), "index page".to_string())
            .await?;
        let leased = queue
            .lease_next("worker-a".to_string(), 60_000)
            .await?
            .expect("task should lease");

        queue
            .fail(
                leased.id.clone(),
                "temporary backend error".to_string(),
                true,
            )
            .await?;
        let leased = queue
            .lease_next("worker-b".to_string(), 60_000)
            .await?
            .expect("requeued task should lease");

        assert_eq!(leased.id, task.id);
        assert_eq!(leased.attempts, 2);
        Ok(())
    }
}

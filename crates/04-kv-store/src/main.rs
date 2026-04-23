use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};

// --- Data Types ---

#[derive(Debug, Serialize, Deserialize)]
enum Command {
    Set(String, String),
    Remove(String),
}

pub struct KvStore {
    map: DashMap<String, String>,
    wal: File,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the WAL file
    #[arg(short, long, default_value = "kv.wal")]
    wal: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Set a value in the store
    Set { key: String, value: String },
    /// Get a value from the store
    Get { key: String },
    /// Remove a value from the store
    Remove { key: String },
}

// --- Implementation ---

impl KvStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let wal_path = path.as_ref().to_path_buf();
        let map = DashMap::new();

        if wal_path.exists() {
            info!("Replaying WAL from {:?}", wal_path);
            let mut file = File::open(&wal_path).await?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer).await?;

            let mut cursor = 0;
            while cursor < buffer.len() {
                if cursor + 8 > buffer.len() {
                    break;
                }
                let len =
                    u64::from_le_bytes(buffer[cursor..cursor + 8].try_into().unwrap()) as usize;
                cursor += 8;

                if cursor + len > buffer.len() {
                    warn!("Incomplete WAL command at end of file");
                    break;
                }

                let command: Command = bincode::deserialize(&buffer[cursor..cursor + len])
                    .context("Failed to deserialize WAL entry")?;
                cursor += len;

                match command {
                    Command::Set(k, v) => {
                        map.insert(k, v);
                    }
                    Command::Remove(k) => {
                        map.remove(&k);
                    }
                }
            }
        }

        let wal = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)
            .await?;

        Ok(Self { map, wal })
    }

    pub async fn set(&self, key: String, value: String) -> Result<()> {
        let command = Command::Set(key.clone(), value.clone());
        self.append_to_wal(&command).await?;
        self.map.insert(key, value);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).map(|v| v.value().clone())
    }

    pub async fn remove(&self, key: String) -> Result<()> {
        if self.map.contains_key(&key) {
            let command = Command::Remove(key.clone());
            self.append_to_wal(&command).await?;
            self.map.remove(&key);
        }
        Ok(())
    }

    async fn append_to_wal(&self, command: &Command) -> Result<()> {
        let mut wal = self.wal.try_clone().await?;
        let encoded = bincode::serialize(command).context("Failed to serialize command")?;
        let len = encoded.len() as u64;

        wal.write_all(&len.to_le_bytes()).await?;
        wal.write_all(&encoded).await?;
        wal.flush().await?;
        Ok(())
    }
}

// --- CLI Entrypoint ---

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let store = KvStore::open(&cli.wal).await?;

    match cli.command {
        Commands::Set { key, value } => {
            info!("Setting key '{}' to '{}'", key, value);
            store.set(key, value).await?;
        }
        Commands::Get { key } => {
            if let Some(val) = store.get(&key) {
                println!("{}", val);
            } else {
                eprintln!("Key '{}' not found", key);
                std::process::exit(1);
            }
        }
        Commands::Remove { key } => {
            info!("Removing key '{}'", key);
            store.remove(key).await?;
        }
    }

    Ok(())
}

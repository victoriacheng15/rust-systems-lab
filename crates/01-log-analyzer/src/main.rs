use clap::Parser;
use serde::Deserialize;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;

/// A simple log analyzer for Phase 1.
/// Parses JSON logs and provides basic filtering and summary.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the log file (reads from stdin if not provided)
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Filter by log level (e.g., INFO, ERROR, WARN)
    #[arg(short, long)]
    level: Option<String>,
}

#[derive(Deserialize, Debug)]
struct LogEntry<'a> {
    #[serde(borrow)]
    timestamp: &'a str,
    #[serde(borrow)]
    level: &'a str,
    #[serde(borrow)]
    message: &'a str,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let reader: Box<dyn BufRead> = match args.file {
        Some(path) => {
            let file = File::open(path)?;
            Box::new(BufReader::new(file))
        }
        None => Box::new(BufReader::new(io::stdin())),
    };

    let mut info_count = 0;
    let mut warn_count = 0;
    let mut error_count = 0;

    println!("{:<25} | {:<7} | {}", "Timestamp", "Level", "Message");
    println!("{}", "-".repeat(60));

    for (line_num, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading line {}: {}", line_num + 1, e);
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<LogEntry>(&line) {
            Ok(entry) => {
                // Count levels regardless of filter
                match entry.level.to_uppercase().as_str() {
                    "INFO" => info_count += 1,
                    "WARN" => warn_count += 1,
                    "ERROR" => error_count += 1,
                    _ => {}
                }

                // Apply filter if specified
                if let Some(ref target_level) = args.level {
                    if !entry.level.eq_ignore_ascii_case(target_level) {
                        continue;
                    }
                }

                println!(
                    "{:<25} | {:<7} | {}",
                    entry.timestamp, entry.level, entry.message
                );
            }
            Err(e) => {
                eprintln!(
                    "Malformed JSON at line {}: {} -> \"{}\"",
                    line_num + 1,
                    e,
                    line
                );
            }
        }
    }

    println!("\nSummary:");
    println!("INFO:  {}", info_count);
    println!("WARN:  {}", warn_count);
    println!("ERROR: {}", error_count);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_log_entry() {
        let json = r#"{"timestamp": "2024-04-06T10:00:00Z", "level": "INFO", "message": "System started"}"#;
        let entry: LogEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.message, "System started");
    }
}

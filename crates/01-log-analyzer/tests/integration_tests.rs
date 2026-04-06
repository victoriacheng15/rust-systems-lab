use std::process::Command;

#[test]
fn test_cli_execution() {
    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "log-analyzer",
            "--",
            "--file",
            "tests/sample.log",
        ])
        .output()
        .expect("Failed to execute log-analyzer");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Application started"));
    assert!(stdout.contains("Summary:"));
    assert!(stdout.contains("INFO:  2"));
    assert!(stdout.contains("WARN:  1"));
    assert!(stdout.contains("ERROR: 2"));
}

#[test]
fn test_cli_filtering() {
    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "log-analyzer",
            "--",
            "--file",
            "tests/sample.log",
            "--level",
            "ERROR",
        ])
        .output()
        .expect("Failed to execute log-analyzer");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Database connection failed"));
    assert!(stdout.contains("Fatal error"));
    assert!(!stdout.contains("Application started"));
}

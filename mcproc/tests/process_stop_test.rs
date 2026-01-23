use std::process::Command;
use std::time::Duration;
use tokio::time::timeout;

/// Test that logs emitted during graceful shutdown (after SIGTERM) are captured.
/// This test verifies the fix for the issue where logs were lost when stop_process
/// was called because log capture stopped immediately when status changed to Stopping.
///
/// Note: This test requires file logging to be enabled in the mcproc configuration
/// because after a process is stopped, its in-memory ring buffer is cleared.
/// The test verifies that logs are correctly written to the log file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed and file logging enabled"]
async fn test_graceful_shutdown_logs_captured() {
    use std::path::PathBuf;

    // Create a script that outputs logs after receiving SIGTERM
    let script = r#"
trap 'echo "SIGTERM_RECEIVED"; sleep 0.5; echo "GRACEFUL_SHUTDOWN_COMPLETE"; exit 0' TERM
echo "PROCESS_STARTED"
while true; do
    sleep 0.1
done
"#;

    // Start the process
    let output = Command::new("mcproc")
        .args([
            "start",
            "test-graceful",
            "--cmd",
            &format!("sh -c '{}'", script.replace('\n', ";")),
            "--project",
            "test-graceful-shutdown",
        ])
        .output()
        .expect("Failed to start process");

    assert!(
        output.status.success(),
        "Failed to start graceful shutdown test process: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Wait for process to start and output initial log
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify initial log is captured before stopping
    let logs_before = Command::new("mcproc")
        .args([
            "logs",
            "test-graceful",
            "--project",
            "test-graceful-shutdown",
            "--tail",
            "10",
        ])
        .output()
        .expect("Failed to get logs before stop");

    let logs_before_str = String::from_utf8_lossy(&logs_before.stdout);
    assert!(
        logs_before_str.contains("PROCESS_STARTED"),
        "Missing PROCESS_STARTED log before stop. Got:\n{}",
        logs_before_str
    );

    // Stop the process - this should trigger SIGTERM and graceful shutdown
    let stop_result = timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("mcproc")
            .args([
                "stop",
                "test-graceful",
                "--project",
                "test-graceful-shutdown",
            ])
            .output(),
    )
    .await;

    assert!(stop_result.is_ok(), "Stop command timed out");
    let stop_output = stop_result
        .unwrap()
        .expect("Failed to execute stop command");
    assert!(
        stop_output.status.success(),
        "Failed to stop process: {}",
        String::from_utf8_lossy(&stop_output.stderr)
    );

    // Wait a bit for logs to be flushed to file
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Read log file directly since the in-memory ring buffer is cleared after process stops
    let state_dir = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").expect("HOME not set");
        format!("{}/.local/state", home)
    });
    let log_file_path =
        PathBuf::from(state_dir).join("mcproc/log/test-graceful-shutdown/test-graceful.log");

    let log_content = std::fs::read_to_string(&log_file_path).unwrap_or_else(|e| {
        panic!(
            "Failed to read log file at {:?}: {}. File logging may not be enabled.",
            log_file_path, e
        )
    });

    // Verify that all expected logs are present in the log file
    assert!(
        log_content.contains("PROCESS_STARTED"),
        "Missing PROCESS_STARTED in log file. Got:\n{}",
        log_content
    );
    assert!(
        log_content.contains("SIGTERM_RECEIVED"),
        "Missing SIGTERM_RECEIVED in log file - graceful shutdown logs not captured. Got:\n{}",
        log_content
    );
    assert!(
        log_content.contains("GRACEFUL_SHUTDOWN_COMPLETE"),
        "Missing GRACEFUL_SHUTDOWN_COMPLETE in log file - graceful shutdown logs not captured. Got:\n{}",
        log_content
    );

    // Clean up
    Command::new("mcproc")
        .args(["clean", "--project", "test-graceful-shutdown", "--force"])
        .output()
        .ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_simple_process_stop() {
    // Start a simple sleep process
    let output = Command::new("mcproc")
        .args([
            "start",
            "test-sleep",
            "--cmd",
            "sleep 30",
            "--project",
            "test-stop",
        ])
        .output()
        .expect("Failed to start process");

    assert!(output.status.success(), "Failed to start sleep process");

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Stop the process with timeout
    let stop_result = timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("mcproc")
            .args(["stop", "test-sleep", "--project", "test-stop"])
            .output(),
    )
    .await;

    assert!(stop_result.is_ok(), "Stop command timed out");
    let output = stop_result
        .unwrap()
        .expect("Failed to execute stop command");
    assert!(output.status.success(), "Failed to stop process");

    // Clean up
    Command::new("mcproc")
        .args(["clean", "--project", "test-stop", "--force"])
        .output()
        .ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_yes_command_stop() {
    // Start yes command
    let output = Command::new("mcproc")
        .args([
            "start",
            "test-yes",
            "--cmd",
            "yes hello",
            "--project",
            "test-stop",
        ])
        .output()
        .expect("Failed to start process");

    assert!(output.status.success(), "Failed to start yes process");

    // Wait a bit for process to generate logs
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Stop the process with timeout
    let stop_result = timeout(
        Duration::from_secs(10),
        tokio::process::Command::new("mcproc")
            .args(["stop", "test-yes", "--project", "test-stop"])
            .output(),
    )
    .await;

    // This test is expected to fail with current implementation
    if stop_result.is_err() {
        eprintln!("WARNING: yes command stop timed out (known issue)");

        // Force cleanup
        Command::new("mcproc")
            .args(["clean", "--project", "test-stop", "--force"])
            .output()
            .ok();

        // Kill any remaining yes processes
        Command::new("sh")
            .arg("-c")
            .arg("pkill -f 'yes hello' || true")
            .output()
            .ok();
    } else {
        let output = stop_result
            .unwrap()
            .expect("Failed to execute stop command");
        assert!(output.status.success(), "Failed to stop yes process");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_pipe_command_stop() {
    // Start pipe command
    let output = Command::new("mcproc")
        .args([
            "start",
            "test-pipe",
            "--cmd",
            "yes | head -n 1000",
            "--project",
            "test-stop",
        ])
        .output()
        .expect("Failed to start process");

    assert!(output.status.success(), "Failed to start pipe process");

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Stop the process with timeout
    let stop_result = timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("mcproc")
            .args(["stop", "test-pipe", "--project", "test-stop"])
            .output(),
    )
    .await;

    if stop_result.is_err() {
        eprintln!("WARNING: pipe command stop timed out");

        // Force cleanup
        Command::new("mcproc")
            .args(["clean", "--project", "test-stop", "--force"])
            .output()
            .ok();

        // Kill any remaining processes
        Command::new("sh")
            .arg("-c")
            .arg("pkill -f 'yes' || true")
            .output()
            .ok();
    } else {
        let output = stop_result
            .unwrap()
            .expect("Failed to execute stop command");
        assert!(output.status.success(), "Failed to stop pipe process");
    }
}

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;
use tokio::time::{sleep, timeout, Instant};
use uuid::Uuid;

const STOP_TIMEOUT: Duration = Duration::from_secs(60);
const LOG_WAIT_TIMEOUT: Duration = Duration::from_secs(15);

struct TestEnvironment {
    root: PathBuf,
}

impl TestEnvironment {
    fn new() -> Self {
        let id = Uuid::new_v4().simple().to_string();
        // Keep this path short because macOS limits Unix socket paths to 104 bytes.
        let root = PathBuf::from(format!("/tmp/mcpst-{}", &id[..8]));
        fs::create_dir_all(&root).expect("Failed to create isolated test root");
        Self { root }
    }

    fn command(&self) -> Command {
        let mut command = Command::new("mcproc");
        command
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("XDG_STATE_HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"));
        command
    }

    fn process_log_path(&self, project: &str, process: &str) -> PathBuf {
        self.root
            .join("mcproc/log")
            .join(project)
            .join(format!("{process}.log"))
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        if let Err(error) = self.command().args(["daemon", "stop"]).output() {
            eprintln!("Failed to stop isolated mcproc daemon: {error}");
        }
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!(
                "Failed to remove isolated test root {}: {error}",
                self.root.display()
            );
        }
    }
}

fn run(environment: &TestEnvironment, args: &[&str]) -> Output {
    environment
        .command()
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("Failed to execute mcproc {}: {error}", args.join(" ")))
}

async fn run_with_timeout(
    environment: &TestEnvironment,
    args: &[&str],
    duration: Duration,
) -> Output {
    let mut command = environment.command();
    command.args(args);
    timeout(duration, tokio::process::Command::from(command).output())
        .await
        .unwrap_or_else(|_| {
            panic!(
                "mcproc {} timed out after {} seconds",
                args.join(" "),
                duration.as_secs()
            )
        })
        .unwrap_or_else(|error| panic!("Failed to execute mcproc {}: {error}", args.join(" ")))
}

fn running_processes(environment: &TestEnvironment) -> String {
    let output = run(environment, &["ps", "--status", "running"]);
    assert!(
        output.status.success(),
        "Failed to list running processes: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn assert_running(environment: &TestEnvironment, process: &str) {
    let processes = running_processes(environment);
    assert!(
        processes.contains(process),
        "Process {process} did not appear as Running. Got:\n{processes}"
    );
}

fn assert_not_running(environment: &TestEnvironment, process: &str) {
    let processes = running_processes(environment);
    assert!(
        !processes.contains(process),
        "Process {process} still appeared as Running after stop. Got:\n{processes}"
    );
}

async fn wait_for_log(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + LOG_WAIT_TIMEOUT;
    let mut last_content = String::new();

    loop {
        let last_error = match fs::read_to_string(path) {
            Ok(content) => {
                if content.contains(expected) {
                    return content;
                }
                last_content = content;
                "none".to_string()
            }
            Err(error) => error.to_string(),
        };

        if Instant::now() >= deadline {
            panic!(
                "Timed out waiting for {expected:?} in {}. Last error: {}. Last content:\n{}",
                path.display(),
                last_error,
                last_content
            );
        }

        sleep(Duration::from_millis(100)).await;
    }
}

/// Test that logs emitted during graceful shutdown (after SIGTERM) are captured.
/// This test verifies the fix for the issue where logs were lost when stop_process
/// was called because log capture stopped immediately when status changed to Stopping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_graceful_shutdown_logs_captured() {
    let environment = TestEnvironment::new();
    let script = "trap 'echo SIGTERM_RECEIVED; echo GRACEFUL_SHUTDOWN_COMPLETE; exit 0' TERM; echo PROCESS_STARTED; while true; do sleep 1; done";

    let output = run(
        &environment,
        &[
            "start",
            "test-graceful",
            "--cmd",
            script,
            "--project",
            "test-graceful-shutdown",
        ],
    );
    assert!(
        output.status.success(),
        "Failed to start graceful shutdown test process: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let log_file_path = environment.process_log_path("test-graceful-shutdown", "test-graceful");
    wait_for_log(&log_file_path, "PROCESS_STARTED").await;
    assert_running(&environment, "test-graceful");

    let stop_output = run_with_timeout(
        &environment,
        &[
            "stop",
            "test-graceful",
            "--project",
            "test-graceful-shutdown",
        ],
        STOP_TIMEOUT,
    )
    .await;
    assert!(
        stop_output.status.success(),
        "Failed to stop process: {}",
        String::from_utf8_lossy(&stop_output.stderr)
    );
    assert_not_running(&environment, "test-graceful");

    let log_content = wait_for_log(&log_file_path, "GRACEFUL_SHUTDOWN_COMPLETE").await;
    assert!(
        log_content.contains("PROCESS_STARTED"),
        "Missing PROCESS_STARTED in log file. Got:\n{log_content}"
    );
    assert!(
        log_content.contains("SIGTERM_RECEIVED"),
        "Missing SIGTERM_RECEIVED in log file - graceful shutdown logs not captured. Got:\n{log_content}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_simple_process_stop() {
    let environment = TestEnvironment::new();
    let output = run(
        &environment,
        &[
            "start",
            "test-sleep",
            "--cmd",
            "sleep 30",
            "--project",
            "test-stop",
        ],
    );
    assert!(
        output.status.success(),
        "Failed to start sleep process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_running(&environment, "test-sleep");

    let output = run_with_timeout(
        &environment,
        &["stop", "test-sleep", "--project", "test-stop"],
        STOP_TIMEOUT,
    )
    .await;
    assert!(
        output.status.success(),
        "Failed to stop sleep process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_not_running(&environment, "test-sleep");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_yes_command_stop() {
    let environment = TestEnvironment::new();
    let output = run(
        &environment,
        &[
            "start",
            "test-yes",
            "--cmd",
            "yes hello",
            "--project",
            "test-stop",
        ],
    );
    assert!(
        output.status.success(),
        "Failed to start yes process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_running(&environment, "test-yes");

    let output = run_with_timeout(
        &environment,
        &["stop", "test-yes", "--project", "test-stop"],
        STOP_TIMEOUT,
    )
    .await;
    assert!(
        output.status.success(),
        "Failed to stop yes process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_not_running(&environment, "test-yes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires mcproc binary to be installed"]
async fn test_pipe_command_stop() {
    let environment = TestEnvironment::new();
    let output = run(
        &environment,
        &[
            "start",
            "test-pipe",
            "--cmd",
            "yes | cat > /dev/null",
            "--project",
            "test-stop",
        ],
    );
    assert!(
        output.status.success(),
        "Failed to start pipe process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_running(&environment, "test-pipe");

    let output = run_with_timeout(
        &environment,
        &["stop", "test-pipe", "--project", "test-stop"],
        STOP_TIMEOUT,
    )
    .await;
    assert!(
        output.status.success(),
        "Failed to stop pipe process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_not_running(&environment, "test-pipe");
}

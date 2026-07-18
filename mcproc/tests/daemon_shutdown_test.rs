//! Reproduces the bug where daemon shutdown unconditionally sleeps for
//! daemon_shutdown_timeout_ms even when there are no managed processes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct TestEnvironment {
    base: PathBuf,
}

impl TestEnvironment {
    fn new(base: PathBuf) -> Self {
        Self { base }
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let pid_file = self.base.join("rt/mcproc/mcprocd.pid");
        if let Ok(pid) = fs::read_to_string(pid_file) {
            if let Ok(pid) = pid.trim().parse::<u32>() {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
        }

        let _ = fs::remove_dir_all(&self.base);
    }
}

fn mcproc_command(base: &Path) -> Command {
    let mut command = Command::new("mcproc");
    command
        .env("XDG_RUNTIME_DIR", base.join("rt"))
        .env("XDG_STATE_HOME", base.join("state"))
        .env("XDG_CONFIG_HOME", base.join("config"))
        .env("XDG_DATA_HOME", base.join("data"));
    command
}

#[test]
#[ignore = "requires mcproc binary in PATH"]
fn daemon_stop_with_no_processes_is_fast() {
    // Keep this path short because macOS limits Unix socket paths to 104 bytes.
    let base = PathBuf::from(format!("/tmp/mcproc-shdt-{}", std::process::id()));
    let _environment = TestEnvironment::new(base.clone());

    // "state/mcproc/log" is pre-created as a workaround for issue #37
    // (daemon start fails when the state log directory is missing).
    for directory in ["rt", "state/mcproc/log", "config", "data"] {
        fs::create_dir_all(base.join(directory))
            .unwrap_or_else(|error| panic!("failed to create {directory} directory: {error}"));
    }

    let start_output = mcproc_command(&base)
        .args(["daemon", "start"])
        .output()
        .expect("failed to execute mcproc daemon start");
    assert!(
        start_output.status.success(),
        "mcproc daemon start failed: {}",
        String::from_utf8_lossy(&start_output.stderr)
    );

    let started_at = Instant::now();
    let stop_output = mcproc_command(&base)
        .args(["daemon", "stop"])
        .output()
        .expect("failed to execute mcproc daemon stop");
    assert!(
        stop_output.status.success(),
        "mcproc daemon stop failed: {}",
        String::from_utf8_lossy(&stop_output.stderr)
    );
    let elapsed = started_at.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "mcproc daemon stop took {:.3} seconds with no managed processes",
        elapsed.as_secs_f64()
    );
}

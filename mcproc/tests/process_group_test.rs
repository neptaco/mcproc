#![cfg(unix)]

use mcproc::daemon::process::launcher::{
    CreateProxyInfoParams, LaunchProcessParams, ProcessLauncher,
};
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::{getpgid, Pid};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{sleep, timeout, Instant};

struct ProcessCleanup {
    pids: Vec<Pid>,
}

impl ProcessCleanup {
    fn new(pid: Pid) -> Self {
        Self { pids: vec![pid] }
    }

    fn add(&mut self, pid: Pid) {
        self.pids.push(pid);
    }

    fn remove(&mut self, pid: Pid) {
        self.pids.retain(|candidate| *candidate != pid);
    }
}

impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        for pid in &self.pids {
            let _ = kill(*pid, Signal::SIGKILL);
        }
    }
}

fn launch_params(name: &str, project: &str, cmd: &str) -> LaunchProcessParams {
    LaunchProcessParams {
        name: name.to_string(),
        project: project.to_string(),
        cmd: Some(cmd.to_string()),
        args: Vec::new(),
        cwd: None,
        env: None,
        toolchain: None,
    }
}

fn child_pid(child: &tokio::process::Child) -> Pid {
    let pid = child.id().expect("spawned process should have a PID");
    Pid::from_raw(i32::try_from(pid).expect("child PID should fit in i32"))
}

#[tokio::test]
async fn test_spawned_process_gets_own_process_group() {
    let launcher = ProcessLauncher::new();
    let test_id = std::process::id();
    let (mut child, _) = launcher
        .launch_process(launch_params(
            &format!("process-group-leader-{test_id}"),
            &format!("process-group-test-{test_id}"),
            "sleep 30",
        ))
        .await
        .expect("failed to launch sleep process");

    let pid = child_pid(&child);
    let mut cleanup = ProcessCleanup::new(pid);
    let child_pgid = getpgid(Some(pid)).expect("failed to get child process group");
    let test_pgid = getpgid(None).expect("failed to get test process group");

    // Capture process-group values before cleanup so assertions test the launch state.
    let _ = kill(pid, Signal::SIGKILL);
    if timeout(Duration::from_secs(1), child.wait()).await.is_ok() {
        cleanup.remove(pid);
    }

    assert_ne!(
        child_pgid, test_pgid,
        "spawned process should not share the test process group"
    );
    assert_eq!(
        child_pgid, pid,
        "spawned process should be its process-group leader"
    );
}

#[tokio::test]
async fn test_stop_kills_orphaned_grandchild() {
    const COMMAND: &str = r#"(sleep 300 & echo "GRANDCHILD_PID=$!"); sleep 300"#;

    let launcher = ProcessLauncher::new();
    let test_id = std::process::id();
    let name = format!("orphan-grandchild-{test_id}");
    let project = format!("orphan-grandchild-test-{test_id}");
    let (mut child, _) = launcher
        .launch_process(launch_params(&name, &project, COMMAND))
        .await
        .expect("failed to launch orphan-grandchild command");

    let pid = child_pid(&child);
    let mut cleanup = ProcessCleanup::new(pid);
    let stdout = child.stdout.take().expect("child stdout should be piped");
    let mut lines = BufReader::new(stdout).lines();

    let grandchild_pid = timeout(Duration::from_secs(5), async {
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(raw_pid) = line.strip_prefix("GRANDCHILD_PID=") {
                        let parsed = raw_pid
                            .trim()
                            .parse::<i32>()
                            .expect("grandchild PID should be an integer");
                        break Pid::from_raw(parsed);
                    }
                }
                Ok(None) => panic!("child stdout closed before reporting grandchild PID"),
                Err(error) => panic!("failed to read child stdout: {error}"),
            }
        }
    })
    .await
    .expect("timed out waiting for grandchild PID");
    cleanup.add(grandchild_pid);

    assert_eq!(
        kill(grandchild_pid, None),
        Ok(()),
        "grandchild should be alive before stop"
    );

    let proxy = launcher.create_proxy_info(CreateProxyInfoParams {
        name,
        project,
        cmd: Some(COMMAND.to_string()),
        args: Vec::new(),
        cwd: None,
        env: None,
        wait_for_log: None,
        wait_timeout: None,
        toolchain: None,
        pid: u32::try_from(pid.as_raw()).expect("child PID should be positive"),
    });

    proxy
        .stop(false, 2_000)
        .await
        .expect("failed to stop process");

    if timeout(Duration::from_secs(1), child.wait()).await.is_ok() {
        cleanup.remove(pid);
    }

    let deadline = Instant::now() + Duration::from_secs(3);
    let grandchild_exited = loop {
        match kill(grandchild_pid, None) {
            Err(Errno::ESRCH) => break true,
            _ if Instant::now() >= deadline => break false,
            _ => sleep(Duration::from_millis(50)).await,
        }
    };

    if grandchild_exited {
        cleanup.remove(grandchild_pid);
    } else {
        // Ensure the Red test cannot leave the orphaned process running.
        let _ = kill(grandchild_pid, Signal::SIGKILL);
    }

    assert!(
        grandchild_exited,
        "stop should terminate the orphaned grandchild process"
    );
}

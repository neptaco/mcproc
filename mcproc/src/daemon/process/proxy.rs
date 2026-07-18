use crate::common::process_key::ProcessKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

#[cfg(unix)]
use nix::{
    sys::signal::{killpg, Signal},
    unistd::Pid,
};

/// Process lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProcessStatus {
    /// Process is being spawned
    Starting = 1,
    /// Process is active and healthy
    Running = 2,
    /// SIGTERM sent, waiting for graceful shutdown
    Stopping = 3,
    /// Process has exited normally
    Stopped = 4,
    /// Process exited with error
    Failed = 5,
}

impl From<u8> for ProcessStatus {
    fn from(value: u8) -> Self {
        match value {
            1 => ProcessStatus::Starting,
            2 => ProcessStatus::Running,
            3 => ProcessStatus::Stopping,
            4 => ProcessStatus::Stopped,
            5 => ProcessStatus::Failed,
            _ => ProcessStatus::Stopped,
        }
    }
}

impl From<ProcessStatus> for proto::ProcessStatus {
    fn from(status: ProcessStatus) -> Self {
        // Safe because the numeric values are identical
        match status {
            ProcessStatus::Starting => proto::ProcessStatus::Starting,
            ProcessStatus::Running => proto::ProcessStatus::Running,
            ProcessStatus::Stopping => proto::ProcessStatus::Stopping,
            ProcessStatus::Stopped => proto::ProcessStatus::Stopped,
            ProcessStatus::Failed => proto::ProcessStatus::Failed,
        }
    }
}

/// Metadata kept per managed process
///
/// This structure contains all the information about a process managed by mcprocd.
/// The global registry is a `DashMap<ProcessKey, Arc<ProxyInfo>>` for concurrent access.
pub struct ProxyInfo {
    /// Unique identifier (UUID)
    pub id: String,
    /// Composite key (project, name)
    #[allow(dead_code)]
    pub key: ProcessKey,
    /// Process name (must be unique within project)
    pub name: String,
    /// Project name for organization
    pub project: String,
    /// Shell command to execute (mutually exclusive with args)
    pub cmd: Option<String>,
    /// Direct command arguments (mutually exclusive with cmd)
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables
    pub env: Option<std::collections::HashMap<String, String>>,
    /// Regex pattern to wait for in logs before considering process ready
    pub wait_for_log: Option<String>,
    /// Timeout in seconds for wait_for_log pattern
    pub wait_timeout: Option<u32>,
    /// Version management tool (mise, asdf, nvm, etc.)
    pub toolchain: Option<String>,
    /// Process start time
    pub start_time: DateTime<Utc>,
    /// Current process status (atomic for thread-safe updates)
    pub status: Arc<AtomicU8>,
    /// Process ID
    pub pid: u32,
    /// Configured port (if any)
    pub port: Option<u16>,
    /// Detected port from process output
    pub detected_port: Arc<Mutex<Option<u16>>>,
    /// Exit code when process terminates
    pub exit_code: Arc<Mutex<Option<i32>>>,
    /// Time when process exited
    pub exit_time: Arc<Mutex<Option<DateTime<Utc>>>>,
    /// Hyperlog task handles for cleanup
    pub hyperlog_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl ProxyInfo {
    pub fn new(params: crate::daemon::process::types::ProxyInfoParams) -> Self {
        let key = ProcessKey::new(params.project.clone(), params.name.clone());
        Self {
            id: params.id,
            key,
            name: params.name,
            project: params.project,
            cmd: params.cmd,
            args: params.args,
            cwd: params.cwd,
            env: params.env,
            wait_for_log: params.wait_for_log,
            wait_timeout: params.wait_timeout,
            toolchain: params.toolchain,
            start_time: Utc::now(),
            status: Arc::new(AtomicU8::new(ProcessStatus::Starting as u8)),
            pid: params.pid,
            port: None,
            detected_port: Arc::new(Mutex::new(None)),
            exit_code: Arc::new(Mutex::new(None)),
            exit_time: Arc::new(Mutex::new(None)),
            hyperlog_handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn get_status(&self) -> ProcessStatus {
        ProcessStatus::from(self.status.load(Ordering::Relaxed))
    }

    pub fn set_status(&self, status: ProcessStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn get_key(&self) -> &ProcessKey {
        &self.key
    }

    pub async fn stop(&self, force: bool, process_stop_timeout_ms: u64) -> Result<(), String> {
        info!(
            "Stopping process {} (PID: {}, force: {})",
            self.name, self.pid, force
        );
        self.set_status(ProcessStatus::Stopping);

        // NOTE: We do NOT cancel tasks here. Cancelling the monitor task
        // would drop the Child object, closing stdout/stderr pipes and causing
        // group members to receive SIGPIPE instead of our group signal.

        // First attempt: Send SIGTERM (unless force is specified)
        if !force {
            info!(
                "Sending SIGTERM to process group {} (PGID: {})",
                self.name, self.pid
            );

            #[cfg(unix)]
            {
                match Self::send_signal_to_group(self.pid, Signal::SIGTERM) {
                    Ok(()) => {
                        info!("SIGTERM sent successfully to PGID {}", self.pid);
                    }
                    Err(e) => {
                        info!("Failed to send SIGTERM to PGID {}: {}", self.pid, e);
                        // If the process group doesn't exist, mark as stopped
                        if !Self::is_process_group_alive(self.pid) {
                            self.set_status(ProcessStatus::Stopped);
                            if let Ok(mut exit_time) = self.exit_time.lock() {
                                *exit_time = Some(Utc::now());
                            }
                            return Ok(());
                        }
                    }
                }
            }

            // Wait for graceful shutdown
            let timeout = tokio::time::Duration::from_millis(process_stop_timeout_ms);
            let start = tokio::time::Instant::now();

            #[cfg(unix)]
            {
                while start.elapsed() < timeout {
                    // The group is stopped only after every member has exited.
                    if !Self::is_process_group_alive(self.pid) {
                        info!("Process group {} has stopped", self.name);
                        self.set_status(ProcessStatus::Stopped);
                        if let Ok(mut exit_time) = self.exit_time.lock() {
                            *exit_time = Some(Utc::now());
                        }
                        return Ok(());
                    }

                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }

            info!("Process did not stop gracefully, sending SIGKILL");
        }

        // Force kill with SIGKILL
        info!("Sending SIGKILL to process group {}", self.name);

        #[cfg(unix)]
        {
            match Self::send_signal_to_group(self.pid, Signal::SIGKILL) {
                Ok(()) => {
                    info!("SIGKILL sent successfully to PGID {}", self.pid);
                }
                Err(e) => {
                    info!("Failed to send SIGKILL to PGID {}: {}", self.pid, e);
                    // If the process group doesn't exist, that's OK
                    if !Self::is_process_group_alive(self.pid) {
                        info!("Process group {} already terminated", self.name);
                    }
                }
            }

            self.confirm_stopped_after_force_kill().await?;
        }

        self.set_status(ProcessStatus::Stopped);
        if let Ok(mut exit_time) = self.exit_time.lock() {
            *exit_time = Some(Utc::now());
        }

        // NOTE: We intentionally do NOT cancel monitoring tasks here.
        // The monitor task holds the Child object, and cancelling it would drop the Child,
        // which closes the stdout/stderr pipes and causes the child process to receive
        // SIGPIPE instead of SIGTERM. The monitor task will exit naturally when the
        // process dies, and will be cleaned up when ProxyInfo is dropped.

        Ok(())
    }

    #[cfg(unix)]
    async fn confirm_stopped_after_force_kill(&self) -> Result<(), String> {
        // Generous deadline: SIGKILL cleanup (zombie transition and reaping)
        // can stretch to seconds under CPU starvation from parallel builds/CI.
        self.confirm_stopped_within(tokio::time::Duration::from_secs(10))
            .await
    }

    #[cfg(unix)]
    async fn confirm_stopped_within(&self, timeout: tokio::time::Duration) -> Result<(), String> {
        let start = tokio::time::Instant::now();
        while start.elapsed() < timeout {
            if !Self::is_process_group_alive(self.pid) {
                return Ok(());
            }
            // killpg(pgid, 0) keeps succeeding while unreaped zombies remain in
            // the group, so only report failure if a non-zombie member survives.
            if !Self::group_has_live_non_zombie_member(self.pid).await {
                return Ok(());
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Err(format!(
            "Process group {} (PGID {}) still has live members after SIGKILL",
            self.name, self.pid
        ))
    }

    #[cfg(unix)]
    async fn group_has_live_non_zombie_member(pgid: u32) -> bool {
        let output = match tokio::process::Command::new("ps")
            .args(["-axo", "pgid=,stat="])
            .output()
            .await
        {
            Ok(output) => output,
            // If membership cannot be inspected, assume survivors so the
            // failure is reported rather than silently ignored.
            Err(_) => return true,
        };

        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.split_whitespace();
            let line_pgid = fields.next().and_then(|field| field.parse::<u32>().ok());
            let stat = fields.next();
            line_pgid == Some(pgid) && stat.is_some_and(|stat| !stat.starts_with('Z'))
        })
    }

    pub fn set_detected_port(&self, port: u16) {
        if let Ok(mut detected) = self.detected_port.lock() {
            *detected = Some(port);
        }
    }

    /// Send a signal to every process in the managed process group.
    #[cfg(unix)]
    fn send_signal_to_group(pgid: u32, sig: Signal) -> Result<(), String> {
        match killpg(Pid::from_raw(pgid as i32), sig) {
            Ok(()) => Ok(()),
            Err(nix::Error::ESRCH) => {
                // The process group has already exited.
                Ok(())
            }
            Err(e) => Err(format!("Failed to send process group signal: {}", e)),
        }
    }

    /// Check whether any process remains in the managed process group.
    #[cfg(unix)]
    fn is_process_group_alive(pgid: u32) -> bool {
        match killpg(Pid::from_raw(pgid as i32), None) {
            Ok(()) => true,
            Err(nix::Error::ESRCH) => false,
            Err(_) => true, // Other errors mean the group exists but we may lack permissions
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::daemon::process::types::ProxyInfoParams;
    use uuid::Uuid;

    fn proxy_for_pid(pid: u32) -> ProxyInfo {
        ProxyInfo::new(ProxyInfoParams {
            id: Uuid::new_v4().to_string(),
            name: "stop-test".into(),
            project: "test".into(),
            cmd: None,
            args: Vec::new(),
            cwd: None,
            env: None,
            wait_for_log: None,
            wait_timeout: None,
            toolchain: None,
            pid,
        })
    }

    #[tokio::test]
    async fn force_stop_errors_when_process_survives_sigkill() {
        let mut child = tokio::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let proxy = proxy_for_pid(pid);

        let result = proxy
            .confirm_stopped_within(tokio::time::Duration::from_millis(300))
            .await;

        assert!(
            result.is_err(),
            "live process group unexpectedly reported as stopped"
        );
        assert_ne!(proxy.get_status(), ProcessStatus::Stopped);
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn force_stop_succeeds_for_killable_process() {
        let mut child = tokio::process::Command::new("sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let waiter = tokio::spawn(async move { child.wait().await.unwrap() });
        let proxy = proxy_for_pid(pid);
        proxy.stop(true, 1_000).await.unwrap();
        tokio::time::timeout(tokio::time::Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(proxy.get_status(), ProcessStatus::Stopped);
    }
}

use crate::common::process_key::ProcessKey;
use chrono::{DateTime, Utc};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tracing::info;

/// Log entry with timestamp for ring buffer storage
#[derive(Debug, Clone)]
pub struct LogChunk {
    pub data: Vec<u8>,
    pub timestamp: DateTime<Utc>,
    #[allow(dead_code)] // Reserved for future stdout/stderr distinction
    pub is_stderr: bool,
}

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
    /// Ring buffer for recent log lines (10K capacity)
    pub ring: Arc<Mutex<HeapRb<LogChunk>>>,
    /// Process ID
    pub pid: u32,
    /// Configured port (if any)
    pub port: Option<u16>,
    /// Detected port from process output
    pub detected_port: Arc<Mutex<Option<u16>>>,
    /// Flag indicating if port detection is complete
    pub port_ready: Arc<Mutex<bool>>,
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
            ring: Arc::new(Mutex::new(HeapRb::new(params.ring_buffer_size))),
            pid: params.pid,
            port: None,
            detected_port: Arc::new(Mutex::new(None)),
            port_ready: Arc::new(Mutex::new(false)),
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

    pub async fn stop(&self, force: bool) -> Result<(), String> {
        info!(
            "Stopping process {} (PID: {}, force: {})",
            self.name, self.pid, force
        );
        self.set_status(ProcessStatus::Stopping);

        // Cancel all tasks first
        if let Ok(mut handles) = self.hyperlog_handles.lock() {
            info!(
                "Cancelling {} tasks for process {}",
                handles.len(),
                self.name
            );

            // Abort all tasks
            for handle in handles.drain(..) {
                handle.abort();
            }

            info!("All tasks abort requested for process {}", self.name);
        }

        // Find all child processes of this PID
        let child_pids = self.find_child_processes(self.pid).await;
        if !child_pids.is_empty() {
            info!(
                "Found {} child process(es) for {} (PID: {}): {:?}",
                child_pids.len(),
                self.name,
                self.pid,
                child_pids
            );
        }

        // First attempt: Send SIGTERM (unless force is specified)
        if !force {
            let pid_arg = self.pid.to_string();

            info!(
                "Sending SIGTERM to process {} (PID: {})",
                self.name, self.pid
            );
            let output = tokio::process::Command::new("kill")
                .arg("-TERM")
                .arg(&pid_arg)
                .output()
                .await
                .map_err(|e| format!("Failed to send SIGTERM: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                info!("kill -TERM {} failed: {}", pid_arg, stderr);
                // Check if the error is "No such process" which is ok if process already exited
                if stderr.contains("No such process") || stderr.contains("no such process") {
                    self.set_status(ProcessStatus::Stopped);
                    if let Ok(mut exit_time) = self.exit_time.lock() {
                        *exit_time = Some(Utc::now());
                    }
                    return Ok(());
                }
            } else {
                info!("SIGTERM sent successfully to {}", pid_arg);
            }

            // Also send SIGTERM to all child processes directly
            for child_pid in &child_pids {
                let _ = tokio::process::Command::new("kill")
                    .arg("-TERM")
                    .arg(child_pid.to_string())
                    .output()
                    .await;
            }

            // Wait up to 5 seconds for graceful shutdown
            let timeout = tokio::time::Duration::from_secs(5);
            let start = tokio::time::Instant::now();

            while start.elapsed() < timeout {
                // Check if process is still alive
                let check_output = tokio::process::Command::new("kill")
                    .arg("-0")
                    .arg(&pid_arg)
                    .output()
                    .await;

                if let Ok(output) = check_output {
                    if !output.status.success() {
                        info!("Process {} has stopped (kill -0 failed)", self.name);
                        // Process has stopped
                        self.set_status(ProcessStatus::Stopped);
                        if let Ok(mut exit_time) = self.exit_time.lock() {
                            *exit_time = Some(Utc::now());
                        }
                        return Ok(());
                    }
                } else {
                    info!("Failed to check process status with kill -0");
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }

            info!("Process did not stop gracefully, sending SIGKILL");
        }

        // Force kill with SIGKILL
        let pid_arg = self.pid.to_string();

        info!("Sending SIGKILL to process {}", self.name);
        let output = tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(pid_arg)
            .output()
            .await
            .map_err(|e| format!("Failed to send SIGKILL: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check if the error is "No such process" which is ok if process already exited
            if !stderr.contains("No such process") && !stderr.contains("no such process") {
                // Process group kill failed, try to kill child processes directly
                info!("Process group kill failed, killing child processes directly");
                for child_pid in &child_pids {
                    let _ = tokio::process::Command::new("kill")
                        .arg("-KILL")
                        .arg(child_pid.to_string())
                        .output()
                        .await;
                }
            }
        }

        self.set_status(ProcessStatus::Stopped);
        if let Ok(mut exit_time) = self.exit_time.lock() {
            *exit_time = Some(Utc::now());
        }

        Ok(())
    }

    pub fn mark_port_ready(&self) {
        if let Ok(mut ready) = self.port_ready.lock() {
            *ready = true;
        }
    }

    pub fn set_detected_port(&self, port: u16) {
        if let Ok(mut detected) = self.detected_port.lock() {
            *detected = Some(port);
        }
    }

    /// Find all child processes of a given PID
    async fn find_child_processes(&self, parent_pid: u32) -> Vec<u32> {
        let mut child_pids = Vec::new();

        #[cfg(target_os = "macos")]
        {
            // Use ps command to find child processes on macOS
            if let Ok(output) = tokio::process::Command::new("pgrep")
                .arg("-P")
                .arg(parent_pid.to_string())
                .output()
                .await
            {
                if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                    for line in stdout.lines() {
                        if let Ok(pid) = line.trim().parse::<u32>() {
                            child_pids.push(pid);
                            // Recursively find children of children
                            let grandchildren = Box::pin(self.find_child_processes(pid)).await;
                            child_pids.extend(grandchildren);
                        }
                    }
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            // Use /proc to find child processes on Linux
            if let Ok(entries) = std::fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        if let Ok(pid) = name.parse::<u32>() {
                            // Read the stat file to get parent PID
                            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid))
                            {
                                // Parse parent PID from stat file
                                // Format: pid (comm) state ppid ...
                                if let Some(end) = stat.rfind(')') {
                                    let fields: Vec<&str> =
                                        stat[end + 1..].split_whitespace().collect();
                                    if fields.len() > 2 {
                                        if let Ok(ppid) = fields[1].parse::<u32>() {
                                            if ppid == parent_pid {
                                                child_pids.push(pid);
                                                // Recursively find children of children
                                                let grandchildren =
                                                    Box::pin(self.find_child_processes(pid)).await;
                                                child_pids.extend(grandchildren);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        child_pids
    }
}

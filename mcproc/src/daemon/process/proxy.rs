use crate::common::process_key::ProcessKey;
use chrono::{DateTime, Utc};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

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
    /// Process start time
    pub start_time: DateTime<Utc>,
    /// Current process status (atomic for thread-safe updates)
    pub status: Arc<AtomicU8>,
    /// Ring buffer for recent log lines (10K capacity)
    pub ring: Arc<Mutex<HeapRb<Vec<u8>>>>,
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
            start_time: Utc::now(),
            status: Arc::new(AtomicU8::new(ProcessStatus::Running as u8)),
            ring: Arc::new(Mutex::new(HeapRb::new(params.ring_buffer_size))),
            pid: params.pid,
            port: None,
            detected_port: Arc::new(Mutex::new(None)),
            port_ready: Arc::new(Mutex::new(false)),
            exit_code: Arc::new(Mutex::new(None)),
            exit_time: Arc::new(Mutex::new(None)),
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
        self.set_status(ProcessStatus::Stopping);

        // Send SIGTERM or SIGKILL based on force flag
        let signal = if force { "KILL" } else { "TERM" };
        let output = tokio::process::Command::new("kill")
            .arg(format!("-{}", signal))
            .arg(self.pid.to_string())
            .output()
            .await
            .map_err(|e| format!("Failed to send signal: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to stop process: {}", stderr));
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
}

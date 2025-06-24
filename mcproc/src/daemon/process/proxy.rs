use crate::common::process_key::ProcessKey;
use chrono::{DateTime, Utc};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ProcessStatus {
    Starting = 1,
    Running = 2,
    Stopping = 3,
    Stopped = 4,
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

pub struct ProxyInfo {
    pub id: String,
    #[allow(dead_code)]
    pub key: ProcessKey,
    pub name: String,
    pub project: String,
    pub cmd: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub wait_for_log: Option<String>,
    pub wait_timeout: Option<u32>,
    pub start_time: DateTime<Utc>,
    pub status: Arc<AtomicU8>,
    pub ring: Arc<Mutex<HeapRb<Vec<u8>>>>,
    pub pid: u32,
    pub port: Option<u16>,
    pub detected_port: Arc<Mutex<Option<u16>>>,
    pub port_ready: Arc<Mutex<bool>>,
    pub exit_code: Arc<Mutex<Option<i32>>>,
    pub exit_time: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl ProxyInfo {
    pub fn new(
        id: String,
        name: String,
        project: String,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<std::collections::HashMap<String, String>>,
        wait_for_log: Option<String>,
        wait_timeout: Option<u32>,
        pid: u32,
        ring_buffer_size: usize,
    ) -> Self {
        let key = ProcessKey::new(project.clone(), name.clone());
        Self {
            id,
            key,
            name,
            project,
            cmd,
            args,
            cwd,
            env,
            wait_for_log,
            wait_timeout,
            start_time: Utc::now(),
            status: Arc::new(AtomicU8::new(ProcessStatus::Running as u8)),
            ring: Arc::new(Mutex::new(HeapRb::new(ring_buffer_size))),
            pid,
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

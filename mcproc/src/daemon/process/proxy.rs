use crate::common::process_key::ProcessKey;
use chrono::{DateTime, Utc};
use ringbuf::HeapRb;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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
    pub id: Uuid,
    #[allow(dead_code)]
    pub key: ProcessKey,
    pub name: String,
    pub project: String,
    pub cmd: String,
    pub cwd: PathBuf,
    pub start_time: DateTime<Utc>,
    pub status: Arc<AtomicU8>,
    pub ring: Arc<Mutex<HeapRb<Vec<u8>>>>,
    pub log_file: PathBuf,
    pub pid: Option<u32>,
    pub ports: Arc<Mutex<Vec<u32>>>,
    #[allow(dead_code)]
    pub child_handle: Option<tokio::process::Child>,
    pub exit_code: Arc<Mutex<Option<i32>>>,
    pub exit_time: Arc<Mutex<Option<DateTime<Utc>>>>,
}

impl ProxyInfo {
    pub fn new(
        name: String,
        project: String,
        cmd: String,
        cwd: PathBuf,
        log_file: PathBuf,
        ring_buffer_size: usize,
    ) -> Self {
        let key = ProcessKey::new(&project, &name);
        Self {
            id: Uuid::new_v4(),
            key,
            name,
            project,
            cmd,
            cwd,
            start_time: Utc::now(),
            status: Arc::new(AtomicU8::new(ProcessStatus::Starting as u8)),
            ring: Arc::new(Mutex::new(HeapRb::new(ring_buffer_size))),
            log_file,
            pid: None,
            ports: Arc::new(Mutex::new(Vec::new())),
            child_handle: None,
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
<<<<<<< HEAD

    #[allow(dead_code)]
    pub fn get_key(&self) -> &ProcessKey {
        &self.key
    }
=======
>>>>>>> main
}

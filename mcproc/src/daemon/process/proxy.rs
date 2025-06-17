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

pub struct ProxyInfo {
    pub id: Uuid,
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
}

impl ProxyInfo {
    pub fn new(name: String, project: String, cmd: String, cwd: PathBuf, log_file: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            project,
            cmd,
            cwd,
            start_time: Utc::now(),
            status: Arc::new(AtomicU8::new(ProcessStatus::Starting as u8)),
            ring: Arc::new(Mutex::new(HeapRb::new(10000))),
            log_file,
            pid: None,
            ports: Arc::new(Mutex::new(Vec::new())),
            child_handle: None,
        }
    }
    
    pub fn get_status(&self) -> ProcessStatus {
        ProcessStatus::from(self.status.load(Ordering::Relaxed))
    }
    
    pub fn set_status(&self, status: ProcessStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }
}
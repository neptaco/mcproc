use std::collections::HashMap;
use std::path::PathBuf;

/// Parameters for creating a ProxyInfo
pub struct ProxyInfoParams {
    pub id: String,
    pub name: String,
    pub project: String,
    pub cmd: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub wait_for_log: Option<String>,
    pub wait_timeout: Option<u32>,
    pub toolchain: Option<String>,
    pub pid: u32,
    pub ring_buffer_size: usize,
}

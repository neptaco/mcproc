//! Common path management for mcproc CLI and daemon

use std::path::PathBuf;

/// Configuration for mcproc paths
#[derive(Debug, Clone)]
pub struct McprocPaths {
    /// Base data directory (e.g., ~/.mcproc)
    pub data_dir: PathBuf,
    /// PID file path
    pub pid_file: PathBuf,
    /// Socket file path
    pub socket_path: PathBuf,
    /// Log directory
    pub log_dir: PathBuf,
    /// Main daemon log file
    pub daemon_log_file: PathBuf,
}

impl Default for McprocPaths {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data_dir = home.join(".mcproc");
        let log_dir = data_dir.join("log");
        
        Self {
            data_dir: data_dir.clone(),
            pid_file: data_dir.join("mcprocd.pid"),
            socket_path: data_dir.join("mcprocd.sock"),
            log_dir,
            daemon_log_file: data_dir.join("mcprocd.log"),
        }
    }
}

impl McprocPaths {
    /// Create a new McprocPaths instance with default paths
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Ensure all necessary directories exist
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
    
    /// Get the log file path for a specific process
    pub fn process_log_file(&self, process_name: &str) -> PathBuf {
        let safe_name = process_name.replace('/', "_");
        self.log_dir.join(format!("{}.log", safe_name))
    }
}
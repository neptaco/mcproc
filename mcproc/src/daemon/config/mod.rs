use crate::common::paths::McprocPaths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub daemon: DaemonConfig,
    pub log: LogConfig,
    pub api: ApiConfig,
    pub process: ProcessConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub data_dir: PathBuf,
    pub pid_file: PathBuf,
    pub socket_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub dir: PathBuf,
    pub max_size_mb: u64,
    pub max_files: u32,
    pub ring_buffer_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub grpc_port: u16,
    pub unix_socket_permissions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    pub max_restart_attempts: u32,
    pub restart_delay_ms: u64,
    pub shutdown_timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        let paths = McprocPaths::new();

        Self {
            daemon: DaemonConfig {
                data_dir: paths.data_dir.clone(),
                pid_file: paths.pid_file,
                socket_path: paths.socket_path,
            },
            log: LogConfig {
                dir: paths.log_dir,
                max_size_mb: 100,
                max_files: 10,
                ring_buffer_size: 10000,
            },
            api: ApiConfig {
                grpc_port: 50051,
                unix_socket_permissions: 0o600,
            },
            process: ProcessConfig {
                max_restart_attempts: 3,
                restart_delay_ms: 1000,
                shutdown_timeout_ms: 5000,
            },
        }
    }
}

impl Config {
    pub fn load() -> crate::daemon::error::Result<Self> {
        // For now, just use defaults
        // TODO: Load from config file if exists
        Ok(Self::default())
    }

    pub fn ensure_directories(&self) -> crate::daemon::error::Result<()> {
        std::fs::create_dir_all(&self.daemon.data_dir)?;
        std::fs::create_dir_all(&self.log.dir)?;
        Ok(())
    }
}

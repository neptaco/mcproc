use crate::common::xdg;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// File system paths
    pub paths: PathConfig,
    /// Daemon lifecycle configuration
    pub daemon: DaemonConfig,
    /// Process management configuration
    pub process: ProcessConfig,
    /// Logging configuration
    pub logging: LoggingConfig,
    /// API server configuration
    pub api: ApiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConfig {
    /// Base data directory (e.g., ~/.mcproc)
    pub data_dir: PathBuf,
    /// Directory for log files
    pub log_dir: PathBuf,
    /// PID file path for daemon process tracking
    pub pid_file: PathBuf,
    /// Unix domain socket path for client-daemon communication
    pub socket_path: PathBuf,
    /// Main daemon log file path
    pub daemon_log_file: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Maximum time to wait for daemon startup (milliseconds)
    pub startup_timeout_ms: u64,
    /// Grace period for processes to shut down during daemon shutdown (milliseconds)
    pub shutdown_grace_period_ms: u64,
    /// Interval for checking daemon stop status (milliseconds)
    pub stop_check_interval_ms: u64,
    /// Timeout for client connecting to daemon (seconds)
    pub client_connection_timeout_secs: u64,
    /// Time to wait after starting daemon before attempting connection (milliseconds)
    /// Note: This is now used as a maximum wait time with multiple checks
    pub client_startup_wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Enable file logging (default: true)
    pub enable_file_logging: bool,
    /// Maximum size per log file in MB (not yet implemented)
    pub max_size_mb: u64,
    /// Maximum number of log files to keep (not yet implemented)
    pub max_files: u32,
    /// Size of in-memory ring buffer for each process
    pub ring_buffer_size: usize,
    /// Polling interval for log file following (milliseconds)
    pub follow_poll_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Unix socket file permissions (octal, e.g., 0o600)
    pub unix_socket_permissions: u32,
    /// Additional buffer time for gRPC requests beyond wait_timeout (seconds)
    pub grpc_request_buffer_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    /// Startup configuration
    pub startup: ProcessStartupConfig,
    /// Restart configuration
    pub restart: ProcessRestartConfig,
    /// Port detection configuration
    pub port_detection: PortDetectionConfig,
    /// Log buffer size (number of lines)
    pub log_buffer_size: usize,
    /// Whether to create independent process groups for managed processes
    /// - true: Processes survive daemon crashes but may become orphaned (default)
    /// - false: Processes are terminated when daemon stops (safer cleanup)
    pub independent_process_groups: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStartupConfig {
    /// Default timeout for wait_for_log pattern matching (seconds)
    pub default_wait_timeout_secs: u32,
    /// Delay before health check when no wait_for_log pattern is provided (milliseconds)
    pub health_check_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRestartConfig {
    /// Maximum number of automatic restart attempts (not yet implemented)
    pub max_attempts: u32,
    /// Delay between stop and start during restart (milliseconds)
    pub delay_ms: u64,
    /// Timeout for graceful process shutdown (milliseconds, not yet implemented)
    pub shutdown_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDetectionConfig {
    /// Initial delay before starting port detection (seconds)
    pub initial_delay_secs: u64,
    /// Interval between port detection attempts (seconds)
    pub interval_secs: u64,
    /// Maximum number of port detection attempts
    pub max_attempts: u32,
}

impl Default for Config {
    fn default() -> Self {
        // Get XDG directories
        let data_dir = xdg::get_data_dir();
        let state_dir = xdg::get_state_dir();
        let runtime_dir = xdg::get_runtime_dir();
        let log_dir = state_dir.join("log");

        Self {
            paths: PathConfig {
                data_dir: data_dir.clone(),
                log_dir: log_dir.clone(),
                pid_file: runtime_dir.join("mcprocd.pid"),
                socket_path: runtime_dir.join("mcprocd.sock"),
                daemon_log_file: log_dir.join("mcprocd.log"),
            },
            daemon: DaemonConfig {
                startup_timeout_ms: 2000,
                shutdown_grace_period_ms: 500,
                stop_check_interval_ms: 100,
                client_connection_timeout_secs: 5,
                client_startup_wait_ms: 1000, // Max wait time with multiple checks
            },
            process: ProcessConfig {
                startup: ProcessStartupConfig {
                    default_wait_timeout_secs: 30,
                    health_check_delay_ms: 500,
                },
                restart: ProcessRestartConfig {
                    max_attempts: 3,
                    delay_ms: 1000,
                    shutdown_timeout_ms: 5000,
                },
                port_detection: PortDetectionConfig {
                    initial_delay_secs: 3,
                    interval_secs: 3,
                    max_attempts: 30,
                },
                log_buffer_size: 10000,
                independent_process_groups: false, // Default to safer cleanup
            },
            logging: LoggingConfig {
                enable_file_logging: true, // Default ON
                max_size_mb: 100,
                max_files: 10,
                ring_buffer_size: 10000,
                follow_poll_interval_ms: 100,
            },
            api: ApiConfig {
                unix_socket_permissions: 0o600,
                grpc_request_buffer_secs: 5,
            },
        }
    }
}

impl Config {
    /// Get the config file path
    pub fn get_config_file_path() -> PathBuf {
        xdg::get_config_dir().join("config.toml")
    }

    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = Self::get_config_file_path();

        if config_path.exists() {
            // Load from config file
            let contents = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            // Use defaults
            Ok(Self::default())
        }
    }

    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.paths.data_dir)?;
        std::fs::create_dir_all(&self.paths.log_dir)?;

        // Ensure runtime directory exists
        if let Some(parent) = self.paths.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Ensure config directory exists
        let config_dir = xdg::get_config_dir();
        std::fs::create_dir_all(&config_dir)?;

        Ok(())
    }

    pub fn daemon_log_file(&self) -> PathBuf {
        self.paths.daemon_log_file.clone()
    }

    // Create a minimal config for CLI/client use (no daemon dependencies)
    pub fn for_client() -> Self {
        Self::default()
    }
}

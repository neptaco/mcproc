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
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".local/share")
            })
            .join("mcproc");

        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let uid = unsafe { libc::getuid() };
                PathBuf::from("/tmp").join(format!("mcproc-{}", uid))
            })
            .join("mcproc");

        let log_dir = data_dir.join("log");

        Self {
            data_dir: data_dir.clone(),
            pid_file: runtime_dir.join("mcprocd.pid"),
            socket_path: runtime_dir.join("mcprocd.sock"),
            log_dir,
            daemon_log_file: data_dir.join("mcprocd.log"),
        }
    }
}

impl McprocPaths {
    /// Create a new McprocPaths instance with XDG support and migration
    pub fn new() -> Self {
        let paths = Self::default();

        if let Some(home) = dirs::home_dir() {
            let old_mcproc_dir = home.join(".mcproc");
            if old_mcproc_dir.exists() && !paths.data_dir.exists() {
                if let Err(e) = paths.migrate_from_legacy(&old_mcproc_dir) {
                    eprintln!("Warning: Failed to migrate from ~/.mcproc: {}", e);
                    eprintln!("Continuing with XDG directories...");
                }
            }
        }

        paths
    }

    fn migrate_from_legacy(&self, old_dir: &std::path::Path) -> std::io::Result<()> {
        use std::fs;

        self.ensure_directories()?;

        let old_log_dir = old_dir.join("log");
        if old_log_dir.exists() {
            for entry in fs::read_dir(&old_log_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let dest = self.log_dir.join(entry.file_name());
                    fs::copy(entry.path(), dest)?;
                }
            }
        }

        let old_daemon_log = old_dir.join("mcprocd.log");
        if old_daemon_log.exists() {
            fs::copy(&old_daemon_log, &self.daemon_log_file)?;
        }

        println!("Successfully migrated mcproc data from ~/.mcproc to XDG directories");
        println!("  Data: {}", self.data_dir.display());
        println!("  Logs: {}", self.log_dir.display());

        Ok(())
    }

    /// Ensure all necessary directories exist
    pub fn ensure_directories(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;

        if let Some(runtime_parent) = self.pid_file.parent() {
            std::fs::create_dir_all(runtime_parent)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(runtime_parent)?.permissions();
                perms.set_mode(0o700);
                std::fs::set_permissions(runtime_parent, perms)?;
            }
        }

        Ok(())
    }

    /// Get the log file path for a specific process
    pub fn process_log_file(&self, process_name: &str) -> PathBuf {
        let safe_name = process_name.replace('/', "_");
        self.log_dir.join(format!("{}.log", safe_name))
    }
}

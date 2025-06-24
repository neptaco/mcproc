use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use crate::daemon::error::{McprocdError, Result};
use crate::daemon::process::proxy::ProxyInfo;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, info};
use uuid::Uuid;

pub struct ProcessLauncher {
    config: Arc<Config>,
}

impl ProcessLauncher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    /// Build and spawn a process with the given configuration
    pub async fn launch_process(
        &self,
        name: String,
        project: String,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<HashMap<String, String>>,
    ) -> Result<(tokio::process::Child, ProcessKey)> {
        let process_key = ProcessKey::new(project.clone(), name.clone());

        // Build command
        let mut command = if !args.is_empty() {
            let mut cmd = Command::new(&args[0]);
            cmd.args(&args[1..]);
            cmd
        } else if let Some(cmd_str) = cmd {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(cmd_str);
            cmd
        } else {
            return Err(McprocdError::InvalidCommand {
                message: "Either cmd or args must be provided".to_string(),
            });
        };

        // Set working directory
        if let Some(cwd_path) = &cwd {
            debug!("Setting working directory to: {:?}", cwd_path);
            command.current_dir(cwd_path);
        }

        // Set environment variables
        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                command.env(key, value);
            }
        }

        // Setup stdio
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command.stdin(Stdio::null());

        // Kill on drop to ensure cleanup
        command.kill_on_drop(true);

        // Log file will be created automatically on first write
        info!("Starting process {} in project {}", name, project);

        // Spawn the process
        let child = command
            .spawn()
            .map_err(|e| McprocdError::ProcessSpawnFailed {
                name: name.clone(),
                error: e.to_string(),
            })?;

        Ok((child, process_key))
    }

    /// Create a ProxyInfo instance for the launched process
    pub fn create_proxy_info(
        &self,
        name: String,
        project: String,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<HashMap<String, String>>,
        wait_for_log: Option<String>,
        wait_timeout: Option<u32>,
        pid: u32,
    ) -> Arc<ProxyInfo> {
        // Extract port from environment if available
        let port = env
            .as_ref()
            .and_then(|e| e.get("PORT"))
            .and_then(|p| p.parse::<u16>().ok());
        let id = Uuid::new_v4().to_string();
        let mut proxy = ProxyInfo::new(
            id,
            name,
            project,
            cmd,
            args,
            cwd,
            env,
            wait_for_log,
            wait_timeout,
            pid,
            self.config.process.log_buffer_size,
        );
        proxy.port = port;
        Arc::new(proxy)
    }

    /// Parse wait_for_log pattern if provided
    pub fn parse_wait_pattern(&self, wait_for_log: Option<String>) -> Result<Option<Arc<Regex>>> {
        wait_for_log
            .map(|pattern| {
                Regex::new(&pattern)
                    .map(Arc::new)
                    .map_err(|e| McprocdError::InvalidRegex {
                        pattern,
                        error: e.to_string(),
                    })
            })
            .transpose()
    }
}

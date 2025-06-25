use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use crate::daemon::error::{McprocdError, Result};
use crate::daemon::process::proxy::ProxyInfo;
use crate::daemon::process::toolchain::Toolchain;
use crate::daemon::process::types::ProxyInfoParams;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info};
use uuid::Uuid;

/// Parameters for launching a process
pub struct LaunchProcessParams {
    pub name: String,
    pub project: String,
    pub cmd: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub toolchain: Option<String>,
}

/// Parameters for creating a ProxyInfo via launcher
pub struct CreateProxyInfoParams {
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
}

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
        params: LaunchProcessParams,
    ) -> Result<(tokio::process::Child, ProcessKey)> {
        let process_key = ProcessKey::new(params.project.clone(), params.name.clone());

        // Build command
        // Construct the command to execute via shell
        let shell_command = if !params.args.is_empty() {
            // Join args into a single command string, properly escaping each argument
            params
                .args
                .iter()
                .map(|arg| {
                    // Simple escaping: wrap in single quotes and escape any single quotes
                    format!("'{}'", arg.replace("'", "'\"'\"'"))
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else if let Some(cmd_str) = params.cmd {
            // Use the cmd string as-is
            cmd_str
        } else {
            return Err(McprocdError::InvalidCommand {
                message: "Either cmd or args must be provided".to_string(),
            });
        };

        // Build the actual command considering toolchain
        let (final_command, exec_description) = if let Some(tool_str) = params.toolchain {
            match Toolchain::parse(&tool_str) {
                Some(toolchain) => toolchain.wrap_command(&shell_command),
                None => {
                    return Err(McprocdError::InvalidCommand {
                        message: format!(
                            "Unsupported toolchain: '{}'. Supported toolchains: {}",
                            tool_str,
                            Toolchain::all_supported()
                        ),
                    });
                }
            }
        } else {
            (shell_command.clone(), format!("sh -c '{}'", shell_command))
        };

        // Always execute via shell for consistent behavior
        let mut command = Command::new("sh");
        command.arg("-c").arg(&final_command);

        debug!("Executing command via shell: {}", exec_description);

        // Set working directory
        if let Some(cwd_path) = &params.cwd {
            debug!("Setting working directory to: {:?}", cwd_path);
            command.current_dir(cwd_path);
        }

        // Set environment variables
        if let Some(env_vars) = params.env {
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
        info!(
            "Starting process {} in project {}",
            params.name, params.project
        );

        // Spawn the process
        let child = command.spawn().map_err(|e| {
            error!("Failed to spawn process '{}': {}", params.name, e);
            McprocdError::ProcessSpawnFailed {
                name: params.name.clone(),
                error: e.to_string(),
            }
        })?;

        info!(
            "Successfully spawned process '{}' with PID {:?}",
            params.name,
            child.id()
        );

        Ok((child, process_key))
    }

    /// Create a ProxyInfo instance for the launched process
    pub fn create_proxy_info(&self, params: CreateProxyInfoParams) -> Arc<ProxyInfo> {
        // Extract port from environment if available
        let port = params
            .env
            .as_ref()
            .and_then(|e| e.get("PORT"))
            .and_then(|p| p.parse::<u16>().ok());
        let id = Uuid::new_v4().to_string();
        let mut proxy = ProxyInfo::new(ProxyInfoParams {
            id,
            name: params.name,
            project: params.project,
            cmd: params.cmd,
            args: params.args,
            cwd: params.cwd,
            env: params.env,
            wait_for_log: params.wait_for_log,
            wait_timeout: params.wait_timeout,
            toolchain: params.toolchain,
            pid: params.pid,
            ring_buffer_size: self.config.process.log_buffer_size,
        });
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

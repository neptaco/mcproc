use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use crate::daemon::error::{McprocdError, Result};
use crate::daemon::log::LogHub;
use crate::daemon::process::exit_handler::ExitHandler;
use crate::daemon::process::launcher::ProcessLauncher;
use crate::daemon::process::log_stream::LogStreamConfig;
use crate::daemon::process::port_detector;
use crate::daemon::process::proxy::{ProcessStatus, ProxyInfo};
use crate::daemon::process::registry::ProcessRegistry;
use crate::daemon::stream::{SharedStreamEventHub, StreamEvent};
use colored::Colorize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

pub struct ProcessManager {
    registry: ProcessRegistry,
    config: Arc<Config>,
    log_hub: Arc<LogHub>,
    launcher: ProcessLauncher,
    event_hub: Option<SharedStreamEventHub>,
}

impl ProcessManager {
    pub fn with_event_hub(
        config: Arc<Config>,
        log_hub: Arc<LogHub>,
        event_hub: SharedStreamEventHub,
    ) -> Self {
        let launcher = ProcessLauncher::new(config.clone());
        Self {
            registry: ProcessRegistry::new(),
            config,
            log_hub,
            launcher,
            event_hub: Some(event_hub),
        }
    }

    /// Publish a process event to the event hub
    fn publish_process_event(&self, event: crate::daemon::process::event::ProcessEvent) {
        if let Some(ref event_hub) = self.event_hub {
            event_hub.publish(StreamEvent::Process(event));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_process_with_log_stream(
        &self,
        name: String,
        project: Option<String>,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<HashMap<String, String>>,
        wait_for_log: Option<String>,
        wait_timeout: Option<u32>,
        toolchain: Option<String>,
    ) -> Result<(Arc<ProxyInfo>, bool, bool, Vec<String>, Option<String>)> {
        let project = project.unwrap_or_else(|| {
            cwd.as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string()
        });

        // Check if process already exists
        if let Some(existing) = self.registry.get_process_by_name(&name) {
            if matches!(existing.get_status(), ProcessStatus::Running) {
                return Err(McprocdError::ProcessAlreadyExists(name));
            }
        }

        // Parse wait pattern if provided
        let log_pattern = self.launcher.parse_wait_pattern(wait_for_log.clone())?;

        // Setup log ready channel
        let (log_ready_tx, log_ready_rx) = if log_pattern.is_some() {
            let (tx, rx) = oneshot::channel();
            (Some(Arc::new(Mutex::new(Some(tx)))), Some(rx))
        } else {
            (None, None)
        };

        // Pattern match tracking
        let pattern_matched = Arc::new(Mutex::new(false));
        let timeout_occurred = Arc::new(Mutex::new(false));
        let matched_line = Arc::new(Mutex::new(None::<String>));

        // Launch the process
        let (mut child, process_key) = self
            .launcher
            .launch_process(crate::daemon::process::launcher::LaunchProcessParams {
                name: name.clone(),
                project: project.clone(),
                cmd: cmd.clone(),
                args: args.clone(),
                cwd: cwd.clone(),
                env: env.clone(),
                toolchain: toolchain.clone(),
            })
            .await?;

        let pid = child.id().ok_or_else(|| McprocdError::ProcessSpawnFailed {
            name: name.clone(),
            error: "Failed to get PID".to_string(),
        })?;

        // Create proxy info
        let proxy_arc = self.launcher.create_proxy_info(
            crate::daemon::process::launcher::CreateProxyInfoParams {
                name: name.clone(),
                project: project.clone(),
                cmd,
                args,
                cwd,
                env,
                wait_for_log: wait_for_log.clone(),
                wait_timeout,
                toolchain,
                pid,
            },
        );

        // Add to registry
        self.registry.add_process(proxy_arc.clone());

        // Publish Starting event
        self.publish_process_event(crate::daemon::process::event::ProcessEvent::Starting {
            process_id: proxy_arc.id.clone(),
            name: name.clone(),
            project: project.clone(),
        });

        // Log the start event with color (green for starting)
        let start_msg = format!(
            "{} Starting process '{}' (PID: {})\n",
            "[mcproc]".green().bold(),
            name.green(),
            pid.to_string().green()
        );
        if let Err(e) = self
            .log_hub
            .append_log_for_key(&process_key, start_msg.as_bytes(), true)
            .await
        {
            error!("Failed to write start log: {}", e);
        }

        // Setup stdout/stderr capture
        let stdout = child.stdout.take().expect("stdout should be captured");
        let stderr = child.stderr.take().expect("stderr should be captured");

        // Spawn stdout reader
        let stdout_config = LogStreamConfig {
            stream_name: "stdout",
            process_key: process_key.clone(),
            proxy: proxy_arc.clone(),
            log_hub: self.log_hub.clone(),
            log_pattern: log_pattern.clone(),
            log_ready_tx: log_ready_tx.clone(),
            pattern_matched: pattern_matched.clone(),
            timeout_occurred: timeout_occurred.clone(),
            wait_timeout,
            default_wait_timeout_secs: self.config.process.startup.default_wait_timeout_secs,
            matched_line: matched_line.clone(),
        };
        stdout_config.spawn_log_reader(stdout).await;

        // Spawn stderr reader
        let stderr_config = LogStreamConfig {
            stream_name: "stderr",
            process_key: process_key.clone(),
            proxy: proxy_arc.clone(),
            log_hub: self.log_hub.clone(),
            log_pattern: log_pattern.clone(),
            log_ready_tx: log_ready_tx.clone(),
            pattern_matched: pattern_matched.clone(),
            timeout_occurred: timeout_occurred.clone(),
            wait_timeout,
            default_wait_timeout_secs: self.config.process.startup.default_wait_timeout_secs,
            matched_line: matched_line.clone(),
        };
        stderr_config.spawn_log_reader(stderr).await;

        // Spawn process monitor
        let monitor_proxy = proxy_arc.clone();
        let monitor_name = name.clone();
        let monitor_key = process_key.clone();
        let monitor_log_hub = self.log_hub.clone();
        let monitor_registry = self.registry.clone();
        let monitor_event_hub = self.event_hub.clone();
        let monitor_project = project.clone();

        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    let exit_code = status.code();
                    if let Ok(mut code_guard) = monitor_proxy.exit_code.lock() {
                        *code_guard = exit_code;
                    }

                    // Set exit time
                    if let Ok(mut exit_time) = monitor_proxy.exit_time.lock() {
                        *exit_time = Some(chrono::Utc::now());
                    }

                    // Set status based on exit code
                    match exit_code {
                        Some(0) => monitor_proxy.set_status(ProcessStatus::Stopped),
                        Some(_) => monitor_proxy.set_status(ProcessStatus::Failed),
                        None => monitor_proxy.set_status(ProcessStatus::Failed), // Terminated by signal
                    }

                    // Publish Stopped event
                    if let Some(ref event_hub) = monitor_event_hub {
                        event_hub.publish(StreamEvent::Process(
                            crate::daemon::process::event::ProcessEvent::Stopped {
                                process_id: monitor_proxy.id.clone(),
                                name: monitor_name.clone(),
                                project: monitor_project.clone(),
                                exit_code,
                            },
                        ));
                    }

                    let exit_msg = ExitHandler::format_exit_message(&monitor_name, exit_code);
                    info!("{}", exit_msg);

                    // Log the exit with appropriate color based on exit code
                    let log_msg = match exit_code {
                        Some(0) => format!("{} {}\n", "[mcproc]".green().bold(), exit_msg.green()),
                        _ => format!("{} {}\n", "[mcproc]".red().bold(), exit_msg.red()),
                    };
                    if let Err(e) = monitor_log_hub
                        .append_log_for_key(&monitor_key, log_msg.as_bytes(), true)
                        .await
                    {
                        error!("Failed to write exit log: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to wait for process {}: {}", monitor_name, e);
                    monitor_proxy.set_status(ProcessStatus::Failed);

                    // Publish Failed event
                    if let Some(ref event_hub) = monitor_event_hub {
                        event_hub.publish(StreamEvent::Process(
                            crate::daemon::process::event::ProcessEvent::Failed {
                                process_id: monitor_proxy.id.clone(),
                                name: monitor_name.clone(),
                                project: monitor_project.clone(),
                                error: e.to_string(),
                            },
                        ));
                    }
                }
            }

            // Clean up: close log file and remove from registry
            let _ = monitor_log_hub.close_log_for_key(&monitor_key).await;
            monitor_registry.remove_process(&monitor_proxy.id);
        });

        // Spawn port detector
        if let Some(configured_port) = proxy_arc.port {
            let port_proxy = proxy_arc.clone();
            let port_name = name.clone();
            tokio::spawn(async move {
                if let Err(e) = port_detector::wait_for_port(configured_port, 30).await {
                    warn!(
                        "Port {} not available for process {}: {}",
                        configured_port, port_name, e
                    );
                } else {
                    info!(
                        "Port {} is now available for process {}",
                        configured_port, port_name
                    );
                    port_proxy.mark_port_ready();
                }
            });
        } else {
            let detect_proxy = proxy_arc.clone();
            let detect_name = name.clone();
            let detect_pid = pid;
            tokio::spawn(async move {
                match port_detector::detect_port_for_pid(detect_pid).await {
                    Ok(Some(port)) => {
                        info!("Detected port {} for process {}", port, detect_name);
                        detect_proxy.set_detected_port(port);
                    }
                    Ok(None) => {
                        debug!("No port detected for process {}", detect_name);
                    }
                    Err(e) => {
                        debug!("Error detecting port for process {}: {}", detect_name, e);
                    }
                }
            });
        }

        // Wait for pattern match or initial startup time with timeout
        if let Some(rx) = log_ready_rx {
            let wait_duration = tokio::time::Duration::from_secs(
                wait_timeout.unwrap_or(self.config.process.startup.default_wait_timeout_secs)
                    as u64
                    + 10, // Extra 10 seconds buffer (longer than CLI timeout)
            );

            match tokio::time::timeout(wait_duration, rx).await {
                Ok(Ok(_)) => {
                    debug!("Log pattern matched for process {}", name);
                    // Pattern matched - but still need to verify process is running
                }
                Ok(Err(_)) => {
                    // Channel closed without match (likely timeout in HyperLogStreamer)
                    debug!("Pattern match channel closed for process {}", name);
                }
                Err(_) => {
                    // Timeout in ProcessManager (safety net)
                    warn!(
                        "ProcessManager timeout waiting for pattern match for process {}",
                        name
                    );
                }
            }
        } else {
            // No wait_for_log pattern, wait a bit to collect initial logs
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        // Always sync process status to ensure accuracy
        // This is critical for detecting processes that exit immediately (e.g., "command not found")
        self.sync_process_status(&proxy_arc, &name).await;

        // Check final status and publish appropriate event
        let current_status = proxy_arc.get_status();
        if matches!(current_status, ProcessStatus::Running) {
            // Publish Started event only if actually running
            self.publish_process_event(crate::daemon::process::event::ProcessEvent::Started {
                process_id: proxy_arc.id.clone(),
                name: name.clone(),
                project: project.clone(),
                pid,
            });
        }

        let status = proxy_arc.get_status();
        match status {
            ProcessStatus::Running => {
                info!("Started process {} with PID {:?}", name, proxy_arc.pid)
            }
            ProcessStatus::Failed => info!("Process {} failed to start", name),
            _ => info!("Process {} in status {:?}", name, status),
        }

        // Get matched line if pattern was found
        let collected_matched_line = matched_line.lock().ok().and_then(|g| g.clone());

        Ok((
            proxy_arc,
            timeout_occurred.lock().map(|g| *g).unwrap_or(false),
            pattern_matched.lock().map(|g| *g).unwrap_or(false),
            Vec::new(), // TODO: log_context is not collected by HyperLogStreamer
            collected_matched_line,
        ))
    }

    pub async fn stop_process(
        &self,
        name_or_id: &str,
        project: Option<&str>,
        force: bool,
    ) -> Result<()> {
        let process = self
            .registry
            .get_process_by_name_or_id_with_project(name_or_id, project)
            .ok_or_else(|| McprocdError::ProcessNotFound {
                name: name_or_id.to_string(),
            })?;

        let name = process.name.clone();
        let project = process.project.clone();
        let process_key = ProcessKey::new(project.clone(), name.clone());

        info!("Stopping process {} in project {}", name, project);

        // Publish Stopping event
        self.publish_process_event(crate::daemon::process::event::ProcessEvent::Stopping {
            process_id: process.id.clone(),
            name: name.clone(),
            project: project.clone(),
        });

        // Log the stop event with color (yellow for stopping)
        let log_msg = format!(
            "{} Stopping process {}\n",
            "[mcproc]".yellow().bold(),
            name.yellow()
        );
        if let Err(e) = self
            .log_hub
            .append_log_for_key(&process_key, log_msg.as_bytes(), true)
            .await
        {
            error!("Failed to write stop log: {}", e);
        }

        process.stop(force).await.map_err(McprocdError::StopError)?;

        // Remove from registry
        self.registry.remove_process(&process.id);

        // Close log file to ensure all data is flushed
        if let Err(e) = self.log_hub.close_log_for_key(&process_key).await {
            error!("Failed to close log file for {}/{}: {}", project, name, e);
        }

        // Log the stopped event with color (red for stopped)
        let log_msg = format!("{} Process stopped\n", "[mcproc]".red().bold());
        // Write to a new log entry after reopening (if process restarts)
        if let Err(e) = self
            .log_hub
            .append_log_for_key(&process_key, log_msg.as_bytes(), true)
            .await
        {
            // This is expected if the file was closed, so just log at debug level
            debug!("Could not write final stop log (expected): {}", e);
        }

        // Wait a bit for graceful shutdown and log flushing
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        info!("Stopped process {} in project {}", name, project);
        Ok(())
    }

    pub async fn restart_process_with_log_stream(
        &self,
        name_or_id: &str,
        project: Option<String>,
        override_wait_for_log: Option<String>,
        override_wait_timeout: Option<u32>,
    ) -> Result<(Arc<ProxyInfo>, bool, bool, Vec<String>, Option<String>)> {
        if let Some(process) = self
            .registry
            .get_process_by_name_or_id_with_project(name_or_id, project.as_deref())
        {
            let name = process.name.clone();
            let project = process.project.clone();
            let cmd = process.cmd.clone();
            let args = process.args.clone();
            let cwd = process.cwd.clone();
            let env = process.env.clone();

            // Use override values if provided, otherwise use saved values
            let wait_for_log = override_wait_for_log.or(process.wait_for_log.clone());
            let wait_timeout = override_wait_timeout.or(process.wait_timeout);
            let toolchain = process.toolchain.clone();
            drop(process);

            self.stop_process(name_or_id, Some(&project), false).await?;

            // Wait a bit for process to stop
            tokio::time::sleep(tokio::time::Duration::from_millis(
                self.config.process.restart.delay_ms,
            ))
            .await;

            self.start_process_with_log_stream(
                name,
                Some(project),
                cmd,
                args,
                cwd,
                env,
                wait_for_log,
                wait_timeout,
                toolchain,
            )
            .await
        } else {
            Err(McprocdError::ProcessNotFound {
                name: name_or_id.to_string(),
            })
        }
    }

    pub fn get_process_by_name_or_id_with_project(
        &self,
        name_or_id: &str,
        project: Option<&str>,
    ) -> Option<Arc<ProxyInfo>> {
        self.registry
            .get_process_by_name_or_id_with_project(name_or_id, project)
    }

    pub fn get_all_processes(&self) -> Vec<Arc<ProxyInfo>> {
        self.registry.get_all_processes()
    }

    pub fn list_processes(&self) -> Vec<Arc<ProxyInfo>> {
        self.registry.list_processes()
    }

    pub async fn clean_project(&self, project: &str, force: bool) -> Result<Vec<String>> {
        let processes = self.registry.get_processes_by_project(project);
        let mut stopped = Vec::new();

        for process in processes {
            let name = process.name.clone();
            if let Err(e) = self.stop_process(&process.id, Some(project), force).await {
                error!(
                    "Failed to stop process {} in project {}: {}",
                    name, project, e
                );
            } else {
                stopped.push(name);
            }
        }

        Ok(stopped)
    }

    pub async fn clean_all_projects(&self, force: bool) -> Result<HashMap<String, Vec<String>>> {
        let projects = self.registry.get_all_projects();
        let mut results = HashMap::new();

        for project in projects {
            match self.clean_project(&project, force).await {
                Ok(stopped) => {
                    results.insert(project, stopped);
                }
                Err(e) => {
                    error!("Failed to clean project {}: {}", project, e);
                }
            }
        }

        Ok(results)
    }

    /// Synchronize process status with actual process state
    /// This is critical to ensure we report accurate status to MCP
    async fn sync_process_status(&self, proxy: &Arc<ProxyInfo>, name: &str) {
        debug!(
            "sync_process_status: checking process {} (PID: {})",
            name, proxy.pid
        );
        // Check if process monitor has already detected exit
        if let Ok(exit_code) = proxy.exit_code.lock() {
            if let Some(code) = *exit_code {
                // Process has exited - ensure status reflects this
                let current_status = proxy.get_status();
                if matches!(
                    current_status,
                    ProcessStatus::Running | ProcessStatus::Starting
                ) {
                    // Update status based on exit code
                    let new_status = if code == 0 {
                        ProcessStatus::Stopped
                    } else {
                        ProcessStatus::Failed
                    };
                    proxy.set_status(new_status);
                    debug!(
                        "Synchronized status for process {} from {:?} to {:?} (exit_code: {})",
                        name, current_status, new_status, code
                    );
                }
                return;
            }
        }

        // Double-check process is actually running using kill -0
        let pid = proxy.pid;
        match tokio::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .await
        {
            Ok(output) => {
                if !output.status.success() {
                    // Process is not running
                    let current_status = proxy.get_status();
                    if matches!(current_status, ProcessStatus::Starting) {
                        // Process died during startup - mark as failed
                        proxy.set_status(ProcessStatus::Failed);
                        debug!(
                            "Process {} (PID {}) died during startup, updating status to Failed",
                            name, pid
                        );
                    } else if matches!(current_status, ProcessStatus::Running) {
                        // Process was running but is now dead
                        proxy.set_status(ProcessStatus::Failed);
                        debug!(
                            "Process {} (PID {}) is not running, updating status to Failed",
                            name, pid
                        );
                    }
                } else if matches!(proxy.get_status(), ProcessStatus::Starting) {
                    // Process is alive and was starting, update to Running
                    proxy.set_status(ProcessStatus::Running);
                    debug!(
                        "Process {} (PID {}) is running, updating status from Starting to Running",
                        name, pid
                    );
                }
            }
            Err(e) => {
                warn!("Failed to check process {} status: {}", name, e);
            }
        }
    }

    /// Start a background task that periodically checks and synchronizes process states
    pub fn start_periodic_sync(&self) {
        let registry = self.registry.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;

                // Get all processes that should be checked
                let processes = registry.list_processes();
                let active_processes: Vec<_> = processes
                    .into_iter()
                    .filter(|p| {
                        matches!(
                            p.get_status(),
                            ProcessStatus::Running | ProcessStatus::Starting
                        )
                    })
                    .collect();

                if !active_processes.is_empty() {
                    debug!(
                        "Periodic sync: checking {} active processes",
                        active_processes.len()
                    );

                    for process in active_processes {
                        // Use the existing sync_process_status logic
                        Self::sync_process_status_static(&process).await;
                    }
                }
            }
        });

        info!("Started periodic process state synchronization (interval: 10s)");
    }

    /// Static version of sync_process_status for use in background tasks
    async fn sync_process_status_static(proxy: &Arc<ProxyInfo>) {
        let name = &proxy.name;
        debug!(
            "sync_process_status: checking process {} (PID: {})",
            name, proxy.pid
        );

        // Check if process monitor has already detected exit
        if let Ok(exit_code) = proxy.exit_code.lock() {
            if let Some(code) = *exit_code {
                // Process has exited - ensure status reflects this
                let current_status = proxy.get_status();
                if matches!(
                    current_status,
                    ProcessStatus::Running | ProcessStatus::Starting
                ) {
                    // Update status based on exit code
                    let new_status = if code == 0 {
                        ProcessStatus::Stopped
                    } else {
                        ProcessStatus::Failed
                    };
                    proxy.set_status(new_status);
                    info!(
                        "Periodic sync: updated process {} from {:?} to {:?} (exit_code: {})",
                        name, current_status, new_status, code
                    );
                }
                return;
            }
        }

        // Double-check process is actually running using kill -0
        let pid = proxy.pid;
        match tokio::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .await
        {
            Ok(output) => {
                if !output.status.success() {
                    // Process is not running
                    let current_status = proxy.get_status();
                    if matches!(current_status, ProcessStatus::Starting) {
                        // Process died during startup - mark as failed
                        proxy.set_status(ProcessStatus::Failed);
                        info!(
                            "Periodic sync: process {} (PID {}) died during startup, marked as Failed",
                            name, pid
                        );
                    } else if matches!(current_status, ProcessStatus::Running) {
                        // Process was running but is now dead
                        proxy.set_status(ProcessStatus::Failed);
                        info!(
                            "Periodic sync: process {} (PID {}) is no longer running, marked as Failed",
                            name, pid
                        );
                    }
                } else if matches!(proxy.get_status(), ProcessStatus::Starting) {
                    // Process is alive and was starting, update to Running
                    proxy.set_status(ProcessStatus::Running);
                    info!(
                        "Periodic sync: process {} (PID {}) is now running, updated status",
                        name, pid
                    );
                }
            }
            Err(e) => {
                warn!(
                    "Failed to check process {} status during periodic sync: {}",
                    name, e
                );
            }
        }
    }
}

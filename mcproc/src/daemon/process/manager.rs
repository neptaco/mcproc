use crate::daemon::config::Config;
use crate::daemon::error::{McprocdError, Result};
use crate::daemon::log::LogHub;
use crate::daemon::process::port_detector;
use crate::daemon::process::proxy::{ProcessStatus, ProxyInfo};
use dashmap::DashMap;
use regex::Regex;
use ringbuf::traits::RingBuffer;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::oneshot;
use tracing::{error, info, debug, warn};
use uuid::Uuid;

pub struct ProcessManager {
    processes: Arc<DashMap<String, Arc<ProxyInfo>>>,
    config: Arc<Config>,
    log_hub: Arc<LogHub>,
}

impl ProcessManager {
    pub fn new(config: Arc<Config>, log_hub: Arc<LogHub>) -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
            config,
            log_hub,
        }
    }
    
    #[allow(clippy::too_many_arguments)]
    pub async fn start_process(
        &self,
        name: String,
        project: Option<String>,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<std::collections::HashMap<String, String>>,
        wait_for_log: Option<String>,
        wait_timeout: Option<u32>,
    ) -> Result<Arc<ProxyInfo>> {
        let (proxy, _timeout) = self.start_process_with_log_stream(
            name,
            project,
            cmd,
            args,
            cwd,
            env,
            wait_for_log,
            wait_timeout,
            None,
        ).await?;
        Ok(proxy)
    }
    
    pub async fn start_process_with_log_stream(
        &self,
        name: String,
        project: Option<String>,
        cmd: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Option<std::collections::HashMap<String, String>>,
        wait_for_log: Option<String>,
        wait_timeout: Option<u32>,
        log_stream_tx: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> Result<(Arc<ProxyInfo>, bool)> {
        let project = project.expect("Project name must be provided");
        
        // Create unique key for process: project/name
        let process_key = format!("{}/{}", &project, &name);
        
        // Check if process already exists
        if let Some(existing) = self.processes.get(&process_key) {
            if matches!(existing.get_status(), ProcessStatus::Running | ProcessStatus::Starting) {
                info!("Process {}/{} already running", project, name);
                return Err(McprocdError::ProcessAlreadyExists(format!("{}/{}", project, name)));
            }
        }
        
        let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap());
        let log_file = self.config.log.dir.join(format!("{}_{}.log", 
            project.replace("/", "_"), // Sanitize project name for filesystem
            name
        ));
        
        // Determine command string for ProxyInfo
        let cmd_string = if let Some(cmd) = cmd.clone() {
            cmd
        } else if !args.is_empty() {
            args.join(" ")
        } else {
            return Err(McprocdError::SpawnError("No command provided".to_string()));
        };
        
        let mut proxy = ProxyInfo::new(name.clone(), project.clone(), cmd_string.clone(), cwd.clone(), log_file);
        
        // Create command based on whether cmd or args was provided
        let mut command = if let Some(cmd) = cmd {
            // Use shell to execute the command
            if cfg!(unix) {
                let mut shell_cmd = Command::new("sh");
                shell_cmd.arg("-c");
                shell_cmd.arg(&cmd);
                shell_cmd
            } else {
                let mut shell_cmd = Command::new("cmd");
                shell_cmd.arg("/C");
                shell_cmd.arg(&cmd);
                shell_cmd
            }
        } else if !args.is_empty() {
            // Direct execution without shell
            let mut direct_cmd = Command::new(&args[0]);
            if args.len() > 1 {
                direct_cmd.args(&args[1..]);
            }
            direct_cmd
        } else {
            return Err(McprocdError::SpawnError("No command provided".to_string()));
        };
        
        command
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());
        
        if let Some(env_vars) = env {
            for (key, value) in env_vars {
                command.env(key, value);
            }
        }
        
        let mut child = command.spawn()
            .map_err(|e| McprocdError::SpawnError(format!("Failed to spawn process: {}", e)))?;
        
        proxy.pid = child.id();
        proxy.set_status(ProcessStatus::Running);
        
        // Write startup information to log file immediately
        let startup_info = format!(
            "=== Process Started ===\nProject: {}\nName: {}\nCommand: {}\nWorking Directory: {}\nPID: {:?}\nStart Time: {}\n===================\n",
            project, name, cmd_string, cwd.display(), proxy.pid, proxy.start_time.format("%Y-%m-%d %H:%M:%S UTC")
        );
        if let Err(e) = self.log_hub.append_log(&process_key, startup_info.as_bytes(), false).await {
            error!("Failed to write startup log for {}: {}", process_key, e);
        }
        
        let proxy_arc = Arc::new(proxy);
        self.processes.insert(process_key.clone(), proxy_arc.clone());
        
        // Setup log wait channel if pattern is provided
        let log_ready_tx = if wait_for_log.is_some() {
            let (tx, rx) = oneshot::channel();
            let shared_tx = Arc::new(Mutex::new(Some(tx)));
            Some((shared_tx, rx))
        } else {
            None
        };
        
        // Create a shared flag to track pattern match
        let pattern_matched = Arc::new(Mutex::new(false));
        
        // Create a shared flag to track timeout
        let timeout_occurred = Arc::new(Mutex::new(false));
        
        // Wrap log_stream_tx in Arc<Mutex> for shared access
        let log_stream_tx_shared = log_stream_tx.map(|tx| Arc::new(Mutex::new(Some(tx))));
        
        // Compile regex pattern if provided (case-insensitive)
        let log_pattern = if let Some(pattern) = &wait_for_log {
            // Prepend (?i) to make the pattern case-insensitive
            let case_insensitive_pattern = format!("(?i){}", pattern);
            match Regex::new(&case_insensitive_pattern) {
                Ok(regex) => Some(regex),
                Err(e) => {
                    warn!("Invalid log pattern '{}': {}", pattern, e);
                    None
                }
            }
        } else {
            None
        };
        
        // Setup stdout/stderr capture
        let stdout = child.stdout.take().expect("stdout should be captured");
        let stderr = child.stderr.take().expect("stderr should be captured");
        
        let log_hub = self.log_hub.clone();
        let proxy_stdout = proxy_arc.clone();
        let log_key_stdout = process_key.clone();
        let log_pattern_stdout = log_pattern.clone();
        let log_ready_tx_stdout = log_ready_tx.as_ref().map(|(tx, _)| tx.clone());
        let log_stream_tx_stdout = log_stream_tx_shared.clone();
        let pattern_matched_stdout = pattern_matched.clone();
        let wait_timeout_stdout = wait_timeout;
        let has_pattern_stdout = log_pattern_stdout.is_some();
        let timeout_occurred_stdout = timeout_occurred.clone();
        
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            
            // Set up timeout future if we're waiting for a pattern
            let timeout_future = if has_pattern_stdout {
                let duration = tokio::time::Duration::from_secs(wait_timeout_stdout.unwrap_or(30) as u64);
                tokio::time::sleep(duration)
            } else {
                tokio::time::sleep(tokio::time::Duration::from_secs(u64::MAX)) // Never timeout
            };
            tokio::pin!(timeout_future);
            
            loop {
                tokio::select! {
                    // Check for timeout
                    _ = &mut timeout_future, if has_pattern_stdout => {
                        warn!("Log streaming timeout reached for stdout");
                        // Mark timeout occurred
                        if let Ok(mut timeout_flag) = timeout_occurred_stdout.lock() {
                            *timeout_flag = true;
                        }
                        // Close the channel on timeout
                        if let Some(ref tx_shared) = log_stream_tx_stdout {
                            if let Ok(mut guard) = tx_shared.lock() {
                                guard.take();
                            }
                        }
                        break;
                    }
                        // Read next line
                        line_result = lines.next_line() => {
                            match line_result {
                                Ok(Some(line)) => {
                                    if let Err(e) = log_hub.append_log(&log_key_stdout, line.as_bytes(), false).await {
                                        error!("Failed to write stdout log for {}: {}", log_key_stdout, e);
                                    }
                                    
                                    // Send log to stream if provided
                                    if let Some(ref tx_shared) = log_stream_tx_stdout {
                                        let tx_opt = tx_shared.lock().ok().and_then(|guard| guard.clone());
                                        if let Some(tx) = tx_opt {
                                            let _ = tx.send(line.clone()).await;
                                        }
                                    }
                                    
                                    // Check if line matches the wait pattern
                                    if let (Some(ref pattern), Some(ref tx)) = (&log_pattern_stdout, &log_ready_tx_stdout) {
                                        if pattern.is_match(&line) {
                                            debug!("Found log pattern match: {}", line);
                                            if let Ok(mut tx_guard) = tx.lock() {
                                                if let Some(sender) = tx_guard.take() {
                                                    let _ = sender.send(());
                                                }
                                            }
                                            // Mark pattern as matched
                                            if let Ok(mut matched) = pattern_matched_stdout.lock() {
                                                *matched = true;
                                            }
                                            // Close the channel to signal completion
                                            if let Some(ref tx_shared) = log_stream_tx_stdout {
                                                if let Ok(mut guard) = tx_shared.lock() {
                                                    guard.take(); // Drop the sender by taking it out
                                                }
                                            }
                                            // Stop streaming logs
                                            break;
                                        }
                                    }
                                    
                                    if let Ok(mut ring) = proxy_stdout.ring.lock() {
                                        let _ = ring.push_overwrite(line.into_bytes());
                                    }
                                }
                                Ok(None) => break, // EOF
                                Err(_) => break,
                            }
                        }
                    }
            }
        });
        
        let log_hub_stderr = self.log_hub.clone();
        let proxy_stderr = proxy_arc.clone();
        let log_key_stderr = process_key.clone();
        let log_pattern_stderr = log_pattern;
        let log_ready_tx_stderr = log_ready_tx.as_ref().map(|(tx, _)| tx.clone());
        let log_stream_tx_stderr = log_stream_tx_shared.clone();
        let pattern_matched_stderr = pattern_matched.clone();
        let wait_timeout_stderr = wait_timeout;
        let has_pattern_stderr = log_pattern_stderr.is_some();
        let timeout_occurred_stderr = timeout_occurred.clone();
        
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            
            // Set up timeout future if we're waiting for a pattern
            let timeout_future = if has_pattern_stderr {
                let duration = tokio::time::Duration::from_secs(wait_timeout_stderr.unwrap_or(30) as u64);
                tokio::time::sleep(duration)
            } else {
                tokio::time::sleep(tokio::time::Duration::from_secs(u64::MAX)) // Never timeout
            };
            tokio::pin!(timeout_future);
            
            loop {
                tokio::select! {
                    // Check for timeout
                    _ = &mut timeout_future, if has_pattern_stderr => {
                        warn!("Log streaming timeout reached for stderr");
                        // Mark timeout occurred
                        if let Ok(mut timeout_flag) = timeout_occurred_stderr.lock() {
                            *timeout_flag = true;
                        }
                        // Close the channel on timeout
                        if let Some(ref tx_shared) = log_stream_tx_stderr {
                            if let Ok(mut guard) = tx_shared.lock() {
                                guard.take();
                            }
                        }
                        break;
                    }
                        // Read next line
                        line_result = lines.next_line() => {
                            match line_result {
                                Ok(Some(line)) => {
                if let Err(e) = log_hub_stderr.append_log(&log_key_stderr, line.as_bytes(), true).await {
                    error!("Failed to write stderr log for {}: {}", log_key_stderr, e);
                }
                
                // Send log to stream if provided
                if let Some(ref tx_shared) = log_stream_tx_stderr {
                    let tx_opt = tx_shared.lock().ok().and_then(|guard| guard.clone());
                    if let Some(tx) = tx_opt {
                        let _ = tx.send(line.clone()).await;
                    }
                }
                
                // Check if line matches the wait pattern
                if let (Some(ref pattern), Some(ref tx)) = (&log_pattern_stderr, &log_ready_tx_stderr) {
                    if pattern.is_match(&line) {
                        debug!("Found log pattern match in stderr: {}", line);
                        if let Ok(mut tx_guard) = tx.lock() {
                            if let Some(sender) = tx_guard.take() {
                                let _ = sender.send(());
                            }
                        }
                        // Mark pattern as matched
                        if let Ok(mut matched) = pattern_matched_stderr.lock() {
                            *matched = true;
                        }
                        // Close the channel to signal completion
                        if let Some(ref tx_shared) = log_stream_tx_stderr {
                            if let Ok(mut guard) = tx_shared.lock() {
                                guard.take(); // Drop the sender by taking it out
                            }
                        }
                        // Stop streaming logs
                        break;
                    }
                }
                
                                    if let Ok(mut ring) = proxy_stderr.ring.lock() {
                                        let _ = ring.push_overwrite(line.into_bytes());
                                    }
                                }
                                Ok(None) => break, // EOF
                                Err(_) => break,
                            }
                        }
                    }
            }
        });
        
        // Monitor process
        let processes = self.processes.clone();
        let proxy_monitor = proxy_arc.clone();
        let name_clone = name.clone();
        let log_hub_monitor = self.log_hub.clone();
        let log_key_monitor = process_key.clone();
        
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    info!("Process {} exited with status: {:?}", name_clone, status);
                    
                    // Write exit information to log
                    let exit_info = format!(
                        "\n=== Process Exited ===\nName: {}\nExit Status: {:?}\nExit Time: {}\n===================\n",
                        name_clone,
                        status,
                        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    if let Err(e) = log_hub_monitor.append_log(&log_key_monitor, exit_info.as_bytes(), false).await {
                        error!("Failed to write exit log for {}: {}", log_key_monitor, e);
                    }
                    
                    proxy_monitor.set_status(if status.success() {
                        ProcessStatus::Stopped
                    } else {
                        ProcessStatus::Failed
                    });
                }
                Err(e) => {
                    error!("Failed to wait for process {}: {}", name_clone, e);
                    
                    // Write error information to log
                    let error_info = format!(
                        "\n=== Process Error ===\nName: {}\nError: {}\nTime: {}\n===================\n",
                        name_clone,
                        e,
                        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    if let Err(e) = log_hub_monitor.append_log(&log_key_monitor, error_info.as_bytes(), true).await {
                        error!("Failed to write error log for {}: {}", log_key_monitor, e);
                    }
                    
                    proxy_monitor.set_status(ProcessStatus::Failed);
                }
            }
            
            // Close the log file handle
            log_hub_monitor.close_log(&log_key_monitor).await;
            
            // Remove from active processes
            processes.remove(&log_key_monitor);
        });
        
        // Start port detection task
        if let Some(pid) = proxy_arc.pid {
            let proxy_port = proxy_arc.clone();
            tokio::spawn(async move {
                // Initial delay to let the process start up (Next.js needs more time)
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                
                // Detect ports periodically
                let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
                let mut consecutive_checks = 0;
                let mut total_checks = 0;
                
                loop {
                    interval.tick().await;
                    
                    // Stop checking if process is no longer running
                    if !matches!(proxy_port.get_status(), ProcessStatus::Running) {
                        break;
                    }
                    
                    let detected_ports = port_detector::detect_ports(pid);
                    
                    // Update ports if changed
                    if let Ok(mut ports) = proxy_port.ports.lock() {
                        if *ports != detected_ports {
                            debug!("Updated ports for PID {}: {:?}", pid, detected_ports);
                            *ports = detected_ports.clone();
                        }
                        
                        // Stop checking after finding stable ports
                        if !detected_ports.is_empty() {
                            consecutive_checks += 1;
                            if consecutive_checks >= 3 {
                                debug!("Port detection stabilized for PID {}, stopping checks", pid);
                                break;
                            }
                        } else {
                            consecutive_checks = 0;
                        }
                    }
                    
                    total_checks += 1;
                    
                    // Stop checking after 90 seconds (30 checks * 3 seconds)
                    if total_checks >= 30 {
                        debug!("Port detection timeout for PID {} after {} checks", pid, total_checks);
                        break;
                    }
                }
            });
        }
        
        // Wait for log pattern if specified
        if let Some((_, rx)) = log_ready_tx {
            let timeout_duration = tokio::time::Duration::from_secs(wait_timeout.unwrap_or(30) as u64);
            
            match tokio::time::timeout(timeout_duration, rx).await {
                Ok(Ok(())) => {
                    info!("Process {} is ready (log pattern matched)", name);
                }
                Ok(Err(_)) => {
                    warn!("Log wait channel closed for process {}", name);
                }
                Err(_) => {
                    warn!("Timeout waiting for log pattern for process {} after {}s", name, timeout_duration.as_secs());
                    // Mark timeout occurred
                    if let Ok(mut timeout_flag) = timeout_occurred.lock() {
                        *timeout_flag = true;
                    }
                    // Close the log stream channel on timeout
                    if let Some(ref tx_shared) = log_stream_tx_shared {
                        if let Ok(mut guard) = tx_shared.lock() {
                            guard.take(); // Drop the sender to close the channel
                        }
                    }
                }
            }
        }
        
        info!("Started process {} with PID {:?}", name, proxy_arc.pid);
        
        // Add timeout status to the proxy info if it's available
        if let Ok(timeout_flag) = timeout_occurred.lock() {
            if *timeout_flag {
                // Store timeout status in proxy (we'll need to modify ProxyInfo struct)
                // For now, we'll handle this in the gRPC layer
            }
        }
        
        Ok((proxy_arc, timeout_occurred.lock().map(|g| *g).unwrap_or(false)))
    }
    
    pub async fn stop_process(&self, name_or_id: &str, project: Option<&str>, force: bool) -> Result<()> {
        let process = self.get_process_by_name_or_id_with_project(name_or_id, project)
            .ok_or_else(|| McprocdError::ProcessNotFound(name_or_id.to_string()))?;
        
        process.set_status(ProcessStatus::Stopping);
        
        if let Some(pid) = process.pid {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;
            
            let signal = if force { Signal::SIGKILL } else { Signal::SIGTERM };
            
            match signal::kill(Pid::from_raw(pid as i32), signal) {
                Ok(()) => {
                    info!("Sent {} to process {} (PID {})", signal, process.name, pid);
                    // Remove from processes map after sending signal
                    if force {
                        let process_key = format!("{}/{}", process.project, process.name);
                        self.processes.remove(&process_key);
                    }
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to send signal to process {}: {}", process.name, e);
                    Err(McprocdError::StopError(format!("Failed to stop process: {}", e)))
                }
            }
        } else {
            Err(McprocdError::StopError("Process has no PID".to_string()))
        }
    }
    
    pub async fn restart_process(&self, name_or_id: &str, project: Option<String>) -> Result<Arc<ProxyInfo>> {
        if let Some(process) = self.get_process_by_name_or_id_with_project(name_or_id, project.as_deref()) {
            let name = process.name.clone();
            let project = process.project.clone();
            let cmd = process.cmd.clone();
            let cwd = process.cwd.clone();
            drop(process);
            
            self.stop_process(name_or_id, Some(&project), false).await?;
            
            // Wait a bit for process to stop
            tokio::time::sleep(tokio::time::Duration::from_millis(self.config.process.restart_delay_ms)).await;
            
            // For restart, we use the original command as a shell command
            self.start_process(name, Some(project), Some(cmd), vec![], Some(cwd), None, None, None).await
        } else {
            Err(McprocdError::ProcessNotFound(name_or_id.to_string()))
        }
    }
    
    #[allow(dead_code)]
    pub fn get_process(&self, name: &str) -> Option<Arc<ProxyInfo>> {
        self.processes.get(name).map(|p| p.clone())
    }
    
    #[allow(dead_code)]
    pub fn get_process_by_id(&self, id: &str) -> Option<Arc<ProxyInfo>> {
        if let Ok(uuid) = Uuid::parse_str(id) {
            for entry in self.processes.iter() {
                if entry.value().id == uuid {
                    return Some(entry.value().clone());
                }
            }
        }
        None
    }
    
    #[allow(dead_code)]
    pub fn get_process_by_name_or_id(&self, name_or_id: &str) -> Option<Arc<ProxyInfo>> {
        // First try as name
        if let Some(process) = self.get_process(name_or_id) {
            return Some(process);
        }
        // Then try as ID
        self.get_process_by_id(name_or_id)
    }
    
    pub fn get_process_by_name_or_id_with_project(&self, name_or_id: &str, project: Option<&str>) -> Option<Arc<ProxyInfo>> {
        // If project is provided, try project/name first
        if let Some(proj) = project {
            let key = format!("{}/{}", proj, name_or_id);
            if let Some(process) = self.processes.get(&key) {
                return Some(process.clone());
            }
        }
        
        // Try to find by ID
        if let Ok(uuid) = Uuid::parse_str(name_or_id) {
            for entry in self.processes.iter() {
                if entry.value().id == uuid {
                    return Some(entry.value().clone());
                }
            }
        }
        
        // If no project specified, try to find by name in any project
        if project.is_none() {
            for entry in self.processes.iter() {
                if entry.value().name == name_or_id {
                    return Some(entry.value().clone());
                }
            }
        }
        
        None
    }
    
    pub fn list_processes(&self) -> Vec<Arc<ProxyInfo>> {
        self.processes.iter().map(|entry| entry.value().clone()).collect()
    }
}
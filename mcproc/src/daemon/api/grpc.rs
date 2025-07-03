use crate::common::config::Config;
use crate::common::exit_code::format_exit_reason;
use crate::common::version::VERSION;
use crate::daemon::log::LogHub;
use crate::daemon::process::proxy::LogChunk;
use crate::daemon::process::{ProcessManager, ProcessStatus};
use crate::daemon::stream::{SharedStreamEventHub, StreamEvent, StreamFilter};
use chrono::Utc;
use proto::process_manager_server::{
    ProcessManager as ProcessManagerService, ProcessManagerServer,
};
use proto::*;
use ringbuf::traits::Consumer;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{transport::Server, Request, Response, Status};
use tracing::{debug, error, info};
use uuid::Uuid;

pub struct GrpcService {
    process_manager: Arc<ProcessManager>,
    log_hub: Arc<LogHub>,
    config: Arc<Config>,
    event_hub: SharedStreamEventHub,
    start_time: chrono::DateTime<Utc>,
}

impl GrpcService {
    pub fn new(
        process_manager: Arc<ProcessManager>,
        log_hub: Arc<LogHub>,
        config: Arc<Config>,
        event_hub: SharedStreamEventHub,
    ) -> Self {
        Self {
            process_manager,
            log_hub,
            config,
            event_hub,
            start_time: Utc::now(),
        }
    }
}

#[tonic::async_trait]
impl ProcessManagerService for GrpcService {
    type StartProcessStream =
        Pin<Box<dyn Stream<Item = Result<StartProcessResponse, Status>> + Send>>;

    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<Self::StartProcessStream>, Status> {
        let req = request.into_inner();

        // Validate process name
        if let Err(e) = crate::common::validation::validate_process_name(&req.name) {
            return Err(Status::invalid_argument(format!(
                "Invalid process name: {}",
                e
            )));
        }

        // Validate project name
        if let Err(e) = crate::common::validation::validate_project_name(&req.project) {
            return Err(Status::invalid_argument(format!(
                "Invalid project name: {}",
                e
            )));
        }

        let cwd = req.cwd.map(std::path::PathBuf::from);

        let name = req.name.clone();
        let project = req.project.clone();
        let wait_for_log = req.wait_for_log.clone();
        let wait_timeout = req.wait_timeout;
        let cmd_for_error = req.cmd.clone(); // Clone for use in error handling
        let cwd_for_error = cwd.clone(); // Clone for use in error handling
        let log_dir = self.config.paths.log_dir.clone(); // Clone for use in async block
        let force_restart = req.force_restart.unwrap_or(false);

        // We no longer need log streaming channel since log_context is included in ProcessInfo

        let process_manager = self.process_manager.clone();
        let _log_hub = self.log_hub.clone();

        // Handle force_restart
        if force_restart {
            if let Some(existing) = process_manager
                .get_process_by_name_or_id_with_project(&name, Some(project.as_str()))
            {
                // Stop existing process
                let _ = process_manager
                    .stop_process(&existing.id, Some(project.as_str()), false)
                    .await;

                // Wait a bit for process to stop
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        // Create the response stream
        let stream = async_stream::try_stream! {
            // Start the process with log streaming
            match process_manager.start_process_with_log_stream(
                name.clone(),
                Some(project.clone()),
                req.cmd,
                req.args,
                cwd,
                Some(req.env),
                wait_for_log.clone(),
                wait_timeout,
                req.toolchain,
            ).await {
                Ok((process, timeout_occurred, _pattern_matched, log_context, matched_line)) => {
                    // We no longer stream logs during startup
                    // Log context is now included in ProcessInfo

                    // Send final process info
                    let current_status = process.get_status();

                    // Get exit details without blocking
                    let (exit_code, exit_reason, stderr_tail) = match process.exit_code.try_lock() {
                        Ok(code_guard) => {
                            if let Some(code) = *code_guard {
                                let reason = Some(format_exit_reason(code));
                                let stderr = process.ring.try_lock().ok().map(|ring| {
                                    ring.iter()
                                        .take(5)
                                        .map(|chunk| String::from_utf8_lossy(&chunk.data).to_string())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                }).unwrap_or_default();
                                (Some(code), reason, Some(stderr))
                            } else {
                                (None, None, None)
                            }
                        }
                        Err(_) => {
                            // Don't block if mutex is locked
                            (None, None, None)
                        }
                    };

                    // Get detected ports (non-blocking)
                    let ports = if let Some(port) = process.port {
                        vec![port as u32]
                    } else if let Ok(detected) = process.detected_port.try_lock() {
                        detected.map(|p| vec![p as u32]).unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    let info = ProcessInfo {
                        id: process.id.clone(),
                        name: process.name.clone(),
                        cmd: process.cmd.clone().unwrap_or_default(),
                        cwd: process.cwd.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                        status: proto::ProcessStatus::from(current_status).into(),
                        start_time: Some(prost_types::Timestamp {
                            seconds: process.start_time.timestamp(),
                            nanos: process.start_time.timestamp_subsec_nanos() as i32,
                        }),
                        pid: Some(process.pid),
                        log_file: log_dir.join(format!("{}-{}.log", process.project, process.name)).to_string_lossy().to_string(),
                        project: process.project.clone(),
                        ports,
                        wait_timeout_occurred: if wait_for_log.is_some() { Some(timeout_occurred) } else { None },
                        exit_code,
                        exit_reason,
                        stderr_tail,
                        log_context,
                        matched_line,
                    };

                    yield StartProcessResponse {
                        response: Some(start_process_response::Response::Process(info)),
                    };
                }
                Err(e) => {
                    // Return ProcessInfo with failed status instead of error
                    match &e {
                        crate::daemon::error::McprocdError::ProcessAlreadyExists(name) => {
                            // For already exists, still return as error for backward compatibility
                            let status = Status::already_exists(format!("Process '{}' is already running", name));
                            Err(status)?;
                        }
                        crate::daemon::error::McprocdError::ProcessFailedToStart { name, exit_code, exit_reason, stderr } => {
                            error!("Process '{}' failed to start: {} (exit code: {})", name, exit_reason, exit_code);

                            // Create a failed ProcessInfo
                            let failed_info = ProcessInfo {
                                id: Uuid::new_v4().to_string(),
                                name: name.clone(),
                                cmd: cmd_for_error.clone().unwrap_or_default(),
                                cwd: cwd_for_error.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                                status: proto::ProcessStatus::Failed as i32,
                                start_time: Some(prost_types::Timestamp {
                                    seconds: chrono::Utc::now().timestamp(),
                                    nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                }),
                                pid: None,
                                log_file: log_dir
                                    .join(&project)
                                    .join(format!("{}.log", name.replace("/", "_")))
                                    .to_string_lossy().to_string(),
                                project: project.clone(),
                                ports: vec![],
                                wait_timeout_occurred: None,
                                exit_code: Some(*exit_code),
                                exit_reason: Some(exit_reason.clone()),
                                stderr_tail: Some(stderr.clone()),
                                log_context: vec![],
                                matched_line: None,
                            };

                            yield StartProcessResponse {
                                response: Some(start_process_response::Response::Process(failed_info)),
                            };
                        }
                        _ => {
                            // Other errors still return as status errors
                            let status = Status::internal(e.to_string());
                            Err(status)?;
                        }
                    };
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn stop_process(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        let req = request.into_inner();

        match self
            .process_manager
            .stop_process(
                &req.name,
                Some(req.project.as_str()),
                req.force.unwrap_or(false),
            )
            .await
        {
            Ok(()) => Ok(Response::new(StopProcessResponse {
                success: true,
                message: None,
            })),
            Err(e) => Ok(Response::new(StopProcessResponse {
                success: false,
                message: Some(e.to_string()),
            })),
        }
    }

    async fn restart_process(
        &self,
        request: Request<RestartProcessRequest>,
    ) -> Result<Response<RestartProcessResponse>, Status> {
        let req = request.into_inner();

        match self
            .process_manager
            .restart_process_with_log_stream(
                &req.name,
                Some(req.project),
                req.wait_for_log,
                req.wait_timeout,
            )
            .await
        {
            Ok((process, timeout_occurred, _pattern_matched, log_context, matched_line)) => {
                // Get detected ports
                let ports = if let Some(port) = process.port {
                    vec![port as u32]
                } else if let Ok(detected) = process.detected_port.lock() {
                    detected.map(|p| vec![p as u32]).unwrap_or_default()
                } else {
                    Vec::new()
                };

                // Check if process failed during restart
                let current_status = process.get_status();
                let (exit_code, exit_reason, stderr_tail) =
                    if matches!(current_status, ProcessStatus::Failed) {
                        let code = *process.exit_code.lock().unwrap();
                        let reason = code.map(format_exit_reason);
                        let stderr = process
                            .ring
                            .lock()
                            .ok()
                            .map(|ring| {
                                ring.iter()
                                    .take(5)
                                    .map(|chunk| String::from_utf8_lossy(&chunk.data).to_string())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .unwrap_or_default();
                        (code, reason, Some(stderr))
                    } else {
                        (None, None, None)
                    };

                let info = ProcessInfo {
                    id: process.id.clone(),
                    name: process.name.clone(),
                    cmd: process.cmd.clone().unwrap_or_default(),
                    cwd: process
                        .cwd
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    status: proto::ProcessStatus::from(current_status).into(),
                    start_time: Some(prost_types::Timestamp {
                        seconds: process.start_time.timestamp(),
                        nanos: process.start_time.timestamp_subsec_nanos() as i32,
                    }),
                    pid: Some(process.pid),
                    log_file: self
                        .config
                        .paths
                        .log_dir
                        .join(format!("{}-{}.log", process.project, process.name))
                        .to_string_lossy()
                        .to_string(),
                    project: process.project.clone(),
                    ports,
                    wait_timeout_occurred: if process.wait_for_log.is_some() {
                        Some(timeout_occurred)
                    } else {
                        None
                    },
                    exit_code,
                    exit_reason,
                    stderr_tail,
                    log_context,
                    matched_line,
                };

                Ok(Response::new(RestartProcessResponse {
                    process: Some(info),
                }))
            }
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn get_process(
        &self,
        request: Request<GetProcessRequest>,
    ) -> Result<Response<GetProcessResponse>, Status> {
        let req = request.into_inner();

        match self
            .process_manager
            .get_process_by_name_or_id_with_project(&req.name, Some(req.project.as_str()))
        {
            Some(process) => {
                // Get detected ports
                let ports = if let Some(port) = process.port {
                    vec![port as u32]
                } else if let Ok(detected) = process.detected_port.lock() {
                    detected.map(|p| vec![p as u32]).unwrap_or_default()
                } else {
                    Vec::new()
                };

                let info = ProcessInfo {
                    id: process.id.clone(),
                    name: process.name.clone(),
                    cmd: process.cmd.clone().unwrap_or_default(),
                    cwd: process
                        .cwd
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    status: match process.get_status() {
                        ProcessStatus::Starting => proto::ProcessStatus::Starting as i32,
                        ProcessStatus::Running => proto::ProcessStatus::Running as i32,
                        ProcessStatus::Stopping => proto::ProcessStatus::Stopping as i32,
                        ProcessStatus::Stopped => proto::ProcessStatus::Stopped as i32,
                        ProcessStatus::Failed => proto::ProcessStatus::Failed as i32,
                    },
                    start_time: Some(prost_types::Timestamp {
                        seconds: process.start_time.timestamp(),
                        nanos: process.start_time.timestamp_subsec_nanos() as i32,
                    }),
                    pid: Some(process.pid),
                    log_file: self
                        .config
                        .paths
                        .log_dir
                        .join(format!("{}-{}.log", process.project, process.name))
                        .to_string_lossy()
                        .to_string(),
                    project: process.project.clone(),
                    ports,
                    wait_timeout_occurred: None,
                    exit_code: None,
                    exit_reason: None,
                    stderr_tail: None,
                    log_context: vec![],
                    matched_line: None,
                };

                Ok(Response::new(GetProcessResponse {
                    process: Some(info),
                }))
            }
            None => Err(Status::not_found("Process not found")),
        }
    }

    async fn list_processes(
        &self,
        request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        let req = request.into_inner();
        let mut processes = self.process_manager.list_processes();

        // Filter by project if specified
        if let Some(project_filter) = req.project_filter {
            processes.retain(|p| p.project == project_filter);
        }

        let log_dir = self.config.paths.log_dir.clone();
        let process_infos: Vec<ProcessInfo> = processes
            .into_iter()
            .map(|process| {
                // Get detected ports
                let ports = if let Some(port) = process.port {
                    vec![port as u32]
                } else if let Ok(detected) = process.detected_port.lock() {
                    detected.map(|p| vec![p as u32]).unwrap_or_default()
                } else {
                    Vec::new()
                };

                ProcessInfo {
                    id: process.id.clone(),
                    name: process.name.clone(),
                    cmd: process.cmd.clone().unwrap_or_default(),
                    cwd: process
                        .cwd
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    status: match process.get_status() {
                        ProcessStatus::Starting => proto::ProcessStatus::Starting as i32,
                        ProcessStatus::Running => proto::ProcessStatus::Running as i32,
                        ProcessStatus::Stopping => proto::ProcessStatus::Stopping as i32,
                        ProcessStatus::Stopped => proto::ProcessStatus::Stopped as i32,
                        ProcessStatus::Failed => proto::ProcessStatus::Failed as i32,
                    },
                    start_time: Some(prost_types::Timestamp {
                        seconds: process.start_time.timestamp(),
                        nanos: process.start_time.timestamp_subsec_nanos() as i32,
                    }),
                    pid: Some(process.pid),
                    log_file: log_dir
                        .join(format!("{}-{}.log", process.project, process.name))
                        .to_string_lossy()
                        .to_string(),
                    project: process.project.clone(),
                    ports,
                    wait_timeout_occurred: None,
                    exit_code: None,
                    exit_reason: None,
                    stderr_tail: None,
                    log_context: vec![],
                    matched_line: None,
                }
            })
            .collect();

        Ok(Response::new(ListProcessesResponse {
            processes: process_infos,
        }))
    }

    type GetLogsStream = Pin<Box<dyn Stream<Item = Result<GetLogsResponse, Status>> + Send>>;

    async fn get_logs(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        let req = request.into_inner();

        let project = req.project.clone();
        let process_names = req.process_names.clone();
        let tail = req.tail.unwrap_or(100) as usize;
        let follow = req.follow.unwrap_or(false);
        let include_events = req.include_events.unwrap_or(false);

        // Log the request details for debugging
        info!(
            "get_logs request: project={}, process_names={:?}, tail={}, follow={}, include_events={}",
            project, process_names, tail, follow, include_events
        );

        // Create filter based on request
        let filter = StreamFilter {
            project: Some(project.clone()),
            process_names: process_names.clone(),
            include_events,
        };

        // Subscribe to event hub
        let mut event_receiver = self.event_hub.subscribe();

        // For tail functionality, read existing logs from files first
        let log_hub = self.log_hub.clone();
        let process_manager = self.process_manager.clone();

        // Create stream
        let stream = async_stream::try_stream! {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::fs::File;
            use crate::common::process_key::ProcessKey;

            // First, send existing logs if tail is requested
            if tail > 0 {
                info!("Reading tail logs (tail={})", tail);
                // Get list of processes matching the filter
                let processes = process_manager.list_processes();
                info!("Found {} total processes", processes.len());
                let matching_processes: Vec<_> = processes
                    .into_iter()
                    .filter(|p| filter.matches_process(&p.project, &p.name))
                    .collect();
                info!("Found {} matching processes", matching_processes.len());

                // Read tail lines from each matching process's log file
                for process_info in matching_processes {
                    let key = ProcessKey {
                        name: process_info.name.clone(),
                        project: process_info.project.clone(),
                    };
                    let log_file = log_hub.get_log_file_path_for_key(&key);

                    if log_file.exists() {
                        match File::open(&log_file).await {
                            Ok(file) => {
                                let reader = BufReader::new(file);
                                let mut lines = reader.lines();
                                let mut all_lines = Vec::new();

                                // Read all lines
                                while let Ok(Some(line)) = lines.next_line().await {
                                    all_lines.push(line);
                                }

                                // Get tail lines
                                let start_idx = all_lines.len().saturating_sub(tail);
                                let mut line_num = start_idx as u32;

                                for line in &all_lines[start_idx..] {
                                    line_num += 1;
                                    let (timestamp, level, content) = parse_log_line(line);

                                    let log_entry = LogEntry {
                                        line_number: line_num,
                                        content,
                                        timestamp,
                                        level: level as i32,
                                        process_name: Some(process_info.name.clone()),
                                    };

                                    yield GetLogsResponse {
                                        content: Some(proto::get_logs_response::Content::LogEntry(log_entry)),
                                    };
                                }
                            }
                            Err(e) => {
                                error!("Failed to open log file for {}/{}: {}",
                                    process_info.project, process_info.name, e);
                            }
                        }
                    }
                }
            }

            // If follow mode, subscribe to event hub for new logs
            if follow {
                info!("Starting follow mode for filter: project={:?}, process_names={:?}",
                    filter.project, filter.process_names);
                loop {
                    tokio::select! {
                        event = event_receiver.recv() => {
                            match event {
                                Ok(stream_event) => {
                                    // Check if event matches filter
                                    if !filter.matches(&stream_event) {
                                        continue;
                                    }
                                    debug!("Received matching event: {:?}", stream_event);

                                    match stream_event {
                                        StreamEvent::Log { process_name, entry, .. } => {
                                            // Set process_name in log entry
                                            let mut log_entry = entry;
                                            log_entry.process_name = Some(process_name);

                                            yield GetLogsResponse {
                                                content: Some(proto::get_logs_response::Content::LogEntry(log_entry)),
                                            };
                                        }
                                        StreamEvent::Process(event) => {
                                            if include_events {
                                                // Convert ProcessEvent to ProcessLifecycleEvent
                                                let lifecycle_event = match event {
                                                    crate::daemon::process::event::ProcessEvent::Starting { process_id, name, project } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Starting as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: Some(prost_types::Timestamp {
                                                                seconds: chrono::Utc::now().timestamp(),
                                                                nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                                            }),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Started { process_id, name, project, pid } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Started as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: Some(pid),
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: Some(prost_types::Timestamp {
                                                                seconds: chrono::Utc::now().timestamp(),
                                                                nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                                            }),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Stopping { process_id, name, project } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Stopping as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: Some(prost_types::Timestamp {
                                                                seconds: chrono::Utc::now().timestamp(),
                                                                nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                                            }),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Stopped { process_id, name, project, exit_code } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Stopped as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code,
                                                            error: None,
                                                            timestamp: Some(prost_types::Timestamp {
                                                                seconds: chrono::Utc::now().timestamp(),
                                                                nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                                            }),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Failed { process_id, name, project, error } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Failed as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: Some(error),
                                                            timestamp: Some(prost_types::Timestamp {
                                                                seconds: chrono::Utc::now().timestamp(),
                                                                nanos: chrono::Utc::now().timestamp_subsec_nanos() as i32,
                                                            }),
                                                        }
                                                    }
                                                };

                                                yield GetLogsResponse {
                                                    content: Some(proto::get_logs_response::Content::Event(lifecycle_event)),
                                                };
                                            }
                                        }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                                    // We missed some events due to lag
                                    error!("Event receiver lagged by {} events", count);
                                    // Continue processing
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    // Event hub closed, exit
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn grep_logs(
        &self,
        request: Request<GrepLogsRequest>,
    ) -> Result<Response<GrepLogsResponse>, Status> {
        let req = request.into_inner();

        // Construct the log file path
        let project = req.project.clone();

        // Use project-based directory structure
        let log_file = self
            .log_hub
            .config
            .paths
            .log_dir
            .join(&project)
            .join(format!("{}.log", req.name));

        // Try to get process first for memory-based search

        let process = self
            .process_manager
            .get_process_by_name_or_id_with_project(&req.name, Some(&req.project));

        // If file doesn't exist and process doesn't exist, return error
        if !log_file.exists() && process.is_none() {
            return Err(Status::not_found(format!(
                "Process '{}' not found in project '{}'",
                req.name, req.project
            )));
        }

        // Parse time filters
        let (since_time, until_time) =
            parse_time_filters(&req.since, &req.until, &req.last).map_err(|e| *e)?;

        // Compile regex pattern
        let pattern = regex::Regex::new(&req.pattern)
            .map_err(|e| Status::invalid_argument(format!("Invalid regex pattern: {}", e)))?;

        // Determine context settings
        let context = req.context.unwrap_or(3) as usize;
        let before = req.before.map(|b| b as usize).unwrap_or(context);
        let after = req.after.map(|a| a as usize).unwrap_or(context);

        // Read and process logs (from file or memory)
        let matches = if log_file.exists() {
            // Read from file
            info!(
                "grep_logs: Using file-based search for {}",
                log_file.display()
            );
            grep_log_file(&log_file, &pattern, before, after, since_time, until_time)
                .await
                .map_err(|e| Status::internal(format!("Failed to grep log file: {}", e)))?
        } else if let Some(process) = process {
            // Read from memory (ring buffer)
            info!(
                "grep_logs: Using memory-based search for {}/{}",
                req.project, req.name
            );
            grep_from_memory(
                &process, &pattern, before, after, since_time, until_time, &req.name,
            )
            .map_err(|e| Status::internal(format!("Failed to grep memory: {}", e)))?
        } else {
            info!(
                "grep_logs: No file and no process found for {}/{}",
                req.project, req.name
            );
            Vec::new()
        };

        Ok(Response::new(GrepLogsResponse { matches }))
    }

    async fn clean_project(
        &self,
        request: Request<CleanProjectRequest>,
    ) -> Result<Response<CleanProjectResponse>, Status> {
        let req = request.into_inner();

        if req.all_projects {
            // Clean all projects
            let results = self
                .process_manager
                .clean_all_projects(req.force)
                .await
                .map_err(|e| Status::internal(format!("Failed to clean all projects: {}", e)))?;

            let project_results: Vec<_> = results
                .into_iter()
                .map(
                    |(project, stopped_names)| proto::clean_project_response::ProjectCleanResult {
                        project,
                        processes_stopped: stopped_names.len() as u32,
                        logs_deleted: 0,
                        stopped_process_names: stopped_names,
                        deleted_log_files: vec![],
                    },
                )
                .collect();

            Ok(Response::new(CleanProjectResponse {
                processes_stopped: 0,
                logs_deleted: 0,
                stopped_process_names: vec![],
                deleted_log_files: vec![],
                project_results,
            }))
        } else {
            // Clean single project
            let project = req.project.as_deref().unwrap_or("default");
            let stopped_names = self
                .process_manager
                .clean_project(project, req.force)
                .await
                .map_err(|e| {
                    Status::internal(format!("Failed to clean project {}: {}", project, e))
                })?;

            Ok(Response::new(CleanProjectResponse {
                processes_stopped: stopped_names.len() as u32,
                logs_deleted: 0,
                stopped_process_names: stopped_names,
                deleted_log_files: vec![],
                project_results: vec![],
            }))
        }
    }

    async fn get_daemon_status(
        &self,
        _request: Request<GetDaemonStatusRequest>,
    ) -> Result<Response<GetDaemonStatusResponse>, Status> {
        let pid = std::process::id();
        let now = Utc::now();
        let uptime_seconds = (now - self.start_time).num_seconds() as u64;

        let active_processes = self.process_manager.get_all_processes().len() as u32;

        let response = GetDaemonStatusResponse {
            version: VERSION.to_string(),
            pid,
            start_time: Some(prost_types::Timestamp {
                seconds: self.start_time.timestamp(),
                nanos: self.start_time.timestamp_subsec_nanos() as i32,
            }),
            uptime_seconds,
            data_dir: self.config.paths.data_dir.to_string_lossy().to_string(),
            active_processes,
        };

        Ok(Response::new(response))
    }
}

type TimeRange = (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

fn parse_time_filters(
    since: &Option<String>,
    until: &Option<String>,
    last: &Option<String>,
) -> Result<TimeRange, Box<Status>> {
    let now = chrono::Utc::now();

    let since_time = if let Some(last_str) = last {
        // Parse "last" duration (e.g., "1h", "30m", "2d")
        let duration = parse_duration(last_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid duration '{}': {}",
                last_str, e
            )))
        })?;
        Some(now - duration)
    } else if let Some(since_str) = since {
        Some(parse_time_string(since_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid since time '{}': {}",
                since_str, e
            )))
        })?)
    } else {
        None
    };

    let until_time = if let Some(until_str) = until {
        Some(parse_time_string(until_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid until time '{}': {}",
                until_str, e
            )))
        })?)
    } else {
        None
    };

    Ok((since_time, until_time))
}

fn parse_duration(duration_str: &str) -> Result<chrono::Duration, String> {
    let duration_str = duration_str.trim();

    if duration_str.is_empty() {
        return Err("Empty duration".to_string());
    }

    let (num_str, unit) = if let Some(pos) = duration_str.rfind(char::is_alphabetic) {
        duration_str.split_at(pos)
    } else {
        return Err("No time unit specified".to_string());
    };

    let number: i64 = num_str
        .parse()
        .map_err(|_| format!("Invalid number: {}", num_str))?;

    match unit {
        "s" => Ok(chrono::Duration::seconds(number)),
        "m" => Ok(chrono::Duration::minutes(number)),
        "h" => Ok(chrono::Duration::hours(number)),
        "d" => Ok(chrono::Duration::days(number)),
        _ => Err(format!("Unknown time unit: {}", unit)),
    }
}

fn parse_time_string(time_str: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let time_str = time_str.trim();

    // Try different time formats
    let formats = [
        "%Y-%m-%d %H:%M:%S", // 2025-06-17 10:30:00
        "%Y-%m-%d %H:%M",    // 2025-06-17 10:30
        "%H:%M:%S",          // 10:30:00 (today)
        "%H:%M",             // 10:30 (today)
    ];

    for format in &formats {
        if let Ok(naive_time) = chrono::NaiveDateTime::parse_from_str(time_str, format) {
            return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                naive_time,
                chrono::Utc,
            ));
        }

        // For time-only formats, combine with today's date
        if format.starts_with("%H") {
            if let Ok(naive_time) = chrono::NaiveTime::parse_from_str(time_str, format) {
                let today = chrono::Utc::now().date_naive();
                let naive_datetime = today.and_time(naive_time);
                return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    naive_datetime,
                    chrono::Utc,
                ));
            }
        }
    }

    Err(format!("Could not parse time: {}", time_str))
}

async fn grep_log_file(
    log_file: &std::path::Path,
    pattern: &regex::Regex,
    before: usize,
    after: usize,
    since_time: Option<chrono::DateTime<chrono::Utc>>,
    until_time: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<GrepMatch>, std::io::Error> {
    use tokio::fs::File;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = File::open(log_file).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let mut all_lines = Vec::new();
    let mut line_num = 0u32;

    // Read all lines and parse them
    while let Ok(Some(line)) = lines.next_line().await {
        line_num += 1;
        let (timestamp, level, content) = parse_log_line(&line);

        // Apply time filters
        if let Some(ts) = &timestamp {
            let log_time =
                chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                    .unwrap_or_else(chrono::Utc::now);

            if let Some(since) = since_time {
                if log_time < since {
                    continue;
                }
            }

            if let Some(until) = until_time {
                if log_time > until {
                    continue;
                }
            }
        }

        all_lines.push(LogEntry {
            line_number: line_num,
            content,
            timestamp,
            level: level as i32,
            process_name: None,
        });
    }

    let mut matches = Vec::new();

    // Find matches and collect context
    for (idx, entry) in all_lines.iter().enumerate() {
        if pattern.is_match(&entry.content) {
            let context_before = if before > 0 && idx >= before {
                all_lines[idx.saturating_sub(before)..idx].to_vec()
            } else {
                all_lines[0..idx].to_vec()
            };

            let context_after = if after > 0 && idx + 1 + after <= all_lines.len() {
                all_lines[idx + 1..idx + 1 + after].to_vec()
            } else {
                all_lines[idx + 1..].to_vec()
            };

            matches.push(GrepMatch {
                matched_line: Some(entry.clone()),
                context_before,
                context_after,
            });
        }
    }

    Ok(matches)
}

fn parse_log_line(line: &str) -> (Option<prost_types::Timestamp>, log_entry::LogLevel, String) {
    // Expected format: "2025-06-16 12:30:45.123 [INFO] Log message"
    // or: "2025-06-16 12:30:45.123 [ERROR] Error message"

    let parts: Vec<&str> = line.splitn(3, ' ').collect();

    if parts.len() >= 3 {
        // Try to parse timestamp
        let timestamp = if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
            &format!("{} {}", parts[0], parts[1]),
            "%Y-%m-%d %H:%M:%S%.3f",
        ) {
            let dt_utc =
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
            Some(prost_types::Timestamp {
                seconds: dt_utc.timestamp(),
                nanos: dt_utc.timestamp_subsec_nanos() as i32,
            })
        } else {
            None
        };

        // Parse level and content
        if let Some(rest) = parts.get(2) {
            if let Some(content) = rest.strip_prefix("[ERROR]") {
                return (
                    timestamp,
                    log_entry::LogLevel::Stderr,
                    content.trim().to_string(),
                );
            } else if let Some(content) = rest.strip_prefix("[INFO]") {
                return (
                    timestamp,
                    log_entry::LogLevel::Stdout,
                    content.trim().to_string(),
                );
            }
        }
    }

    // Fallback: treat entire line as content
    (None, log_entry::LogLevel::Stdout, line.to_string())
}

pub async fn start_grpc_server(
    config: Arc<Config>,
    process_manager: Arc<ProcessManager>,
    log_hub: Arc<LogHub>,
    event_hub: SharedStreamEventHub,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = GrpcService::new(process_manager, log_hub, config.clone(), event_hub);

    // Remove old socket file if it exists
    if config.paths.socket_path.exists() {
        std::fs::remove_file(&config.paths.socket_path)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = config.paths.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use tokio::net::UnixListener;
        use tokio_stream::wrappers::UnixListenerStream;

        // Create Unix socket
        let uds = UnixListener::bind(&config.paths.socket_path)?;
        let uds_stream = UnixListenerStream::new(uds);

        // Set permissions
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(config.api.unix_socket_permissions);
        std::fs::set_permissions(&config.paths.socket_path, permissions)?;

        info!(
            "Starting gRPC server on Unix socket: {:?}",
            config.paths.socket_path
        );

        let result = Server::builder()
            .add_service(ProcessManagerServer::new(service))
            .serve_with_incoming(uds_stream)
            .await;

        if let Err(e) = &result {
            error!("gRPC server failed: {}", e);
        } else {
            info!("gRPC server shutdown gracefully");
        }
        result?;
    }

    #[cfg(not(unix))]
    {
        return Err("Unix sockets are not supported on this platform".into());
    }

    Ok(())
}

/// Grep logs from memory (ring buffer)
fn grep_from_memory(
    process: &std::sync::Arc<crate::daemon::process::proxy::ProxyInfo>,
    pattern: &regex::Regex,
    before: usize,
    after: usize,
    since_time: Option<chrono::DateTime<chrono::Utc>>,
    until_time: Option<chrono::DateTime<chrono::Utc>>,
    process_name: &str,
) -> Result<Vec<proto::GrepMatch>, String> {
    let mut all_lines = Vec::new();

    // Get logs from ring buffer
    if let Ok(ring) = process.ring.lock() {
        let chunks: Vec<LogChunk> = ring.iter().cloned().collect();
        info!(
            "DEBUG grep_from_memory: Found {} chunks in ring buffer for process {}",
            chunks.len(),
            process_name
        );

        // Convert chunks to lines with timestamps
        for log_chunk in chunks {
            if let Ok(text) = std::str::from_utf8(&log_chunk.data) {
                for line in text.lines() {
                    // Apply time filter if specified
                    if let Some(since) = since_time {
                        if log_chunk.timestamp < since {
                            continue;
                        }
                    }
                    if let Some(until) = until_time {
                        if log_chunk.timestamp > until {
                            continue;
                        }
                    }

                    let log_entry = proto::LogEntry {
                        line_number: (all_lines.len() + 1) as u32,
                        content: line.to_string(),
                        timestamp: Some(prost_types::Timestamp {
                            seconds: log_chunk.timestamp.timestamp(),
                            nanos: log_chunk.timestamp.timestamp_subsec_nanos() as i32,
                        }),
                        level: 1, // Default to INFO
                        process_name: Some(process_name.to_string()),
                    };
                    all_lines.push(log_entry);
                }
            }
        }
    }

    let mut matches = Vec::new();

    // Find matches and collect context
    for (idx, entry) in all_lines.iter().enumerate() {
        if pattern.is_match(&entry.content) {
            let context_before = if before > 0 && idx >= before {
                all_lines[idx.saturating_sub(before)..idx].to_vec()
            } else {
                all_lines[0..idx].to_vec()
            };

            let context_after = if after > 0 && idx + 1 + after <= all_lines.len() {
                all_lines[idx + 1..idx + 1 + after].to_vec()
            } else {
                all_lines[idx + 1..].to_vec()
            };

            matches.push(proto::GrepMatch {
                matched_line: Some(entry.clone()),
                context_before,
                context_after,
            });
        }
    }

    Ok(matches)
}

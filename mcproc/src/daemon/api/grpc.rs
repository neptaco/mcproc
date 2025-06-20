use crate::common::exit_code::format_exit_reason;
use crate::common::config::Config;
use crate::daemon::log::LogHub;
use crate::daemon::process::{ProcessManager, ProcessStatus};
use proto::process_manager_server::{ProcessManager as ProcessManagerService, ProcessManagerServer};
use proto::*;
use ringbuf::traits::Consumer;
use std::pin::Pin;
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status};
use tokio_stream::Stream;
use tracing::{info, error};
use uuid::Uuid;

pub struct GrpcService {
    process_manager: Arc<ProcessManager>,
    log_hub: Arc<LogHub>,
    config: Arc<Config>,
}

impl GrpcService {
    pub fn new(process_manager: Arc<ProcessManager>, log_hub: Arc<LogHub>, config: Arc<Config>) -> Self {
        Self {
            process_manager,
            log_hub,
            config,
        }
    }
}

#[tonic::async_trait]
impl ProcessManagerService for GrpcService {
    type StartProcessStream = Pin<Box<dyn Stream<Item = Result<StartProcessResponse, Status>> + Send>>;
    
    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<Self::StartProcessStream>, Status> {
        let req = request.into_inner();
        let cwd = req.cwd.map(std::path::PathBuf::from);
        
        let name = req.name.clone();
        let project = req.project.clone();
        let wait_for_log = req.wait_for_log.clone();
        let wait_timeout = req.wait_timeout;
        let cmd_for_error = req.cmd.clone();  // Clone for use in error handling
        let cwd_for_error = cwd.clone();  // Clone for use in error handling
        let log_dir = self.config.paths.log_dir.clone();  // Clone for use in async block
        
        // Create a channel for log streaming if wait_for_log is specified
        let (log_tx, log_rx) = if wait_for_log.is_some() {
            let (tx, rx) = tokio::sync::mpsc::channel(100);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        
        let process_manager = self.process_manager.clone();
        let _log_hub = self.log_hub.clone();
        
        // Create the response stream
        let stream = async_stream::try_stream! {
            // Start the process with log streaming
            match process_manager.start_process_with_log_stream(
                name.clone(),
                project.clone(),
                req.cmd,
                req.args,
                cwd,
                Some(req.env),
                wait_for_log.clone(),
                wait_timeout,
                log_tx,
            ).await {
                Ok((process, timeout_occurred)) => {
                    // If we have a log receiver, stream logs until pattern matches or timeout
                    if let Some(mut rx) = log_rx {
                        // Stream logs until channel closes (pattern matched or process ends)
                        while let Some(line) = rx.recv().await {
                            // Create log entry
                            let (timestamp, level, content) = parse_log_line(&line);
                            
                            yield StartProcessResponse {
                                response: Some(start_process_response::Response::LogEntry(LogEntry {
                                    line_number: 0,
                                    content,
                                    timestamp,
                                    level: level as i32,
                                })),
                            };
                        }
                    }
                    
                    // Send final process info
                    let current_status = process.get_status();
                    let (exit_code, exit_reason, stderr_tail) = if matches!(current_status, ProcessStatus::Failed) {
                        // Get exit details if process failed
                        let code = process.exit_code.lock().unwrap().clone();
                        let reason = code.map(format_exit_reason);
                        let stderr = process.ring.lock().ok().map(|ring| {
                            ring.iter()
                                .take(5)
                                .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                                .collect::<Vec<_>>()
                                .join("\n")
                        }).unwrap_or_default();
                        (code, reason, Some(stderr))
                    } else {
                        (None, None, None)
                    };
                    
                    let info = ProcessInfo {
                        id: process.id.to_string(),
                        name: process.name.clone(),
                        cmd: process.cmd.clone(),
                        cwd: process.cwd.to_string_lossy().to_string(),
                        status: proto::ProcessStatus::from(current_status).into(),
                        start_time: Some(prost_types::Timestamp {
                            seconds: process.start_time.timestamp(),
                            nanos: process.start_time.timestamp_subsec_nanos() as i32,
                        }),
                        pid: process.pid,
                        log_file: process.log_file.to_string_lossy().to_string(),
                        project: process.project.clone(),
                        ports: process.ports.lock().unwrap().clone(),
                        wait_timeout_occurred: if wait_for_log.is_some() { Some(timeout_occurred) } else { None },
                        exit_code,
                        exit_reason,
                        stderr_tail,
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
                                log_file: log_dir.join(format!("{}_{}.log", 
                                    project.as_ref().unwrap_or(&"default".to_string()).replace("/", "_"), 
                                    name
                                )).to_string_lossy().to_string(),
                                project: project.clone().unwrap_or_default(),
                                ports: vec![],
                                wait_timeout_occurred: None,
                                exit_code: Some(*exit_code),
                                exit_reason: Some(exit_reason.clone()),
                                stderr_tail: Some(stderr.clone()),
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
        
        match self.process_manager.stop_process(&req.name, req.project.as_deref(), req.force.unwrap_or(false)).await {
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
        
        match self.process_manager.restart_process(&req.name, req.project).await {
            Ok(process) => {
                let info = ProcessInfo {
                    id: process.id.to_string(),
                    name: process.name.clone(),
                    cmd: process.cmd.clone(),
                    cwd: process.cwd.to_string_lossy().to_string(),
                    status: proto::ProcessStatus::Running as i32,
                    start_time: Some(prost_types::Timestamp {
                        seconds: process.start_time.timestamp(),
                        nanos: process.start_time.timestamp_subsec_nanos() as i32,
                    }),
                    pid: process.pid,
                    log_file: process.log_file.to_string_lossy().to_string(),
                    project: process.project.clone(),
                    ports: process.ports.lock().unwrap().clone(),
                    wait_timeout_occurred: None,
                    exit_code: None,
                    exit_reason: None,
                    stderr_tail: None,
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
        
        match self.process_manager.get_process_by_name_or_id_with_project(&req.name, req.project.as_deref()) {
            Some(process) => {
                let info = ProcessInfo {
                    id: process.id.to_string(),
                    name: process.name.clone(),
                    cmd: process.cmd.clone(),
                    cwd: process.cwd.to_string_lossy().to_string(),
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
                    pid: process.pid,
                    log_file: process.log_file.to_string_lossy().to_string(),
                    project: process.project.clone(),
                    ports: process.ports.lock().unwrap().clone(),
                    wait_timeout_occurred: None,
                    exit_code: None,
                    exit_reason: None,
                    stderr_tail: None,
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
        
        let process_infos: Vec<ProcessInfo> = processes.into_iter().map(|process| {
            ProcessInfo {
                id: process.id.to_string(),
                name: process.name.clone(),
                cmd: process.cmd.clone(),
                cwd: process.cwd.to_string_lossy().to_string(),
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
                pid: process.pid,
                log_file: process.log_file.to_string_lossy().to_string(),
                project: process.project.clone(),
                ports: process.ports.lock().unwrap().clone(),
                wait_timeout_occurred: None,
                exit_code: None,
                exit_reason: None,
                stderr_tail: None,
            }
        }).collect();
        
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
        
        // Construct the log file path
        let project = req.project.clone().unwrap_or_else(|| "default".to_string());
        
        let log_file = self.log_hub.config.paths.log_dir.join(format!("{}_{}.log", 
            project.replace("/", "_"),
            req.name
        ));
        
        if !log_file.exists() {
            return Err(Status::not_found(format!("Log file not found: {}", log_file.display())));
        }
        
        let tail = req.tail.unwrap_or(100) as usize;
        let follow = req.follow.unwrap_or(false);
        
        // Get process for follow mode status check (optional)
        let _process = self.process_manager.get_process_by_name_or_id_with_project(&req.name, req.project.as_deref());
        
        // Get config value before the stream
        let follow_poll_interval_ms = self.config.logging.follow_poll_interval_ms;
        
        // Create stream from log file
        let stream = async_stream::try_stream! {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::fs::File;
            
            let file = File::open(&log_file).await
                .map_err(|e| Status::internal(format!("Failed to open log file: {}", e)))?;
            
            let reader = BufReader::new(file);
            let mut lines = reader.lines();
            let mut all_lines = Vec::new();
            
            // Read all existing lines first
            while let Ok(Some(line)) = lines.next_line().await {
                all_lines.push(line);
            }
            
            // Get the tail
            let start_idx = if follow {
                // If follow mode, show all lines initially or tail amount
                all_lines.len().saturating_sub(tail)
            } else {
                all_lines.len().saturating_sub(tail)
            };
            
            let mut entries = Vec::new();
            let mut line_num = start_idx as u32;
            
            // Send initial lines
            for line in &all_lines[start_idx..] {
                line_num += 1;
                
                // Parse log line
                let (timestamp, level, content) = parse_log_line(line);
                
                entries.push(LogEntry {
                    line_number: line_num,
                    content,
                    timestamp,
                    level: level as i32,
                });
                
                // Send batch of entries
                if entries.len() >= 100 {
                    yield GetLogsResponse {
                        entries: std::mem::take(&mut entries),
                    };
                }
            }
            
            // Send remaining entries
            if !entries.is_empty() {
                yield GetLogsResponse { entries };
            }
            
            // Follow mode: continue reading new lines
            if follow {
                use tokio::io::{AsyncSeekExt, SeekFrom};
                use std::fs;
                
                let mut last_size = all_lines.len();
                let mut file_reopened = false;
                
                // Re-open file for continuous reading
                let mut file = File::open(&log_file).await
                    .map_err(|e| Status::internal(format!("Failed to reopen log file: {}", e)))?;
                
                // Seek to end of file
                file.seek(SeekFrom::End(0)).await
                    .map_err(|e| Status::internal(format!("Failed to seek to end: {}", e)))?;
                
                let mut reader = BufReader::new(file);
                let mut lines = reader.lines();
                line_num = all_lines.len() as u32;
                
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            line_num += 1;
                            let (timestamp, level, content) = parse_log_line(&line);
                            
                            yield GetLogsResponse {
                                entries: vec![LogEntry {
                                    line_number: line_num,
                                    content,
                                    timestamp,
                                    level: level as i32,
                                }],
                            };
                        }
                        Ok(None) => {
                            // No more lines, wait and check for new content
                            tokio::time::sleep(tokio::time::Duration::from_millis(follow_poll_interval_ms)).await;
                            
                            // Check if the file has been recreated (e.g., after process restart)
                            if let Ok(metadata) = fs::metadata(&log_file) {
                                let current_size = metadata.len() as usize;
                                
                                // If file is smaller or we've had read errors, it might have been recreated
                                if current_size < last_size || file_reopened {
                                    // Re-open the file
                                    match File::open(&log_file).await {
                                        Ok(new_file) => {
                                            let mut new_file = new_file;
                                            // For a recreated file, start from beginning
                                            new_file.seek(SeekFrom::Start(0)).await
                                                .map_err(|e| Status::internal(format!("Failed to seek: {}", e)))?;
                                            
                                            reader = BufReader::new(new_file);
                                            lines = reader.lines();
                                            file_reopened = false;
                                            // Note: line_num continues incrementing
                                        }
                                        Err(_) => {
                                            // File might be temporarily unavailable during rotation
                                            file_reopened = true;
                                        }
                                    }
                                }
                                last_size = current_size;
                            }
                            
                            // Continue following the file regardless of process status
                            // This allows us to see logs even after restart
                        }
                        Err(e) => {
                            // Handle read errors gracefully - file might be rotating
                            eprintln!("Error reading log file (will retry): {}", e);
                            tokio::time::sleep(tokio::time::Duration::from_millis(follow_poll_interval_ms)).await;
                            file_reopened = true;
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
        let project = req.project.clone().unwrap_or_else(|| "default".to_string());
        
        let log_file = self.log_hub.config.paths.log_dir.join(format!("{}_{}.log", 
            project.replace("/", "_"),
            req.name
        ));
        
        if !log_file.exists() {
            return Err(Status::not_found(format!("Log file not found: {}", log_file.display())));
        }
        
        // Parse time filters
        let (since_time, until_time) = parse_time_filters(&req.since, &req.until, &req.last)
            .map_err(|e| *e)?;
        
        // Compile regex pattern
        let pattern = regex::Regex::new(&req.pattern)
            .map_err(|e| Status::invalid_argument(format!("Invalid regex pattern: {}", e)))?;
        
        // Determine context settings
        let context = req.context.unwrap_or(3) as usize;
        let before = req.before.map(|b| b as usize).unwrap_or(context);
        let after = req.after.map(|a| a as usize).unwrap_or(context);
        
        // Read and process log file
        let matches = grep_log_file(&log_file, &pattern, before, after, since_time, until_time)
            .await
            .map_err(|e| Status::internal(format!("Failed to grep log file: {}", e)))?;
        
        Ok(Response::new(GrepLogsResponse { matches }))
    }
}

type TimeRange = (Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>);

fn parse_time_filters(
    since: &Option<String>,
    until: &Option<String>,
    last: &Option<String>,
) -> Result<TimeRange, Box<Status>> {
    let now = chrono::Utc::now();
    
    let since_time = if let Some(last_str) = last {
        // Parse "last" duration (e.g., "1h", "30m", "2d")
        let duration = parse_duration(last_str)
            .map_err(|e| Box::new(Status::invalid_argument(format!("Invalid duration '{}': {}", last_str, e))))?;
        Some(now - duration)
    } else if let Some(since_str) = since {
        Some(parse_time_string(since_str)
            .map_err(|e| Box::new(Status::invalid_argument(format!("Invalid since time '{}': {}", since_str, e))))?)
    } else {
        None
    };
    
    let until_time = if let Some(until_str) = until {
        Some(parse_time_string(until_str)
            .map_err(|e| Box::new(Status::invalid_argument(format!("Invalid until time '{}': {}", until_str, e))))?)
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
    
    let number: i64 = num_str.parse()
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
        "%Y-%m-%d %H:%M:%S",     // 2025-06-17 10:30:00
        "%Y-%m-%d %H:%M",        // 2025-06-17 10:30
        "%H:%M:%S",              // 10:30:00 (today)
        "%H:%M",                 // 10:30 (today)
    ];
    
    for format in &formats {
        if let Ok(naive_time) = chrono::NaiveDateTime::parse_from_str(time_str, format) {
            return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive_time, chrono::Utc));
        }
        
        // For time-only formats, combine with today's date
        if format.starts_with("%H") {
            if let Ok(naive_time) = chrono::NaiveTime::parse_from_str(time_str, format) {
                let today = chrono::Utc::now().date_naive();
                let naive_datetime = today.and_time(naive_time);
                return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive_datetime, chrono::Utc));
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
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::fs::File;
    
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
            let log_time = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
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
            "%Y-%m-%d %H:%M:%S%.3f"
        ) {
            let dt_utc = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
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
                return (timestamp, log_entry::LogLevel::Stderr, content.trim().to_string());
            } else if let Some(content) = rest.strip_prefix("[INFO]") {
                return (timestamp, log_entry::LogLevel::Stdout, content.trim().to_string());
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
) -> Result<(), Box<dyn std::error::Error>> {
    let service = GrpcService::new(process_manager, log_hub, config.clone());
    
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
    
        info!("Starting gRPC server on Unix socket: {:?}", config.paths.socket_path);
    
        Server::builder()
            .add_service(ProcessManagerServer::new(service))
            .serve_with_incoming(uds_stream)
            .await?;
    }
    
    #[cfg(not(unix))]
    {
        return Err("Unix sockets are not supported on this platform".into());
    }
    
    Ok(())
}
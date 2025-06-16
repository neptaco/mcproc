use crate::config::Config;
use crate::log::LogHub;
use crate::process::{ProcessManager, ProcessStatus};
use proto::process_manager_server::{ProcessManager as ProcessManagerService, ProcessManagerServer};
use proto::*;
use prost_types;
use std::pin::Pin;
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status};
use tokio_stream::Stream;
use tracing::info;

pub struct GrpcService {
    process_manager: Arc<ProcessManager>,
    log_hub: Arc<LogHub>,
}

impl GrpcService {
    pub fn new(process_manager: Arc<ProcessManager>, log_hub: Arc<LogHub>) -> Self {
        Self {
            process_manager,
            log_hub,
        }
    }
}

#[tonic::async_trait]
impl ProcessManagerService for GrpcService {
    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<StartProcessResponse>, Status> {
        let req = request.into_inner();
        let cwd = req.cwd.map(|c| std::path::PathBuf::from(c));
        
        match self.process_manager.start_process(
            req.name,
            req.project,
            req.cmd,
            req.args,
            cwd,
            Some(req.env),
        ).await {
            Ok(process) => {
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
                };
                
                Ok(Response::new(StartProcessResponse {
                    process: Some(info),
                }))
            }
            Err(e) => match e {
                crate::error::McprocdError::ProcessAlreadyExists(name) => {
                    Err(Status::already_exists(format!("Process '{}' is already running", name)))
                }
                _ => Err(Status::internal(e.to_string())),
            },
        }
    }
    
    async fn stop_process(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        let req = request.into_inner();
        
        match self.process_manager.stop_process(&req.name, req.force.unwrap_or(false)).await {
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
        
        // Get the process to verify it exists
        let process = self.process_manager.get_process_by_name_or_id_with_project(&req.name, req.project.as_deref())
            .ok_or_else(|| Status::not_found(format!("Process '{}' not found", req.name)))?;
        
        // Get the log directory and find log files for this process
        let log_dir = &self.log_hub.config.log.dir;
        let mut log_files = Vec::new();
        
        // Find all log files for this process
        if let Ok(entries) = std::fs::read_dir(log_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                    let expected_prefix = format!("{}_{}_", process.project.replace("/", "_"), process.name);
                    if filename.starts_with(&expected_prefix) && filename.ends_with(".log") {
                        log_files.push(path);
                    }
                }
            }
        }
        
        // Sort log files by name (which includes date)
        log_files.sort();
        
        if log_files.is_empty() {
            return Err(Status::not_found("No log files found for process"));
        }
        
        // Use the latest log file
        let log_file = log_files.last().unwrap().clone();
        
        let follow = req.follow.unwrap_or(false);
        let from_line = req.from_line.unwrap_or(0);
        let to_line = req.to_line;
        
        // Create stream from log file
        let stream = async_stream::try_stream! {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::fs::File;
            
            let file = File::open(&log_file).await
                .map_err(|e| Status::internal(format!("Failed to open log file: {}", e)))?;
            
            let reader = BufReader::new(file);
            let mut lines = reader.lines();
            let mut line_num = 0u32;
            let mut entries = Vec::new();
            
            // Read lines
            while let Ok(Some(line)) = lines.next_line().await {
                line_num += 1;
                
                // Skip lines before from_line
                if line_num < from_line {
                    continue;
                }
                
                // Stop if we've reached to_line
                if let Some(to) = to_line {
                    if line_num > to {
                        break;
                    }
                }
                
                // Parse log line
                let (timestamp, level, content) = parse_log_line(&line);
                
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
            
            // Follow mode: continue reading as new lines are added
            if follow {
                // Keep reading the file
                loop {
                    // Try to read new lines
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
                            // No more lines available, wait a bit
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            
                            // Check if process is still running
                            if !matches!(process.get_status(), crate::process::ProcessStatus::Running) {
                                break;
                            }
                        }
                        Err(e) => {
                            // Error reading, log and break
                            eprintln!("Error reading log file: {}", e);
                            break;
                        }
                    }
                }
            }
        };
        
        Ok(Response::new(Box::pin(stream)))
    }
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
            if rest.starts_with("[ERROR]") {
                return (timestamp, log_entry::LogLevel::Stderr, rest[7..].trim().to_string());
            } else if rest.starts_with("[INFO]") {
                return (timestamp, log_entry::LogLevel::Stdout, rest[6..].trim().to_string());
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
    let service = GrpcService::new(process_manager, log_hub);
    
    // Try to find an available port, starting from the configured port
    let mut port = config.api.grpc_port;
    let mut addr = None;
    
    for attempt in 0..10 {
        let test_addr: std::net::SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        
        // Check if port is available
        match std::net::TcpListener::bind(test_addr) {
            Ok(listener) => {
                drop(listener); // Release the port
                addr = Some(test_addr);
                break;
            }
            Err(_) => {
                if attempt < 9 {
                    port += 1;
                    continue;
                }
            }
        }
    }
    
    let addr = addr.ok_or("Could not find available port for gRPC server")?;
    
    // Write the actual port to a file
    let port_file = config.daemon.data_dir.join("mcprocd.port");
    std::fs::write(&port_file, port.to_string())?;
    
    info!("Starting gRPC server on {}", addr);
    
    Server::builder()
        .add_service(ProcessManagerServer::new(service))
        .serve(addr)
        .await?;
    
    Ok(())
}
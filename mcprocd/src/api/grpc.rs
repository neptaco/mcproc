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
                    project: process.project.clone()
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
                    project: process.project.clone()
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
                    project: process.project.clone()
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
                project: process.project.clone()
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
        
        let log_file = self.log_hub.config.log.dir.join(format!("{}_{}.log", 
            project.replace("/", "_"),
            req.name
        ));
        
        if !log_file.exists() {
            return Err(Status::not_found(format!("Log file not found: {}", log_file.display())));
        }
        
        let tail = req.tail.unwrap_or(100) as usize;
        let follow = req.follow.unwrap_or(false);
        
        // Get process for follow mode status check (optional)
        let process = self.process_manager.get_process_by_name_or_id_with_project(&req.name, req.project.as_deref());
        
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
            
            // Follow mode: continue reading new lines
            if follow {
                // Re-open file for continuous reading
                let file = File::open(&log_file).await
                    .map_err(|e| Status::internal(format!("Failed to reopen log file: {}", e)))?;
                
                // Seek to end of file
                use tokio::io::{AsyncSeekExt, SeekFrom};
                let mut file = file;
                file.seek(SeekFrom::End(0)).await
                    .map_err(|e| Status::internal(format!("Failed to seek to end: {}", e)))?;
                
                let reader = BufReader::new(file);
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
                            // No more lines, wait and check process status
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            
                            // Check if process is still running (if process exists)
                            if let Some(ref proc) = process {
                                if !matches!(proc.get_status(), crate::process::ProcessStatus::Running) {
                                    break;
                                }
                            }
                            // If no process found, continue following the file
                        }
                        Err(e) => {
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
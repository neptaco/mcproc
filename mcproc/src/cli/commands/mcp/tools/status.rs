//! Status tool implementation

use crate::client::DaemonClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};
use serde::Deserialize;
use tokio_stream::StreamExt;

pub struct StatusTool {
    client: DaemonClient,
    default_project: Option<String>,
}

impl StatusTool {
    pub fn new(client: DaemonClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
    }
}

#[derive(Deserialize)]
struct StatusParams {
    name: String,
    project: Option<String>,
}

#[async_trait]
impl ToolHandler for StatusTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "get_process_status".to_string(),
            description: "Get comprehensive status information for a specific process including: current state (running/stopped/failed), PID, uptime, command line, working directory, detected ports, and recent log preview. Use this to check if a process is healthy or to debug why it might have stopped.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the process to check status for" },
                    "project": { "type": "string", "description": "Optional project name to scope the process lookup. Useful when multiple projects have processes with the same name." }
                },
                "required": ["name"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>, _context: mcp_rs::ToolContext) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: StatusParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        // Determine project name if not provided
        let project = params.project.or(self.default_project.clone()).or_else(|| {
            std::env::current_dir().ok()
                .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                .and_then(|n| n.into_string().ok())
        });
        
        let request = proto::GetProcessRequest {
            name: params.name.clone(),
            project,
        };
        
        let mut client = self.client.clone();
        match client.inner().get_process(request).await {
            Ok(response) => {
                let process = response.into_inner().process
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;
                
                // Calculate uptime if process is running
                let uptime = if process.status == proto::ProcessStatus::Running as i32 {
                    process.start_time.as_ref().map(|start_time| {
                        let start = chrono::DateTime::<chrono::Utc>::from_timestamp(
                            start_time.seconds, 
                            start_time.nanos as u32
                        ).unwrap_or_else(chrono::Utc::now);
                        
                        let now = chrono::Utc::now();
                        let duration = now - start;
                        
                        // Format duration as human-readable string
                        let days = duration.num_days();
                        let hours = duration.num_hours() % 24;
                        let minutes = duration.num_minutes() % 60;
                        let seconds = duration.num_seconds() % 60;
                        
                        if days > 0 {
                            format!("{}d {}h {}m {}s", days, hours, minutes, seconds)
                        } else if hours > 0 {
                            format!("{}h {}m {}s", hours, minutes, seconds)
                        } else if minutes > 0 {
                            format!("{}m {}s", minutes, seconds)
                        } else {
                            format!("{}s", seconds)
                        }
                    })
                } else {
                    None
                };
                
                // Get recent logs preview
                let logs_request = proto::GetLogsRequest {
                    name: params.name.clone(),
                    tail: Some(5), // Get last 5 lines as preview
                    follow: Some(false),
                    project: Some(process.project.clone()),
                };
                
                let mut logs_preview = Vec::new();
                if let Ok(stream) = client.inner().get_logs(logs_request).await {
                    let mut stream = stream.into_inner();
                    while let Some(Ok(logs_response)) = stream.next().await {
                        for entry in logs_response.entries {
                            logs_preview.push(entry.content);
                        }
                    }
                }
                
                let response = json!({
                    "id": process.id,
                    "project": process.project,
                    "name": process.name,
                    "status": format_status(process.status),
                    "pid": process.pid,
                    "command": process.cmd,
                    "working_directory": process.cwd,
                    "log_file": process.log_file,
                    "start_time": process.start_time.map(|t| {
                        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                            .unwrap_or_else(chrono::Utc::now);
                        ts.to_rfc3339()
                    }),
                    "uptime": uptime,
                    "ports": process.ports,
                    "recent_logs": logs_preview,
                });
                
                Ok(response)
            }
            Err(e) => {
                if e.code() == tonic::Code::NotFound {
                    Err(McpError::InvalidParams(format!("Process '{}' not found", params.name)))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}
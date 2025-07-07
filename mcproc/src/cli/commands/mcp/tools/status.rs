//! Status tool implementation

use crate::cli::utils::resolve_mcp_project_name;
use crate::client::DaemonClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::StreamExt;

pub struct StatusTool {
    client: DaemonClient,
}

impl StatusTool {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
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

    async fn handle(
        &self,
        params: Option<Value>,
        _context: mcp_rs::ToolContext,
    ) -> McpResult<Value> {
        let params =
            params.ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;

        let params: StatusParams =
            serde_json::from_value(params).map_err(|e| McpError::InvalidParams(e.to_string()))?;

        // Determine project name if not provided
        let project = resolve_mcp_project_name(params.project)?;

        let request = proto::GetProcessRequest {
            name: params.name.clone(),
            project: project.clone(),
        };

        let mut client = self.client.clone();
        match client.inner().get_process(request).await {
            Ok(response) => {
                let process = response
                    .into_inner()
                    .process
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;

                // Calculate uptime if process is running
                let uptime = if process.status == proto::ProcessStatus::Running as i32 {
                    process.start_time.as_ref().map(|start_time| {
                        let start = chrono::DateTime::<chrono::Utc>::from_timestamp(
                            start_time.seconds,
                            start_time.nanos as u32,
                        )
                        .unwrap_or_else(chrono::Utc::now);

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
                    process_names: vec![params.name.clone()],
                    tail: Some(5), // Get last 5 lines as preview
                    follow: Some(false),
                    project: process.project.clone(),
                    include_events: Some(false),
                };

                let mut logs_preview = Vec::new();
                if let Ok(stream) = client.inner().get_logs(logs_request).await {
                    let mut stream = stream.into_inner();
                    // Take at most 100 log entries to avoid blocking
                    let mut count = 0;
                    while let Ok(Some(logs_response)) = stream.try_next().await {
                        if let Some(content) = logs_response.content {
                            match content {
                                proto::get_logs_response::Content::LogEntry(entry) => {
                                    logs_preview.push(entry.content);
                                }
                                proto::get_logs_response::Content::Event(_) => {
                                    // Ignore events in status tool
                                }
                            }
                        }
                        count += 1;
                        if count >= 100 {
                            break;
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
                    // Get list of existing processes to help user
                    let list_request = proto::ListProcessesRequest {
                        status_filter: None,
                        project_filter: Some(project.clone()),
                    };

                    let existing_processes = match client.inner().list_processes(list_request).await
                    {
                        Ok(response) => {
                            let processes: Vec<Value> = response
                                .into_inner()
                                .processes
                                .into_iter()
                                .map(|p| {
                                    json!({
                                        "name": p.name,
                                        "status": format_status(p.status),
                                        "project": p.project,
                                    })
                                })
                                .collect();
                            processes
                        }
                        Err(_) => Vec::new(),
                    };

                    let error_msg = if existing_processes.is_empty() {
                        format!(
                            "Process '{}' not found in project '{}'. No processes are currently running in this project.",
                            params.name, project
                        )
                    } else {
                        format!(
                            "Process '{}' not found in project '{}'. Available processes in this project: {}",
                            params.name,
                            project,
                            existing_processes.iter()
                                .map(|p| format!("{} ({})", p["name"].as_str().unwrap_or(""), p["status"].as_str().unwrap_or("")))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };

                    Err(McpError::InvalidParams(error_msg))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}

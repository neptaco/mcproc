//! Logs tool implementation

use crate::cli::utils::resolve_mcp_project_name;
use crate::client::DaemonClient;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::StreamExt;

pub struct LogsTool {
    client: DaemonClient,
}

impl LogsTool {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
    }
}

#[derive(Deserialize)]
struct LogsParams {
    name: String,
    tail: Option<u32>,
    project: Option<String>,
}

#[async_trait]
impl ToolHandler for LogsTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "get_process_logs".to_string(),
            description: "Retrieve console output and logs from a process. Returns the most recent log entries including stdout, stderr, and any output from the process. Useful for debugging issues, checking server status, or monitoring process behavior. Logs are persisted even after process stops.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the process to get logs from" },
                    "tail": { "type": "integer", "description": "Number of most recent lines to retrieve. Default is 100. Use larger values to see more history." },
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

        let params: LogsParams =
            serde_json::from_value(params).map_err(|e| McpError::InvalidParams(e.to_string()))?;

        // Determine project name if not provided
        let project = resolve_mcp_project_name(params.project)?;

        // Use gRPC get_logs method instead of direct file access
        let mut client = self.client.clone();
        let request = proto::GetLogsRequest {
            name: params.name.clone(),
            tail: params.tail,
            follow: Some(false),
            project,
        };

        let mut stream = client
            .inner()
            .get_logs(request)
            .await
            .map_err(|e| McpError::Internal(e.to_string()))?
            .into_inner();

        let mut all_logs = Vec::new();

        // Collect all log entries from the stream
        while let Some(response) = stream.next().await {
            match response {
                Ok(logs_response) => {
                    for entry in logs_response.entries {
                        // Format log entry similar to the CLI output
                        let timestamp = entry
                            .timestamp
                            .as_ref()
                            .map(|ts| {
                                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(
                                    ts.seconds,
                                    ts.nanos as u32,
                                )
                                .unwrap_or_else(chrono::Utc::now);
                                dt.format("%Y-%m-%d %H:%M:%S").to_string()
                            })
                            .unwrap_or_else(|| "".to_string());

                        let level = match entry.level {
                            2 => "E",
                            _ => "I",
                        };

                        let formatted = if timestamp.is_empty() {
                            format!("{} {}", level, entry.content)
                        } else {
                            format!("{} {} {}", timestamp, level, entry.content)
                        };

                        all_logs.push(formatted);
                    }
                }
                Err(e) => {
                    return Err(McpError::Internal(format!("Error receiving logs: {}", e)));
                }
            }
        }

        // Return as an array for better MCP display
        Ok(json!({
            "logs": all_logs
        }))
    }
}

//! Restart tool implementation

use crate::cli::utils::resolve_mcp_project_name;
use crate::client::DaemonClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use strip_ansi_escapes::strip;

pub struct RestartTool {
    client: DaemonClient,
}

impl RestartTool {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
    }
}

#[derive(Deserialize)]
struct RestartParams {
    name: String,
    project: Option<String>,
    wait_for_log: Option<String>,
    wait_timeout: Option<u32>,
}

#[async_trait]
impl ToolHandler for RestartTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "restart_process".to_string(),
            description: "Restart a running process by stopping it and starting it again. By default, uses the same wait_for_log pattern and timeout from the original start. You can override these values to change the startup detection behavior. This is especially useful when the server's startup log pattern changes or when you need to adjust the timeout. The process will be restarted with the same command, working directory, and environment variables.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the process to restart (must be currently running or recently stopped)" },
                    "project": { "type": "string", "description": "Optional project name to scope the process lookup. Useful when multiple projects have processes with the same name." },
                    "wait_for_log": { 
                        "type": "string", 
                        "description": "Override the regex pattern to wait for in logs before considering the process ready. If not specified, uses the pattern from the original start command. Use this when the server's startup message has changed or to detect a different ready state." 
                    },
                    "wait_timeout": { 
                        "type": "integer", 
                        "description": "Override the timeout in seconds for waiting for the log pattern. If not specified, uses the timeout from the original start command. Increase this if the server takes longer to start after updates." 
                    }
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

        let params: RestartParams =
            serde_json::from_value(params).map_err(|e| McpError::InvalidParams(e.to_string()))?;

        let project = resolve_mcp_project_name(params.project)?;

        let request = proto::RestartProcessRequest {
            name: params.name.clone(),
            project,
            wait_for_log: params.wait_for_log,
            wait_timeout: params.wait_timeout,
        };

        let mut client = self.client.clone();
        match client.inner().restart_process(request).await {
            Ok(response) => {
                let process = response
                    .into_inner()
                    .process
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;
                let mut response = json!({
                    "id": process.id,
                    "project": process.project,
                    "name": process.name,
                    "pid": process.pid,
                    "status": format_status(process.status),
                    "log_file": process.log_file,
                    "start_time": process.start_time.map(|t| {
                        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                            .unwrap_or_else(chrono::Utc::now);
                        ts.to_rfc3339()
                    }),
                    "ports": process.ports,
                });

                // Add wait pattern match info if process has wait_for_log configured (strip ANSI codes)
                if !process.log_context.is_empty() {
                    let cleaned_context: Vec<String> = process.log_context
                        .iter()
                        .map(|line| String::from_utf8_lossy(&strip(line.as_bytes())).to_string())
                        .collect();
                    response["log_context"] = json!(cleaned_context);
                }

                if let Some(matched_line) = process.matched_line {
                    let cleaned_line = String::from_utf8_lossy(&strip(matched_line.as_bytes())).to_string();
                    response["matched_line"] = json!(cleaned_line);
                }

                // Add timeout information if available
                if let Some(timeout_occurred) = process.wait_timeout_occurred {
                    if timeout_occurred {
                        response["wait_timeout_occurred"] = json!(true);
                        response["message"] = json!(
                            "Process restarted but wait_for_log pattern was not found within timeout"
                        );
                    } else {
                        response["pattern_matched"] = json!(true);
                        response["message"] =
                            json!("Process restarted successfully. Pattern matched in logs.");
                    }
                }

                Ok(response)
            }
            Err(e) => Err(McpError::Internal(e.message().to_string())),
        }
    }
}

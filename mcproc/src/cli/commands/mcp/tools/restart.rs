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
        context: mcp_rs::ToolContext,
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
                let mut stream = response.into_inner();
                let mut process_info = None;
                let mut _log_count = 0;

                // Process streaming responses
                while let Some(msg) = stream
                    .message()
                    .await
                    .map_err(|e| McpError::Internal(e.to_string()))?
                {
                    match msg.response {
                        Some(proto::restart_process_response::Response::LogEntry(entry)) => {
                            // Send log entry as notification
                            context
                                .send_log(mcp_rs::MessageLevel::Info, entry.content.clone())
                                .await?;
                            _log_count += 1;
                        }
                        Some(proto::restart_process_response::Response::Process(info)) => {
                            process_info = Some(info);
                        }
                        None => {}
                    }
                }

                let process = process_info
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;

                let mut output = String::from("RESTART PROCESS\n\n");
                output.push_str(&format!("Process: {}\n", process.name));
                output.push_str(&format!("  ID: {}\n", process.id));
                output.push_str(&format!("  Project: {}\n", process.project));
                output.push_str(&format!("  Status: {}\n", format_status(process.status)));

                if let Some(pid) = process.pid {
                    output.push_str(&format!("  PID: {}\n", pid));
                }

                output.push_str(&format!("  Log file: {}\n", process.log_file));

                if let Some(start_time) = process.start_time {
                    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(
                        start_time.seconds,
                        start_time.nanos as u32,
                    )
                    .unwrap_or_else(chrono::Utc::now);
                    output.push_str(&format!("  Started: {}\n", ts.to_rfc3339()));
                }

                if !process.ports.is_empty() {
                    let ports_str = process
                        .ports
                        .iter()
                        .map(|port| port.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    output.push_str(&format!("  Ports: {}\n", ports_str));
                }

                // Add exit information if process failed
                if process.status == proto::ProcessStatus::Failed as i32 {
                    output.push_str("\nProcess failed:\n");
                    if let Some(exit_code) = process.exit_code {
                        output.push_str(&format!("  Exit code: {}\n", exit_code));
                    }
                    if let Some(exit_reason) = &process.exit_reason {
                        output.push_str(&format!("  Reason: {}\n", exit_reason));
                    }
                    if let Some(stderr_tail) = &process.stderr_tail {
                        output.push_str(&format!("  Stderr (last lines):\n{}\n", stderr_tail));
                    }
                }

                // Add wait pattern match info if process has wait_for_log configured (strip ANSI codes)
                if !process.log_context.is_empty() {
                    output.push_str("\nLog context:\n");
                    for line in &process.log_context {
                        let cleaned_line =
                            String::from_utf8_lossy(&strip(line.as_bytes())).to_string();
                        output.push_str(&format!("  {}\n", cleaned_line));
                    }
                }

                if let Some(matched_line) = &process.matched_line {
                    let cleaned_line =
                        String::from_utf8_lossy(&strip(matched_line.as_bytes())).to_string();
                    output.push_str(&format!("\nMatched line: {}\n", cleaned_line));
                }

                // Add timeout information if available
                if let Some(timeout_occurred) = process.wait_timeout_occurred {
                    if timeout_occurred {
                        output.push_str("\nNote: Process restarted but wait_for_log pattern was not found within timeout.\n");
                    } else {
                        output.push_str("\n✓ Process restarted successfully. Pattern matched in logs.\n");
                    }
                }

                Ok(json!({ "content": [{ "type": "text", "text": output }] }))
            }
            Err(e) => Err(McpError::Internal(e.message().to_string())),
        }
    }
}

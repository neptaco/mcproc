//! Start process tool implementation

use crate::client::DaemonClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tonic::Request;

pub struct StartTool {
    client: DaemonClient,
    default_project: Option<String>,
}

impl StartTool {
    pub fn new(client: DaemonClient) -> Self {
        Self {
            client,
            default_project: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_project(client: DaemonClient, default_project: Option<String>) -> Self {
        Self {
            client,
            default_project,
        }
    }
}

#[derive(Deserialize)]
struct StartParams {
    name: String,
    #[serde(default)]
    cmd: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    cwd: Option<String>,
    project: Option<String>,
    env: Option<std::collections::HashMap<String, String>>,
    wait_for_log: Option<String>,
    wait_timeout: Option<u32>,
    #[serde(default)]
    force_restart: Option<bool>,
}

#[async_trait]
impl ToolHandler for StartTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "start_process".to_string(),
            description: "Start and manage a long-running development process (web servers, build watchers, etc). The process will continue running in the background and can be monitored/controlled later. Use this for commands like 'npm run dev', 'python app.py', 'cargo watch', etc. Each process needs a unique name for identification. Use force_restart=true to automatically stop and restart an existing process with the same name, which is useful when you're unsure if the process is already running.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique identifier for this process. Use descriptive names like 'frontend-dev', 'backend-api', 'docs-server'. This name is used to reference the process in other commands." },
                    "cmd": { "type": "string", "description": "Shell command to execute. Use this for commands that need shell features like pipes (|), redirects (>), or environment variable expansion. Examples: 'npm run dev', 'yarn start', 'python -m http.server 8000', 'NODE_ENV=development npm start'. Choose either 'cmd' or 'args', not both." },
                    "args": { 
                        "type": "array", 
                        "items": { "type": "string" },
                        "description": "Command and arguments as an array for direct execution without shell interpretation. Safer than 'cmd' for user input. Examples: ['npm', 'run', 'dev'], ['python', '-m', 'http.server', '8000']. Choose either 'cmd' or 'args', not both."
                    },
                    "cwd": { "type": "string", "description": "Working directory path (defaults to current directory)" },
                    "project": { "type": "string", "description": "Project name (defaults to directory name)" },
                    "env": { 
                        "type": "object", 
                        "description": "Environment variables to set for the process",
                        "additionalProperties": { "type": "string" }
                    },
                    "wait_for_log": { 
                        "type": "string", 
                        "description": "Optional regex pattern to wait for in the process output before considering it ready. Useful for servers that take time to start. Examples: 'Server running on', 'Compiled successfully', 'Ready on http://'. The tool will wait up to wait_timeout seconds." 
                    },
                    "wait_timeout": { 
                        "type": "integer", 
                        "description": "Timeout for log wait in seconds (default: 30)" 
                    },
                    "force_restart": { 
                        "type": "boolean", 
                        "description": "If true, automatically stop any existing process with the same name before starting. This prevents 'already running' errors and ensures a fresh start. Useful when the LLM agent isn't sure if a process is running or when you want to guarantee a clean restart. (default: false)" 
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

        let params: StartParams =
            serde_json::from_value(params).map_err(|e| McpError::InvalidParams(e.to_string()))?;

        // Validate that either cmd or args is provided, but not both
        // Note: Empty args array is treated as None
        match (&params.cmd, &params.args) {
            (Some(_), Some(args)) if !args.is_empty() => {
                return Err(McpError::InvalidParams(
                    "Cannot specify both 'cmd' and 'args'".to_string(),
                ));
            }
            (None, None) => {
                return Err(McpError::InvalidParams(
                    "Must specify either 'cmd' or 'args'".to_string(),
                ));
            }
            (None, Some(args)) if args.is_empty() => {
                return Err(McpError::InvalidParams(
                    "Must specify either 'cmd' or 'args'".to_string(),
                ));
            }
            _ => {}
        }

        // Determine project name if not provided
        let project = params
            .project
            .or(self.default_project.clone())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                    .and_then(|n| n.into_string().ok())
            })
            .ok_or_else(|| {
                McpError::InvalidParams(
                    "Unable to determine project name from current directory".to_string(),
                )
            })?;

        // Use gRPC client to start process
        let name = params.name.clone();
        let wait_for_log_flag = params.wait_for_log.is_some();
        let wait_timeout_value = params.wait_timeout;

        let grpc_request = proto::StartProcessRequest {
            name: params.name,
            cmd: params.cmd,
            args: params.args.unwrap_or_default(),
            cwd: params.cwd,
            project: Some(project),
            env: params.env.unwrap_or_default(),
            wait_for_log: params.wait_for_log,
            wait_timeout: params.wait_timeout,
            force_restart: params.force_restart,
        };

        // Set timeout to wait_timeout + 5 seconds to allow for process startup
        let timeout = Duration::from_secs((wait_timeout_value.unwrap_or(30) + 5) as u64);
        let mut request = Request::new(grpc_request);
        request.set_timeout(timeout);

        // Send initial progress notification if we're waiting for log
        if wait_for_log_flag {
            context
                .send_log(
                    mcp_rs::MessageLevel::Info,
                    format!("Starting process '{}' and waiting for log pattern...", name),
                )
                .await?;

            if let Some(ref _token) = context.progress_token {
                context
                    .send_progress(0, 100, Some("Starting process...".to_string()))
                    .await?;
            }
        }

        let mut client = self.client.clone();
        match client.inner().start_process(request).await {
            Ok(response) => {
                let mut stream = response.into_inner();
                let mut process_info = None;

                // Collect all streaming responses
                let mut log_count = 0;

                while let Some(msg) = stream
                    .message()
                    .await
                    .map_err(|e| McpError::Internal(e.to_string()))?
                {
                    match msg.response {
                        Some(proto::start_process_response::Response::LogEntry(entry)) => {
                            // Send log entry as notification
                            context
                                .send_log(mcp_rs::MessageLevel::Info, entry.content.clone())
                                .await?;

                            log_count += 1;

                            // Update progress if we have a token
                            if let Some(ref _token) = context.progress_token {
                                // Estimate progress based on time elapsed vs timeout
                                let progress = std::cmp::min(90, (log_count * 10).min(90));
                                context
                                    .send_progress(
                                        progress,
                                        100,
                                        Some(format!("Received {} log lines...", log_count)),
                                    )
                                    .await?;
                            }
                        }
                        Some(proto::start_process_response::Response::Process(info)) => {
                            process_info = Some(info);

                            // Send completion progress
                            if let Some(ref _token) = context.progress_token {
                                context
                                    .send_progress(
                                        100,
                                        100,
                                        Some("Process started successfully".to_string()),
                                    )
                                    .await?;
                            }
                        }
                        None => {}
                    }
                }

                let process = process_info
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

                // Add exit information if process failed
                if process.status == proto::ProcessStatus::Failed as i32 {
                    if let Some(exit_code) = process.exit_code {
                        response["exit_code"] = json!(exit_code);
                    }
                    if let Some(exit_reason) = process.exit_reason {
                        response["exit_reason"] = json!(exit_reason);
                    }
                    if let Some(stderr_tail) = process.stderr_tail {
                        response["stderr_tail"] = json!(stderr_tail);
                    }
                }

                // Add timeout information if available
                if let Some(timeout_occurred) = process.wait_timeout_occurred {
                    if timeout_occurred {
                        response["wait_timeout_occurred"] = json!(true);
                        response["message"] = json!(
                            "Process started but wait_for_log pattern was not found within timeout"
                        );
                    }
                }

                // Always include log context from ProcessInfo
                if !process.log_context.is_empty() {
                    response["log_context"] = json!(process.log_context);
                }

                // Add matched line if available
                if let Some(matched_line) = process.matched_line {
                    response["matched_line"] = json!(matched_line);
                }

                // Add pattern match information if wait_for_log was used
                if wait_for_log_flag {
                    // If pattern was found (not timeout), mark it in the response
                    if !process.wait_timeout_occurred.unwrap_or(false) {
                        response["pattern_matched"] = json!(true);
                        response["message"] = json!(format!(
                            "Process started successfully. Pattern matched in logs."
                        ));
                    }
                }

                Ok(response)
            }
            Err(e) => {
                if e.code() == tonic::Code::AlreadyExists {
                    Err(McpError::InvalidParams(format!(
                        "Process '{}' is already running",
                        name
                    )))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}

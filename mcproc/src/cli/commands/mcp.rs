//! MCP server command implementation

use crate::client::McpClient;
use clap::{Parser, Subcommand};
use std::sync::Arc;
use tokio_stream::StreamExt;

#[derive(Parser)]
pub struct MpcCommand {
    #[command(subcommand)]
    subcommand: MpcSubcommand,
}

#[derive(Subcommand)]
enum MpcSubcommand {
    /// Start MCP server with stdio transport
    Serve {
        /// Server name (defaults to "mcproc")
        #[arg(long, default_value = "mcproc")]
        name: String,
        
        /// Project name (defaults to directory name)
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
}

impl MpcCommand {
    pub async fn execute(self, client: McpClient) -> Result<(), Box<dyn std::error::Error>> {
        match self.subcommand {
            MpcSubcommand::Serve { name, project } => run_stdio_server(client, name, project).await,
        }
    }
}

async fn run_stdio_server(client: McpClient, name: String, default_project: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    use mcp_rs::{ServerBuilder, StdioTransport};
    
    // Don't print any startup messages to stderr as it may be interpreted as errors
    
    let transport = Box::new(StdioTransport::new());
    
    let mut server = ServerBuilder::new(&name, env!("CARGO_PKG_VERSION"))
        .add_tool(Arc::new(StartTool::new(client.clone(), default_project.clone())))
        .add_tool(Arc::new(StopTool::new(client.clone(), default_project.clone())))
        .add_tool(Arc::new(RestartTool::new(client.clone(), default_project.clone())))
        .add_tool(Arc::new(PsTool::new(client.clone())))
        .add_tool(Arc::new(LogsTool::new(client.clone(), default_project.clone())))
        .add_tool(Arc::new(GrepTool::new(client.clone(), default_project.clone())))
        .add_tool(Arc::new(StatusTool::new(client, default_project)))
        .build(transport)
        .await?;
    
    server.start().await?;
    Ok(())
}

// Tool implementations that use gRPC client to communicate with mcprocd

use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};
use serde::Deserialize;
use tonic::Request;
use std::time::Duration;

// Helper function to convert status code to string
fn format_status(status: i32) -> &'static str {
    match status {
        1 => "starting",
        2 => "running",
        3 => "stopping",
        4 => "stopped",
        5 => "failed",
        _ => "unknown",
    }
}

// Start tool
struct StartTool {
    client: McpClient,
    default_project: Option<String>,
}

impl StartTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
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
}

#[async_trait]
impl ToolHandler for StartTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "start_process".to_string(),
            description: "Start and manage a long-running development process (web servers, build watchers, etc). The process will continue running in the background and can be monitored/controlled later. Use this for commands like 'npm run dev', 'python app.py', 'cargo watch', etc. Each process needs a unique name for identification.".to_string(),
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
                    }
                },
                "required": ["name"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: StartParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        // Validate that either cmd or args is provided, but not both
        match (&params.cmd, &params.args) {
            (Some(_), Some(_)) => {
                return Err(McpError::InvalidParams("Cannot specify both 'cmd' and 'args'".to_string()));
            }
            (None, None) => {
                return Err(McpError::InvalidParams("Must specify either 'cmd' or 'args'".to_string()));
            }
            (None, Some(args)) if args.is_empty() => {
                return Err(McpError::InvalidParams("args array cannot be empty".to_string()));
            }
            _ => {}
        }
        
        // Determine project name if not provided
        let project = params.project
            .or(self.default_project.clone())
            .or_else(|| {
                std::env::current_dir().ok()
                    .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                    .and_then(|n| n.into_string().ok())
            }).unwrap_or_else(|| "default".to_string());
        
        // Use gRPC client to start process
        let name = params.name.clone();
        let grpc_request = proto::StartProcessRequest {
            name: params.name,
            cmd: params.cmd,
            args: params.args.unwrap_or_default(),
            cwd: params.cwd,
            project: Some(project),
            env: params.env.unwrap_or_default(),
            wait_for_log: params.wait_for_log,
            wait_timeout: params.wait_timeout,
        };
        
        // Set timeout to wait_timeout + 5 seconds to allow for process startup
        let timeout = Duration::from_secs((params.wait_timeout.unwrap_or(30) + 5) as u64);
        let mut request = Request::new(grpc_request);
        request.set_timeout(timeout);
        
        let mut client = self.client.clone();
        match client.inner().start_process(request).await {
            Ok(response) => {
                let process = response.into_inner().process
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;
                let response = json!({
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
                
                
                Ok(response)
            }
            Err(e) => {
                if e.code() == tonic::Code::AlreadyExists {
                    Err(McpError::InvalidParams(format!("Process '{}' is already running", name)))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}

// Stop tool
struct StopTool {
    client: McpClient,
    default_project: Option<String>,
}

impl StopTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
    }
}

#[derive(Deserialize)]
struct StopParams {
    name: String,
    project: Option<String>,
}

#[async_trait]
impl ToolHandler for StopTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "stop_process".to_string(),
            description: "Gracefully stop a running process by name. This sends a SIGTERM signal to allow the process to clean up before exiting. Use this to stop servers, watchers, or any background process started with start_process.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the process to stop (the same name used when starting it with start_process)" },
                    "project": { "type": "string", "description": "Optional project name to scope the process lookup. Useful when multiple projects have processes with the same name." }
                },
                "required": ["name"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: StopParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        let request = proto::StopProcessRequest {
            name: params.name,
            force: None,
            project: params.project.or(self.default_project.clone()),
        };
        
        let mut client = self.client.clone();
        let response = client.inner()
            .stop_process(request)
            .await
            .map_err(|e| McpError::Internal(e.to_string()))?
            .into_inner();
        
        Ok(json!({
            "success": response.success,
            "message": response.message,
        }))
    }
}

// Restart tool
struct RestartTool {
    client: McpClient,
    default_project: Option<String>,
}

impl RestartTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
    }
}

#[derive(Deserialize)]
struct RestartParams {
    name: String,
    project: Option<String>,
}

#[async_trait]
impl ToolHandler for RestartTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "restart_process".to_string(),
            description: "Restart a running process by stopping it and starting it again with the same configuration. Useful when you need to reload configuration changes or recover from issues. The process will be started with the exact same command and environment as before.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the process to restart (must be currently running or recently stopped)" },
                    "project": { "type": "string", "description": "Optional project name to scope the process lookup. Useful when multiple projects have processes with the same name." }
                },
                "required": ["name"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: RestartParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        let request = proto::RestartProcessRequest {
            name: params.name.clone(),
            project: params.project.or(self.default_project.clone()),
        };
        
        let mut client = self.client.clone();
        match client.inner().restart_process(request).await {
            Ok(response) => {
                let process = response.into_inner().process
                    .ok_or_else(|| McpError::Internal("No process info returned".to_string()))?;
                Ok(json!({
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
                }))
            }
            Err(e) => {
                Err(McpError::Internal(e.message().to_string()))
            }
        }
    }
}

// Ps tool (list processes)
struct PsTool {
    client: McpClient,
}

impl PsTool {
    fn new(client: McpClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for PsTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "list_processes".to_string(),
            description: "List all processes managed by mcproc across all projects. Shows process names, status (running/stopped/failed), PIDs, start times, and detected ports. Use this to see what's currently running before starting new processes or to find process names for other commands.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
    
    async fn handle(&self, _params: Option<Value>) -> McpResult<Value> {
        let request = proto::ListProcessesRequest { 
            status_filter: None,
            project_filter: None,
        };
        
        let mut client = self.client.clone();
        let response = client.inner()
            .list_processes(request)
            .await
            .map_err(|e| McpError::Internal(e.to_string()))?
            .into_inner();
        
        let processes: Vec<Value> = response.processes.into_iter().map(|p| {
            json!({
                "id": p.id,
                "project": p.project,
                "name": p.name,
                "pid": p.pid,
                "status": format_status(p.status),
                "cmd": p.cmd,
                "log_file": p.log_file,
                "start_time": p.start_time.map(|t| {
                    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                        .unwrap_or_else(chrono::Utc::now);
                    ts.to_rfc3339()
                }),
                "ports": p.ports,
            })
        }).collect();
        
        Ok(json!({ "processes": processes }))
    }
}

// Logs tool
struct LogsTool {
    client: McpClient,
    default_project: Option<String>,
}

impl LogsTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
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
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: LogsParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        // Determine project name if not provided (use current working directory where mcproc is run)
        let project = params.project.or(self.default_project.clone()).or_else(|| {
            std::env::current_dir().ok()
                .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                .and_then(|n| n.into_string().ok())
        });
        
        // Use gRPC get_logs method instead of direct file access
        let mut client = self.client.clone();
        let request = proto::GetLogsRequest {
            name: params.name.clone(),
            tail: params.tail,
            follow: Some(false),
            project,
        };
        
        let mut stream = client.inner()
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
                        let timestamp = entry.timestamp
                            .as_ref()
                            .map(|ts| {
                                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
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

// Status tool (get detailed process status)
struct StatusTool {
    client: McpClient,
    default_project: Option<String>,
}

impl StatusTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
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
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
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

// Grep tool
struct GrepTool {
    client: McpClient,
    default_project: Option<String>,
}

impl GrepTool {
    fn new(client: McpClient, default_project: Option<String>) -> Self {
        Self { client, default_project }
    }
}

#[derive(Deserialize)]
struct GrepParams {
    pattern: String,
    name: String,
    project: Option<String>,
    context: Option<u32>,
    before: Option<u32>,
    after: Option<u32>,
    since: Option<String>,
    until: Option<String>,
    last: Option<String>,
}

#[async_trait]
impl ToolHandler for GrepTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "search_process_logs".to_string(),
            description: "Search through process logs using regex patterns to find specific errors, events, or messages. Returns matching lines with surrounding context to help understand what happened. Perfect for debugging issues like 'find all error messages' or 'show when the server started'. Searches through the entire log history, not just recent entries.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for. Examples: 'error', 'failed.*connection', 'started on port \\d+', '\\[ERROR\\]|\\[WARN\\]'" },
                    "name": { "type": "string", "description": "Name of the process whose logs to search" },
                    "project": { "type": "string", "description": "Optional project name to scope the process lookup. Useful when multiple projects have processes with the same name." },
                    "context": { "type": "integer", "description": "Number of lines to show before and after each match for context. Default is 3. Set to 0 for matches only." },
                    "before": { "type": "integer", "description": "Override context - number of lines to show before each match" },
                    "after": { "type": "integer", "description": "Override context - number of lines to show after each match" },
                    "since": { "type": "string", "description": "Only search logs after this time. Format: 'YYYY-MM-DD HH:MM' or just 'HH:MM' for today" },
                    "until": { "type": "string", "description": "Only search logs before this time. Format: 'YYYY-MM-DD HH:MM' or just 'HH:MM' for today" },
                    "last": { "type": "string", "description": "Only search recent logs. Examples: '1h' (last hour), '30m' (last 30 minutes), '2d' (last 2 days)" }
                },
                "required": ["pattern", "name"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> McpResult<Value> {
        let params = params
            .ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;
        
        let params: GrepParams = serde_json::from_value(params)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
        
        // Determine project name if not provided
        let project = params.project
            .or(self.default_project.clone())
            .or_else(|| {
                std::env::current_dir().ok()
                    .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                    .and_then(|n| n.into_string().ok())
            }).unwrap_or_else(|| "default".to_string());
        
        let request = proto::GrepLogsRequest {
            name: params.name.clone(),
            pattern: params.pattern.clone(),
            project: Some(project),
            context: params.context,
            before: params.before,
            after: params.after,
            since: params.since,
            until: params.until,
            last: params.last,
        };
        
        let mut client = self.client.clone();
        match client.inner().grep_logs(request).await {
            Ok(response) => {
                let grep_response = response.into_inner();
                
                let mut matches = Vec::new();
                
                for grep_match in grep_response.matches {
                    let mut match_obj = json!({});
                    
                    // Matched line
                    if let Some(matched_line) = grep_match.matched_line {
                        match_obj["matched_line"] = json!({
                            "line_number": matched_line.line_number,
                            "content": matched_line.content,
                            "timestamp": matched_line.timestamp.map(|t| {
                                let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                                    .unwrap_or_else(chrono::Utc::now);
                                ts.to_rfc3339()
                            }),
                            "level": if matched_line.level == 2 { "error" } else { "info" }
                        });
                    }
                    
                    // Context before
                    if !grep_match.context_before.is_empty() {
                        let context_before: Vec<Value> = grep_match.context_before.iter().map(|entry| {
                            json!({
                                "line_number": entry.line_number,
                                "content": entry.content,
                                "timestamp": entry.timestamp.as_ref().map(|t| {
                                    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                                        .unwrap_or_else(chrono::Utc::now);
                                    ts.to_rfc3339()
                                }),
                                "level": if entry.level == 2 { "error" } else { "info" }
                            })
                        }).collect();
                        match_obj["context_before"] = Value::Array(context_before);
                    }
                    
                    // Context after
                    if !grep_match.context_after.is_empty() {
                        let context_after: Vec<Value> = grep_match.context_after.iter().map(|entry| {
                            json!({
                                "line_number": entry.line_number,
                                "content": entry.content,
                                "timestamp": entry.timestamp.as_ref().map(|t| {
                                    let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                                        .unwrap_or_else(chrono::Utc::now);
                                    ts.to_rfc3339()
                                }),
                                "level": if entry.level == 2 { "error" } else { "info" }
                            })
                        }).collect();
                        match_obj["context_after"] = Value::Array(context_after);
                    }
                    
                    matches.push(match_obj);
                }
                
                let response = json!({
                    "pattern": params.pattern,
                    "process": params.name,
                    "total_matches": matches.len(),
                    "matches": matches
                });
                
                Ok(response)
            }
            Err(e) => {
                if e.code() == tonic::Code::NotFound {
                    Err(McpError::InvalidParams(format!("Log file for process \"{}\" not found", params.name)))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}

//! MCP server command implementation

use crate::client::McpClient;
use crate::cli::utils::resolve_project_name_optional;
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
    },
}

impl MpcCommand {
    pub async fn execute(self, client: McpClient) -> Result<(), Box<dyn std::error::Error>> {
        match self.subcommand {
            MpcSubcommand::Serve { name } => run_stdio_server(client, name).await,
        }
    }
}

async fn run_stdio_server(client: McpClient, name: String) -> Result<(), Box<dyn std::error::Error>> {
    use mcp_rs::{ServerBuilder, StdioTransport};
    
    // Don't print any startup messages to stderr as it may be interpreted as errors
    
    let transport = Box::new(StdioTransport::new());
    
    let mut server = ServerBuilder::new(&name, env!("CARGO_PKG_VERSION"))
        .add_tool(Arc::new(StartTool::new(client.clone())))
        .add_tool(Arc::new(StopTool::new(client.clone())))
        .add_tool(Arc::new(RestartTool::new(client.clone())))
        .add_tool(Arc::new(PsTool::new(client.clone())))
        .add_tool(Arc::new(LogsTool::new(client)))
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
}

impl StartTool {
    fn new(client: McpClient) -> Self {
        Self { client }
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
}

#[async_trait]
impl ToolHandler for StartTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "mcproc_start".to_string(),
            description: "Start a development server or process (e.g., npm run dev, python app.py, etc.)".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique name for this process (e.g., 'frontend', 'backend', 'api')" },
                    "cmd": { "type": "string", "description": "Command to execute with shell (e.g., 'npm run dev', 'yarn start', 'python app.py'). Use this for commands with pipes, redirects, or shell features." },
                    "args": { 
                        "type": "array", 
                        "items": { "type": "string" },
                        "description": "Command and arguments as array (e.g., ['npm', 'run', 'dev']). Use this for direct execution without shell."
                    },
                    "cwd": { "type": "string", "description": "Working directory path (defaults to current directory)" },
                    "project": { "type": "string", "description": "Project name (defaults to directory name)" },
                    "env": { 
                        "type": "object", 
                        "description": "Environment variables to set for the process",
                        "additionalProperties": { "type": "string" }
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
        let project = params.project.or_else(|| {
            std::env::current_dir().ok()
                .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                .and_then(|n| n.into_string().ok())
        }).unwrap_or_else(|| "default".to_string());
        
        // Use gRPC client to start process
        let name = params.name.clone();
        let request = proto::StartProcessRequest {
            name: params.name,
            cmd: params.cmd,
            args: params.args.unwrap_or_default(),
            cwd: params.cwd,
            project: Some(project),
            env: params.env.unwrap_or_default(),
        };
        
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
}

impl StopTool {
    fn new(client: McpClient) -> Self {
        Self { client }
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
            name: "mcproc_stop".to_string(),
            description: "Stop a running development server or process".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Process name or ID" },
                    "project": { "type": "string", "description": "Project name (optional, helps disambiguate)" }
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
            project: params.project,
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
}

impl RestartTool {
    fn new(client: McpClient) -> Self {
        Self { client }
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
            name: "mcproc_restart".to_string(),
            description: "Restart a development server or process".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Process name or ID" },
                    "project": { "type": "string", "description": "Project name (optional, helps disambiguate)" }
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
            project: params.project,
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
            name: "mcproc_ps".to_string(),
            description: "List all running development servers and processes managed by mcproc".to_string(),
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
            })
        }).collect();
        
        Ok(json!({ "processes": processes }))
    }
}

// Logs tool
struct LogsTool {
    client: McpClient,
}

impl LogsTool {
    fn new(client: McpClient) -> Self {
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
            name: "mcproc_logs".to_string(),
            description: "View console output and logs from a running process".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Process name" },
                    "tail": { "type": "integer", "description": "Number of lines from the end (default: 100)" },
                    "project": { "type": "string", "description": "Project name (optional, helps disambiguate)" }
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
        let project = resolve_project_name_optional(params.project);
        
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
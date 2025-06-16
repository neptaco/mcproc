//! MCP server command implementation

use crate::client::McpClient;
use clap::{Parser, Subcommand};
use std::sync::Arc;

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
                Ok(json!({
                    "id": process.id,
                    "name": process.name,
                    "pid": process.pid,
                    "status": process.status,
                    "log_file": process.log_file,
                    "start_time": process.start_time.map(|t| format!("{:?}", t)),
                }))
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
                "status": p.status,
                "cmd": p.cmd,
                "log_file": p.log_file,
                "start_time": p.start_time.map(|t| format!("{:?}", t)),
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
    from: Option<i32>,
    to: Option<i32>,
    follow: Option<bool>,
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
                    "from": { "type": "integer", "description": "Start line number" },
                    "to": { "type": "integer", "description": "End line number" },
                    "follow": { "type": "boolean", "description": "Follow logs (not supported in MCP)" },
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
        
        if params.follow.unwrap_or(false) {
            return Ok(json!({
                "error": "Follow mode is not supported in MCP context. Use range queries instead."
            }));
        }
        
        use tokio_stream::StreamExt;
        
        let request = proto::GetLogsRequest {
            name: params.name,
            from_line: params.from.map(|f| f as u32),
            to_line: params.to.map(|t| t as u32),
            follow: Some(false),
            project: params.project,
        };
        
        let mut client = self.client.clone();
        let mut stream = client.inner()
            .get_logs(request)
            .await
            .map_err(|e| McpError::Internal(e.to_string()))?
            .into_inner();
        
        let mut logs = Vec::new();
        while let Some(response) = stream.next().await {
            let response = response
                .map_err(|e| McpError::Internal(e.to_string()))?;
            for entry in response.entries {
                logs.push(entry.content);
            }
        }
        
        Ok(json!({
            "logs": logs.join("\n")
        }))
    }
}
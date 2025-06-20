//! Restart tool implementation

use crate::client::McpClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};
use serde::Deserialize;

pub struct RestartTool {
    client: McpClient,
    default_project: Option<String>,
}

impl RestartTool {
    pub fn new(client: McpClient, default_project: Option<String>) -> Self {
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
    
    async fn handle(&self, params: Option<Value>, _context: mcp_rs::ToolContext) -> McpResult<Value> {
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
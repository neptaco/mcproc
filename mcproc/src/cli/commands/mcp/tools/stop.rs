//! Stop tool implementation

use crate::client::DaemonClient;
use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};
use serde::Deserialize;

pub struct StopTool {
    client: DaemonClient,
    default_project: Option<String>,
}

impl StopTool {
    pub fn new(client: DaemonClient, default_project: Option<String>) -> Self {
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
    
    async fn handle(&self, params: Option<Value>, _context: mcp_rs::ToolContext) -> McpResult<Value> {
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
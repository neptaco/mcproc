//! Process list tool implementation

use crate::client::McpClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};

pub struct PsTool {
    client: McpClient,
}

impl PsTool {
    pub fn new(client: McpClient) -> Self {
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
    
    async fn handle(&self, _params: Option<Value>, _context: mcp_rs::ToolContext) -> McpResult<Value> {
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
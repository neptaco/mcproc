//! Process list tool implementation

use crate::client::DaemonClient;
use crate::common::status::format_status;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde_json::{json, Value};

pub struct PsTool {
    client: DaemonClient,
}

impl PsTool {
    pub fn new(client: DaemonClient) -> Self {
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

    async fn handle(
        &self,
        _params: Option<Value>,
        _context: mcp_rs::ToolContext,
    ) -> McpResult<Value> {
        let request = proto::ListProcessesRequest {
            status_filter: None,
            project_filter: None,
        };

        let mut client = self.client.clone();
        let response = client
            .inner()
            .list_processes(request)
            .await
            .map_err(|e| McpError::Internal(e.to_string()))?
            .into_inner();

        if response.processes.is_empty() {
            return Ok(json!({ "content": [{ "type": "text", "text": "No processes found." }] }));
        }

        let mut output = String::from("PROCESSES\n\n");

        for (idx, p) in response.processes.iter().enumerate() {
            if idx > 0 {
                output.push_str("\n---\n\n");
            }

            output.push_str(&format!("Process: {}\n", p.name));
            output.push_str(&format!("  ID: {}\n", p.id));
            output.push_str(&format!("  Project: {}\n", p.project));
            output.push_str(&format!("  Status: {}\n", format_status(p.status)));

            if let Some(pid) = p.pid {
                output.push_str(&format!("  PID: {}\n", pid));
            }

            output.push_str(&format!("  Command: {}\n", p.cmd));
            output.push_str(&format!("  Log file: {}\n", p.log_file));

            if let Some(start_time) = &p.start_time {
                let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(
                    start_time.seconds,
                    start_time.nanos as u32,
                )
                .unwrap_or_else(chrono::Utc::now);
                output.push_str(&format!("  Started: {}\n", ts.to_rfc3339()));
            }

            if !p.ports.is_empty() {
                let ports_str = p
                    .ports
                    .iter()
                    .map(|port| port.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                output.push_str(&format!("  Ports: {}\n", ports_str));
            }
        }

        Ok(json!({ "content": [{ "type": "text", "text": output }] }))
    }
}

//! Grep tool implementation

use crate::client::DaemonClient;
use async_trait::async_trait;
use mcp_rs::{ToolHandler, ToolInfo, Result as McpResult, Error as McpError};
use serde_json::{json, Value};
use serde::Deserialize;

pub struct GrepTool {
    client: DaemonClient,
    default_project: Option<String>,
}

impl GrepTool {
    pub fn new(client: DaemonClient, default_project: Option<String>) -> Self {
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
    
    async fn handle(&self, params: Option<Value>, _context: mcp_rs::ToolContext) -> McpResult<Value> {
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
                
                if grep_response.matches.is_empty() {
                    return Ok(json!({
                        "process": params.name,
                        "pattern": params.pattern,
                        "total_matches": 0,
                        "message": format!("No matches found for pattern '{}' in process '{}'", params.pattern, params.name)
                    }));
                }
                
                // Format timestamp helper
                let format_timestamp = |timestamp: Option<&prost_types::Timestamp>| -> String {
                    timestamp.map(|t| {
                        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(t.seconds, t.nanos as u32)
                            .unwrap_or_else(chrono::Utc::now);
                        ts.format("%Y-%m-%d %H:%M:%S").to_string()
                    }).unwrap_or_default()
                };
                
                let mut matches = Vec::new();
                
                for grep_match in grep_response.matches {
                    let mut lines = Vec::new();
                    
                    // Context before
                    for entry in &grep_match.context_before {
                        lines.push(format!("{:>6}: {} {}",
                            entry.line_number,
                            format_timestamp(entry.timestamp.as_ref()),
                            entry.content
                        ));
                    }
                    
                    // Matched line (highlighted)
                    let matched_line_info = if let Some(matched_line) = &grep_match.matched_line {
                        lines.push(format!("{:>6}: {} {} <<< MATCH",
                            matched_line.line_number,
                            format_timestamp(matched_line.timestamp.as_ref()),
                            matched_line.content
                        ));
                        
                        Some(json!({
                            "line_number": matched_line.line_number,
                            "timestamp": format_timestamp(matched_line.timestamp.as_ref()),
                            "content": matched_line.content
                        }))
                    } else {
                        None
                    };
                    
                    // Context after
                    for entry in &grep_match.context_after {
                        lines.push(format!("{:>6}: {} {}",
                            entry.line_number,
                            format_timestamp(entry.timestamp.as_ref()),
                            entry.content
                        ));
                    }
                    
                    matches.push(json!({
                        "match_info": matched_line_info,
                        "context": lines.join("\n")
                    }));
                }
                
                Ok(json!({
                    "process": params.name,
                    "pattern": params.pattern,
                    "total_matches": matches.len(),
                    "matches": matches
                }))
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
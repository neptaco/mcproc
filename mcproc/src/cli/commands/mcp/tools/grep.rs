//! Grep tool implementation

use crate::cli::utils::resolve_mcp_project_name;
use crate::client::DaemonClient;
use async_trait::async_trait;
use mcp_rs::{Error as McpError, Result as McpResult, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use strip_ansi_escapes::strip;

pub struct GrepTool {
    client: DaemonClient,
}

impl GrepTool {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
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

    async fn handle(
        &self,
        params: Option<Value>,
        _context: mcp_rs::ToolContext,
    ) -> McpResult<Value> {
        let params =
            params.ok_or_else(|| McpError::InvalidParams("Missing parameters".to_string()))?;

        let params: GrepParams =
            serde_json::from_value(params).map_err(|e| McpError::InvalidParams(e.to_string()))?;

        // Determine project name if not provided
        let project = resolve_mcp_project_name(params.project)?;

        let request = proto::GrepLogsRequest {
            name: params.name.clone(),
            pattern: params.pattern.clone(),
            project,
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
                    timestamp
                        .map(|t| {
                            let ts = chrono::DateTime::<chrono::Utc>::from_timestamp(
                                t.seconds,
                                t.nanos as u32,
                            )
                            .unwrap_or_else(chrono::Utc::now);
                            ts.format("%Y-%m-%d %H:%M:%S").to_string()
                        })
                        .unwrap_or_default()
                };

                let mut matches = Vec::new();

                for grep_match in grep_response.matches {
                    let mut lines = Vec::new();

                    // Context before
                    for entry in &grep_match.context_before {
                        let content = String::from_utf8_lossy(&strip(entry.content.as_bytes())).to_string();
                        lines.push(format!(
                            "{:>6}: {} {}",
                            entry.line_number,
                            format_timestamp(entry.timestamp.as_ref()),
                            content
                        ));
                    }

                    // Matched line (highlighted)
                    let matched_line_info = if let Some(matched_line) = &grep_match.matched_line {
                        let content = String::from_utf8_lossy(&strip(matched_line.content.as_bytes())).to_string();
                        lines.push(format!(
                            "{:>6}: {} {} <<< MATCH",
                            matched_line.line_number,
                            format_timestamp(matched_line.timestamp.as_ref()),
                            content
                        ));

                        Some(json!({
                            "line_number": matched_line.line_number,
                            "timestamp": format_timestamp(matched_line.timestamp.as_ref()),
                            "content": content
                        }))
                    } else {
                        None
                    };

                    // Context after
                    for entry in &grep_match.context_after {
                        let content = String::from_utf8_lossy(&strip(entry.content.as_bytes())).to_string();
                        lines.push(format!(
                            "{:>6}: {} {}",
                            entry.line_number,
                            format_timestamp(entry.timestamp.as_ref()),
                            content
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
                    Err(McpError::InvalidParams(format!(
                        "Log file for process \"{}\" not found",
                        params.name
                    )))
                } else {
                    Err(McpError::Internal(e.message().to_string()))
                }
            }
        }
    }
}

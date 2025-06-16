//! Example of MCP server with SSE transport

use async_trait::async_trait;
use mcp_rs::{Result, ServerBuilder, SseTransport, SseTransportConfig, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct EchoParams {
    message: String,
}

struct EchoTool;

#[async_trait]
impl ToolHandler for EchoTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "echo".to_string(),
            description: Some("Echoes back the message".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> Result<Value> {
        let params = params
            .ok_or_else(|| mcp_rs::Error::InvalidParams("Missing parameters".to_string()))?;
        
        let params: EchoParams = serde_json::from_value(params)
            .map_err(|e| mcp_rs::Error::InvalidParams(e.to_string()))?;
        
        Ok(json!({ 
            "echoed": params.message,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Configure SSE transport
    let config = SseTransportConfig {
        addr: ([127, 0, 0, 1], 3435).into(),
        events_path: "/events".to_string(),
        messages_path: "/messages".to_string(),
    };
    
    // Create server with SSE transport
    let transport = Box::new(SseTransport::new(config));
    
    let mut server = ServerBuilder::new("sse-example", "0.1.0")
        .add_tool(Arc::new(EchoTool))
        .build(transport)
        .await?;
    
    println!("SSE MCP server running on http://localhost:3435");
    println!("  POST /messages - Send JSON-RPC messages");
    println!("  GET  /events   - Connect to SSE stream");
    
    // Start server
    server.start().await?;
    
    Ok(())
}
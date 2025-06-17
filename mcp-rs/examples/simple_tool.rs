//! Example of creating a simple MCP tool

use async_trait::async_trait;
use mcp_rs::{Result, ServerBuilder, StdioTransport, ToolHandler, ToolInfo};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct CalculatorParams {
    operation: String,
    a: f64,
    b: f64,
}

struct CalculatorTool;

#[async_trait]
impl ToolHandler for CalculatorTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "calculator".to_string(),
            description: "Simple calculator tool".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["add", "subtract", "multiply", "divide"]
                    },
                    "a": { "type": "number" },
                    "b": { "type": "number" }
                },
                "required": ["operation", "a", "b"]
            }),
        }
    }
    
    async fn handle(&self, params: Option<Value>) -> Result<Value> {
        let params = params
            .ok_or_else(|| mcp_rs::Error::InvalidParams("Missing parameters".to_string()))?;
        
        let params: CalculatorParams = serde_json::from_value(params)
            .map_err(|e| mcp_rs::Error::InvalidParams(e.to_string()))?;
        
        let result = match params.operation.as_str() {
            "add" => params.a + params.b,
            "subtract" => params.a - params.b,
            "multiply" => params.a * params.b,
            "divide" => {
                if params.b == 0.0 {
                    return Err(mcp_rs::Error::InvalidParams("Division by zero".to_string()));
                }
                params.a / params.b
            }
            _ => return Err(mcp_rs::Error::InvalidParams("Invalid operation".to_string())),
        };
        
        Ok(json!({ "result": result }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    // Create server with stdio transport
    let transport = Box::new(StdioTransport::new());
    
    let mut server = ServerBuilder::new("calculator-mcp", "0.1.0")
        .add_tool(Arc::new(CalculatorTool))
        .build(transport)
        .await?;
    
    // Start server
    server.start().await?;
    
    Ok(())
}
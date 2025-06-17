# mcp-rs

A Rust implementation of the Model Context Protocol (MCP), providing a standardized way for AI models to interact with external tools and resources.

## Features

- **Multiple Transports**:
  - **stdio**: For local CLI tools and integrations
  - **Streamable HTTP**: Single `/mcp` endpoint (MCP specification compliant)
  - **SSE**: Separate `/events` and `/messages` endpoints (TypeScript SDK compatible)
  
- **Easy Tool Creation**: Simple `ToolHandler` trait for implementing custom tools
- **Full JSON-RPC 2.0 Support**: Including batching and notifications
- **Type-safe**: Leverages Rust's type system for safety

## Usage

### Creating a Simple Tool

```rust
use mcp_rs::{ToolHandler, ToolInfo, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

struct MyTool;

#[async_trait]
impl ToolHandler for MyTool {
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: "my_tool".to_string(),
            description: Some("A simple example tool".to_string()),
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
        let message = params
            .and_then(|p| p.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("Hello");
            
        Ok(json!({ "response": format!("You said: {}", message) }))
    }
}
```

### Starting a Server

#### With stdio Transport (for CLI tools)

```rust
use mcp_rs::{ServerBuilder, StdioTransport};
use std::sync::Arc;

#[tokio::main]
async fn main() -> mcp_rs::Result<()> {
    let mut server = ServerBuilder::new("my-server", "1.0.0")
        .add_tool(Arc::new(MyTool))
        .build(Box::new(StdioTransport::new()))
        .await?;
        
    server.start().await
}
```

#### With Streamable HTTP Transport

```rust
use mcp_rs::{ServerBuilder, HttpTransport, HttpTransportConfig};

let config = HttpTransportConfig {
    addr: ([127, 0, 0, 1], 3434).into(),
    path: "/mcp".to_string(),
};

let mut server = ServerBuilder::new("my-server", "1.0.0")
    .add_tool(Arc::new(MyTool))
    .build(Box::new(HttpTransport::new(config)))
    .await?;
```

#### With SSE Transport

```rust
use mcp_rs::{ServerBuilder, SseTransport, SseTransportConfig};

let config = SseTransportConfig {
    addr: ([127, 0, 0, 1], 3435).into(),
    events_path: "/events".to_string(),
    messages_path: "/messages".to_string(),
};

let mut server = ServerBuilder::new("my-server", "1.0.0")
    .add_tool(Arc::new(MyTool))
    .build(Box::new(SseTransport::new(config)))
    .await?;
```

## Transport Comparison

| Transport | Use Case | Endpoints | Communication |
|-----------|----------|-----------|---------------|
| stdio | Local CLI tools | - | stdin/stdout |
| Streamable HTTP | Web services, modern MCP clients | `POST /mcp`, `GET /mcp` | HTTP + optional SSE |
| SSE | TypeScript SDK compatibility | `POST /messages`, `GET /events` | HTTP + SSE |

## License

MIT
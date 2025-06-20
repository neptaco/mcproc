//! MCP server command implementation

pub mod tools;

use crate::client::DaemonClient;
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Parser)]
pub struct McpCommand {
    #[command(subcommand)]
    command: McpSubcommands,
}

#[derive(Subcommand)]
enum McpSubcommands {
    /// Start the MCP server
    Serve,
}

impl McpCommand {
    pub async fn execute(self, client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        match self.command {
            McpSubcommands::Serve => {
                serve_mcp(client).await
            }
        }
    }
}

async fn serve_mcp(client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
    use mcp_rs::{ServerBuilder, StdioTransport};
    use tools::{StartTool, StopTool, RestartTool, PsTool, LogsTool, StatusTool, GrepTool};
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    // Configure tracing to output to stderr to avoid interfering with JSON-RPC on stdout
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::from_default_env()
                .add_directive("mcproc=warn".parse()?)
                .add_directive("mcp_rs=warn".parse()?)
        )
        .init();

    // Create server with stdio transport
    let transport = Box::new(StdioTransport::new());
    
    let mut server = ServerBuilder::new("mcproc", "0.1.0")
        .add_tool(Arc::new(StartTool::new(client.clone())))
        .add_tool(Arc::new(StopTool::new(client.clone(), None)))
        .add_tool(Arc::new(RestartTool::new(client.clone(), None)))
        .add_tool(Arc::new(PsTool::new(client.clone())))
        .add_tool(Arc::new(LogsTool::new(client.clone(), None)))
        .add_tool(Arc::new(StatusTool::new(client.clone(), None)))
        .add_tool(Arc::new(GrepTool::new(client.clone(), None)))
        .build(transport)
        .await?;
    
    // Start server
    server.start().await?;
    
    Ok(())
}
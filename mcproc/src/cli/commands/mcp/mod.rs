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
    Serve {
        /// Default project name for all MCP operations
        #[arg(long)]
        project: Option<String>,
    },
}

impl McpCommand {
    pub async fn execute(self, client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        match self.command {
            McpSubcommands::Serve { project } => serve_mcp(client, project).await,
        }
    }
}

async fn serve_mcp(
    client: DaemonClient,
    default_project: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::common::validation::validate_project_name;
    use crate::common::version::VERSION;
    use mcp_rs::{ServerBuilder, StdioTransport};
    use tools::{GrepTool, LogsTool, PsTool, RestartTool, StartTool, StatusTool, StopTool};
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    // Validate and set default project as environment variable if provided
    if let Some(project) = default_project {
        validate_project_name(&project)?;
        std::env::set_var("MCPROC_DEFAULT_PROJECT", project);
    }

    // Configure tracing to output to stderr to avoid interfering with JSON-RPC on stdout
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::from_default_env()
                .add_directive("mcproc=warn".parse()?)
                .add_directive("mcp_rs=warn".parse()?),
        )
        .init();

    // Create server with stdio transport
    let transport = Box::new(StdioTransport::new());

    let mut server = ServerBuilder::new("mcproc", VERSION)
        .add_tool(Arc::new(StartTool::new(client.clone())))
        .add_tool(Arc::new(StopTool::new(client.clone())))
        .add_tool(Arc::new(RestartTool::new(client.clone())))
        .add_tool(Arc::new(PsTool::new(client.clone())))
        .add_tool(Arc::new(LogsTool::new(client.clone())))
        .add_tool(Arc::new(StatusTool::new(client.clone())))
        .add_tool(Arc::new(GrepTool::new(client.clone())))
        .build(transport)
        .await?;

    // Start server
    server.start().await?;

    Ok(())
}

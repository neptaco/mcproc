use crate::error::Result;
use crate::protocol::{Protocol, ToolHandler};
use crate::transport::Transport;
use std::sync::Arc;
use tracing::{debug, info};

/// MCP Server
pub struct Server {
    protocol: Arc<Protocol>,
    transport: Box<dyn Transport>,
}

impl Server {
    /// Create a new server with the given protocol and transport
    pub fn new(protocol: Arc<Protocol>, transport: Box<dyn Transport>) -> Self {
        Self { protocol, transport }
    }
    
    /// Start the server
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting MCP server");
        
        // Start transport
        self.transport.start().await?;
        
        // Main message loop
        loop {
            match self.transport.receive().await? {
                Some(message) => {
                    debug!("Received message: {:?}", message);
                    
                    // Handle message
                    if let Some(response) = self.protocol.handle_message(message).await {
                        debug!("Sending response: {:?}", response);
                        self.transport.send(response).await?;
                    }
                }
                None => {
                    info!("Transport closed, shutting down server");
                    break;
                }
            }
        }
        
        // Close transport
        self.transport.close().await?;
        
        Ok(())
    }
}

/// Server builder
pub struct ServerBuilder {
    name: String,
    version: String,
    tools: Vec<Arc<dyn ToolHandler>>,
}

impl ServerBuilder {
    /// Create a new server builder
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            tools: Vec::new(),
        }
    }
    
    /// Add a tool handler
    pub fn add_tool(mut self, tool: Arc<dyn ToolHandler>) -> Self {
        self.tools.push(tool);
        self
    }
    
    /// Build the server with the given transport
    pub async fn build(self, transport: Box<dyn Transport>) -> Result<Server> {
        let protocol = Arc::new(Protocol::new(self.name, self.version));
        
        // Register tools
        for tool in self.tools {
            protocol.register_tool(tool).await;
        }
        
        Ok(Server::new(protocol, transport))
    }
}
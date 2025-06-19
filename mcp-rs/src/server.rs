use crate::error::Result;
use crate::notification::NotificationSender;
use crate::protocol::{Protocol, ToolHandler};
use crate::transport::Transport;
use crate::types::{JsonRpcMessage, JsonRpcNotification, MessageNotification, ProgressNotification};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// Real-time notification sender that sends notifications immediately
struct RealtimeNotificationSender {
    tx: mpsc::Sender<JsonRpcNotification>,
}

impl RealtimeNotificationSender {
    fn new(tx: mpsc::Sender<JsonRpcNotification>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl NotificationSender for RealtimeNotificationSender {
    async fn send_message(&self, notification: MessageNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/message".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };
        
        self.tx.send(notif).await
            .map_err(|_| crate::error::Error::Internal("Failed to send notification".to_string()))?;
        Ok(())
    }
    
    async fn send_progress(&self, notification: ProgressNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/progress".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };
        
        self.tx.send(notif).await
            .map_err(|_| crate::error::Error::Internal("Failed to send notification".to_string()))?;
        Ok(())
    }
    
    async fn send_raw(&self, method: String, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        };
        
        self.tx.send(notif).await
            .map_err(|_| crate::error::Error::Internal("Failed to send notification".to_string()))?;
        Ok(())
    }
}

/// MCP Server
pub struct Server {
    protocol: Arc<Protocol>,
    transport: Box<dyn Transport>,
    notification_tx: mpsc::Sender<JsonRpcNotification>,
}

impl Server {
    /// Create a new server with the given protocol and transport
    pub fn new(protocol: Arc<Protocol>, transport: Box<dyn Transport>, notification_tx: mpsc::Sender<JsonRpcNotification>) -> Self {
        Self { protocol, transport, notification_tx }
    }
    
    /// Start the server
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting MCP server");
        
        // Start transport
        self.transport.start().await?;
        
        // Create notification channel
        let (notification_tx, mut notification_rx) = mpsc::channel::<JsonRpcNotification>(100);
        
        // Create real-time notification sender
        let realtime_sender = Arc::new(RealtimeNotificationSender::new(notification_tx.clone()));
        self.protocol.set_notification_sender(realtime_sender).await;
        
        // Main message loop
        loop {
            tokio::select! {
                // Handle incoming messages
                message = self.transport.receive() => {
                    match message? {
                        Some(message) => {
                            debug!("Received message: {:?}", message);
                            
                            // Handle message and get response
                            let response = self.protocol.handle_message_realtime(message).await;
                            
                            // Send the response if any
                            if let Some(response) = response {
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
                // Handle notifications
                Some(notification) = notification_rx.recv() => {
                    debug!("Sending notification: {:?}", notification);
                    self.transport.send(JsonRpcMessage::Notification(notification)).await?;
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
        
        // Create notification channel
        let (notification_tx, _) = mpsc::channel(100);
        
        Ok(Server::new(protocol, transport, notification_tx))
    }
}
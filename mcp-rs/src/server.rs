use crate::error::Result;
use crate::notification::NotificationSender;
use crate::protocol::{Protocol, ToolHandler};
use crate::transport::Transport;
use crate::types::{
    JsonRpcMessage, JsonRpcNotification, MessageNotification, ProgressNotification,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// Real-time notification sender that sends notifications immediately
struct RealtimeNotificationSender {
    tx: mpsc::UnboundedSender<JsonRpcNotification>,
}

impl RealtimeNotificationSender {
    fn new(tx: mpsc::UnboundedSender<JsonRpcNotification>) -> Self {
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

        self.tx.send(notif).map_err(|_| {
            crate::error::Error::Internal("Failed to send notification".to_string())
        })?;
        Ok(())
    }

    async fn send_progress(&self, notification: ProgressNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/progress".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };

        self.tx.send(notif).map_err(|_| {
            crate::error::Error::Internal("Failed to send notification".to_string())
        })?;
        Ok(())
    }

    async fn send_raw(&self, method: String, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        };

        self.tx.send(notif).map_err(|_| {
            crate::error::Error::Internal("Failed to send notification".to_string())
        })?;
        Ok(())
    }
}

/// MCP Server
pub struct Server {
    protocol: Arc<Protocol>,
    transport: Box<dyn Transport>,
}

impl Server {
    /// Create a new server with the given protocol and transport
    pub fn new(protocol: Arc<Protocol>, transport: Box<dyn Transport>) -> Self {
        Self {
            protocol,
            transport,
        }
    }

    /// Start the server
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting MCP server");

        // Start transport
        self.transport.start().await?;

        // Create notification channel
        let (notification_tx, mut notification_rx) =
            mpsc::unbounded_channel::<JsonRpcNotification>();

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

        Ok(Server::new(protocol, transport))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{JsonRpcId, JsonRpcRequest, MessageLevel, ToolInfo};
    use serde_json::json;
    use std::collections::VecDeque;

    struct MockTransport {
        incoming: VecDeque<JsonRpcMessage>,
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn start(&mut self) -> Result<()> {
            Ok(())
        }

        async fn send(&mut self, _message: JsonRpcMessage) -> Result<()> {
            Ok(())
        }

        async fn receive(&mut self) -> Result<Option<JsonRpcMessage>> {
            Ok(self.incoming.pop_front())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    struct NoisyTool;

    #[async_trait]
    impl ToolHandler for NoisyTool {
        fn tool_info(&self) -> ToolInfo {
            ToolInfo {
                name: "noisy".to_string(),
                description: "sends many notifications".to_string(),
                input_schema: json!({"type": "object"}),
            }
        }

        async fn handle(
            &self,
            _params: Option<Value>,
            context: crate::notification::ToolContext,
        ) -> Result<Value> {
            for index in 0..150 {
                context
                    .send_log(MessageLevel::Info, format!("notification {index}"))
                    .await?;
            }
            Ok(json!({"done": true}))
        }
    }

    #[tokio::test]
    async fn tool_call_with_more_than_channel_capacity_does_not_deadlock() {
        let transport = MockTransport {
            incoming: VecDeque::from([JsonRpcMessage::Request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: "tools/call".to_string(),
                params: Some(json!({"name": "noisy", "arguments": {}})),
                id: JsonRpcId::Number(1),
            })]),
        };
        let mut server = ServerBuilder::new("test", "1")
            .add_tool(Arc::new(NoisyTool))
            .build(Box::new(transport))
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), server.start())
            .await
            .expect("server deadlocked while tool sent notifications")
            .unwrap();
    }
}

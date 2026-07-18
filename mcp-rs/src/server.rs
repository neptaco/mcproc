use crate::error::{Error, Result};
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
use tokio::task::JoinSet;
use tracing::{debug, info};

const OUTGOING_CHANNEL_CAPACITY: usize = 1024;

/// Real-time notification sender that sends notifications immediately
struct RealtimeNotificationSender {
    tx: mpsc::Sender<JsonRpcMessage>,
}

impl RealtimeNotificationSender {
    fn new(tx: mpsc::Sender<JsonRpcMessage>) -> Self {
        Self { tx }
    }

    async fn send_notification(&self, notification: JsonRpcNotification) -> Result<()> {
        self.tx
            .send(JsonRpcMessage::Notification(notification))
            .await
            .map_err(|_| Error::Internal("Failed to send notification".to_string()))
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

        self.send_notification(notif).await
    }

    async fn send_progress(&self, notification: ProgressNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/progress".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };

        self.send_notification(notif).await
    }

    async fn send_raw(&self, method: String, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        };

        self.send_notification(notif).await
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

        // Use one bounded queue for notifications and responses so backpressure and ordering
        // are preserved while handlers run independently from transport I/O.
        let (outgoing_tx, mut outgoing_rx) =
            mpsc::channel::<JsonRpcMessage>(OUTGOING_CHANNEL_CAPACITY);

        // Create real-time notification sender
        let realtime_sender = Arc::new(RealtimeNotificationSender::new(outgoing_tx.clone()));
        self.protocol.set_notification_sender(realtime_sender).await;

        let mut handler_tasks: JoinSet<Result<()>> = JoinSet::new();

        // Main message loop
        loop {
            tokio::select! {
                // Handle incoming messages
                message = self.transport.receive() => {
                    match message? {
                        Some(message) => {
                            debug!("Received message: {:?}", message);

                            let protocol = self.protocol.clone();
                            let response_tx = outgoing_tx.clone();
                            handler_tasks.spawn(async move {
                                if let Some(response) = protocol.handle_message_realtime(message).await {
                                    response_tx.send(response).await.map_err(|_| {
                                        Error::Internal("Failed to send response".to_string())
                                    })?;
                                }
                                Ok(())
                            });
                        }
                        None => {
                            info!("Transport closed, draining server work");
                            break;
                        }
                    }
                }
                Some(message) = outgoing_rx.recv() => {
                    debug!("Sending message: {:?}", message);
                    self.transport.send(message).await?;
                }
                task_result = handler_tasks.join_next(), if !handler_tasks.is_empty() => {
                    if let Some(task_result) = task_result {
                        task_result
                            .map_err(|error| Error::Internal(format!("Message handler task failed: {error}")))??;
                    }
                }
            }
        }

        // A handler may be blocked on the bounded queue when receive returns None. Keep
        // draining messages while joining every task, then flush anything queued by the last
        // completed task before closing the transport.
        while !handler_tasks.is_empty() || !outgoing_rx.is_empty() {
            tokio::select! {
                Some(message) = outgoing_rx.recv() => {
                    debug!("Sending message during shutdown: {:?}", message);
                    self.transport.send(message).await?;
                }
                task_result = handler_tasks.join_next(), if !handler_tasks.is_empty() => {
                    if let Some(task_result) = task_result {
                        task_result
                            .map_err(|error| Error::Internal(format!("Message handler task failed: {error}")))??;
                    }
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
    use std::sync::Mutex;

    struct MockTransport {
        incoming: VecDeque<JsonRpcMessage>,
        sent: Arc<Mutex<Vec<JsonRpcMessage>>>,
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn start(&mut self) -> Result<()> {
            Ok(())
        }

        async fn send(&mut self, message: JsonRpcMessage) -> Result<()> {
            self.sent.lock().unwrap().push(message);
            Ok(())
        }

        async fn receive(&mut self) -> Result<Option<JsonRpcMessage>> {
            Ok(self.incoming.pop_front())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    struct NoisyTool {
        notification_count: usize,
    }

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
            for index in 0..self.notification_count {
                context
                    .send_log(MessageLevel::Info, format!("notification {index}"))
                    .await?;
            }
            Ok(json!({"done": true}))
        }
    }

    #[tokio::test]
    async fn tool_call_with_more_than_channel_capacity_does_not_deadlock() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            incoming: VecDeque::from([tool_call_request()]),
            sent,
        };
        let mut server = ServerBuilder::new("test", "1")
            .add_tool(Arc::new(NoisyTool {
                notification_count: 150,
            }))
            .build(Box::new(transport))
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(5), server.start())
            .await
            .expect("server deadlocked while tool sent notifications")
            .unwrap();
    }

    #[tokio::test]
    async fn notifications_are_sent_before_tool_response() {
        let sent = run_noisy_tool(150).await;
        let messages = sent.lock().unwrap();

        assert!(
            matches!(messages.first(), Some(JsonRpcMessage::Notification(_))),
            "the first outgoing message should be a notification, got {messages:?}"
        );
        assert_eq!(messages.len(), 151, "all notifications and the response");

        let response_index = messages
            .iter()
            .position(|message| matches!(message, JsonRpcMessage::Response(_)))
            .expect("tool response was not sent");
        assert!(
            messages[..response_index]
                .iter()
                .all(|message| matches!(message, JsonRpcMessage::Notification(_))),
            "all tool notifications should precede its response"
        );
        assert_eq!(response_index, 150);
    }

    #[tokio::test]
    async fn tool_call_exceeding_outgoing_channel_capacity_sends_every_message() {
        let sent = tokio::time::timeout(std::time::Duration::from_secs(5), run_noisy_tool(1_500))
            .await
            .expect("server deadlocked when the outgoing channel reached capacity");
        let messages = sent.lock().unwrap();

        assert_eq!(
            messages
                .iter()
                .filter(|message| matches!(message, JsonRpcMessage::Notification(_)))
                .count(),
            1_500
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| matches!(message, JsonRpcMessage::Response(_)))
                .count(),
            1
        );
    }

    fn tool_call_request() -> JsonRpcMessage {
        JsonRpcMessage::Request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: Some(json!({"name": "noisy", "arguments": {}})),
            id: JsonRpcId::Number(1),
        })
    }

    async fn run_noisy_tool(notification_count: usize) -> Arc<Mutex<Vec<JsonRpcMessage>>> {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let transport = MockTransport {
            incoming: VecDeque::from([tool_call_request()]),
            sent: sent.clone(),
        };
        let mut server = ServerBuilder::new("test", "1")
            .add_tool(Arc::new(NoisyTool { notification_count }))
            .build(Box::new(transport))
            .await
            .unwrap();

        server.start().await.unwrap();
        sent
    }
}

use crate::error::Result;
use crate::types::{JsonRpcNotification, MessageNotification, ProgressNotification};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Trait for sending MCP notifications
#[async_trait]
pub trait NotificationSender: Send + Sync {
    /// Send a message notification
    async fn send_message(&self, notification: MessageNotification) -> Result<()>;
    
    /// Send a progress notification
    async fn send_progress(&self, notification: ProgressNotification) -> Result<()>;
    
    /// Send a raw JSON-RPC notification
    async fn send_raw(&self, method: String, params: Option<Value>) -> Result<()>;
}

/// Context information for tool execution
#[derive(Clone)]
pub struct ToolContext {
    /// Notification sender for sending MCP notifications
    pub notification_sender: Arc<dyn NotificationSender>,
    
    /// Progress token if provided in request metadata
    pub progress_token: Option<String>,
    
    /// Request ID for correlation
    pub request_id: Option<crate::types::JsonRpcId>,
}

impl ToolContext {
    /// Create a new tool context
    pub fn new(
        notification_sender: Arc<dyn NotificationSender>,
        progress_token: Option<String>,
        request_id: Option<crate::types::JsonRpcId>,
    ) -> Self {
        Self {
            notification_sender,
            progress_token,
            request_id,
        }
    }
    
    /// Send a log message notification
    pub async fn send_log(&self, level: crate::types::MessageLevel, message: String) -> Result<()> {
        let notification = MessageNotification {
            level,
            logger: Some("mcproc".to_string()),
            data: json!({ "message": message }),
        };
        self.notification_sender.send_message(notification).await
    }
    
    /// Send a progress update if progress token is available
    pub async fn send_progress(&self, progress: u64, total: u64, message: Option<String>) -> Result<()> {
        if let Some(ref token) = self.progress_token {
            let notification = ProgressNotification {
                progress_token: token.clone(),
                progress,
                total,
                message,
            };
            self.notification_sender.send_progress(notification).await
        } else {
            Ok(())
        }
    }
}

/// Internal notification sender that queues notifications
pub struct QueuedNotificationSender {
    queue: Arc<RwLock<Vec<JsonRpcNotification>>>,
}

impl Default for QueuedNotificationSender {
    fn default() -> Self {
        Self {
            queue: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl QueuedNotificationSender {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub async fn take_all(&self) -> Vec<JsonRpcNotification> {
        let mut queue = self.queue.write().await;
        std::mem::take(&mut *queue)
    }
}

#[async_trait]
impl NotificationSender for QueuedNotificationSender {
    async fn send_message(&self, notification: MessageNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/message".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };
        
        let mut queue = self.queue.write().await;
        queue.push(notif);
        Ok(())
    }
    
    async fn send_progress(&self, notification: ProgressNotification) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/progress".to_string(),
            params: Some(serde_json::to_value(notification)?),
        };
        
        let mut queue = self.queue.write().await;
        queue.push(notif);
        Ok(())
    }
    
    async fn send_raw(&self, method: String, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        };
        
        let mut queue = self.queue.write().await;
        queue.push(notif);
        Ok(())
    }
}
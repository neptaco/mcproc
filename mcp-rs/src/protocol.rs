use crate::error::{Error, Result};
use crate::types::*;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Handler trait for MCP methods
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Get tool information
    fn tool_info(&self) -> ToolInfo;

    /// Handle tool call with context
    async fn handle(
        &self,
        params: Option<Value>,
        context: crate::notification::ToolContext,
    ) -> Result<Value>;
}

/// MCP Protocol implementation
pub struct Protocol {
    server_info: ServerInfo,
    tools: Arc<RwLock<HashMap<String, Arc<dyn ToolHandler>>>>,
    custom_handlers: Arc<RwLock<HashMap<String, Box<dyn McpHandler>>>>,
    notification_sender: Arc<RwLock<Option<Arc<dyn crate::notification::NotificationSender>>>>,
}

/// Generic handler for custom methods
#[async_trait]
pub trait McpHandler: Send + Sync {
    async fn handle(&self, params: Option<Value>) -> Result<Value>;
}

impl Protocol {
    pub fn new(name: String, version: String) -> Self {
        Self {
            server_info: ServerInfo { name, version },
            tools: Arc::new(RwLock::new(HashMap::new())),
            custom_handlers: Arc::new(RwLock::new(HashMap::new())),
            notification_sender: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the notification sender
    pub async fn set_notification_sender(
        &self,
        sender: Arc<dyn crate::notification::NotificationSender>,
    ) {
        let mut ns = self.notification_sender.write().await;
        *ns = Some(sender);
    }

    /// Register a tool handler
    pub async fn register_tool(&self, handler: Arc<dyn ToolHandler>) {
        let tool_info = handler.tool_info();
        let mut tools = self.tools.write().await;
        tools.insert(tool_info.name.clone(), handler);
    }

    /// Register a custom method handler
    pub async fn register_custom_handler(&self, method: String, handler: Box<dyn McpHandler>) {
        let mut handlers = self.custom_handlers.write().await;
        handlers.insert(method, handler);
    }

    /// Handle incoming message and return response and any notifications
    pub async fn handle_message(
        &self,
        message: JsonRpcMessage,
    ) -> (Option<JsonRpcMessage>, Vec<JsonRpcNotification>) {
        // Create a queued notification sender for this request
        let queued_sender = Arc::new(crate::notification::QueuedNotificationSender::new());

        // Temporarily set it as the active sender
        {
            let mut ns = self.notification_sender.write().await;
            *ns = Some(queued_sender.clone());
        }

        let response = match message {
            JsonRpcMessage::Request(req) => {
                let response = self.handle_request(req, queued_sender.clone()).await;
                Some(JsonRpcMessage::Response(response))
            }
            JsonRpcMessage::Notification(notif) => {
                self.handle_notification(notif).await;
                None
            }
            JsonRpcMessage::Batch(batch) => {
                let mut responses = Vec::new();
                for msg in batch {
                    let (resp, _) = Box::pin(self.handle_message(msg)).await;
                    if let Some(r) = resp {
                        responses.push(r);
                    }
                }
                if responses.is_empty() {
                    None
                } else {
                    Some(JsonRpcMessage::Batch(responses))
                }
            }
            JsonRpcMessage::Response(_) => None, // Server doesn't handle responses
        };

        // Collect any notifications that were queued
        let notifications = queued_sender.take_all().await;

        (response, notifications)
    }

    pub async fn handle_message_realtime(&self, message: JsonRpcMessage) -> Option<JsonRpcMessage> {
        match message {
            JsonRpcMessage::Request(req) => {
                // Use the real-time notification sender if available
                let ns = self.notification_sender.read().await;
                let sender = if let Some(ref sender) = *ns {
                    sender.clone()
                } else {
                    // Fallback to queued sender
                    Arc::new(crate::notification::QueuedNotificationSender::new())
                };
                drop(ns);

                Some(JsonRpcMessage::Response(
                    self.handle_request_with_sender(req, sender).await,
                ))
            }
            JsonRpcMessage::Notification(notif) => {
                self.handle_notification(notif).await;
                None
            }
            JsonRpcMessage::Batch(batch) => {
                let mut responses = Vec::new();
                for msg in batch {
                    if let Some(resp) = Box::pin(self.handle_message_realtime(msg)).await {
                        responses.push(resp);
                    }
                }
                if responses.is_empty() {
                    None
                } else {
                    Some(JsonRpcMessage::Batch(responses))
                }
            }
            JsonRpcMessage::Response(_) => None, // Server doesn't handle responses
        }
    }

    async fn handle_request(
        &self,
        req: JsonRpcRequest,
        notification_sender: Arc<crate::notification::QueuedNotificationSender>,
    ) -> JsonRpcResponse {
        self.handle_request_with_sender(
            req,
            notification_sender as Arc<dyn crate::notification::NotificationSender>,
        )
        .await
    }

    async fn handle_request_with_sender(
        &self,
        req: JsonRpcRequest,
        notification_sender: Arc<dyn crate::notification::NotificationSender>,
    ) -> JsonRpcResponse {
        let result = match req.method.parse::<McpMethod>().unwrap() {
            McpMethod::Initialize => self.handle_initialize(req.params).await,
            McpMethod::ToolsList => self.handle_tools_list(req.params).await,
            McpMethod::ToolsCall => {
                self.handle_tools_call(req.params, req.id.clone(), notification_sender)
                    .await
            }
            McpMethod::Shutdown => self.handle_shutdown(req.params).await,
            McpMethod::Custom(method) => {
                self.handle_custom(&method, req.params, req.id.clone(), notification_sender)
                    .await
            }
        };

        match result {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(result),
                error: None,
                id: req.id,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(self.error_to_json_rpc(error)),
                id: req.id,
            },
        }
    }

    async fn handle_notification(&self, _notif: JsonRpcNotification) {
        // Notifications don't require a response
        // Log or handle as needed
    }

    async fn handle_initialize(&self, _params: Option<Value>) -> Result<Value> {
        Ok(json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": self.server_info.name,
                "version": self.server_info.version
            }
        }))
    }

    async fn handle_shutdown(&self, _params: Option<Value>) -> Result<Value> {
        Ok(json!({}))
    }

    async fn handle_tools_list(&self, _params: Option<Value>) -> Result<Value> {
        let tools = self.tools.read().await;
        let tools_list: Vec<ToolInfo> = tools.values().map(|handler| handler.tool_info()).collect();
        Ok(json!({
            "tools": tools_list
        }))
    }

    async fn handle_tools_call(
        &self,
        params: Option<Value>,
        request_id: JsonRpcId,
        notification_sender: Arc<dyn crate::notification::NotificationSender>,
    ) -> Result<Value> {
        let params =
            params.ok_or_else(|| Error::InvalidParams("Missing parameters".to_string()))?;

        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::InvalidParams("Missing tool name".to_string()))?;

        let tool_params = params.get("arguments");

        // Extract progress token from metadata if present
        let progress_token = params
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tools = self.tools.read().await;
        if let Some(handler) = tools.get(tool_name) {
            // Create tool context with notification sender
            let context = crate::notification::ToolContext::new(
                notification_sender,
                progress_token,
                Some(request_id),
            );

            match handler.handle(tool_params.cloned(), context).await {
                Ok(result) => {
                    // Wrap the result in MCP tool response format
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
                        }],
                        "isError": false
                    }))
                }
                Err(e) => {
                    // Return error in MCP format
                    Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Error: {}", e)
                        }],
                        "isError": true
                    }))
                }
            }
        } else {
            Err(Error::MethodNotFound(format!(
                "Tool not found: {}",
                tool_name
            )))
        }
    }

    async fn handle_custom(
        &self,
        method: &str,
        params: Option<Value>,
        request_id: JsonRpcId,
        notification_sender: Arc<dyn crate::notification::NotificationSender>,
    ) -> Result<Value> {
        // First check if it's a tool method (e.g., "tool_name" format)
        let tools = self.tools.read().await;
        if let Some(handler) = tools.get(method) {
            // Extract progress token from params if present
            let progress_token = params
                .as_ref()
                .and_then(|p| p.get("_meta"))
                .and_then(|meta| meta.get("progressToken"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let context = crate::notification::ToolContext::new(
                notification_sender,
                progress_token,
                Some(request_id),
            );

            return handler.handle(params, context).await;
        }

        // Then check custom handlers
        let handlers = self.custom_handlers.read().await;
        if let Some(handler) = handlers.get(method) {
            handler.handle(params).await
        } else {
            Err(Error::MethodNotFound(format!(
                "Method not found: {}",
                method
            )))
        }
    }

    fn error_to_json_rpc(&self, error: Error) -> JsonRpcError {
        match error {
            Error::MethodNotFound(msg) => JsonRpcError {
                code: error_codes::METHOD_NOT_FOUND,
                message: msg,
                data: None,
            },
            Error::InvalidParams(msg) => JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: msg,
                data: None,
            },
            Error::Internal(msg) => JsonRpcError {
                code: error_codes::INTERNAL_ERROR,
                message: msg,
                data: None,
            },
            _ => JsonRpcError {
                code: error_codes::SERVER_ERROR,
                message: error.to_string(),
                data: None,
            },
        }
    }
}

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

fn invalid_request_response(id: JsonRpcId) -> JsonRpcMessage {
    JsonRpcMessage::Response(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        result: None,
        error: Some(JsonRpcError {
            code: error_codes::INVALID_REQUEST,
            message: "Invalid Request".to_string(),
            data: None,
        }),
        id,
    })
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
                if batch.is_empty() {
                    return (
                        Some(invalid_request_response(JsonRpcId::Null)),
                        queued_sender.take_all().await,
                    );
                }
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
                if batch.is_empty() {
                    return Some(invalid_request_response(JsonRpcId::Null));
                }
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
        if req.jsonrpc != "2.0" {
            return match invalid_request_response(req.id) {
                JsonRpcMessage::Response(response) => response,
                _ => unreachable!(),
            };
        }

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
            // MCP spec: unknown tool is an Invalid params error, not Method not found
            Err(Error::InvalidParams(format!("Unknown tool: {}", tool_name)))
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

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedTool;

    #[async_trait]
    impl ToolHandler for FixedTool {
        fn tool_info(&self) -> ToolInfo {
            ToolInfo {
                name: "fixed".to_string(),
                description: "Returns a fixed value".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "ignored": { "type": "string" }
                    }
                }),
            }
        }

        async fn handle(
            &self,
            _params: Option<Value>,
            _context: crate::notification::ToolContext,
        ) -> Result<Value> {
            Ok(json!({ "answer": 42 }))
        }
    }

    fn request(method: &str, params: Option<Value>, id: JsonRpcId) -> JsonRpcMessage {
        JsonRpcMessage::Request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id,
        })
    }

    fn notification(method: &str, params: Option<Value>) -> JsonRpcMessage {
        JsonRpcMessage::Notification(JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        })
    }

    fn response(message: Option<JsonRpcMessage>) -> JsonRpcResponse {
        match message {
            Some(JsonRpcMessage::Response(response)) => response,
            other => panic!("expected response, got {other:?}"),
        }
    }

    fn batch(message: Option<JsonRpcMessage>) -> Vec<JsonRpcMessage> {
        match message {
            Some(JsonRpcMessage::Batch(batch)) => batch,
            other => panic!("expected batch response, got {other:?}"),
        }
    }

    fn result(response: &JsonRpcResponse) -> &Value {
        assert!(response.error.is_none());
        match response.result.as_ref() {
            Some(result) => result,
            None => panic!("expected successful result, got {response:?}"),
        }
    }

    fn error(response: &JsonRpcResponse) -> &JsonRpcError {
        assert!(response.result.is_none());
        match response.error.as_ref() {
            Some(error) => error,
            None => panic!("expected error, got {response:?}"),
        }
    }

    fn assert_invalid_request(message: Option<JsonRpcMessage>) {
        match message {
            Some(JsonRpcMessage::Response(response)) => {
                assert_eq!(response.id, JsonRpcId::Null);
                assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
            }
            other => panic!("expected invalid-request response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_batch_returns_invalid_request() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());

        let (queued, _) = protocol
            .handle_message(JsonRpcMessage::Batch(Vec::new()))
            .await;
        let realtime = protocol
            .handle_message_realtime(JsonRpcMessage::Batch(Vec::new()))
            .await;

        assert_invalid_request(queued);
        assert_invalid_request(realtime);
    }

    #[tokio::test]
    async fn non_2_0_request_returns_invalid_request() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        let request = JsonRpcMessage::Request(JsonRpcRequest {
            jsonrpc: "1.0".to_string(),
            method: "initialize".to_string(),
            params: None,
            id: JsonRpcId::Number(7),
        });

        let response = protocol.handle_message_realtime(request).await;

        match response {
            Some(JsonRpcMessage::Response(response)) => {
                assert_eq!(response.id, JsonRpcId::Number(7));
                assert_eq!(response.error.unwrap().code, error_codes::INVALID_REQUEST);
            }
            other => panic!("expected invalid-request response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn initialize_returns_server_metadata_and_capabilities() {
        let protocol = Protocol::new("test-server".to_string(), "1.2.3".to_string());
        let response = response(
            protocol
                .handle_message_realtime(request(
                    "initialize",
                    Some(json!({
                        "protocolVersion": "2025-03-26",
                        "capabilities": {},
                        "clientInfo": { "name": "test-client", "version": "1" }
                    })),
                    JsonRpcId::String("init-1".to_string()),
                ))
                .await,
        );

        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, JsonRpcId::String("init-1".to_string()));
        assert_eq!(result(&response)["protocolVersion"], "2025-03-26");
        assert_eq!(result(&response)["serverInfo"]["name"], "test-server");
        assert_eq!(result(&response)["serverInfo"]["version"], "1.2.3");
        assert!(result(&response)["capabilities"]["tools"].is_object());
    }

    #[tokio::test]
    async fn tools_list_returns_registered_tool_metadata() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        protocol.register_tool(Arc::new(FixedTool)).await;

        let response = response(
            protocol
                .handle_message_realtime(request("tools/list", None, JsonRpcId::Number(1)))
                .await,
        );
        let tools = match result(&response)["tools"].as_array() {
            Some(tools) => tools,
            None => panic!("expected tools array"),
        };

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "fixed");
        assert_eq!(tools[0]["description"], "Returns a fixed value");
        assert_eq!(
            tools[0]["inputSchema"],
            json!({
                "type": "object",
                "properties": {
                    "ignored": { "type": "string" }
                }
            })
        );
    }

    #[tokio::test]
    async fn tools_call_returns_tool_result_and_echoes_id() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        protocol.register_tool(Arc::new(FixedTool)).await;

        let response = response(
            protocol
                .handle_message_realtime(request(
                    "tools/call",
                    Some(json!({ "name": "fixed", "arguments": { "ignored": "x" } })),
                    JsonRpcId::String("call-1".to_string()),
                ))
                .await,
        );

        assert_eq!(response.id, JsonRpcId::String("call-1".to_string()));
        assert_eq!(result(&response)["isError"], false);
        assert_eq!(result(&response)["content"][0]["type"], "text");
        assert_eq!(
            result(&response)["content"][0]["text"],
            serde_json::to_string_pretty(&json!({ "answer": 42 })).unwrap()
        );
    }

    #[tokio::test]
    async fn tools_call_missing_params_or_name_returns_invalid_params() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        let cases = [
            (None, JsonRpcId::Number(10)),
            (Some(json!({})), JsonRpcId::Number(11)),
        ];

        for (params, id) in cases {
            let response = response(
                protocol
                    .handle_message_realtime(request("tools/call", params, id.clone()))
                    .await,
            );

            assert_eq!(response.id, id);
            assert_eq!(error(&response).code, error_codes::INVALID_PARAMS);
        }
    }

    #[tokio::test]
    async fn tools_call_unknown_tool_returns_invalid_params() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());

        let response = response(
            protocol
                .handle_message_realtime(request(
                    "tools/call",
                    Some(json!({ "name": "no-such-tool", "arguments": {} })),
                    JsonRpcId::Number(12),
                ))
                .await,
        );

        // MCP spec: unknown tool is an Invalid params error (-32602), not
        // Method not found (https://modelcontextprotocol.io/specification/2025-03-26/server/tools)
        assert_eq!(error(&response).code, error_codes::INVALID_PARAMS);
    }

    #[test]
    fn error_variants_map_to_json_rpc_error_codes() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        let serialization_error = match serde_json::from_str::<Value>("{") {
            Err(error) => error,
            Ok(value) => panic!("expected malformed JSON to fail, got {value}"),
        };
        let cases = [
            (
                Error::Transport("transport".to_string()),
                error_codes::SERVER_ERROR,
            ),
            (
                Error::Protocol("protocol".to_string()),
                error_codes::SERVER_ERROR,
            ),
            (
                Error::Serialization(serialization_error),
                error_codes::SERVER_ERROR,
            ),
            (
                Error::Io(std::io::Error::other("io")),
                error_codes::SERVER_ERROR,
            ),
            (
                Error::MethodNotFound("missing".to_string()),
                error_codes::METHOD_NOT_FOUND,
            ),
            (
                Error::InvalidParams("invalid".to_string()),
                error_codes::INVALID_PARAMS,
            ),
            (
                Error::Internal("internal".to_string()),
                error_codes::INTERNAL_ERROR,
            ),
            (
                Error::NotImplemented("todo".to_string()),
                error_codes::SERVER_ERROR,
            ),
        ];

        for (source, expected_code) in cases {
            let mapped = protocol.error_to_json_rpc(source);
            assert_eq!(mapped.code, expected_code);
            assert!(mapped.data.is_none());
        }
    }

    #[tokio::test]
    async fn unknown_custom_method_returns_method_not_found() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        let response = response(
            protocol
                .handle_message_realtime(request("unknown/custom", None, JsonRpcId::Number(20)))
                .await,
        );

        assert_eq!(response.id, JsonRpcId::Number(20));
        assert_eq!(error(&response).code, error_codes::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn nonempty_batch_returns_responses_in_request_order() {
        let protocol = Protocol::new("test-server".to_string(), "1".to_string());
        let responses = batch(
            protocol
                .handle_message_realtime(JsonRpcMessage::Batch(vec![
                    request("initialize", None, JsonRpcId::Number(31)),
                    request("tools/list", None, JsonRpcId::Number(32)),
                ]))
                .await,
        );

        assert_eq!(responses.len(), 2);
        for (message, expected_id) in responses.iter().zip([31, 32]) {
            match message {
                JsonRpcMessage::Response(response) => {
                    assert_eq!(response.id, JsonRpcId::Number(expected_id));
                    assert!(response.error.is_none());
                }
                other => panic!("expected response in batch, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn mixed_batch_omits_notification_response() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());
        let responses = batch(
            protocol
                .handle_message_realtime(JsonRpcMessage::Batch(vec![
                    request("tools/list", None, JsonRpcId::Number(41)),
                    notification("notifications/initialized", None),
                ]))
                .await,
        );

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            JsonRpcMessage::Response(response) => {
                assert_eq!(response.id, JsonRpcId::Number(41));
            }
            other => panic!("expected response in batch, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn standalone_notification_produces_no_response() {
        let protocol = Protocol::new("test".to_string(), "1".to_string());

        let response = protocol
            .handle_message_realtime(notification(
                "notifications/initialized",
                Some(json!({ "ready": true })),
            ))
            .await;

        assert!(response.is_none());
    }
}

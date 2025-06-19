use serde::{Deserialize, Serialize};
use serde_json::Value;

// JSON-RPC 2.0 Base Types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
    Batch(Vec<JsonRpcMessage>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    pub id: JsonRpcId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: JsonRpcId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
    Null,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

// MCP-specific types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// MCP Notification Types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageNotification {
    pub level: MessageLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressNotification {
    #[serde(rename = "progressToken")]
    pub progress_token: String,
    pub progress: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// Request metadata for progress tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMeta {
    #[serde(rename = "progressToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_token: Option<String>,
}

// These types are no longer used as we're returning JSON directly
// keeping them commented for reference
//
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ServerCapabilities {
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub tools: Option<ToolCapabilities>,
// }
//
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ToolCapabilities {
//     pub list: bool,
//     pub call: bool,
// }
//
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct InitializeResult {
//     #[serde(rename = "protocolVersion")]
//     pub protocol_version: String,
//     pub capabilities: ServerCapabilities,
//     #[serde(rename = "serverInfo")]
//     pub server_info: ServerInfo,
// }

// MCP Methods
#[derive(Debug, Clone)]
pub enum McpMethod {
    // Lifecycle
    Initialize,
    Shutdown,
    
    // Tools
    ToolsList,
    ToolsCall,
    
    // Custom methods
    Custom(String),
}

impl std::str::FromStr for McpMethod {
    type Err = std::convert::Infallible;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "initialize" => McpMethod::Initialize,
            "shutdown" => McpMethod::Shutdown,
            "tools/list" => McpMethod::ToolsList,
            "tools/call" => McpMethod::ToolsCall,
            _ => McpMethod::Custom(s.to_string()),
        })
    }
}

// Error codes
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    
    // MCP-specific error codes
    pub const SERVER_ERROR: i32 = -32000;
}
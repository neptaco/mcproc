use crate::error::{Error, Result};
use crate::transport::Transport;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;

/// Streamable HTTP transport for MCP communication
///
/// This transport implements the MCP HTTP with Server-Sent Events specification:
/// - Single endpoint for both request/response and SSE streaming
/// - Supports bidirectional communication over HTTP
///
/// Status: Not yet implemented
pub struct StreamableHttpTransport;

impl StreamableHttpTransport {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Transport for StreamableHttpTransport {
    async fn start(&mut self) -> Result<()> {
        Err(Error::NotImplemented(
            "Streamable HTTP transport is not yet implemented".to_string(),
        ))
    }

    async fn send(&mut self, _message: JsonRpcMessage) -> Result<()> {
        Err(Error::NotImplemented(
            "Streamable HTTP transport is not yet implemented".to_string(),
        ))
    }

    async fn receive(&mut self) -> Result<Option<JsonRpcMessage>> {
        Err(Error::NotImplemented(
            "Streamable HTTP transport is not yet implemented".to_string(),
        ))
    }

    async fn close(&mut self) -> Result<()> {
        Err(Error::NotImplemented(
            "Streamable HTTP transport is not yet implemented".to_string(),
        ))
    }
}

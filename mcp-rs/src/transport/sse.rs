use crate::error::{Error, Result};
use crate::transport::Transport;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;

/// SSE (Server-Sent Events) transport for MCP communication
///
/// This transport implements the MCP SSE specification:
/// - POST endpoint for client-to-server messages
/// - GET endpoint for server-to-client SSE stream
///
/// Status: Not yet implemented
pub struct SseTransport;

impl SseTransport {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn start(&mut self) -> Result<()> {
        Err(Error::NotImplemented(
            "SSE transport is not yet implemented".to_string(),
        ))
    }

    async fn send(&mut self, _message: JsonRpcMessage) -> Result<()> {
        Err(Error::NotImplemented(
            "SSE transport is not yet implemented".to_string(),
        ))
    }

    async fn receive(&mut self) -> Result<Option<JsonRpcMessage>> {
        Err(Error::NotImplemented(
            "SSE transport is not yet implemented".to_string(),
        ))
    }

    async fn close(&mut self) -> Result<()> {
        Err(Error::NotImplemented(
            "SSE transport is not yet implemented".to_string(),
        ))
    }
}

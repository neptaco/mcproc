use crate::error::Result;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;

#[cfg(feature = "stdio")]
pub mod stdio;

#[cfg(feature = "sse")]
pub mod sse;

#[cfg(feature = "streamable-http")]
pub mod streamable_http;

/// Transport trait for MCP communication
#[async_trait]
pub trait Transport: Send + Sync {
    /// Start the transport
    async fn start(&mut self) -> Result<()>;
    
    /// Send a message
    async fn send(&mut self, message: JsonRpcMessage) -> Result<()>;
    
    /// Receive a message
    async fn receive(&mut self) -> Result<Option<JsonRpcMessage>>;
    
    /// Close the transport
    async fn close(&mut self) -> Result<()>;
}
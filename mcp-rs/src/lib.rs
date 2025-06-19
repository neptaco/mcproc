//! Model Context Protocol (MCP) implementation for Rust
//! 
//! This library provides a complete implementation of the Model Context Protocol.
//! 
//! ## Supported Transports
//! - **stdio**: Standard input/output transport (implemented)
//! - **sse**: Server-Sent Events transport (not yet implemented)
//! - **streamable-http**: HTTP with Server-Sent Events (not yet implemented)

pub mod error;
pub mod protocol;
pub mod server;
pub mod transport;
pub mod types;

// Re-export commonly used types
pub use error::{Error, Result};
pub use protocol::{Protocol, ToolHandler};
pub use server::{Server, ServerBuilder};
pub use types::*;

#[cfg(feature = "stdio")]
pub use transport::stdio::StdioTransport;

#[cfg(feature = "sse")]
pub use transport::sse::SseTransport;

#[cfg(feature = "streamable-http")]
pub use transport::streamable_http::StreamableHttpTransport;
//! Model Context Protocol (MCP) implementation for Rust
//! 
//! This library provides a complete implementation of the Model Context Protocol,
//! supporting both stdio and HTTP+SSE transports.

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

#[cfg(feature = "http")]
pub use transport::http::{HttpTransport, HttpTransportConfig};

#[cfg(feature = "http")]
pub use transport::sse::{SseTransport, SseTransportConfig};
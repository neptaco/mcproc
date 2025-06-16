use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Transport error: {0}")]
    Transport(String),
    
    #[error("Protocol error: {0}")]
    Protocol(String),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Method not found: {0}")]
    MethodNotFound(String),
    
    #[error("Invalid params: {0}")]
    InvalidParams(String),
    
    #[error("Internal error: {0}")]
    Internal(String),
    
    #[cfg(feature = "http")]
    #[error("HTTP error: {0}")]
    Http(String),
}

pub type Result<T> = std::result::Result<T, Error>;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum McprocdError {
    #[error("Process not found: {0}")]
    ProcessNotFound(String),
    
    #[error("Process already exists: {0}")]
    ProcessAlreadyExists(String),
    
    #[error("Failed to spawn process: {0}")]
    SpawnError(String),
    
    #[error("Failed to stop process: {0}")]
    StopError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("API error: {0}")]
    ApiError(String),
    
    #[error("Log error: {0}")]
    LogError(String),
}

pub type Result<T> = std::result::Result<T, McprocdError>;
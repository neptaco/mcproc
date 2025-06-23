use serde::{Deserialize, Serialize};
use std::fmt;

/// A unique identifier for a process consisting of project and name
///
/// ProcessKey is used throughout the system to uniquely identify processes
/// within their project scope. It provides consistent formatting and parsing
/// for the "project/name" string representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessKey {
    pub project: String,
    pub name: String,
}

impl ProcessKey {
    pub fn new(project: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            name: name.into(),
        }
    }

    /// Parse from "project/name" format
    pub fn parse(key: &str) -> Option<Self> {
        let parts: Vec<&str> = key.split('/').collect();
        if parts.len() == 2 {
            Some(Self::new(parts[0], parts[1]))
        } else {
            None
        }
    }

    /// Get the full key as "project/name"
    pub fn as_str(&self) -> String {
        format!("{}/{}", self.project, self.name)
    }

    /// Get the log file handle key
    pub fn log_handle_key(&self) -> String {
        self.as_str()
    }

    /// Get the sanitized name for file system (replaces / with _)
    pub fn sanitized_name(&self) -> String {
        self.name.replace('/', "_")
    }
}

impl fmt::Display for ProcessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.project, self.name)
    }
}

impl From<ProcessKey> for String {
    fn from(key: ProcessKey) -> Self {
        key.as_str()
    }
}

impl From<&ProcessKey> for String {
    fn from(key: &ProcessKey) -> Self {
        key.as_str()
    }
}

use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use crate::daemon::stream::{SharedStreamEventHub, StreamEvent};
use proto::LogEntry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::debug;

/// Centralized log management for all processes
///
/// LogHub manages log file handles for all processes, ensuring efficient
/// file I/O by keeping files open and providing thread-safe access.
/// Logs are organized by project directories for better organization.
pub struct LogHub {
    pub config: Arc<Config>,
    file_handles: RwLock<std::collections::HashMap<String, tokio::fs::File>>,
    event_hub: Option<SharedStreamEventHub>,
}

impl LogHub {

    pub fn with_event_hub(config: Arc<Config>, event_hub: SharedStreamEventHub) -> Self {
        Self {
            config,
            file_handles: RwLock::new(std::collections::HashMap::new()),
            event_hub: Some(event_hub),
        }
    }

    /// Close log file for a specific process
    pub async fn close_log_for_key(&self, key: &ProcessKey) -> Result<(), std::io::Error> {
        let mut handles = self.file_handles.write().await;
        let handle_key = key.log_handle_key();

        if let Some(mut file) = handles.remove(&handle_key) {
            // Flush any remaining data
            file.flush().await?;
            // File is automatically closed when dropped
        }

        Ok(())
    }

    pub async fn append_log_for_key(
        &self,
        key: &ProcessKey,
        content: &[u8],
        is_stderr: bool,
    ) -> Result<(), std::io::Error> {
        let content_str = String::from_utf8_lossy(content);
        let level = if is_stderr { "ERROR" } else { "INFO" };

        // Get or create file handle for this process
        let mut handles = self.file_handles.write().await;

        let log_file_path = self.get_log_file_path_for_key(key);
        let handle_key = key.log_handle_key();

        if !handles.contains_key(&handle_key) {
            // Ensure directory exists
            if let Some(parent) = log_file_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path)
                .await
            {
                Ok(file) => {
                    handles.insert(handle_key.clone(), file);
                }
                Err(e) => {
                    eprintln!(
                        "Failed to open log file at {:?} for {}: {}",
                        log_file_path, handle_key, e
                    );
                    return Err(e);
                }
            }
        }

        // Format log line
        let now = chrono::Utc::now();
        let timestamp_str = now.format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("{} [{}] {}\n", timestamp_str, level, content_str.trim());

        // Write to file
        if let Some(file) = handles.get_mut(&handle_key) {
            file.write_all(log_line.as_bytes()).await.map_err(|e| {
                eprintln!("Failed to write log for {}: {}", handle_key, e);
                e
            })?;
            // Ensure data is flushed
            file.flush().await?;
        }

        // Publish log event if event hub is available
        if let Some(ref event_hub) = self.event_hub {
            let content_owned = content_str.into_owned();
            
            let log_entry = LogEntry {
                line_number: 0, // Line numbers are tracked per reader, not here
                timestamp: Some(prost_types::Timestamp {
                    seconds: now.timestamp(),
                    nanos: now.timestamp_subsec_nanos() as i32,
                }),
                content: content_owned.clone(),
                level: if is_stderr { 2 } else { 1 }, // ERROR = 2, INFO = 1
                process_name: None, // Will be set by subscriber if needed
            };

            debug!(
                "Publishing log event for {}/{}: {}",
                key.project, key.name, content_owned.trim()
            );

            event_hub.publish(StreamEvent::Log {
                process_name: key.name.clone(),
                project: key.project.clone(),
                entry: log_entry,
            });
        } else {
            debug!("No event hub available for publishing log event");
        }

        Ok(())
    }

    pub fn get_log_file_path_for_key(&self, key: &ProcessKey) -> PathBuf {
        let project_dir = self.config.paths.log_dir.join(&key.project);
        project_dir.join(format!("{}.log", key.sanitized_name()))
    }
}

use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

/// Centralized log management for all processes
///
/// LogHub manages log file handles for all processes, ensuring efficient
/// file I/O by keeping files open and providing thread-safe access.
/// Logs are organized by project directories for better organization.
pub struct LogHub {
    pub config: Arc<Config>,
    file_handles: RwLock<std::collections::HashMap<String, tokio::fs::File>>,
}

impl LogHub {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            file_handles: RwLock::new(std::collections::HashMap::new()),
        }
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
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("{} [{}] {}\n", timestamp, level, content_str.trim());

        // Write to file
        if let Some(file) = handles.get_mut(&handle_key) {
            file.write_all(log_line.as_bytes()).await.map_err(|e| {
                eprintln!("Failed to write log for {}: {}", handle_key, e);
                e
            })?;
            // Ensure data is flushed
            file.flush().await?;
        }
        Ok(())
    }

    pub fn get_log_file_path_for_key(&self, key: &ProcessKey) -> PathBuf {
        let project_dir = self.config.paths.log_dir.join(&key.project);
        project_dir.join(format!("{}.log", key.sanitized_name()))
    }

    pub async fn close_log_for_key(&self, key: &ProcessKey) {
        let mut handles = self.file_handles.write().await;
        handles.remove(&key.log_handle_key());
    }
}

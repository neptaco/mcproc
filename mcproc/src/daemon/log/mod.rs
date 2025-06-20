use crate::daemon::config::Config;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

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

    pub async fn append_log(
        &self,
        process_name: &str,
        content: &[u8],
        is_stderr: bool,
    ) -> Result<(), std::io::Error> {
        let content_str = String::from_utf8_lossy(content);
        let level = if is_stderr { "ERROR" } else { "INFO" };

        // Get or create file handle for this process
        let mut handles = self.file_handles.write().await;

        let log_file_path = self.get_log_file_path(process_name);

        if !handles.contains_key(process_name) {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file_path)
                .await
            {
                Ok(file) => {
                    handles.insert(process_name.to_string(), file);
                }
                Err(e) => {
                    eprintln!("Failed to open log file for {}: {}", process_name, e);
                    return Err(e);
                }
            }
        }

        // Format log line
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("{} [{}] {}\n", timestamp, level, content_str.trim());

        // Write to file
        if let Some(file) = handles.get_mut(process_name) {
            file.write_all(log_line.as_bytes()).await.map_err(|e| {
                eprintln!("Failed to write log for {}: {}", process_name, e);
                e
            })?;
            // Ensure data is flushed
            file.flush().await?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_log_file(&self, process_name: &str) -> Option<PathBuf> {
        let log_file = self.get_log_file_path(process_name);

        if log_file.exists() {
            Some(log_file)
        } else {
            None
        }
    }

    fn get_log_file_path(&self, process_name: &str) -> PathBuf {
        // Replace "/" with "_" to create valid filesystem paths
        let sanitized_name = process_name.replace("/", "_");
        self.config.log.dir.join(format!("{}.log", sanitized_name))
    }

    pub async fn close_log(&self, process_name: &str) {
        let mut handles = self.file_handles.write().await;
        handles.remove(process_name);
    }
}

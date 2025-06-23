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

    pub async fn append_log(
        &self,
        process_name: &str,
        content: &[u8],
        is_stderr: bool,
    ) -> Result<(), std::io::Error> {
        // Try to parse as ProcessKey format
        if let Some(key) = ProcessKey::parse(process_name) {
            self.append_log_for_key(&key, content, is_stderr).await
        } else {
            self.append_log_with_project(process_name, None, content, is_stderr)
                .await
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

    pub async fn append_log_with_project(
        &self,
        process_name: &str,
        project: Option<&str>,
        content: &[u8],
        is_stderr: bool,
    ) -> Result<(), std::io::Error> {
        let content_str = String::from_utf8_lossy(content);
        let level = if is_stderr { "ERROR" } else { "INFO" };

        // Get or create file handle for this process
        let mut handles = self.file_handles.write().await;

        let log_file_path = self.get_log_file_path_with_project(process_name, project);
        let handle_key = if let Some(proj) = project {
            format!("{}/{}", proj, process_name)
        } else {
            format!("default/{}", process_name)
        };

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

    #[allow(dead_code)]
    pub async fn get_log_file(&self, process_name: &str) -> Option<PathBuf> {
        let log_file = self.get_log_file_path(process_name);

        if log_file.exists() {
            Some(log_file)
        } else {
            None
        }
    }

    pub fn get_log_file_path_for_key(&self, key: &ProcessKey) -> PathBuf {
        let project_dir = self.config.paths.log_dir.join(&key.project);
        project_dir.join(format!("{}.log", key.sanitized_name()))
    }

    fn get_log_file_path(&self, process_name: &str) -> PathBuf {
        self.get_log_file_path_with_project(process_name, None)
    }

    pub fn get_log_file_path_with_project(
        &self,
        process_name: &str,
        project: Option<&str>,
    ) -> PathBuf {
        // Replace "/" with "_" to create valid filesystem paths
        let sanitized_name = process_name.replace("/", "_");

        if let Some(proj) = project {
            // Create project-specific log directory
            let project_dir = self.config.paths.log_dir.join(proj);
            project_dir.join(format!("{}.log", sanitized_name))
        } else {
            // Default project directory
            let default_dir = self.config.paths.log_dir.join("default");
            default_dir.join(format!("{}.log", sanitized_name))
        }
    }

    pub async fn close_log(&self, process_name: &str) {
        // Try to parse as ProcessKey format
        if let Some(key) = ProcessKey::parse(process_name) {
            self.close_log_for_key(&key).await;
        } else {
            self.close_log_with_project(process_name, None).await;
        }
    }

    pub async fn close_log_for_key(&self, key: &ProcessKey) {
        let mut handles = self.file_handles.write().await;
        handles.remove(&key.log_handle_key());
    }

    pub async fn close_log_with_project(&self, process_name: &str, project: Option<&str>) {
        let mut handles = self.file_handles.write().await;
        let handle_key = if let Some(proj) = project {
            format!("{}/{}", proj, process_name)
        } else {
            format!("default/{}", process_name)
        };
        handles.remove(&handle_key);
    }
}

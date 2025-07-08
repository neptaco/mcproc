use crate::common::process_key::ProcessKey;
use bytes::Bytes;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info};

const WRITE_BATCH_SIZE: usize = 100; // Write after 100 log entries
const WRITE_BATCH_TIMEOUT_MS: u64 = 500; // Write after 500ms
const CHANNEL_BUFFER_SIZE: usize = 10000; // Buffer up to 10k entries

/// A log entry to be written to file
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub content: Bytes,
    pub is_stderr: bool,
}

/// Batch writer for efficient file logging
pub struct BatchLogWriter {
    process_key: ProcessKey,
    tx: mpsc::Sender<LogEntry>,
    _handle: tokio::task::JoinHandle<()>,
}

impl BatchLogWriter {
    pub async fn new(
        process_key: ProcessKey,
        log_file_path: PathBuf,
    ) -> Result<Self, std::io::Error> {
        let (tx, rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);

        // Ensure directory exists
        if let Some(parent) = log_file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let key_clone = process_key.clone();
        let path_clone = log_file_path.clone();

        // Spawn background writer task
        let handle = tokio::spawn(async move {
            if let Err(e) = Self::writer_task(key_clone, path_clone, rx).await {
                error!("Batch writer task failed: {}", e);
            }
        });

        Ok(Self {
            process_key,
            tx,
            _handle: handle,
        })
    }

    /// Write a log entry (non-blocking)
    pub async fn write(&self, entry: LogEntry) -> Result<(), std::io::Error> {
        self.tx
            .send(entry)
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Log writer closed"))
    }

    /// Background task that batches and writes logs
    async fn writer_task(
        process_key: ProcessKey,
        log_file_path: PathBuf,
        mut rx: mpsc::Receiver<LogEntry>,
    ) -> Result<(), std::io::Error> {
        // Open file
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_file_path)
            .await?;

        info!(
            "Started batch log writer for {}/{}",
            process_key.project, process_key.name
        );

        let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);
        let mut timer = interval(Duration::from_millis(WRITE_BATCH_TIMEOUT_MS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                Some(entry) = rx.recv() => {
                    batch.push(entry);

                    // Write if batch is full
                    if batch.len() >= WRITE_BATCH_SIZE {
                        Self::flush_batch(&mut file, &mut batch).await?;
                    }
                }
                _ = timer.tick() => {
                    // Write on timeout if we have data
                    if !batch.is_empty() {
                        Self::flush_batch(&mut file, &mut batch).await?;
                    }
                }
                else => {
                    // Channel closed, write remaining batch
                    if !batch.is_empty() {
                        Self::flush_batch(&mut file, &mut batch).await?;
                    }
                    break;
                }
            }
        }

        // Final flush
        file.flush().await?;
        info!(
            "Stopped batch log writer for {}/{}",
            process_key.project, process_key.name
        );

        Ok(())
    }

    /// Flush a batch of log entries to file
    async fn flush_batch(file: &mut File, batch: &mut Vec<LogEntry>) -> Result<(), std::io::Error> {
        if batch.is_empty() {
            return Ok(());
        }

        debug!("Flushing batch of {} log entries", batch.len());

        // Build combined buffer
        let mut buffer = Vec::with_capacity(batch.len() * 200); // Estimate ~200 bytes per line

        for entry in batch.drain(..) {
            let level = if entry.is_stderr { "ERROR" } else { "INFO" };
            let timestamp_str = entry.timestamp.format("%Y-%m-%d %H:%M:%S%.3f");

            // Format: TIMESTAMP [LEVEL] CONTENT
            buffer.extend_from_slice(timestamp_str.to_string().as_bytes());
            buffer.extend_from_slice(b" [");
            buffer.extend_from_slice(level.as_bytes());
            buffer.extend_from_slice(b"] ");
            buffer.extend_from_slice(&entry.content);

            // Add newline if not present
            if !entry.content.ends_with(b"\n") {
                buffer.push(b'\n');
            }
        }

        // Single write operation
        file.write_all(&buffer).await?;

        Ok(())
    }
}

impl Drop for BatchLogWriter {
    fn drop(&mut self) {
        debug!(
            "Dropping BatchLogWriter for {}/{}",
            self.process_key.project, self.process_key.name
        );
    }
}

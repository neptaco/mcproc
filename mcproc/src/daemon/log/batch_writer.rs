use crate::common::process_key::ProcessKey;
use crate::common::timestamp::format_datetime_utc_with_tz;
use bytes::Bytes;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
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
            .read(true)
            .append(true)
            .open(&log_file_path)
            .await?;

        let file_len = file.metadata().await?.len();
        if file_len > 0 {
            file.seek(std::io::SeekFrom::End(-1)).await?;
            let mut last_byte = [0];
            file.read_exact(&mut last_byte).await?;
            if last_byte[0] != b'\n' {
                file.write_all(b"\n").await?;
            }
        }

        info!(
            "Started batch log writer for {}/{}",
            process_key.project, process_key.name
        );

        let mut batch = Vec::with_capacity(WRITE_BATCH_SIZE);
        let mut timer = interval(Duration::from_millis(WRITE_BATCH_TIMEOUT_MS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                entry = rx.recv() => {
                    match entry {
                        Some(entry) => {
                            batch.push(entry);

                            // Write if batch is full
                            if batch.len() >= WRITE_BATCH_SIZE {
                                Self::flush_batch(&mut file, &mut batch).await?;
                            }
                        }
                        None => {
                            if !batch.is_empty() {
                                Self::flush_batch(&mut file, &mut batch).await?;
                            }
                            break;
                        }
                    }
                }
                _ = timer.tick() => {
                    // Write on timeout if we have data
                    if !batch.is_empty() {
                        Self::flush_batch(&mut file, &mut batch).await?;
                    }
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
            let timestamp_str = format_datetime_utc_with_tz(entry.timestamp);

            // Format: TIMESTAMP [LEVEL] CONTENT
            buffer.extend_from_slice(timestamp_str.as_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn temp_log_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mcproc-{label}-{}.log", Uuid::new_v4()))
    }

    fn entry(content: &'static [u8]) -> LogEntry {
        LogEntry {
            timestamp: chrono::Utc::now(),
            content: Bytes::from_static(content),
            is_stderr: false,
        }
    }

    async fn wait_for_contents(path: &PathBuf, needles: &[&str]) -> String {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let contents = tokio::fs::read_to_string(path).await.unwrap_or_default();
                if needles.iter().all(|needle| contents.contains(needle)) {
                    return contents;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("log contents were not flushed before timeout")
    }

    #[tokio::test]
    async fn appends_to_existing_log_file() {
        let path = temp_log_path("append");
        tokio::fs::write(&path, "existing\n").await.unwrap();
        let writer = BatchLogWriter::new(ProcessKey::new("p", "n"), path.clone())
            .await
            .unwrap();
        writer.write(entry(b"new-line")).await.unwrap();
        drop(writer);

        let contents = wait_for_contents(&path, &["existing", "new-line"]).await;
        assert!(contents.starts_with("existing\n"), "contents: {contents:?}");
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn appending_to_partial_line_starts_a_new_line() {
        let path = temp_log_path("append-partial");
        tokio::fs::write(&path, "partial").await.unwrap();
        let writer = BatchLogWriter::new(ProcessKey::new("p", "n"), path.clone())
            .await
            .unwrap();
        writer.write(entry(b"new-line")).await.unwrap();
        drop(writer);

        let contents = wait_for_contents(&path, &["partial", "new-line"]).await;
        assert!(contents.starts_with("partial\n"), "contents: {contents:?}");
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn two_writers_append_without_overwriting_each_other() {
        let path = temp_log_path("two-writers");
        let first = BatchLogWriter::new(ProcessKey::new("p", "n"), path.clone())
            .await
            .unwrap();
        first.write(entry(b"from-first")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let second = BatchLogWriter::new(ProcessKey::new("p", "n"), path.clone())
            .await
            .unwrap();
        second.write(entry(b"from-second")).await.unwrap();
        drop(first);
        drop(second);

        wait_for_contents(&path, &["from-first", "from-second"]).await;
        tokio::fs::remove_file(path).await.unwrap();
    }

    #[tokio::test]
    async fn writer_task_exits_when_channel_closes() {
        let path = temp_log_path("channel-close");
        let (tx, rx) = mpsc::channel(1);
        drop(tx);

        tokio::time::timeout(
            Duration::from_secs(2),
            BatchLogWriter::writer_task(ProcessKey::new("p", "n"), path.clone(), rx),
        )
        .await
        .expect("writer task did not exit after channel close")
        .unwrap();
        tokio::fs::remove_file(path).await.unwrap();
    }
}

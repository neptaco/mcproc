use crate::common::process_key::ProcessKey;
use crate::daemon::log::LogHub;
use crate::daemon::process::proxy::{LogChunk, ProxyInfo};
use bytes::{Bytes, BytesMut};
use chrono::Utc;
use regex::Regex;
use ringbuf::traits::RingBuffer;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info};

const CHUNK_SIZE: usize = 8192; // 8KB chunks
const BATCH_SIZE: usize = 16; // Process 16 chunks at a time
const BATCH_TIMEOUT_MS: u64 = 100; // Process after 100ms regardless of batch size

pub struct HyperLogConfig {
    pub stream_name: &'static str,
    pub process_key: ProcessKey,
    pub proxy: Arc<ProxyInfo>,
    pub log_hub: Arc<LogHub>,
    pub log_pattern: Option<Arc<Regex>>,
    pub log_ready_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
    pub pattern_matched: Arc<Mutex<bool>>,
    pub matched_line: Arc<Mutex<Option<String>>>,
    pub timeout_occurred: Arc<Mutex<bool>>,
    pub wait_timeout: Option<u32>,
    pub default_wait_timeout_secs: u32,
    pub is_stderr: bool,
}

pub struct HyperLogStreamer {
    config: HyperLogConfig,
    chunk_tx: mpsc::UnboundedSender<Bytes>,
    chunk_rx: mpsc::UnboundedReceiver<Bytes>,
}

impl HyperLogStreamer {
    pub fn new(config: HyperLogConfig) -> Self {
        let (chunk_tx, chunk_rx) = mpsc::unbounded_channel();
        Self {
            config,
            chunk_tx,
            chunk_rx,
        }
    }

    /// Spawn the high-performance log reader
    pub async fn spawn<R: AsyncRead + Unpin + Send + 'static>(
        self,
        stream: R,
    ) -> tokio::task::JoinHandle<()> {
        let chunk_tx = self.chunk_tx.clone();
        let process_key = self.config.process_key.clone();
        let stream_name = self.config.stream_name;

        // Spawn chunk reader
        let reader_handle = tokio::spawn(async move {
            let mut stream = stream;
            let mut buffer = BytesMut::with_capacity(CHUNK_SIZE);
            buffer.resize(CHUNK_SIZE, 0);

            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => {
                        debug!("Stream ended for {} ({})", process_key, stream_name);
                        break;
                    }
                    Ok(n) => {
                        // Send chunk without copying
                        let chunk = buffer.split_to(n).freeze();
                        if chunk_tx.send(chunk).is_err() {
                            debug!("Channel closed, stopping reader");
                            break;
                        }
                        // Resize buffer for next read
                        buffer.resize(CHUNK_SIZE, 0);
                    }
                    Err(e) => {
                        error!("Error reading stream: {}", e);
                        break;
                    }
                }
            }
        });

        // Spawn batch processor
        let config = self.config;
        let mut chunk_rx = self.chunk_rx;
        let has_pattern = config.log_pattern.is_some();

        let processor_handle = tokio::spawn(async move {
            let mut batch = Vec::with_capacity(BATCH_SIZE);
            let mut line_buffer = Vec::new();
            let mut last_flush = tokio::time::Instant::now();

            // Set up pattern matching timeout if needed
            let timeout_duration = if has_pattern {
                tokio::time::Duration::from_secs(
                    config
                        .wait_timeout
                        .unwrap_or(config.default_wait_timeout_secs) as u64,
                )
            } else {
                tokio::time::Duration::from_secs(86400) // 24 hours timeout
            };
            let timeout_instant = tokio::time::Instant::now() + timeout_duration;

            loop {
                // Check for pattern matching timeout
                let pattern_timeout = tokio::time::sleep_until(timeout_instant);
                tokio::pin!(pattern_timeout);

                // Only process batch timeout if we have data
                if !batch.is_empty() {
                    let batch_timeout = tokio::time::sleep_until(
                        last_flush + tokio::time::Duration::from_millis(BATCH_TIMEOUT_MS),
                    );
                    tokio::pin!(batch_timeout);
                    
                    tokio::select! {
                    // Pattern matching timeout
                    _ = &mut pattern_timeout, if has_pattern && !config.pattern_matched.lock().map(|g| *g).unwrap_or(false) && !config.timeout_occurred.lock().map(|g| *g).unwrap_or(false) => {
                        info!("Log pattern matching timeout reached for {} ({})",
                              config.process_key, config.stream_name);

                        // Mark timeout occurred
                        if let Ok(mut timeout_flag) = config.timeout_occurred.lock() {
                            *timeout_flag = true;
                        }

                        // Close the log_ready channel (only if not already closed)
                        if let Some(ref tx) = config.log_ready_tx {
                            if let Ok(mut tx_guard) = tx.lock() {
                                if tx_guard.take().is_some() {
                                    debug!("Timeout notification sent (channel closed)");
                                } else {
                                    debug!("Timeout notification already sent or channel already closed");
                                }
                            }
                        }

                        // Continue processing to capture remaining logs
                    }
                    chunk = chunk_rx.recv() => {
                        match chunk {
                            Some(chunk) => {
                                batch.push(chunk);

                                // Process batch if full
                                if batch.len() >= BATCH_SIZE {
                                    Self::process_batch(
                                        &config,
                                        &mut batch,
                                        &mut line_buffer,
                                    );
                                    last_flush = tokio::time::Instant::now();
                                }
                            }
                            None => {
                                // Channel closed, process remaining batch
                                if !batch.is_empty() {
                                    Self::process_batch(&config, &mut batch, &mut line_buffer);
                                }
                                break;
                            }
                        }
                    }
                    _ = &mut batch_timeout => {
                        // Timeout reached, process current batch
                        Self::process_batch(&config, &mut batch, &mut line_buffer);
                        last_flush = tokio::time::Instant::now();
                    }
                }
                } else {
                    // No batch to process, just wait for chunks
                    tokio::select! {
                        // Pattern matching timeout
                        _ = &mut pattern_timeout, if has_pattern && !config.pattern_matched.lock().map(|g| *g).unwrap_or(false) && !config.timeout_occurred.lock().map(|g| *g).unwrap_or(false) => {
                            info!("Log pattern matching timeout reached for {} ({})",
                                  config.process_key, config.stream_name);

                            // Mark timeout occurred
                            if let Ok(mut timeout_flag) = config.timeout_occurred.lock() {
                                *timeout_flag = true;
                            }

                            // Close the log_ready channel (only if not already closed)
                            if let Some(ref tx) = config.log_ready_tx {
                                if let Ok(mut tx_guard) = tx.lock() {
                                    if tx_guard.take().is_some() {
                                        debug!("Timeout notification sent (channel closed)");
                                    } else {
                                        debug!("Timeout notification already sent or channel already closed");
                                    }
                                }
                            }

                            // Continue processing to capture remaining logs
                        }
                        chunk = chunk_rx.recv() => {
                            match chunk {
                                Some(chunk) => {
                                    batch.push(chunk);

                                    // Process batch if full
                                    if batch.len() >= BATCH_SIZE {
                                        Self::process_batch(
                                            &config,
                                            &mut batch,
                                            &mut line_buffer,
                                        );
                                        last_flush = tokio::time::Instant::now();
                                    }
                                }
                                None => {
                                    // Channel closed, process remaining batch
                                    if !batch.is_empty() {
                                        Self::process_batch(&config, &mut batch, &mut line_buffer);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Close ready channel if needed
            if let Some(ref tx) = config.log_ready_tx {
                if let Ok(mut tx_guard) = tx.lock() {
                    tx_guard.take();
                }
            }
        });

        // Return a handle that waits for both tasks
        tokio::spawn(async move {
            let _ = tokio::join!(reader_handle, processor_handle);
        })
    }

    /// Process a batch of chunks
    fn process_batch(config: &HyperLogConfig, batch: &mut Vec<Bytes>, line_buffer: &mut Vec<u8>) {
        // Metrics
        let total_bytes: usize = batch.iter().map(|b| b.len()).sum();
        debug!(
            "Processing batch: {} chunks, {} bytes",
            batch.len(),
            total_bytes
        );

        // Process all chunks in the batch
        for chunk in batch.drain(..) {
            // Append chunk to line buffer first
            line_buffer.extend_from_slice(&chunk);
            
            // Process complete lines from the buffer
            while let Some(newline_pos) = line_buffer.iter().position(|&b| b == b'\n') {
                // Extract the complete line (including newline)
                let line_with_newline: Vec<u8> = line_buffer.drain(..=newline_pos).collect();
                
                // Write to ring buffer for in-memory storage with timestamp
                let log_chunk = LogChunk {
                    data: line_with_newline.clone(),
                    timestamp: Utc::now(),
                    is_stderr: config.is_stderr,
                };
                if let Ok(mut ring) = config.proxy.ring.lock() {
                    ring.push_overwrite(log_chunk);
                }

                // Write to log file if enabled (use spawn but maintain order via timestamps)
                let log_hub = config.log_hub.clone();
                let process_key = config.process_key.clone();
                let line_clone = line_with_newline.clone();
                let is_stderr = config.is_stderr;
                tokio::spawn(async move {
                    if let Err(e) = log_hub
                        .append_log_for_key(&process_key, &line_clone, is_stderr)
                        .await
                    {
                        error!("Failed to write log to file: {}", e);
                    }
                });

                // Check for pattern match on this line if needed
                if let Some(ref pattern) = config.log_pattern {
                    if let Ok(pattern_matched) = config.pattern_matched.lock() {
                        if !*pattern_matched {
                            // Convert line to string for pattern matching
                            if let Ok(line_text) = std::str::from_utf8(&line_with_newline) {
                                let line_trimmed = line_text.trim_end();
                                debug!(
                                    "Checking pattern '{}' against line: '{}'",
                                    pattern.as_str(), line_trimmed
                                );
                                if pattern.is_match(line_trimmed) {
                                    info!(
                                        "Found pattern match: pattern='{}', line='{}' in {} ({})",
                                        pattern.as_str(), line_trimmed, config.process_key, config.stream_name
                                    );

                                    if let Ok(mut pattern_matched) = config.pattern_matched.lock() {
                                        *pattern_matched = true;
                                    }

                                    // Store matched line
                                    if let Ok(mut matched_line) = config.matched_line.lock() {
                                        *matched_line = Some(line_trimmed.to_string());
                                    }

                                    // Notify ready (only if not already notified)
                                    if let Some(ref tx) = config.log_ready_tx {
                                        if let Ok(mut tx_guard) = tx.lock() {
                                            if let Some(sender) = tx_guard.take() {
                                                debug!("Sending pattern match notification");
                                                let _ = sender.send(());
                                            } else {
                                                debug!("Pattern match notification already sent or channel closed");
                                            }
                                        }
                                    }
                                } else {
                                    debug!(
                                        "Pattern '{}' did not match line: '{}'",
                                        pattern.as_str(), line_trimmed
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Pattern matching is now done per line above - no need for buffer checking

            // Clean up buffer if it gets too large (incomplete line protection)
            if line_buffer.len() > 1024 * 1024 {
                line_buffer.clear();
            }
        }
    }
}

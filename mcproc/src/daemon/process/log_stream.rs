use crate::common::process_key::ProcessKey;
use crate::daemon::log::LogHub;
use crate::daemon::process::proxy::{ProcessStatus, ProxyInfo};
use colored::Colorize;
use regex::Regex;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{debug, error, warn};

// Type alias for complex type
type LogStreamSender = Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>;

pub struct LogStreamConfig {
    pub stream_name: &'static str, // "stdout" or "stderr"
    pub process_key: ProcessKey,
    pub log_hub: Arc<LogHub>,
    pub proxy: Arc<ProxyInfo>,
    pub log_pattern: Option<Arc<Regex>>,
    pub log_ready_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
    pub log_stream_tx: Option<LogStreamSender>,
    pub pattern_matched: Arc<Mutex<bool>>,
    pub timeout_occurred: Arc<Mutex<bool>>,
    pub wait_timeout: Option<u32>,
    pub default_wait_timeout_secs: u32,
    pub log_context: Arc<Mutex<Vec<String>>>, // Collect log context for pattern match
    pub matched_line: Arc<Mutex<Option<String>>>, // The line that matched the pattern
}

impl LogStreamConfig {
    pub async fn spawn_log_reader<R: AsyncRead + Unpin + Send + 'static>(
        self,
        stream: R,
    ) -> tokio::task::JoinHandle<()> {
        let has_pattern = self.log_pattern.is_some();

        tokio::spawn(async move {
            let reader = BufReader::new(stream);
            let mut lines = reader.lines();

            // Buffer to store recent log lines for context
            let mut log_buffer: Vec<String> = Vec::new();
            const LOG_BUFFER_SIZE: usize = 20;

            // Set up timeout future if we're waiting for a pattern
            let timeout_future = if has_pattern {
                let duration = tokio::time::Duration::from_secs(
                    self.wait_timeout.unwrap_or(self.default_wait_timeout_secs) as u64,
                );
                tokio::time::sleep(duration)
            } else {
                tokio::time::sleep(tokio::time::Duration::from_secs(u64::MAX)) // Never timeout
            };
            tokio::pin!(timeout_future);

            // Set up process status check interval
            let mut status_check_interval =
                tokio::time::interval(tokio::time::Duration::from_millis(100));

            loop {
                tokio::select! {
                    // Check if process has exited
                    _ = status_check_interval.tick() => {
                        if !matches!(self.proxy.get_status(), ProcessStatus::Running | ProcessStatus::Starting) {
                            debug!("Process no longer running, will continue reading remaining {} output", self.stream_name);
                            // Close the log_ready channel to notify waiters about process exit
                            // This prevents hanging when wait_timeout is specified
                            if let Some(ref tx) = self.log_ready_tx {
                                if let Ok(mut tx_guard) = tx.lock() {
                                    tx_guard.take(); // Drop the sender to close the channel
                                }
                            }
                            // Don't break immediately - let the stream close naturally
                            // This ensures we capture all output including error messages
                        }
                    }
                    // Check for timeout
                    _ = &mut timeout_future, if has_pattern => {
                        warn!("Log streaming timeout reached for {}", self.stream_name);
                        // Mark timeout occurred
                        if let Ok(mut timeout_flag) = self.timeout_occurred.lock() {
                            *timeout_flag = true;
                        }
                        // Close the log_ready channel to notify waiters about timeout
                        if let Some(ref tx) = self.log_ready_tx {
                            if let Ok(mut tx_guard) = tx.lock() {
                                tx_guard.take(); // Drop the sender to close the channel
                            }
                        }
                        // Close the channel on timeout
                        if let Some(ref tx_shared) = self.log_stream_tx {
                            if let Ok(mut guard) = tx_shared.lock() {
                                guard.take();
                            }
                        }
                        break;
                    }
                    // Read next line
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(line)) => {
                                // Debug: log first few lines to understand what's happening
                                if log_buffer.len() < 5 {
                                    debug!("Process {} ({}): {}", self.process_key, self.stream_name, line);
                                }
                                
                                // Write to log hub
                                if let Err(e) = self.log_hub.append_log_for_key(&self.process_key, line.as_bytes(), false).await {
                                    error!("Failed to write {} log for {}: {}", self.stream_name, self.process_key, e);
                                }

                                // Always add to buffer for context
                                log_buffer.push(line.clone());
                                if log_buffer.len() > LOG_BUFFER_SIZE {
                                    log_buffer.remove(0);
                                }

                                // Store current buffer state in log_context (always, not just for pattern match)
                                if let Ok(mut context) = self.log_context.lock() {
                                    *context = log_buffer.clone();
                                }

                                // Check if line matches the wait pattern
                                if let (Some(ref pattern), Some(ref tx)) = (&self.log_pattern, &self.log_ready_tx) {
                                    if pattern.is_match(&line) {
                                        debug!("Found log pattern match for process {} ({}): '{}'", 
                                            self.process_key, self.stream_name, line);

                                        // Log the pattern match with color (green for ready)
                                        let ready_msg = format!(
                                            "{} Process ready - pattern matched\n",
                                            "[mcproc]".green().bold()
                                        );
                                        if let Err(e) = self.log_hub.append_log_for_key(&self.process_key, ready_msg.as_bytes(), true).await {
                                            error!("Failed to write ready log: {}", e);
                                        }

                                        if let Ok(mut tx_guard) = tx.lock() {
                                            if let Some(sender) = tx_guard.take() {
                                                let _ = sender.send(());
                                            }
                                        }
                                        // Mark pattern as matched
                                        if let Ok(mut matched) = self.pattern_matched.lock() {
                                            *matched = true;
                                        }
                                        // Store the matched line
                                        if let Ok(mut matched_line) = self.matched_line.lock() {
                                            *matched_line = Some(line.clone());
                                        }
                                        // Log context already contains the matched line (set at line 102)
                                        // Send recent logs for context (for backward compatibility)
                                        if let Some(ref tx_shared) = self.log_stream_tx {
                                            let tx_clone = tx_shared.lock().ok().and_then(|guard| guard.as_ref().cloned());
                                            if let Some(tx) = tx_clone {
                                                for buffered_line in &log_buffer {
                                                    let _ = tx.send(buffered_line.as_bytes().to_vec()).await;
                                                }
                                            }
                                        }
                                    }
                                }

                                // Send log line to stream channel if available
                                if let Some(ref tx_shared) = self.log_stream_tx {
                                    let tx_clone = tx_shared.lock().ok().and_then(|guard| guard.as_ref().cloned());
                                    if let Some(tx) = tx_clone {
                                        let _ = tx.send(line.into_bytes()).await;
                                    }
                                }
                            }
                            Ok(None) => {
                                debug!("{} stream ended for process {}", self.stream_name, self.process_key);
                                // Close the log_ready channel to notify waiters about stream end
                                if let Some(ref tx) = self.log_ready_tx {
                                    if let Ok(mut tx_guard) = tx.lock() {
                                        tx_guard.take(); // Drop the sender to close the channel
                                    }
                                }
                                // Close the channel when stream ends
                                if let Some(ref tx_shared) = self.log_stream_tx {
                                    if let Ok(mut guard) = tx_shared.lock() {
                                        guard.take();
                                    }
                                }
                                break;
                            }
                            Err(e) => {
                                error!("Error reading {} for {}: {}", self.stream_name, self.process_key, e);
                                // Close the log_ready channel to notify waiters about error
                                if let Some(ref tx) = self.log_ready_tx {
                                    if let Ok(mut tx_guard) = tx.lock() {
                                        tx_guard.take(); // Drop the sender to close the channel
                                    }
                                }
                                // Close the channel on error
                                if let Some(ref tx_shared) = self.log_stream_tx {
                                    if let Ok(mut guard) = tx_shared.lock() {
                                        guard.take();
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
        })
    }
}

use crate::common::process_key::ProcessKey;
use crate::daemon::log::LogHub;
use crate::daemon::process::hyperlog::{HyperLogConfig, HyperLogStreamer};
use crate::daemon::process::proxy::ProxyInfo;
use regex::Regex;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncRead;
use tokio::sync::oneshot;

pub struct LogStreamConfig {
    pub stream_name: &'static str, // "stdout" or "stderr"
    pub process_key: ProcessKey,
    pub proxy: Arc<ProxyInfo>,
    pub log_pattern: Option<Arc<Regex>>,
    pub log_ready_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
    pub pattern_matched: Arc<Mutex<bool>>,
    pub timeout_occurred: Arc<Mutex<bool>>,
    pub wait_timeout: Option<u32>,
    pub default_wait_timeout_secs: u32,
    pub matched_line: Arc<Mutex<Option<String>>>, // The line that matched the pattern
    pub log_file_path: Option<PathBuf>,
    pub enable_file_logging: bool,
    pub log_hub: Arc<LogHub>,
}

impl LogStreamConfig {
    pub async fn spawn_log_reader<R: AsyncRead + Unpin + Send + 'static>(
        self,
        stream: R,
    ) -> tokio::task::JoinHandle<()> {
        // Use the new HyperLogStreamer for high-performance log processing
        let hyperlog_config = HyperLogConfig {
            stream_name: self.stream_name,
            process_key: self.process_key.clone(),
            proxy: self.proxy.clone(),
            log_pattern: self.log_pattern.clone(),
            log_ready_tx: self.log_ready_tx.clone(),
            pattern_matched: self.pattern_matched.clone(),
            matched_line: self.matched_line.clone(),
            timeout_occurred: self.timeout_occurred.clone(),
            wait_timeout: self.wait_timeout,
            default_wait_timeout_secs: self.default_wait_timeout_secs,
            is_stderr: self.stream_name == "stderr",
            log_file_path: self.log_file_path,
            enable_file_logging: self.enable_file_logging,
            log_hub: self.log_hub,
        };

        let streamer = HyperLogStreamer::new(hyperlog_config);
        streamer.spawn(stream).await
    }
}

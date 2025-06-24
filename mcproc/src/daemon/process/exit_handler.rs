use crate::common::exit_code::format_exit_reason;
use crate::daemon::error::{McprocdError, Result};
use crate::daemon::process::proxy::ProxyInfo;
use ringbuf::traits::Consumer;
use std::sync::Arc;
use tracing::debug;

pub struct ExitHandler;

impl ExitHandler {
    /// Check if process has exited and return appropriate error if it has
    pub fn check_process_exit(proxy: &Arc<ProxyInfo>, name: &str) -> Result<()> {
        if let Ok(exit_code) = proxy.exit_code.lock() {
            if let Some(code) = *exit_code {
                debug!("Process {} exited with code: {:?}", name, code);

                let exit_reason = format_exit_reason(code);
                let recent_logs = Self::get_recent_logs(proxy, 5);

                return Err(McprocdError::ProcessFailedToStart {
                    name: name.to_string(),
                    exit_code: code,
                    exit_reason,
                    stderr: recent_logs,
                });
            }
        }
        Ok(())
    }

    /// Get recent logs from the process ring buffer
    pub fn get_recent_logs(proxy: &Arc<ProxyInfo>, count: usize) -> String {
        if let Ok(ring) = proxy.ring.lock() {
            ring.iter()
                .take(count)
                .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        }
    }

    /// Format exit message for logging
    pub fn format_exit_message(name: &str, exit_code: Option<i32>) -> String {
        match exit_code {
            Some(code) => {
                let reason = format_exit_reason(code);
                format!("Process {} exited with code {} ({})", name, code, reason)
            }
            None => format!("Process {} exited without exit code", name),
        }
    }
}

pub mod batch_writer;

use crate::common::config::Config;
use crate::common::process_key::ProcessKey;
use crate::daemon::stream::{SharedStreamEventHub, StreamEvent};
use proto::LogEntry;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Centralized log management for all processes
///
/// LogHub manages log events and publishes them to the event hub.
/// It also provides log file path generation based on project organization.
pub struct LogHub {
    pub config: Arc<Config>,
    event_hub: Option<SharedStreamEventHub>,
}

impl LogHub {
    pub fn with_event_hub(config: Arc<Config>, event_hub: SharedStreamEventHub) -> Self {
        Self {
            config,
            event_hub: Some(event_hub),
        }
    }

    /// Publish a log event to the event hub
    pub fn publish_log_event(&self, key: &ProcessKey, content: &str, is_stderr: bool) {
        if let Some(ref event_hub) = self.event_hub {
            let now = chrono::Utc::now();
            let log_entry = LogEntry {
                line_number: 0, // Line numbers are tracked per reader, not here
                timestamp: Some(prost_types::Timestamp {
                    seconds: now.timestamp(),
                    nanos: now.timestamp_subsec_nanos() as i32,
                }),
                content: content.trim_end().to_string(), // Remove trailing newlines only, preserve indentation
                level: if is_stderr { 2 } else { 1 },    // ERROR = 2, INFO = 1
                process_name: None,                      // Will be set by subscriber if needed
            };

            debug!(
                "Publishing log event for {}/{}: {} (timestamp: {})",
                key.project,
                key.name,
                content.trim_end(),
                now.format("%H:%M:%S%.3f")
            );

            event_hub.publish(StreamEvent::Log {
                process_name: key.name.clone(),
                project: key.project.clone(),
                entry: log_entry,
            });
        } else {
            debug!("No event hub available for publishing log event");
        }
    }

    pub fn get_log_file_path_for_key(&self, key: &ProcessKey) -> PathBuf {
        let project_dir = self.config.paths.log_dir.join(&key.project);
        project_dir.join(format!("{}.log", key.sanitized_name()))
    }
}

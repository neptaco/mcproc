use crate::daemon::process::event::ProcessEvent;
use proto::LogEntry;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Unified stream event that includes both process events and log entries
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Process lifecycle event
    Process(ProcessEvent),
    /// Log entry from a process
    Log {
        process_name: String,
        project: String,
        entry: LogEntry,
    },
}

/// Filter for subscribing to specific events
#[derive(Debug, Clone)]
pub struct StreamFilter {
    /// Project name to filter by (None means all projects)
    pub project: Option<String>,
    /// Process names to filter (empty means all processes in project)
    pub process_names: Vec<String>,
    /// Include process events
    pub include_events: bool,
}

impl StreamFilter {
    /// Check if an event matches this filter
    pub fn matches(&self, event: &StreamEvent) -> bool {
        match event {
            StreamEvent::Process(pe) => {
                if !self.include_events {
                    return false;
                }
                self.matches_process(pe.project(), pe.name())
            }
            StreamEvent::Log {
                process_name,
                project,
                ..
            } => self.matches_process(project, process_name),
        }
    }

    pub fn matches_process(&self, project: &str, name: &str) -> bool {
        // Check project filter
        if let Some(ref filter_project) = self.project {
            if filter_project != project {
                return false;
            }
        }

        // Check process names filter
        if !self.process_names.is_empty() && !self.process_names.contains(&name.to_string()) {
            return false;
        }

        true
    }
}

/// Event hub that combines process events and log streams
pub struct StreamEventHub {
    sender: broadcast::Sender<StreamEvent>,
}

impl StreamEventHub {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(10000);
        Self { sender }
    }

    /// Publish a stream event
    pub fn publish(&self, event: StreamEvent) {
        // Note: broadcast::send returns Err when there are no active receivers
        // This is normal behavior and not an error condition
        let _ = self.sender.send(event);
    }

    /// Subscribe to events with a filter
    pub fn subscribe(&self) -> broadcast::Receiver<StreamEvent> {
        self.sender.subscribe()
    }
}

impl Default for StreamEventHub {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedStreamEventHub = Arc<StreamEventHub>;

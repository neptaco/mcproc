use std::sync::Arc;
use tokio::sync::broadcast;

/// Events related to process lifecycle
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ProcessEvent {
    /// Process is starting
    Starting {
        process_id: String,
        name: String,
        project: String,
    },
    /// Process has started and is running
    Started {
        process_id: String,
        name: String,
        project: String,
        pid: u32,
    },
    /// Process is stopping
    Stopping {
        process_id: String,
        name: String,
        project: String,
    },
    /// Process has stopped
    Stopped {
        process_id: String,
        name: String,
        project: String,
        exit_code: Option<i32>,
    },
    /// Process failed to start or crashed
    Failed {
        process_id: String,
        name: String,
        project: String,
        error: String,
    },
}

impl ProcessEvent {
    #[allow(dead_code)]
    pub fn process_id(&self) -> &str {
        match self {
            ProcessEvent::Starting { process_id, .. } => process_id,
            ProcessEvent::Started { process_id, .. } => process_id,
            ProcessEvent::Stopping { process_id, .. } => process_id,
            ProcessEvent::Stopped { process_id, .. } => process_id,
            ProcessEvent::Failed { process_id, .. } => process_id,
        }
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        match self {
            ProcessEvent::Starting { name, .. } => name,
            ProcessEvent::Started { name, .. } => name,
            ProcessEvent::Stopping { name, .. } => name,
            ProcessEvent::Stopped { name, .. } => name,
            ProcessEvent::Failed { name, .. } => name,
        }
    }

    #[allow(dead_code)]
    pub fn project(&self) -> &str {
        match self {
            ProcessEvent::Starting { project, .. } => project,
            ProcessEvent::Started { project, .. } => project,
            ProcessEvent::Stopping { project, .. } => project,
            ProcessEvent::Stopped { project, .. } => project,
            ProcessEvent::Failed { project, .. } => project,
        }
    }
}

/// Event bus for process lifecycle events
#[allow(dead_code)]
pub struct ProcessEventBus {
    sender: broadcast::Sender<ProcessEvent>,
}

impl ProcessEventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);
        Self { sender }
    }

    /// Publish an event to all subscribers
    #[allow(dead_code)]
    pub fn publish(&self, event: ProcessEvent) {
        // Ignore send errors (no receivers)
        let _ = self.sender.send(event);
    }

    /// Subscribe to process events
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.sender.subscribe()
    }
}

impl Default for ProcessEventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared event bus instance
#[allow(dead_code)]
pub type SharedEventBus = Arc<ProcessEventBus>;

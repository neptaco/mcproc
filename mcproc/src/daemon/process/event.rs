/// Events related to process lifecycle
#[derive(Debug, Clone)]
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
    pub fn name(&self) -> &str {
        match self {
            ProcessEvent::Starting { name, .. } => name,
            ProcessEvent::Started { name, .. } => name,
            ProcessEvent::Stopping { name, .. } => name,
            ProcessEvent::Stopped { name, .. } => name,
            ProcessEvent::Failed { name, .. } => name,
        }
    }

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

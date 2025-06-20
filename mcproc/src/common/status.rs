//! Common status formatting utilities

use colored::Colorize;

/// Format process status enum as string
pub fn format_status_enum(status: proto::ProcessStatus) -> String {
    match status {
        proto::ProcessStatus::Unknown => "Unknown".to_string(),
        proto::ProcessStatus::Starting => "Starting".to_string(),
        proto::ProcessStatus::Running => "Running".to_string(),
        proto::ProcessStatus::Stopping => "Stopping".to_string(),
        proto::ProcessStatus::Stopped => "Stopped".to_string(),
        proto::ProcessStatus::Failed => "Failed".to_string(),
    }
}

/// Format process status as string (from i32)
pub fn format_status(status: i32) -> String {
    let status_enum = proto::ProcessStatus::try_from(status)
        .unwrap_or(proto::ProcessStatus::Unknown);
    format_status_enum(status_enum)
}

/// Format process status as colored string for display
pub fn format_status_colored(status: i32) -> colored::ColoredString {
    let status_enum = proto::ProcessStatus::try_from(status)
        .unwrap_or(proto::ProcessStatus::Unknown);
    format_status_colored_enum(status_enum)
}

/// Format process status enum as colored string for display
pub fn format_status_colored_enum(status: proto::ProcessStatus) -> colored::ColoredString {
    let status_str = format_status_enum(status);
    
    match status {
        proto::ProcessStatus::Unknown => status_str.white(),
        proto::ProcessStatus::Starting => status_str.yellow(),
        proto::ProcessStatus::Running => status_str.green(),
        proto::ProcessStatus::Stopping => status_str.yellow(),
        proto::ProcessStatus::Stopped => status_str.red(),
        proto::ProcessStatus::Failed => status_str.red().bold(),
    }
}


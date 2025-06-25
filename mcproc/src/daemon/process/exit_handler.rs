use crate::common::exit_code::format_exit_reason;

pub struct ExitHandler;

impl ExitHandler {
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

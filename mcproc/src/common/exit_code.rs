//! Common exit code and reason formatting utilities

/// Format exit code into human-readable reason
pub fn format_exit_reason(exit_code: i32) -> String {
    match exit_code {
        0 => "Process exited normally".to_string(),
        1 => "General error".to_string(),
        2 => "Misuse of shell builtin".to_string(),
        126 => "Command cannot execute".to_string(),
        127 => "Command not found".to_string(),
        code if code > 128 => format!("Terminated by signal {}", code - 128),
        _ => "Unknown error".to_string(),
    }
}

/// Common exit codes
#[allow(dead_code)]
pub mod codes {
    pub const SUCCESS: i32 = 0;
    pub const GENERAL_ERROR: i32 = 1;
    pub const SHELL_BUILTIN_MISUSE: i32 = 2;
    pub const CANNOT_EXECUTE: i32 = 126;
    pub const COMMAND_NOT_FOUND: i32 = 127;
    pub const SIGNAL_BASE: i32 = 128;
}

/// Check if exit code indicates termination by signal
#[allow(dead_code)]
pub fn is_signal_termination(exit_code: i32) -> bool {
    exit_code > codes::SIGNAL_BASE
}

/// Get signal number from exit code (if terminated by signal)
#[allow(dead_code)]
pub fn get_signal_number(exit_code: i32) -> Option<i32> {
    if is_signal_termination(exit_code) {
        Some(exit_code - codes::SIGNAL_BASE)
    } else {
        None
    }
}
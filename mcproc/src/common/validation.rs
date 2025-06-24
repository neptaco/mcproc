//! Validation functions for project and process names

/// Validate project name
/// Ensures the project name is valid for use as a directory name and identifier
pub fn validate_project_name(name: &str) -> Result<(), String> {
    // Check for empty string
    if name.is_empty() {
        return Err("Project name cannot be empty".to_string());
    }

    // Check for single dot or double dot (reserved directory names)
    if name == "." || name == ".." {
        return Err("Project name cannot be '.' or '..'".to_string());
    }

    // Check for path separators
    if name.contains('/') || name.contains('\\') {
        return Err("Project name cannot contain path separators (/ or \\)".to_string());
    }

    // Check for invalid characters commonly problematic in file systems
    const INVALID_CHARS: &[char] = &[':', '*', '?', '"', '<', '>', '|', '\0'];
    if let Some(invalid_char) = name.chars().find(|c| INVALID_CHARS.contains(c)) {
        return Err(format!("Project name cannot contain '{}'", invalid_char));
    }

    // Check for leading or trailing whitespace
    if name != name.trim() {
        return Err("Project name cannot have leading or trailing whitespace".to_string());
    }

    // Check for control characters
    if name.chars().any(|c| c.is_control()) {
        return Err("Project name cannot contain control characters".to_string());
    }

    // Check maximum length (255 is a common file system limit)
    if name.len() > 255 {
        return Err("Project name cannot exceed 255 characters".to_string());
    }

    // Check for Windows reserved names (for cross-platform compatibility)
    const WINDOWS_RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let name_upper = name.to_uppercase();
    if WINDOWS_RESERVED.contains(&name_upper.as_str()) {
        return Err(format!("'{}' is a reserved name on Windows", name));
    }

    Ok(())
}

/// Validate process name
/// Ensures the process name is valid for use as an identifier
pub fn validate_process_name(name: &str) -> Result<(), String> {
    // Check for empty string
    if name.is_empty() {
        return Err("Process name cannot be empty".to_string());
    }

    // Check for single dot or double dot (could be confusing)
    if name == "." || name == ".." {
        return Err("Process name cannot be '.' or '..'".to_string());
    }

    // Check for path separators (process names shouldn't look like paths)
    if name.contains('/') || name.contains('\\') {
        return Err("Process name cannot contain path separators (/ or \\)".to_string());
    }

    // Check for invalid characters that might cause issues in logs or displays
    const INVALID_CHARS: &[char] = &[':', '*', '?', '"', '<', '>', '|', '\0'];
    if let Some(invalid_char) = name.chars().find(|c| INVALID_CHARS.contains(c)) {
        return Err(format!("Process name cannot contain '{}'", invalid_char));
    }

    // Check for leading or trailing whitespace
    if name != name.trim() {
        return Err("Process name cannot have leading or trailing whitespace".to_string());
    }

    // Check for control characters
    if name.chars().any(|c| c.is_control()) {
        return Err("Process name cannot contain control characters".to_string());
    }

    // Check maximum length (keep it reasonable for display purposes)
    if name.len() > 100 {
        return Err("Process name cannot exceed 100 characters".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_project_name() {
        // Valid names
        assert!(validate_project_name("my-project").is_ok());
        assert!(validate_project_name("project_123").is_ok());
        assert!(validate_project_name("MyProject").is_ok());
        assert!(validate_project_name("a").is_ok());

        // Invalid names - empty
        assert!(validate_project_name("").is_err());

        // Invalid names - dots
        assert!(validate_project_name(".").is_err());
        assert!(validate_project_name("..").is_err());

        // Invalid names - path separators
        assert!(validate_project_name("my/project").is_err());
        assert!(validate_project_name("my\\project").is_err());

        // Invalid names - special characters
        assert!(validate_project_name("my:project").is_err());
        assert!(validate_project_name("my*project").is_err());
        assert!(validate_project_name("my?project").is_err());
        assert!(validate_project_name("my\"project").is_err());
        assert!(validate_project_name("my<project").is_err());
        assert!(validate_project_name("my>project").is_err());
        assert!(validate_project_name("my|project").is_err());

        // Invalid names - whitespace
        assert!(validate_project_name(" project").is_err());
        assert!(validate_project_name("project ").is_err());
        assert!(validate_project_name(" project ").is_err());

        // Invalid names - Windows reserved
        assert!(validate_project_name("CON").is_err());
        assert!(validate_project_name("con").is_err());
        assert!(validate_project_name("PRN").is_err());
        assert!(validate_project_name("AUX").is_err());
        assert!(validate_project_name("NUL").is_err());
        assert!(validate_project_name("COM1").is_err());
        assert!(validate_project_name("LPT1").is_err());

        // Invalid names - control characters
        assert!(validate_project_name("my\nproject").is_err());
        assert!(validate_project_name("my\tproject").is_err());
        assert!(validate_project_name("my\0project").is_err());

        // Invalid names - too long
        let long_name = "a".repeat(256);
        assert!(validate_project_name(&long_name).is_err());
        let max_name = "a".repeat(255);
        assert!(validate_project_name(&max_name).is_ok());
    }

    #[test]
    fn test_validate_process_name() {
        // Valid names
        assert!(validate_process_name("frontend").is_ok());
        assert!(validate_process_name("backend-api").is_ok());
        assert!(validate_process_name("worker_123").is_ok());
        assert!(validate_process_name("MyService").is_ok());
        assert!(validate_process_name("a").is_ok());

        // Invalid names - empty
        assert!(validate_process_name("").is_err());

        // Invalid names - dots
        assert!(validate_process_name(".").is_err());
        assert!(validate_process_name("..").is_err());

        // Invalid names - path separators
        assert!(validate_process_name("my/process").is_err());
        assert!(validate_process_name("my\\process").is_err());

        // Invalid names - special characters
        assert!(validate_process_name("my:process").is_err());
        assert!(validate_process_name("my*process").is_err());
        assert!(validate_process_name("my?process").is_err());
        assert!(validate_process_name("my\"process").is_err());
        assert!(validate_process_name("my<process").is_err());
        assert!(validate_process_name("my>process").is_err());
        assert!(validate_process_name("my|process").is_err());

        // Invalid names - whitespace
        assert!(validate_process_name(" process").is_err());
        assert!(validate_process_name("process ").is_err());
        assert!(validate_process_name(" process ").is_err());

        // Invalid names - control characters
        assert!(validate_process_name("my\nprocess").is_err());
        assert!(validate_process_name("my\tprocess").is_err());
        assert!(validate_process_name("my\0process").is_err());

        // Invalid names - too long (process names have lower limit than project names)
        let long_name = "a".repeat(101);
        assert!(validate_process_name(&long_name).is_err());
        let max_name = "a".repeat(100);
        assert!(validate_process_name(&max_name).is_ok());
    }
}

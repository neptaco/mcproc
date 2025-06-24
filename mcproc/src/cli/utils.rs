//! Utility functions for mcproc

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

/// Get the project name from environment variable
/// Returns None if not set
pub fn get_project_from_env() -> Option<String> {
    std::env::var("MCPROC_DEFAULT_PROJECT").ok()
}

/// Get the project name from the current working directory
/// Returns None if unable to determine the project name
pub fn get_project_from_cwd() -> Option<String> {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .and_then(|n| n.into_string().ok())
}

/// Get the project name, using the provided value or inferring from the current directory
/// If no project is provided and cannot be inferred, returns an error
pub fn resolve_project_name(project: Option<String>) -> Result<String, String> {
    project
        .or_else(get_project_from_cwd)
        .ok_or_else(|| "Unable to determine project name from current directory".to_string())
}

/// Resolve project name for MCP tools
/// Prioritizes: params.project -> MCPROC_DEFAULT_PROJECT env -> current directory
/// All sources are validated and returns error if invalid
pub fn resolve_mcp_project_name(params_project: Option<String>) -> Result<String, mcp_rs::Error> {
    // First priority: explicitly provided parameter
    if let Some(project) = params_project {
        validate_project_name(&project)
            .map_err(|e| mcp_rs::Error::InvalidParams(format!("Invalid project name: {}", e)))?;
        return Ok(project);
    }

    // Second priority: environment variable (also needs validation)
    if let Some(env_project) = get_project_from_env() {
        validate_project_name(&env_project).map_err(|e| {
            mcp_rs::Error::InvalidParams(format!(
                "Invalid project name from MCPROC_DEFAULT_PROJECT: {}",
                e
            ))
        })?;
        return Ok(env_project);
    }

    // Third priority: current directory (also needs validation)
    if let Some(cwd_project) = get_project_from_cwd() {
        validate_project_name(&cwd_project).map_err(|e| {
            mcp_rs::Error::InvalidParams(format!(
                "Invalid project name from current directory '{}': {}",
                cwd_project, e
            ))
        })?;
        return Ok(cwd_project);
    }

    // No valid project name found
    Err(mcp_rs::Error::InvalidParams(
        "Unable to determine project name. Please specify --project, set MCPROC_DEFAULT_PROJECT, or run from a valid project directory".to_string()
    ))
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
}

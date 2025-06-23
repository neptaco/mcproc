/// Utility functions for mcproc
/// Get the project name from the current working directory
/// Returns None if unable to determine the project name
pub fn get_project_from_cwd() -> Option<String> {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_os_string()))
        .and_then(|n| n.into_string().ok())
}

/// Get the project name, using the provided value or inferring from the current directory
/// If no project is provided and cannot be inferred, returns "default"
pub fn resolve_project_name(project: Option<String>) -> String {
    project
        .or_else(get_project_from_cwd)
        .unwrap_or_else(|| "default".to_string())
}

/// Get the project name as Option, using the provided value or inferring from the current directory
/// Returns None if no project is provided and cannot be inferred
pub fn resolve_project_name_optional(project: Option<String>) -> Option<String> {
    project.or_else(get_project_from_cwd)
}

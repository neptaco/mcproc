//! Utility functions for mcproc

use crate::common::validation::validate_project_name;
use std::time::Duration;

pub(crate) fn start_deadline(wait_timeout: u32) -> Duration {
    Duration::from_secs(u64::from(wait_timeout).saturating_add(5))
}

pub(crate) fn stop_deadline(process_stop_timeout_ms: u64) -> Duration {
    Duration::from_millis(process_stop_timeout_ms.saturating_add(15_000))
}

pub(crate) fn restart_deadline(
    process_stop_timeout_ms: u64,
    wait_timeout: Option<u32>,
    default_wait_timeout_secs: u32,
) -> Duration {
    Duration::from_millis(process_stop_timeout_ms)
        .saturating_add(Duration::from_secs(u64::from(
            wait_timeout.unwrap_or(default_wait_timeout_secs),
        )))
        .saturating_add(Duration::from_secs(20))
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
    use super::{restart_deadline, start_deadline, stop_deadline};
    use std::time::Duration;

    #[test]
    fn start_deadline_handles_maximum_u32() {
        assert_eq!(
            start_deadline(u32::MAX),
            Duration::from_secs(u64::from(u32::MAX) + 5)
        );
    }

    #[test]
    fn stop_deadline_covers_force_fallback_and_cleanup() {
        assert_eq!(stop_deadline(30_000), Duration::from_secs(45));
    }

    #[test]
    fn restart_deadline_covers_stop_wait_and_cleanup() {
        assert_eq!(
            restart_deadline(30_000, Some(60), 30),
            Duration::from_secs(110)
        );
        assert_eq!(restart_deadline(30_000, None, 30), Duration::from_secs(80));
    }
}

pub mod api;
pub mod error;
pub mod log;
pub mod process;
pub mod stream;

use self::{log::LogHub, process::ProcessManager, stream::StreamEventHub};
use crate::common::config::Config;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub async fn run_daemon() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("mcprocd=info".parse()?))
        .init();

    // Force colored output for mcproc messages in logs
    // This ensures [mcproc] messages have colors even when writing to files
    colored::control::set_override(true);

    info!("Starting mcprocd daemon");

    // Load configuration
    let config = Arc::new(Config::load()?);
    config.ensure_directories()?;

    // Check if daemon is already running
    if let Ok(pid) = std::fs::read_to_string(&config.paths.pid_file) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            // Check if process is actually running
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                // Process exists, check if it's a zombie
                let is_zombie = check_if_zombie(pid);

                if is_zombie {
                    info!("Found zombie process with PID {}, cleaning up", pid);
                    // Remove stale PID file
                    let _ = std::fs::remove_file(&config.paths.pid_file);
                } else {
                    error!("mcprocd is already running with PID {}", pid);
                    return Err("Daemon already running".into());
                }
            } else {
                // Process doesn't exist, remove stale PID file
                info!("Removing stale PID file for non-existent process {}", pid);
                let _ = std::fs::remove_file(&config.paths.pid_file);
            }
        } else {
            // Invalid PID in file, remove it
            let _ = std::fs::remove_file(&config.paths.pid_file);
        }
    }

    // Write PID file
    let pid = std::process::id();
    std::fs::write(&config.paths.pid_file, pid.to_string())?;
    info!("Written PID {} to {:?}", pid, config.paths.pid_file);

    // Log configuration paths
    info!("Configuration paths:");
    info!("  Socket: {:?}", config.paths.socket_path);
    info!("  Data directory: {:?}", config.paths.data_dir);
    info!("  Log directory: {:?}", config.paths.log_dir);

    // Initialize components
    info!("Initializing log hub and process manager...");
    let event_hub = Arc::new(StreamEventHub::new());
    let log_hub = Arc::new(LogHub::with_event_hub(config.clone(), event_hub.clone()));
    let process_manager = Arc::new(ProcessManager::with_event_hub(
        config.clone(),
        log_hub.clone(),
        event_hub.clone(),
    ));
    info!("Components initialized successfully");

    // Start servers
    let grpc_config = config.clone();
    let grpc_pm = process_manager.clone();
    let grpc_log = log_hub.clone();
    let grpc_event_hub = event_hub.clone();

    info!("Starting gRPC server...");
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = self::api::grpc::start_grpc_server(grpc_config, grpc_pm, grpc_log, grpc_event_hub).await {
            error!("gRPC server error: {}", e);
        }
    });

    // Handle shutdown
    info!("Daemon is ready and waiting for connections");
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Received SIGINT (Ctrl+C)");
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM");
        }
        _ = grpc_handle => {
            error!("gRPC server terminated unexpectedly");
        }
    }

    info!("Shutting down mcprocd daemon");

    // Stop all managed processes
    info!("Stopping all managed processes...");
    let processes = process_manager.list_processes();
    for process in processes {
        if matches!(
            process.get_status(),
            crate::daemon::process::ProcessStatus::Running
                | crate::daemon::process::ProcessStatus::Starting
        ) {
            info!("Stopping process {}/{}", process.project, process.name);
            if let Err(e) = process_manager
                .stop_process(&process.name, Some(&process.project), false)
                .await
            {
                error!(
                    "Failed to stop process {}/{}: {}",
                    process.project, process.name, e
                );
            }
        }
    }

    // Give processes time to shut down gracefully
    tokio::time::sleep(tokio::time::Duration::from_millis(
        config.daemon.shutdown_grace_period_ms,
    ))
    .await;

    // Remove PID file
    if let Err(e) = std::fs::remove_file(&config.paths.pid_file) {
        error!("Failed to remove PID file: {}", e);
    }

    // Port file is no longer used (using Unix sockets instead)

    Ok(())
}

fn check_if_zombie(pid: i32) -> bool {
    #[cfg(target_os = "macos")]
    {
        // Use ps command to check process state on macOS
        if let Ok(output) = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output()
        {
            if let Ok(stat) = std::str::from_utf8(&output.stdout) {
                // On macOS, zombie processes have 'Z' in their state
                return stat.trim().contains('Z');
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Check /proc/{pid}/stat on Linux
        if let Ok(status) = std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
            // Parse the stat file to check process state
            // The state is the third field after the command name in parentheses
            if let Some(end) = status.rfind(')') {
                let after_cmd = &status[end + 1..];
                let fields: Vec<&str> = after_cmd.split_whitespace().collect();
                if !fields.is_empty() {
                    // State is the first field after the command
                    // 'Z' indicates zombie process
                    return fields[0] == "Z";
                }
            }
        }
    }

    // Default to false on other platforms
    false
}

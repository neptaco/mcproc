pub mod api;
pub mod error;
pub mod log;
pub mod process;
pub mod stream;

use self::{log::LogHub, process::ProcessManager, stream::StreamEventHub};
use crate::common::config::Config;
use fs2::FileExt;
use std::fs::File;
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

    // Acquire exclusive lock on PID file to prevent multiple daemon instances
    // The lock is held for the entire lifetime of the daemon process
    let pid_file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&config.paths.pid_file)?;

    // Try to acquire exclusive lock (non-blocking)
    match pid_file.try_lock_exclusive() {
        Ok(()) => {
            info!("Acquired exclusive lock on PID file");
        }
        Err(e) => {
            // Another daemon is holding the lock
            error!("Failed to acquire lock on PID file: {}", e);
            error!("Another mcprocd daemon is likely running");

            // Try to read the existing PID for informational purposes
            if let Ok(existing_pid) = std::fs::read_to_string(&config.paths.pid_file) {
                if let Ok(pid) = existing_pid.trim().parse::<i32>() {
                    error!("Existing daemon PID: {}", pid);
                }
            }

            return Err("Daemon already running (lock held)".into());
        }
    }

    // Write our PID to the file
    let pid = std::process::id();
    use std::io::Write;
    let mut pid_file_write = &pid_file;
    pid_file_write.set_len(0)?; // Truncate
    use std::io::Seek;
    pid_file_write.seek(std::io::SeekFrom::Start(0))?;
    write!(pid_file_write, "{}", pid)?;
    pid_file_write.flush()?;
    info!("Written PID {} to {:?}", pid, config.paths.pid_file);

    // Keep the file handle alive to maintain the lock
    // It will be automatically released when the process exits
    let _pid_file_lock = pid_file;

    // Note: Orphaned process detection is no longer needed
    // With parent-child process management, all child processes
    // are automatically terminated when the daemon stops

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

    // Start periodic process state synchronization
    process_manager.start_periodic_sync();

    info!("Components initialized successfully");

    // Start servers
    let grpc_config = config.clone();
    let grpc_pm = process_manager.clone();
    let grpc_log = log_hub.clone();
    let grpc_event_hub = event_hub.clone();

    info!("Starting gRPC server...");
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) =
            self::api::grpc::start_grpc_server(grpc_config, grpc_pm, grpc_log, grpc_event_hub).await
        {
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
    let running_processes: Vec<_> = processes
        .into_iter()
        .filter(|p| {
            matches!(
                p.get_status(),
                crate::daemon::process::ProcessStatus::Running
                    | crate::daemon::process::ProcessStatus::Starting
            )
        })
        .collect();

    let process_count = running_processes.len();
    if process_count > 0 {
        info!("Found {} running process(es) to stop", process_count);

        // Stop all processes in parallel for faster shutdown
        let mut stop_tasks = Vec::new();
        for process in running_processes {
            let pm = process_manager.clone();
            let name = process.name.clone();
            let project = process.project.clone();

            let task = tokio::spawn(async move {
                info!("Stopping process {}/{}", project, name);
                // Try graceful shutdown first during daemon shutdown
                if let Err(e) = pm.stop_process(&name, Some(&project), false).await {
                    error!("Failed to stop process {}/{}: {}", project, name, e);
                    (project, name, false)
                } else {
                    (project, name, true)
                }
            });
            stop_tasks.push(task);
        }

        // Wait for all stop tasks to complete
        let mut results = Vec::new();
        for task in stop_tasks {
            results.push(task.await);
        }

        // Log any failed processes
        let failed_count = results
            .into_iter()
            .flatten()
            .filter(|(_, _, success)| !success)
            .count();

        if failed_count > 0 {
            error!("{} process(es) failed to stop", failed_count);
        }
    }

    // Give processes time to shut down gracefully
    tokio::time::sleep(tokio::time::Duration::from_millis(
        config.daemon.daemon_shutdown_timeout_ms,
    ))
    .await;

    // Remove PID file
    if let Err(e) = std::fs::remove_file(&config.paths.pid_file) {
        error!("Failed to remove PID file: {}", e);
    }

    // Port file is no longer used (using Unix sockets instead)
    // Note: PID file lock is automatically released when _pid_file_lock is dropped

    Ok(())
}

use mcprocd::{
    api,
    config::Config,
    log::LogHub,
    process::ProcessManager,
};
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("mcprocd=info".parse()?))
        .init();
    
    info!("Starting mcprocd daemon");
    
    // Load configuration
    let config = Arc::new(Config::load()?);
    config.ensure_directories()?;
    
    // Check if daemon is already running
    if let Ok(pid) = std::fs::read_to_string(&config.daemon.pid_file) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            // Check if process is actually running
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                error!("mcprocd is already running with PID {}", pid);
                return Err("Daemon already running".into());
            }
        }
        // Remove stale PID file
        let _ = std::fs::remove_file(&config.daemon.pid_file);
    }
    
    // Write PID file
    let pid = std::process::id();
    std::fs::write(&config.daemon.pid_file, pid.to_string())?;
    info!("Written PID {} to {:?}", pid, config.daemon.pid_file);
    
    // Initialize components
    let log_hub = Arc::new(LogHub::new(config.clone()));
    let process_manager = Arc::new(ProcessManager::new(config.clone(), log_hub.clone()));
    
    // Start servers
    let grpc_config = config.clone();
    let grpc_pm = process_manager.clone();
    let grpc_log = log_hub.clone();
    
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = api::grpc::start_grpc_server(grpc_config, grpc_pm, grpc_log).await {
            error!("gRPC server error: {}", e);
        }
    });
    
    // Handle shutdown
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
        if matches!(process.get_status(), mcprocd::process::ProcessStatus::Running | mcprocd::process::ProcessStatus::Starting) {
            info!("Stopping process {}/{}", process.project, process.name);
            if let Err(e) = process_manager.stop_process(&process.name, Some(&process.project), false).await {
                error!("Failed to stop process {}/{}: {}", process.project, process.name, e);
            }
        }
    }
    
    // Give processes time to shut down gracefully
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Remove PID file
    if let Err(e) = std::fs::remove_file(&config.daemon.pid_file) {
        error!("Failed to remove PID file: {}", e);
    }
    
    // Remove port file
    let port_file = config.daemon.data_dir.join("mcprocd.port");
    if let Err(e) = std::fs::remove_file(&port_file) {
        error!("Failed to remove port file: {}", e);
    }
    
    Ok(())
}
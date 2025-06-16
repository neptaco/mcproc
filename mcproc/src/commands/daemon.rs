//! Daemon management commands

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
pub struct DaemonCommand {
    #[command(subcommand)]
    command: DaemonSubcommands,
}

#[derive(Subcommand)]
enum DaemonSubcommands {
    /// Start the mcprocd daemon
    Start,
    
    /// Stop the mcprocd daemon
    Stop,
    
    /// Restart the mcprocd daemon
    Restart,
    
    /// Show daemon status
    Status,
}

impl DaemonCommand {
    pub async fn execute(self) -> Result<(), Box<dyn std::error::Error>> {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".mcproc");
            
        let pid_file = data_dir.join("mcprocd.pid");
        let port_file = data_dir.join("mcprocd.port");
        
        match self.command {
            DaemonSubcommands::Start => {
                // Check if already running
                if is_daemon_running(&pid_file) {
                    println!("mcprocd daemon is already running");
                    return Ok(());
                }
                
                start_daemon()?;
                println!("Started mcprocd daemon");
                Ok(())
            }
            
            DaemonSubcommands::Stop => {
                if !is_daemon_running(&pid_file) {
                    println!("mcprocd daemon is not running");
                    return Ok(());
                }
                
                let pid = std::fs::read_to_string(&pid_file)?
                    .trim()
                    .parse::<i32>()?;
                
                // Send SIGTERM
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM
                )?;
                
                // Wait for daemon to stop
                for _ in 0..10 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if !is_daemon_running(&pid_file) {
                        println!("Stopped mcprocd daemon");
                        return Ok(());
                    }
                }
                
                println!("Warning: daemon did not stop gracefully, sending SIGKILL");
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL
                )?;
                
                // Clean up files
                let _ = std::fs::remove_file(&pid_file);
                let _ = std::fs::remove_file(&port_file);
                
                println!("Forcefully stopped mcprocd daemon");
                Ok(())
            }
            
            DaemonSubcommands::Restart => {
                // Stop if running
                if is_daemon_running(&pid_file) {
                    println!("Stopping mcprocd daemon...");
                    
                    let pid = std::fs::read_to_string(&pid_file)?
                        .trim()
                        .parse::<i32>()?;
                    
                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGTERM
                    )?;
                    
                    // Wait for stop
                    for _ in 0..10 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        if !is_daemon_running(&pid_file) {
                            break;
                        }
                    }
                }
                
                // Start new daemon
                start_daemon()?;
                println!("Restarted mcprocd daemon");
                Ok(())
            }
            
            DaemonSubcommands::Status => {
                if !pid_file.exists() {
                    println!("mcprocd daemon is not running (no PID file)");
                    return Ok(());
                }
                
                if !is_daemon_running(&pid_file) {
                    println!("mcprocd daemon is not running (stale PID file)");
                    // Clean up stale PID file
                    let _ = std::fs::remove_file(&pid_file);
                    return Ok(());
                }
                
                let pid = std::fs::read_to_string(&pid_file)?
                    .trim()
                    .parse::<i32>()?;
                    
                let port = if port_file.exists() {
                    std::fs::read_to_string(&port_file)
                        .ok()
                        .and_then(|s| s.trim().parse::<u16>().ok())
                        .unwrap_or(0)
                } else {
                    0
                };
                
                println!("mcprocd daemon is running");
                println!("  PID:  {}", pid);
                if port > 0 {
                    println!("  Port: {}", port);
                }
                println!("  Data: {}", data_dir.display());
                
                Ok(())
            }
        }
    }
}

fn is_daemon_running(pid_file: &PathBuf) -> bool {
    if let Ok(pid_str) = std::fs::read_to_string(pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Check if process is actually running
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
        } else {
            false
        }
    } else {
        false
    }
}

fn start_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let mcprocd_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .map(|p| p.join("mcprocd"))
        .unwrap_or_else(|| PathBuf::from("mcprocd"));
    
    // Check if mcprocd exists
    if !mcprocd_path.exists() {
        return Err(format!("mcprocd not found at: {}", mcprocd_path.display()).into());
    }
    
    println!("Starting mcprocd from: {}", mcprocd_path.display());
    
    // Create log file for daemon output
    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".mcproc");
    std::fs::create_dir_all(&data_dir)?;
    
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("mcprocd.log"))?;
    
    let mut cmd = std::process::Command::new(&mcprocd_path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    
    // Detach from parent process
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Create new session
                nix::unistd::setsid()?;
                Ok(())
            });
        }
    }
    
    match cmd.spawn() {
        Ok(child) => {
            println!("Spawned mcprocd with PID: {}", child.id());
            
            // Wait for daemon to start and create PID file
            let pid_file = data_dir.join("mcprocd.pid");
            
            for i in 0..20 {  // Wait up to 2 seconds
                std::thread::sleep(Duration::from_millis(100));
                if pid_file.exists() {
                    println!("Daemon started successfully");
                    return Ok(());
                }
                if i == 9 {
                    println!("Waiting for daemon to start...");
                }
            }
            
            Err("Daemon failed to start (PID file not created)".into())
        }
        Err(e) => {
            Err(format!("Failed to spawn mcprocd: {}", e).into())
        }
    }
}
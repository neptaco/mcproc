//! Daemon management commands

use crate::common::paths::McprocPaths;
use clap::{Parser, Subcommand};
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
        let paths = McprocPaths::new();
        paths.ensure_directories()?;

        match self.command {
            DaemonSubcommands::Start => {
                // Check if already running
                if is_daemon_running(&paths.pid_file) {
                    println!("mcprocd daemon is already running");
                    return Ok(());
                }

                start_daemon()?;
                println!("Started mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Stop => {
                if !is_daemon_running(&paths.pid_file) {
                    println!("mcprocd daemon is not running");
                    return Ok(());
                }

                let pid = std::fs::read_to_string(&paths.pid_file)?
                    .trim()
                    .parse::<i32>()?;

                // Send SIGTERM
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM,
                )?;

                // Wait for daemon to stop
                for _ in 0..10 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    if !is_daemon_running(&paths.pid_file) {
                        println!("Stopped mcprocd daemon");
                        return Ok(());
                    }
                }

                println!("Warning: daemon did not stop gracefully, sending SIGKILL");
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                )?;

                // Clean up files
                let _ = std::fs::remove_file(&paths.pid_file);

                println!("Forcefully stopped mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Restart => {
                // Stop if running
                if is_daemon_running(&paths.pid_file) {
                    println!("Stopping mcprocd daemon...");

                    let pid = std::fs::read_to_string(&paths.pid_file)?
                        .trim()
                        .parse::<i32>()?;

                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGTERM,
                    )?;

                    // Wait for stop
                    for _ in 0..10 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        if !is_daemon_running(&paths.pid_file) {
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
                if !paths.pid_file.exists() {
                    println!("mcprocd daemon is not running (no PID file)");
                    return Ok(());
                }

                if !is_daemon_running(&paths.pid_file) {
                    println!("mcprocd daemon is not running (stale PID file)");
                    // Clean up stale PID file
                    let _ = std::fs::remove_file(&paths.pid_file);
                    return Ok(());
                }

                let pid = std::fs::read_to_string(&paths.pid_file)?
                    .trim()
                    .parse::<i32>()?;

                println!("mcprocd daemon is running");
                println!("  PID:  {}", pid);
                println!("  Data: {}", paths.data_dir.display());

                Ok(())
            }
        }
    }
}

fn is_daemon_running(pid_file: &std::path::Path) -> bool {
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
    let mcproc_path =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("mcproc"));

    println!("Starting mcprocd daemon...");

    // Get paths and create directories
    let paths = McprocPaths::new();
    paths.ensure_directories()?;

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.daemon_log_file)?;

    let mut cmd = std::process::Command::new(&mcproc_path);
    cmd.arg("--daemon")
        .stdin(std::process::Stdio::null())
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
            for i in 0..20 {
                // Wait up to 2 seconds
                std::thread::sleep(Duration::from_millis(100));
                if paths.pid_file.exists() {
                    println!("Daemon started successfully");
                    return Ok(());
                }
                if i == 9 {
                    println!("Waiting for daemon to start...");
                }
            }

            Err("Daemon failed to start (PID file not created)".into())
        }
        Err(e) => Err(format!("Failed to spawn mcprocd: {}", e).into()),
    }
}

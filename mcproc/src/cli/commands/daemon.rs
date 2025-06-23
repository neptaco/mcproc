//! Daemon management commands

use crate::common::config::Config;
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
        let config = Config::for_client();
        config.ensure_directories()?;

        match self.command {
            DaemonSubcommands::Start => {
                // Check if already running
                if is_daemon_running(&config.paths.pid_file) {
                    println!("mcprocd daemon is already running");
                    return Ok(());
                }

                start_daemon()?;
                println!("Started mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Stop => {
                if !is_daemon_running(&config.paths.pid_file) {
                    println!("mcprocd daemon is not running");
                    return Ok(());
                }

                let pid = std::fs::read_to_string(&config.paths.pid_file)?
                    .trim()
                    .parse::<i32>()?;

                // Send SIGTERM
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM,
                )?;

                // Wait for daemon to stop
                let max_wait_iterations =
                    config.daemon.shutdown_grace_period_ms / config.daemon.stop_check_interval_ms;
                for _ in 0..max_wait_iterations {
                    tokio::time::sleep(Duration::from_millis(config.daemon.stop_check_interval_ms))
                        .await;
                    if !is_daemon_running(&config.paths.pid_file) {
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
                let _ = std::fs::remove_file(&config.paths.pid_file);

                println!("Forcefully stopped mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Restart => {
                // Stop if running
                if is_daemon_running(&config.paths.pid_file) {
                    println!("Stopping mcprocd daemon...");

                    let pid = std::fs::read_to_string(&config.paths.pid_file)?
                        .trim()
                        .parse::<i32>()?;

                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGTERM,
                    )?;

                    // Wait for stop
                    let max_wait_iterations = config.daemon.shutdown_grace_period_ms
                        / config.daemon.stop_check_interval_ms;
                    for _ in 0..max_wait_iterations {
                        tokio::time::sleep(Duration::from_millis(
                            config.daemon.stop_check_interval_ms,
                        ))
                        .await;
                        if !is_daemon_running(&config.paths.pid_file) {
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
                if !config.paths.pid_file.exists() {
                    println!("mcprocd daemon is not running (no PID file)");
                    return Ok(());
                }

                if !is_daemon_running(&config.paths.pid_file) {
                    println!("mcprocd daemon is not running (stale PID file)");
                    // Clean up stale PID file
                    let _ = std::fs::remove_file(&config.paths.pid_file);
                    return Ok(());
                }

                let pid = std::fs::read_to_string(&config.paths.pid_file)?
                    .trim()
                    .parse::<i32>()?;

                println!("mcprocd daemon is running");
                println!("  PID:  {}", pid);
                println!("  Data: {}", config.paths.data_dir.display());

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

    // Get config and create directories
    let config = Config::for_client();
    config.ensure_directories()?;

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.daemon_log_file())?;

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
            let max_wait_iterations =
                config.daemon.startup_timeout_ms / config.daemon.stop_check_interval_ms;
            for i in 0..max_wait_iterations {
                std::thread::sleep(Duration::from_millis(config.daemon.stop_check_interval_ms));
                if config.paths.pid_file.exists() {
                    println!("Daemon started successfully");
                    return Ok(());
                }
                if i == max_wait_iterations / 2 {
                    println!("Waiting for daemon to start...");
                }
            }

            Err("Daemon failed to start (PID file not created)".into())
        }
        Err(e) => Err(format!("Failed to spawn mcprocd: {}", e).into()),
    }
}

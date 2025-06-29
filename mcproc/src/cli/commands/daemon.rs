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
                    let mut stopped = false;
                    for _ in 0..max_wait_iterations {
                        tokio::time::sleep(Duration::from_millis(
                            config.daemon.stop_check_interval_ms,
                        ))
                        .await;
                        if !is_daemon_running(&config.paths.pid_file) {
                            stopped = true;
                            break;
                        }
                    }

                    // If daemon didn't stop gracefully, force kill it
                    if !stopped {
                        println!("Daemon did not stop gracefully, sending SIGKILL...");
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGKILL,
                        )?;
                        tokio::time::sleep(Duration::from_millis(200)).await;

                        // Clean up PID file
                        let _ = std::fs::remove_file(&config.paths.pid_file);
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

                // Try to connect to daemon to get detailed status
                match crate::client::DaemonClient::connect(None).await {
                    Ok(mut client) => {
                        let request = proto::GetDaemonStatusRequest {};
                        match client.inner().get_daemon_status(request).await {
                            Ok(response) => {
                                let status = response.into_inner();
                                println!("mcprocd daemon is running");
                                println!("  Version:   {}", status.version);
                                println!("  PID:       {}", status.pid);
                                println!("  Data:      {}", status.data_dir);
                                println!("  Uptime:    {}", format_uptime(status.uptime_seconds));
                                println!("  Processes: {}", status.active_processes);
                            }
                            Err(e) => {
                                // Fallback to basic info if gRPC fails
                                let pid = std::fs::read_to_string(&config.paths.pid_file)?
                                    .trim()
                                    .parse::<i32>()?;
                                println!("mcprocd daemon is running");
                                println!("  PID:  {}", pid);
                                println!("  Data: {}", config.paths.data_dir.display());
                                println!("  (Could not get detailed status: {})", e);
                            }
                        }
                    }
                    Err(_) => {
                        // Fallback to basic info if connection fails
                        let pid = std::fs::read_to_string(&config.paths.pid_file)?
                            .trim()
                            .parse::<i32>()?;
                        println!("mcprocd daemon is running");
                        println!("  PID:  {}", pid);
                        println!("  Data: {}", config.paths.data_dir.display());
                        println!("  (Could not connect to daemon for detailed status)");
                    }
                }

                Ok(())
            }
        }
    }
}

fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if days > 0 {
        format!("{}d {}h {}m {}s", days, hours, minutes, secs)
    } else if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

fn is_daemon_running(pid_file: &std::path::Path) -> bool {
    if let Ok(pid_str) = std::fs::read_to_string(pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Check if process is actually running
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                // Process exists, check if it's a zombie on macOS
                #[cfg(target_os = "macos")]
                {
                    // Use ps command to check process state on macOS
                    if let Ok(output) = std::process::Command::new("ps")
                        .args(["-p", &pid.to_string(), "-o", "stat="])
                        .output()
                    {
                        if let Ok(stat) = std::str::from_utf8(&output.stdout) {
                            // On macOS, zombie processes have 'Z' in their state
                            if stat.trim().contains('Z') {
                                return false;
                            }
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
                                if fields[0] == "Z" {
                                    return false;
                                }
                            }
                        }
                    }
                }

                // Process exists and is not a zombie
                true
            } else {
                false
            }
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

            // Check if daemon process is still running
            match nix::sys::signal::kill(nix::unistd::Pid::from_raw(child.id() as i32), None) {
                Ok(_) => {
                    // Process is running but didn't create PID file
                    eprintln!("Error: Daemon process started but failed to create PID file.");
                    eprintln!(
                        "Check the daemon log for errors: {}",
                        config.daemon_log_file().display()
                    );

                    // Kill the orphaned process
                    let _ = nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(child.id() as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                Err(_) => {
                    // Process exited
                    eprintln!("Error: Daemon process exited unexpectedly.");
                    eprintln!(
                        "Check the daemon log for errors: {}",
                        config.daemon_log_file().display()
                    );
                }
            }
            Err("Daemon failed to start (PID file not created)".into())
        }
        Err(e) => Err(format!("Failed to spawn mcprocd: {}", e).into()),
    }
}

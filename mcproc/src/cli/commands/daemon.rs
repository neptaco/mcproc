//! Daemon management commands

use crate::common::config::Config;
use clap::{Parser, Subcommand};
use std::time::Duration;
use sysinfo::{Pid, System};

fn command_identifies_mcproc_daemon(name: &str, command: &[String]) -> bool {
    let executable_is_mcproc = name == "mcproc"
        || command
            .first()
            .and_then(|argv0| std::path::Path::new(argv0).file_name())
            .and_then(|basename| basename.to_str())
            == Some("mcproc");
    executable_is_mcproc && command.iter().any(|argument| argument == "--daemon")
}

fn pid_is_mcproc_daemon(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err() {
        return false;
    }

    let system = System::new_all();
    let Some(process) = system.process(Pid::from_u32(pid as u32)) else {
        // Preserve the existing liveness behavior if process metadata is unavailable.
        return true;
    };
    let name = process.name().to_string_lossy();
    let command = process
        .cmd()
        .iter()
        .map(|part| part.to_string_lossy())
        .map(|part| part.into_owned())
        .collect::<Vec<_>>();
    command_identifies_mcproc_daemon(&name, &command)
}

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
                let (is_running, pid_opt) = is_daemon_running_with_details(&config);
                if is_running {
                    println!("mcprocd daemon is already running");
                    return Ok(());
                }

                // Clean up stale files if daemon is not running
                if let Some(pid) = pid_opt {
                    eprintln!("Cleaning up stale PID file for process {}", pid);
                    let _ = std::fs::remove_file(&config.paths.pid_file);
                }
                if config.paths.socket_path.exists() {
                    eprintln!("Cleaning up stale socket file");
                    let _ = std::fs::remove_file(&config.paths.socket_path);
                }

                start_daemon()?;
                println!("Started mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Stop => {
                let (is_running, pid_opt) = is_daemon_running_with_details(&config);
                if !is_running {
                    println!("mcprocd daemon is not running");
                    // Clean up stale files
                    if pid_opt.is_some() {
                        let _ = std::fs::remove_file(&config.paths.pid_file);
                    }
                    if config.paths.socket_path.exists() {
                        let _ = std::fs::remove_file(&config.paths.socket_path);
                    }
                    return Ok(());
                }

                let pid = std::fs::read_to_string(&config.paths.pid_file)?
                    .trim()
                    .parse::<i32>()?;

                if !pid_is_mcproc_daemon(pid) {
                    return Err(format!(
                        "Refusing to signal PID {pid}: process is not an mcproc daemon"
                    )
                    .into());
                }

                // Send SIGTERM
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGTERM,
                )?;

                // Wait for daemon to stop
                let max_wait_iterations =
                    config.daemon.daemon_shutdown_timeout_ms / config.daemon.stop_check_interval_ms;
                let mut elapsed = 0;
                for _ in 0..max_wait_iterations {
                    tokio::time::sleep(Duration::from_millis(config.daemon.stop_check_interval_ms))
                        .await;
                    elapsed += config.daemon.stop_check_interval_ms;

                    if !is_daemon_running(&config) {
                        println!("Stopped mcprocd daemon");
                        return Ok(());
                    }

                    // Show progress every 5 seconds
                    if elapsed % 5000 == 0 {
                        println!(
                            "Waiting for daemon to stop gracefully... ({}/{}s)",
                            elapsed / 1000,
                            config.daemon.daemon_shutdown_timeout_ms / 1000
                        );
                    }
                }

                println!("Warning: daemon did not stop gracefully, sending SIGKILL");
                if !pid_is_mcproc_daemon(pid) {
                    return Err(format!(
                        "Refusing to signal PID {pid}: process is not an mcproc daemon"
                    )
                    .into());
                }
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
                let (is_running, _) = is_daemon_running_with_details(&config);
                if is_running {
                    println!("Stopping mcprocd daemon...");

                    let pid = std::fs::read_to_string(&config.paths.pid_file)?
                        .trim()
                        .parse::<i32>()?;

                    if !pid_is_mcproc_daemon(pid) {
                        return Err(format!(
                            "Refusing to signal PID {pid}: process is not an mcproc daemon"
                        )
                        .into());
                    }

                    nix::sys::signal::kill(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGTERM,
                    )?;

                    // Wait for stop
                    let max_wait_iterations = config.daemon.daemon_shutdown_timeout_ms
                        / config.daemon.stop_check_interval_ms;
                    let mut stopped = false;
                    let mut elapsed = 0;
                    for _ in 0..max_wait_iterations {
                        tokio::time::sleep(Duration::from_millis(
                            config.daemon.stop_check_interval_ms,
                        ))
                        .await;
                        elapsed += config.daemon.stop_check_interval_ms;

                        if !is_daemon_running(&config) {
                            stopped = true;
                            break;
                        }

                        // Show progress every 5 seconds
                        if elapsed % 5000 == 0 {
                            println!(
                                "Waiting for daemon to stop gracefully... ({}/{}s)",
                                elapsed / 1000,
                                config.daemon.daemon_shutdown_timeout_ms / 1000
                            );
                        }
                    }

                    // If daemon didn't stop gracefully, force kill it
                    if !stopped {
                        println!("Daemon did not stop gracefully, sending SIGKILL...");
                        if !pid_is_mcproc_daemon(pid) {
                            return Err(format!(
                                "Refusing to signal PID {pid}: process is not an mcproc daemon"
                            )
                            .into());
                        }
                        nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGKILL,
                        )?;
                        tokio::time::sleep(Duration::from_millis(200)).await;

                        // Clean up PID file
                        let _ = std::fs::remove_file(&config.paths.pid_file);
                    }

                    // Also clean up socket file
                    if config.paths.socket_path.exists() {
                        let _ = std::fs::remove_file(&config.paths.socket_path);
                    }
                }

                // Start new daemon
                start_daemon()?;
                println!("Restarted mcprocd daemon");
                Ok(())
            }

            DaemonSubcommands::Status => {
                let (is_running, pid_opt) = is_daemon_running_with_details(&config);

                if !is_running {
                    if pid_opt.is_some() {
                        println!("mcprocd daemon is not running (stale PID file)");
                        // Clean up stale files
                        let _ = std::fs::remove_file(&config.paths.pid_file);
                        if config.paths.socket_path.exists() {
                            let _ = std::fs::remove_file(&config.paths.socket_path);
                        }
                    } else {
                        println!("mcprocd daemon is not running");
                    }
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

/// Check if daemon is running by verifying both PID and socket
fn is_daemon_running(config: &Config) -> bool {
    is_daemon_running_with_details(config).0
}

/// Check if daemon is running and return details (is_running, pid_option)
fn is_daemon_running_with_details(config: &Config) -> (bool, Option<i32>) {
    // First check if socket exists and is connectable
    let socket_path = &config.paths.socket_path;
    if socket_path.exists() {
        // Try to connect to the socket
        if let Ok(_stream) = std::os::unix::net::UnixStream::connect(socket_path) {
            // Socket is active, daemon is likely running
            // Now verify with PID file
            if let Ok(pid_str) = std::fs::read_to_string(&config.paths.pid_file) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    return (true, Some(pid));
                }
            }
            // Socket exists but no valid PID file - suspicious state
            eprintln!("Warning: Socket exists but PID file is invalid");
            return (true, None);
        }
    }

    // Socket doesn't exist or can't connect, check PID file
    if let Ok(pid_str) = std::fs::read_to_string(&config.paths.pid_file) {
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
                                return (false, Some(pid));
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
                                    return (false, Some(pid));
                                }
                            }
                        }
                    }
                }

                if !pid_is_mcproc_daemon(pid) {
                    return (false, Some(pid));
                }

                // Process exists and is not a zombie but socket is not working
                eprintln!(
                    "Warning: Process {} exists but socket is not accessible",
                    pid
                );
                return (true, Some(pid));
            } else {
                // PID file exists but process is not running
                return (false, Some(pid));
            }
        }
    }

    (false, None)
}

/// Find all running mcproc daemon processes
fn find_mcproc_daemons() -> Vec<i32> {
    let mut pids = Vec::new();

    #[cfg(unix)]
    {
        // Use ps to find all mcproc --daemon processes
        if let Ok(output) = std::process::Command::new("ps").args(["aux"]).output() {
            if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                for line in stdout.lines() {
                    if line.contains("mcproc")
                        && line.contains("--daemon")
                        && !line.contains("grep")
                    {
                        // Parse PID from the line
                        let fields: Vec<&str> = line.split_whitespace().collect();
                        if fields.len() > 1 {
                            if let Ok(pid) = fields[1].parse::<i32>() {
                                pids.push(pid);
                            }
                        }
                    }
                }
            }
        }
    }

    pids
}

fn start_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let mcproc_path =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("mcproc"));

    println!("Starting mcprocd daemon...");

    // Get config and create directories
    let config = Config::for_client();
    config.ensure_directories()?;

    // Check for any existing mcproc daemons
    let existing_daemons = find_mcproc_daemons();
    if !existing_daemons.is_empty() {
        eprintln!(
            "Warning: Found {} existing mcproc daemon process(es): {:?}",
            existing_daemons.len(),
            existing_daemons
        );
        eprintln!("Consider running 'mcproc daemon stop' first to clean up");
    }

    let mut log_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(config.daemon_log_file())?;

    use std::io::{Read, Seek, Write};
    let file_len = log_file.metadata()?.len();
    if file_len > 0 {
        log_file.seek(std::io::SeekFrom::End(-1))?;
        let mut last_byte = [0];
        log_file.read_exact(&mut last_byte)?;
        if last_byte[0] != b'\n' {
            log_file.write_all(b"\n")?;
        }
    }

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
                if config.paths.pid_file.exists()
                    && std::os::unix::net::UnixStream::connect(&config.paths.socket_path).is_ok()
                {
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
            Err("Daemon failed to start (PID file or socket not ready)".into())
        }
        Err(e) => Err(format!("Failed to spawn mcprocd: {}", e).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        command_identifies_mcproc_daemon, is_daemon_running_with_details, pid_is_mcproc_daemon,
    };
    use crate::common::config::Config;

    #[test]
    fn command_identity_requires_mcproc() {
        assert!(command_identifies_mcproc_daemon(
            "mcproc",
            &["mcproc".into(), "--daemon".into()]
        ));
        assert!(command_identifies_mcproc_daemon(
            "other",
            &["/usr/local/bin/mcproc".into(), "--daemon".into()]
        ));
        assert!(!command_identifies_mcproc_daemon(
            "mcproc",
            &["mcproc".into(), "start".into(), "x".into()]
        ));
        assert!(!command_identifies_mcproc_daemon(
            "vim",
            &["vim".into(), "mcproc/notes.md".into()]
        ));
        assert!(!command_identifies_mcproc_daemon(
            "sleep",
            &["sleep".into(), "5".into()]
        ));
    }

    #[test]
    fn sleep_process_is_not_an_mcproc_daemon() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        assert!(!pid_is_mcproc_daemon(child.id() as i32));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn connectable_socket_is_running_even_when_pid_is_not_mcproc() {
        let root = std::path::PathBuf::from("/tmp").join(format!("mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut config = Config::default();
        config.paths.socket_path = root.join("mcprocd.sock");
        config.paths.pid_file = root.join("mcprocd.pid");
        let listener = std::os::unix::net::UnixListener::bind(&config.paths.socket_path).unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        std::fs::write(&config.paths.pid_file, pid.to_string()).unwrap();

        assert_eq!(is_daemon_running_with_details(&config), (true, Some(pid)));

        child.kill().unwrap();
        child.wait().unwrap();
        drop(listener);
        std::fs::remove_dir_all(root).unwrap();
    }
}

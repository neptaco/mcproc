use crate::common::config::Config;
use crate::common::version::VERSION;
use proto::process_manager_client::ProcessManagerClient;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::warn;

#[derive(Clone)]
pub struct DaemonClient {
    client: ProcessManagerClient<Channel>,
}

impl DaemonClient {
    pub async fn connect(socket_path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::for_client();

        let socket_path = socket_path.unwrap_or(config.paths.socket_path.clone());

        // Check if daemon is running by checking PID file
        let daemon_running = if let Ok(pid_str) = std::fs::read_to_string(&config.paths.pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // Check if process is actually running
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
            } else {
                false
            }
        } else {
            false
        };

        if !daemon_running {
            eprintln!("mcprocd daemon is not running. Starting it automatically...");

            // Start daemon in background (use current binary with --daemon flag)
            let mcproc_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mcproc"));

            // Open log file for daemon output
            let log_file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(config.daemon_log_file())
                .map_err(|e| format!("Failed to open daemon log file: {}", e))?;

            let mut cmd = std::process::Command::new(&mcproc_path);
            cmd.arg("--daemon");
            cmd.stdin(std::process::Stdio::null())
                .stdout(
                    log_file
                        .try_clone()
                        .map_err(|e| format!("Failed to clone log file: {}", e))?,
                )
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
                Ok(_) => {
                    eprintln!("Started mcprocd daemon");
                    eprintln!("Daemon logs: {}", config.daemon_log_file().display());

                    // Wait for daemon to be ready by checking socket existence
                    let max_attempts = 10;
                    let check_interval = Duration::from_millis(100);
                    let mut socket_ready = false;

                    for i in 0..max_attempts {
                        if socket_path.exists() {
                            // Give it a tiny bit more time for the socket to be fully ready
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            socket_ready = true;
                            break;
                        }
                        tokio::time::sleep(check_interval).await;
                        if i == max_attempts / 2 {
                            eprintln!("Waiting for daemon to start...");
                        }
                    }

                    if !socket_ready {
                        eprintln!(
                            "Warning: Daemon socket not found after {} attempts",
                            max_attempts
                        );
                    }
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to start mcprocd daemon: {}. Please start it manually.",
                        e
                    )
                    .into());
                }
            }
        }

        // Connect to Unix socket with retry logic
        #[cfg(unix)]
        {
            use hyper_util::rt::tokio::TokioIo;
            use tokio::net::UnixStream;

            // If we just started the daemon, wait for socket with retries
            if !daemon_running {
                let max_connection_attempts = 20; // 2 seconds total
                let retry_interval = Duration::from_millis(100);
                let connected = false;

                for attempt in 0..max_connection_attempts {
                    if socket_path.exists() {
                        // Try to connect
                        const DUMMY_URI_FOR_UNIX_SOCKET: &str = "http://[::]:50051";
                        let socket_path_for_connector = socket_path.clone();

                        match Endpoint::try_from(DUMMY_URI_FOR_UNIX_SOCKET)?
                            .connect_timeout(Duration::from_secs(1))
                            .connect_with_connector(service_fn(move |_: Uri| {
                                let socket_path = socket_path_for_connector.clone();
                                async move {
                                    let stream = UnixStream::connect(&socket_path).await?;
                                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                                }
                            }))
                            .await
                        {
                            Ok(ch) => {
                                let channel = ch;
                                let client = ProcessManagerClient::new(channel);
                                let mut daemon_client = Self { client };

                                // Check daemon version
                                if let Err(e) = Self::check_and_restart_if_needed(
                                    &mut daemon_client,
                                    &socket_path,
                                )
                                .await
                                {
                                    warn!("Version check failed: {}", e);
                                }

                                return Ok(daemon_client);
                            }
                            Err(_) if attempt < max_connection_attempts - 1 => {
                                // Retry
                                tokio::time::sleep(retry_interval).await;
                                continue;
                            }
                            Err(e) => {
                                return Err(format!(
                                    "Failed to connect to daemon after {} attempts: {}",
                                    max_connection_attempts, e
                                )
                                .into());
                            }
                        }
                    }

                    tokio::time::sleep(retry_interval).await;
                }

                if !connected {
                    return Err("Failed to connect to daemon: socket not available".into());
                }
            }

            // Normal connection attempt (daemon was already running)
            if !socket_path.exists() {
                return Err(format!(
                    "Unix socket not found at {:?}. Is mcprocd running?",
                    socket_path
                )
                .into());
            }

            const DUMMY_URI_FOR_UNIX_SOCKET: &str = "http://[::]:50051";
            let socket_path_for_connector = socket_path.clone();
            let channel = Endpoint::try_from(DUMMY_URI_FOR_UNIX_SOCKET)?
                .connect_timeout(Duration::from_secs(
                    config.daemon.client_connection_timeout_secs,
                ))
                .connect_with_connector(service_fn(move |_: Uri| {
                    let socket_path = socket_path_for_connector.clone();
                    async move {
                        let stream = UnixStream::connect(&socket_path).await?;
                        Ok::<_, std::io::Error>(TokioIo::new(stream))
                    }
                }))
                .await?;

            let client = ProcessManagerClient::new(channel);

            let mut daemon_client = Self { client };

            // Check daemon version and restart if needed
            if let Err(e) =
                Self::check_and_restart_if_needed(&mut daemon_client, &socket_path).await
            {
                warn!("Version check failed: {}", e);
            }

            Ok(daemon_client)
        }

        #[cfg(not(unix))]
        {
            return Err("Unix sockets are not supported on this platform".into());
        }
    }

    async fn check_and_restart_if_needed(
        client: &mut Self,
        _socket_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request = proto::GetDaemonStatusRequest {};
        match client.inner().get_daemon_status(request).await {
            Ok(response) => {
                let status = response.into_inner();
                if status.version != VERSION {
                    eprintln!(
                        "Daemon version mismatch: daemon={}, client={}",
                        status.version, VERSION
                    );
                    eprintln!("Restarting daemon to update version...");

                    // Use daemon restart command
                    let mcproc_path =
                        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mcproc"));

                    let output = std::process::Command::new(&mcproc_path)
                        .arg("daemon")
                        .arg("restart")
                        .output()?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(format!("Failed to restart daemon: {}", stderr).into());
                    }

                    eprintln!("Daemon restarted successfully. Please run your command again.");
                    std::process::exit(0);
                }
            }
            Err(e) => {
                // If we can't get status, log warning but continue
                warn!("Could not verify daemon version: {}", e);
            }
        }
        Ok(())
    }

    pub fn inner(&mut self) -> &mut ProcessManagerClient<Channel> {
        &mut self.client
    }
}

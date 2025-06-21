use crate::common::config::Config;
use crate::common::version::VERSION;
use proto::process_manager_client::ProcessManagerClient;
use std::path::PathBuf;
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

            let mut cmd = std::process::Command::new(&mcproc_path);
            cmd.arg("--daemon");
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());

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
                    // Wait a bit for daemon to start
                    tokio::time::sleep(Duration::from_millis(config.daemon.client_startup_wait_ms))
                        .await;
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

        // Connect to Unix socket
        #[cfg(unix)]
        {
            use hyper_util::rt::tokio::TokioIo;
            use tokio::net::UnixStream;

            // Check if socket exists
            if !socket_path.exists() {
                return Err(format!(
                    "Unix socket not found at {:?}. Is mcprocd running?",
                    socket_path
                )
                .into());
            }

            // Create a channel using Unix socket transport
            // The URI here is a dummy value - it's required by the tonic API but ignored
            // when using Unix sockets. The actual connection is made via the socket_path.
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
        _socket_path: &PathBuf,
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

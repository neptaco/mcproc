use crate::common::paths::McprocPaths;
use proto::process_manager_client::ProcessManagerClient;
use std::path::PathBuf;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

#[derive(Clone)]
pub struct DaemonClient {
    client: ProcessManagerClient<Channel>,
}

impl DaemonClient {
    pub async fn connect(socket_path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let paths = McprocPaths::new();

        let socket_path = socket_path.unwrap_or(paths.socket_path.clone());

        // Check if daemon is running by checking PID file
        let daemon_running = if let Ok(pid_str) = std::fs::read_to_string(&paths.pid_file) {
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
                    tokio::time::sleep(Duration::from_millis(500)).await;
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

            let channel = Endpoint::try_from(DUMMY_URI_FOR_UNIX_SOCKET)?
                .connect_timeout(Duration::from_secs(5))
                .connect_with_connector(service_fn(move |_: Uri| {
                    let socket_path = socket_path.clone();
                    async move {
                        let stream = UnixStream::connect(&socket_path).await?;
                        Ok::<_, std::io::Error>(TokioIo::new(stream))
                    }
                }))
                .await?;

            let client = ProcessManagerClient::new(channel);

            return Ok(Self { client });
        }

        #[cfg(not(unix))]
        {
            return Err("Unix sockets are not supported on this platform".into());
        }
    }

    pub fn inner(&mut self) -> &mut ProcessManagerClient<Channel> {
        &mut self.client
    }
}

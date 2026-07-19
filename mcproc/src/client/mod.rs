use crate::common::config::Config;
use crate::common::version::VERSION;
use proto::process_manager_client::ProcessManagerClient;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;
use tracing::warn;

#[derive(Debug)]
pub struct DaemonRestartedForUpgrade;

impl std::fmt::Display for DaemonRestartedForUpgrade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "daemon restarted for upgrade")
    }
}

impl std::error::Error for DaemonRestartedForUpgrade {}

fn pid_file_indicates_running(pid_file: &Path) -> bool {
    let Ok(pid_string) = std::fs::read_to_string(pid_file) else {
        return false;
    };
    let Ok(pid) = pid_string.trim().parse::<i32>() else {
        return false;
    };

    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

async fn connect_channel(
    socket_path: &Path,
    connect_timeout: Duration,
) -> Result<Channel, tonic::transport::Error> {
    use hyper_util::rt::tokio::TokioIo;
    use tokio::net::UnixStream;

    const DUMMY_URI_FOR_UNIX_SOCKET: &str = "http://[::]:50051";
    let socket_path = socket_path.to_path_buf();

    Endpoint::from_static(DUMMY_URI_FOR_UNIX_SOCKET)
        .connect_timeout(connect_timeout)
        .connect_with_connector(service_fn(move |_: Uri| {
            let socket_path = socket_path.clone();
            async move {
                let stream = UnixStream::connect(socket_path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(stream))
            }
        }))
        .await
}

async fn connect_with_retry(
    socket_path: &Path,
    attempts: u32,
    interval: Duration,
    connect_timeout: Duration,
) -> Result<Channel, Box<dyn std::error::Error>> {
    for attempt in 0..attempts {
        if socket_path.exists() {
            match connect_channel(socket_path, connect_timeout).await {
                Ok(channel) => return Ok(channel),
                Err(_) if attempt < attempts - 1 => {
                    tokio::time::sleep(interval).await;
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "Failed to connect to daemon after {} attempts: {}",
                        attempts, error
                    )
                    .into());
                }
            }
        }

        tokio::time::sleep(interval).await;
    }

    Err("Failed to connect to daemon: socket not available".into())
}

fn spawn_daemon(config: &Config) -> Result<std::process::Child, Box<dyn std::error::Error>> {
    let mcproc_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("mcproc"));

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.daemon_log_file())
        .map_err(|error| format!("Failed to open daemon log file: {}", error))?;

    let mut command = std::process::Command::new(&mcproc_path);
    command.arg("--daemon");
    command.stdin(std::process::Stdio::null()).stdout(
        log_file
            .try_clone()
            .map_err(|error| format!("Failed to clone log file: {}", error))?,
    );
    command.stderr(log_file);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                nix::unistd::setsid()?;
                Ok(())
            });
        }
    }

    command.spawn().map_err(|error| {
        format!(
            "Failed to start mcprocd daemon: {}. Please start it manually.",
            error
        )
        .into()
    })
}

#[derive(Clone)]
pub struct DaemonClient {
    client: ProcessManagerClient<Channel>,
}

impl DaemonClient {
    pub async fn connect(socket_path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let config = Config::for_client();
        let socket_path = socket_path.unwrap_or(config.paths.socket_path.clone());
        let daemon_running = pid_file_indicates_running(&config.paths.pid_file);

        let channel = if daemon_running {
            if !socket_path.exists() {
                return Err(format!(
                    "Unix socket not found at {:?}. Is mcprocd running?",
                    socket_path
                )
                .into());
            }

            connect_channel(
                &socket_path,
                Duration::from_secs(config.daemon.client_connection_timeout_secs),
            )
            .await?
        } else {
            eprintln!("mcprocd daemon is not running. Starting it automatically...");

            config.ensure_directories()?;
            spawn_daemon(&config)?;

            eprintln!("Started mcprocd daemon");
            eprintln!("Daemon logs: {}", config.daemon_log_file().display());

            let max_attempts = 10;
            let check_interval = Duration::from_millis(100);
            let mut socket_ready = false;

            for attempt in 0..max_attempts {
                if socket_path.exists() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    socket_ready = true;
                    break;
                }
                tokio::time::sleep(check_interval).await;
                if attempt == max_attempts / 2 {
                    eprintln!("Waiting for daemon to start...");
                }
            }

            if !socket_ready {
                eprintln!(
                    "Warning: Daemon socket not found after {} attempts",
                    max_attempts
                );
            }

            connect_with_retry(
                &socket_path,
                20,
                Duration::from_millis(100),
                Duration::from_secs(1),
            )
            .await?
        };

        let mut daemon_client = Self::from_channel(channel);

        if let Err(error) =
            Self::check_and_restart_if_needed(&mut daemon_client, &socket_path).await
        {
            if error.downcast_ref::<DaemonRestartedForUpgrade>().is_some() {
                return Err(error);
            }
            warn!("Version check failed: {}", error);
        }

        Ok(daemon_client)
    }

    pub fn from_channel(channel: Channel) -> Self {
        Self {
            client: ProcessManagerClient::new(channel),
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

                    return Err(DaemonRestartedForUpgrade.into());
                }
            }
            Err(error) => {
                warn!("Could not verify daemon version: {}", error);
            }
        }
        Ok(())
    }

    pub fn inner(&mut self) -> &mut ProcessManagerClient<Channel> {
        &mut self.client
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::pending;
    use std::process::{Command, Stdio};
    use tokio::net::UnixListener;
    use uuid::Uuid;

    fn temporary_path(name: &str) -> (tempfile::TempDir, PathBuf) {
        let prefix = format!("mcproc-client-{}-", Uuid::new_v4());
        let directory = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in("/tmp")
            .unwrap();
        let path = directory.path().join(name);
        (directory, path)
    }

    fn spawn_accept_loop(listener: UnixListener) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _stream = stream;
                    pending::<()>().await;
                });
            }
        })
    }

    #[test]
    fn missing_pid_file_does_not_indicate_running() {
        let (_directory, pid_file) = temporary_path("missing.pid");

        assert!(!pid_file_indicates_running(&pid_file));
    }

    #[test]
    fn invalid_pid_file_does_not_indicate_running() {
        let (_directory, pid_file) = temporary_path("invalid.pid");
        std::fs::write(&pid_file, "abc").unwrap();

        assert!(!pid_file_indicates_running(&pid_file));
    }

    #[test]
    fn dead_process_pid_file_does_not_indicate_running() {
        let (_directory, pid_file) = temporary_path("dead.pid");
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        std::fs::write(&pid_file, pid.to_string()).unwrap();

        assert!(!pid_file_indicates_running(&pid_file));
    }

    #[test]
    fn live_process_pid_file_indicates_running() {
        let (_directory, pid_file) = temporary_path("live.pid");
        std::fs::write(&pid_file, std::process::id().to_string()).unwrap();

        assert!(pid_file_indicates_running(&pid_file));
    }

    #[tokio::test]
    async fn connect_channel_connects_to_unix_listener() {
        let (_directory, socket_path) = temporary_path("daemon.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let accept_task = spawn_accept_loop(listener);

        let result = connect_channel(&socket_path, Duration::from_secs(2)).await;

        accept_task.abort();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_channel_fails_for_missing_socket() {
        let (_directory, socket_path) = temporary_path("missing.sock");

        let result = connect_channel(&socket_path, Duration::from_secs(2)).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_with_retry_connects_when_listener_is_ready() {
        let (_directory, socket_path) = temporary_path("retry.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let accept_task = spawn_accept_loop(listener);

        let result = connect_with_retry(
            &socket_path,
            50,
            Duration::from_millis(100),
            Duration::from_secs(2),
        )
        .await;

        accept_task.abort();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_with_retry_fails_after_configured_attempts() {
        let (_directory, socket_path) = temporary_path("never-created.sock");

        let result = connect_with_retry(
            &socket_path,
            3,
            Duration::from_millis(10),
            Duration::from_millis(100),
        )
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn daemon_restarted_for_upgrade_is_downcastable_error() {
        let error: Box<dyn std::error::Error> = Box::new(DaemonRestartedForUpgrade);

        assert!(error.downcast_ref::<DaemonRestartedForUpgrade>().is_some());
    }
}

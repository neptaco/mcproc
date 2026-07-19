use crate::client::DaemonClient;
use crate::common::config::Config;
use crate::daemon::api::grpc::service::GrpcService;
use crate::daemon::log::LogHub;
use crate::daemon::process::ProcessManager;
use crate::daemon::stream::StreamEventHub;
use hyper_util::rt::tokio::TokioIo;
use mcp_rs::notification::QueuedNotificationSender;
use mcp_rs::ToolContext;
use proto::process_manager_server::ProcessManagerServer;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Endpoint, Server, Uri};
use tower::service_fn;

pub(super) struct McpTestHarness {
    pub client: DaemonClient,
    process_manager: Arc<ProcessManager>,
    root: PathBuf,
    server_task: JoinHandle<()>,
}

impl McpTestHarness {
    pub async fn new() -> Self {
        let root = PathBuf::from(format!("/tmp/mcp-{}", uuid::Uuid::new_v4()));
        let socket_path = root.join("s");

        let mut config = Config::default();
        config.paths.data_dir = root.join("data");
        config.paths.log_dir = root.join("log");
        config.paths.socket_path = socket_path.clone();
        config.paths.pid_file = root.join("pid");
        config.paths.daemon_log_file = root.join("daemon.log");
        config.process.restart.delay_ms = 0;
        config.process.restart.process_stop_timeout_ms = 10_000;
        config.ensure_directories().unwrap();

        let listener = UnixListener::bind(&socket_path).unwrap();
        let config = Arc::new(config);
        let event_hub = Arc::new(StreamEventHub::new());
        let log_hub = Arc::new(LogHub::with_event_hub(config.clone(), event_hub.clone()));
        let process_manager = Arc::new(ProcessManager::with_event_hub(
            config.clone(),
            log_hub.clone(),
            event_hub.clone(),
        ));
        let service = GrpcService::new(process_manager.clone(), log_hub, config, event_hub);

        let server_task = tokio::spawn(async move {
            Server::builder()
                .add_service(ProcessManagerServer::new(service))
                .serve_with_incoming(UnixListenerStream::new(listener))
                .await
                .unwrap();
        });

        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_timeout(Duration::from_secs(2))
            .connect_with_connector(service_fn(move |_: Uri| {
                let socket_path = socket_path.clone();
                async move {
                    let stream = UnixStream::connect(socket_path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(stream))
                }
            }))
            .await
            .unwrap();

        Self {
            client: DaemonClient::from_channel(channel),
            process_manager,
            root,
            server_task,
        }
    }

    pub fn context() -> ToolContext {
        ToolContext::new(Arc::new(QueuedNotificationSender::new()), None, None)
    }

    pub async fn wait_for_log(&self, path: &str, needles: &[&str]) -> String {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let contents = tokio::fs::read_to_string(path).await.unwrap_or_default();
                if needles.iter().all(|needle| contents.contains(needle)) {
                    return contents;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("process log was not flushed before the deadline")
    }

    pub async fn cleanup(mut self) {
        self.stop_all().await;
        self.server_task.abort();
        let _ = (&mut self.server_task).await;
        std::fs::remove_dir_all(&self.root).unwrap();
        self.root = PathBuf::new();
    }

    async fn stop_all(&self) {
        for process in self.process_manager.get_all_processes() {
            let stop = self
                .process_manager
                .stop_process(&process.id, Some(&process.project), true);
            tokio::time::timeout(Duration::from_secs(15), stop)
                .await
                .expect("managed process stop exceeded cleanup deadline")
                .expect("managed process cleanup failed");
        }
    }
}

impl Drop for McpTestHarness {
    fn drop(&mut self) {
        self.server_task.abort();

        #[cfg(unix)]
        for process in self.process_manager.get_all_processes() {
            unsafe {
                libc::kill(-(process.pid as i32), libc::SIGKILL);
            }
        }

        if !self.root.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

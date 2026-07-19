use crate::client::DaemonClient;
use crate::test_support::ProcessTestFixture;
use hyper_util::rt::tokio::TokioIo;
use mcp_rs::notification::QueuedNotificationSender;
use mcp_rs::ToolContext;
use proto::process_manager_server::ProcessManagerServer;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Endpoint, Server, Uri};
use tower::service_fn;

pub(super) struct McpTestHarness {
    pub client: DaemonClient,
    fixture: ProcessTestFixture,
    server_task: JoinHandle<()>,
}

impl McpTestHarness {
    pub async fn new() -> Self {
        let fixture = ProcessTestFixture::new("mcp", 10_000);
        let socket_path = fixture.socket_path();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let service = fixture.grpc_service();

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
            fixture,
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
        self.fixture.stop_all().await;
        self.server_task.abort();
        let _ = (&mut self.server_task).await;
        self.fixture.remove_root();
    }
}

impl Drop for McpTestHarness {
    fn drop(&mut self) {
        self.server_task.abort();
    }
}

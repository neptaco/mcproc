use super::service::GrpcService;
use crate::common::config::Config;
use crate::daemon::log::LogHub;
use crate::daemon::process::ProcessManager;
use crate::daemon::stream::StreamEventHub;
use proto::{
    restart_process_response, start_process_response, ProcessInfo, RestartProcessResponse,
    StartProcessRequest, StartProcessResponse,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::StreamExt;
use tonic::{Code, Request, Status};

pub(super) struct TestHarness {
    pub service: GrpcService,
    root: PathBuf,
}

impl TestHarness {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mcproc-grpc-test-{}", uuid::Uuid::new_v4()));
        let mut config = Config::default();
        config.paths.data_dir = root.join("data");
        config.paths.log_dir = root.join("log");
        config.paths.socket_path = root.join("runtime/mcprocd.sock");
        config.paths.pid_file = root.join("runtime/mcprocd.pid");
        config.paths.daemon_log_file = root.join("state/mcprocd.log");
        config.process.restart.delay_ms = 0;
        config.process.restart.process_stop_timeout_ms = 10_000;

        std::fs::create_dir_all(&config.paths.data_dir).unwrap();
        std::fs::create_dir_all(&config.paths.log_dir).unwrap();
        std::fs::create_dir_all(config.paths.socket_path.parent().unwrap()).unwrap();
        std::fs::create_dir_all(config.paths.daemon_log_file.parent().unwrap()).unwrap();

        let config = Arc::new(config);
        let event_hub = Arc::new(StreamEventHub::new());
        let log_hub = Arc::new(LogHub::with_event_hub(config.clone(), event_hub.clone()));
        let process_manager = Arc::new(ProcessManager::with_event_hub(
            config.clone(),
            log_hub.clone(),
            event_hub.clone(),
        ));
        let service = GrpcService::new(process_manager, log_hub, config, event_hub);

        Self { service, root }
    }

    pub fn start_request(name: &str, project: &str) -> StartProcessRequest {
        StartProcessRequest {
            name: name.to_string(),
            args: vec!["sleep".to_string(), "30".to_string()],
            project: project.to_string(),
            ..Default::default()
        }
    }

    pub async fn start(&self, name: &str, project: &str) -> Result<ProcessInfo, Status> {
        self.start_with_request(Self::start_request(name, project))
            .await
    }

    pub async fn start_with_request(
        &self,
        request: StartProcessRequest,
    ) -> Result<ProcessInfo, Status> {
        let response = self
            .service
            .start_process_impl(Request::new(request))
            .await?;
        process_from_start_stream(response.into_inner()).await
    }

    pub async fn cleanup(mut self) {
        let processes = self.service.process_manager.get_all_processes();
        for process in processes {
            let stop = self.service.process_manager.stop_process(
                &process.id,
                Some(&process.project),
                true,
            );
            tokio::time::timeout(Duration::from_secs(15), stop)
                .await
                .expect("managed process stop exceeded cleanup deadline")
                .expect("managed process cleanup failed");
        }
        std::fs::remove_dir_all(&self.root).unwrap();
        self.root = PathBuf::new();
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        if self.root.as_os_str().is_empty() {
            return;
        }

        #[cfg(unix)]
        for process in self.service.process_manager.get_all_processes() {
            unsafe {
                libc::kill(-(process.pid as i32), libc::SIGKILL);
            }
        }

        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) async fn process_from_start_stream(
    mut stream: <GrpcService as proto::process_manager_server::ProcessManager>::StartProcessStream,
) -> Result<ProcessInfo, Status> {
    while let Some(response) = stream.next().await {
        let StartProcessResponse { response } = response?;
        if let Some(start_process_response::Response::Process(process)) = response {
            return Ok(process);
        }
    }
    Err(Status::new(
        Code::Internal,
        "start stream ended without ProcessInfo",
    ))
}

pub(super) async fn process_from_restart_stream(
    mut stream: <GrpcService as proto::process_manager_server::ProcessManager>::RestartProcessStream,
) -> Result<ProcessInfo, Status> {
    while let Some(response) = stream.next().await {
        let RestartProcessResponse { response } = response?;
        if let Some(restart_process_response::Response::Process(process)) = response {
            return Ok(process);
        }
    }
    Err(Status::new(
        Code::Internal,
        "restart stream ended without ProcessInfo",
    ))
}

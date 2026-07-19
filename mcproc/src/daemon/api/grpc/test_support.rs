use super::service::GrpcService;
use crate::test_support::ProcessTestFixture;
use proto::{
    restart_process_response, start_process_response, ProcessInfo, RestartProcessResponse,
    StartProcessRequest, StartProcessResponse,
};
use tokio_stream::StreamExt;
use tonic::{Code, Request, Status};

pub(super) struct TestHarness {
    pub service: GrpcService,
    fixture: ProcessTestFixture,
}

impl TestHarness {
    pub fn new() -> Self {
        let fixture = ProcessTestFixture::new("mcproc-grpc-test", 10_000);
        let service = fixture.grpc_service();

        Self { service, fixture }
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
        self.fixture.stop_all().await;
        self.fixture.remove_root();
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

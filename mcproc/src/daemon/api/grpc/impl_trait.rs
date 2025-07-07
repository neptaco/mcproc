use super::service::GrpcService;
use proto::process_manager_server::ProcessManager as ProcessManagerService;
use proto::*;
use std::pin::Pin;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl ProcessManagerService for GrpcService {
    type StartProcessStream =
        Pin<Box<dyn Stream<Item = Result<StartProcessResponse, Status>> + Send>>;

    type RestartProcessStream =
        Pin<Box<dyn Stream<Item = Result<RestartProcessResponse, Status>> + Send>>;

    type GetLogsStream = Pin<Box<dyn Stream<Item = Result<GetLogsResponse, Status>> + Send>>;

    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<Self::StartProcessStream>, Status> {
        self.start_process_impl(request).await
    }

    async fn stop_process(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        self.stop_process_impl(request).await
    }

    async fn restart_process(
        &self,
        request: Request<RestartProcessRequest>,
    ) -> Result<Response<Self::RestartProcessStream>, Status> {
        self.restart_process_impl(request).await
    }

    async fn get_process(
        &self,
        request: Request<GetProcessRequest>,
    ) -> Result<Response<GetProcessResponse>, Status> {
        self.get_process_impl(request).await
    }

    async fn list_processes(
        &self,
        request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        self.list_processes_impl(request).await
    }

    async fn get_logs(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<Self::GetLogsStream>, Status> {
        self.get_logs_impl(request).await
    }

    async fn grep_logs(
        &self,
        request: Request<GrepLogsRequest>,
    ) -> Result<Response<GrepLogsResponse>, Status> {
        self.grep_logs_impl(request).await
    }

    async fn clean_project(
        &self,
        request: Request<CleanProjectRequest>,
    ) -> Result<Response<CleanProjectResponse>, Status> {
        self.clean_project_impl(request).await
    }

    async fn get_daemon_status(
        &self,
        request: Request<GetDaemonStatusRequest>,
    ) -> Result<Response<GetDaemonStatusResponse>, Status> {
        self.get_daemon_status_impl(request).await
    }
}

use super::helpers::{
    create_failed_process_info, create_process_info, create_timestamp, FailedProcessParams,
};
use super::service::GrpcService;
use crate::daemon::error::McprocdError;
use proto::process_manager_server::ProcessManager as ProcessManagerService;
use proto::*;
use tonic::{Request, Response, Status};
use tracing::error;

/// Convert McprocdError to appropriate gRPC Status
fn mcprocd_error_to_status(e: &McprocdError) -> Status {
    match e {
        McprocdError::ProcessNotFound { .. } => Status::not_found(e.to_string()),
        McprocdError::ProcessAlreadyExists(_) => Status::already_exists(e.to_string()),
        McprocdError::InvalidRequest(_) => Status::invalid_argument(e.to_string()),
        McprocdError::InvalidCommand { .. } => Status::invalid_argument(e.to_string()),
        McprocdError::InvalidRegex { .. } => Status::invalid_argument(e.to_string()),
        // All other errors are internal
        _ => Status::internal(e.to_string()),
    }
}

fn matches_status_filter(status: crate::daemon::process::ProcessStatus, filter: i32) -> bool {
    let status: proto::ProcessStatus = status.into();
    status as i32 == filter
}

fn validate_wait_timeout(wait_timeout: Option<u32>) -> Result<(), Status> {
    if wait_timeout.is_some_and(|timeout| timeout > 3600) {
        return Err(Status::invalid_argument(
            "wait_timeout must not exceed 3600 seconds",
        ));
    }
    Ok(())
}

fn validate_status_filter(filter: i32) -> Result<(), Status> {
    proto::ProcessStatus::try_from(filter)
        .map(|_| ())
        .map_err(|_| Status::invalid_argument(format!("Unknown status_filter value: {filter}")))
}

fn force_restart_stop_result(result: Result<(), McprocdError>) -> Result<(), Status> {
    result.map_err(|error| {
        Status::failed_precondition(format!(
            "Failed to stop existing process before restart: {error}"
        ))
    })
}

impl GrpcService {
    pub(super) async fn start_process_impl(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<<Self as ProcessManagerService>::StartProcessStream>, Status> {
        let req = request.into_inner();
        validate_wait_timeout(req.wait_timeout)?;

        // Validate process name
        if let Err(e) = crate::common::validation::validate_process_name(&req.name) {
            return Err(Status::invalid_argument(format!(
                "Invalid process name: {}",
                e
            )));
        }

        // Validate project name
        if let Err(e) = crate::common::validation::validate_project_name(&req.project) {
            return Err(Status::invalid_argument(format!(
                "Invalid project name: {}",
                e
            )));
        }

        let cwd = req.cwd.map(std::path::PathBuf::from);
        let name = req.name.clone();
        let project = req.project.clone();
        let wait_for_log = req.wait_for_log.clone();
        let wait_timeout = req.wait_timeout;
        let cmd_for_error = req.cmd.clone();
        let cwd_for_error = cwd.clone();
        let log_dir = self.config.paths.log_dir.clone();
        let force_restart = req.force_restart.unwrap_or(false);

        let process_manager = self.process_manager.clone();

        // Handle force_restart
        if force_restart {
            if let Some(existing) = process_manager
                .get_process_by_name_or_id_with_project(&name, Some(project.as_str()))
            {
                // Stop existing process
                force_restart_stop_result(
                    process_manager
                        .stop_process(&existing.id, Some(project.as_str()), true)
                        .await,
                )?;

                // Wait for process to be completely removed
                process_manager
                    .wait_for_process_removal(&name, Some(project.as_str()))
                    .await;
            }
        }

        // Create the response stream
        let stream = async_stream::try_stream! {
            // Start the process with log streaming
            match process_manager.start_process_with_log_stream(
                name.clone(),
                Some(project.clone()),
                req.cmd,
                req.args,
                cwd,
                Some(req.env),
                wait_for_log.clone(),
                wait_timeout,
                req.toolchain,
            ).await {
                Ok((process, timeout_occurred, _pattern_matched, log_context, matched_line)) => {
                    // Create ProcessInfo using helper
                    let info = create_process_info(
                        &process,
                        &log_dir,
                        Some(timeout_occurred),
                        log_context,
                        matched_line,
                    );

                    yield StartProcessResponse {
                        response: Some(start_process_response::Response::Process(info)),
                    };
                }
                Err(e) => {
                    // Return ProcessInfo with failed status instead of error
                    match &e {
                        crate::daemon::error::McprocdError::ProcessAlreadyExists(name) => {
                            // For already exists, still return as error for backward compatibility
                            let status = Status::already_exists(format!("Process '{}' is already running", name));
                            Err(status)?;
                        }
                        crate::daemon::error::McprocdError::ProcessFailedToStart { name, exit_code, exit_reason, stderr } => {
                            error!("Process '{}' failed to start: {} (exit code: {})", name, exit_reason, exit_code);

                            // Create a failed ProcessInfo using helper
                            let failed_info = create_failed_process_info(FailedProcessParams {
                                name,
                                project: &project,
                                cmd: cmd_for_error,
                                cwd: cwd_for_error.as_deref(),
                                log_dir: &log_dir,
                                exit_code: *exit_code,
                                exit_reason,
                                stderr,
                            });

                            yield StartProcessResponse {
                                response: Some(start_process_response::Response::Process(failed_info)),
                            };
                        }
                        _ => {
                            // Map error to appropriate gRPC status
                            Err(mcprocd_error_to_status(&e))?;
                        }
                    };
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    pub(super) async fn stop_process_impl(
        &self,
        request: Request<StopProcessRequest>,
    ) -> Result<Response<StopProcessResponse>, Status> {
        let req = request.into_inner();
        let process_manager = self.process_manager.clone();
        let name = req.name.clone();
        let project = req.project.clone();
        let force = req.force.unwrap_or(false);

        // Check if process exists
        if process_manager
            .get_process_by_name_or_id_with_project(&name, Some(&project))
            .is_none()
        {
            return Ok(Response::new(StopProcessResponse {
                success: false,
                message: Some(format!("Process '{}' not found", name)),
            }));
        }

        // Execute stop process synchronously to ensure graceful shutdown completes
        match process_manager
            .stop_process(&name, Some(project.as_str()), force)
            .await
        {
            Ok(()) => {
                tracing::info!("Process {} stopped successfully", name);
                Ok(Response::new(StopProcessResponse {
                    success: true,
                    message: Some(format!("Process '{}' stopped successfully", name)),
                }))
            }
            Err(e) => {
                tracing::error!("Failed to stop process {}: {}", name, e);
                Ok(Response::new(StopProcessResponse {
                    success: false,
                    message: Some(format!("Failed to stop process '{}': {}", name, e)),
                }))
            }
        }
    }

    pub(super) async fn restart_process_impl(
        &self,
        request: Request<RestartProcessRequest>,
    ) -> Result<Response<<Self as ProcessManagerService>::RestartProcessStream>, Status> {
        let req = request.into_inner();
        validate_wait_timeout(req.wait_timeout)?;
        let name = req.name.clone();
        let project = req.project.clone();
        let wait_for_log = req.wait_for_log.clone();
        let wait_timeout = req.wait_timeout;

        let process_manager = self.process_manager.clone();
        let log_dir = self.config.paths.log_dir.clone();

        let stream = async_stream::try_stream! {
            match process_manager
                .restart_process_with_log_stream(
                    &name,
                    Some(project.clone()),
                    wait_for_log.clone(),
                    wait_timeout,
                )
                .await
            {
                Ok((process, timeout_occurred, _pattern_matched, log_context, matched_line)) => {
                    // Stream log context if available
                    for (idx, log_line) in log_context.iter().enumerate() {
                        yield RestartProcessResponse {
                            response: Some(restart_process_response::Response::LogEntry(LogEntry {
                                line_number: idx as u32,
                                content: log_line.clone(),
                                timestamp: create_timestamp(chrono::Utc::now()),
                                level: log_entry::LogLevel::Stdout as i32,
                                process_name: Some(name.clone()),
                            })),
                        };
                    }

                    // Create ProcessInfo using helper
                    let info = create_process_info(
                        &process,
                        &log_dir,
                        Some(timeout_occurred),
                        log_context,
                        matched_line,
                    );

                    yield RestartProcessResponse {
                        response: Some(restart_process_response::Response::Process(info)),
                    };
                }
                Err(e) => {
                    match &e {
                        crate::daemon::error::McprocdError::ProcessFailedToStart { name, exit_code, exit_reason, stderr } => {
                            error!("Process '{}' failed to restart: {} (exit code: {})", name, exit_reason, exit_code);

                            // Create a failed ProcessInfo using helper
                            let failed_info = create_failed_process_info(FailedProcessParams {
                                name,
                                project: &project,
                                cmd: None,  // cmd
                                cwd: None,  // cwd
                                log_dir: &log_dir,
                                exit_code: *exit_code,
                                exit_reason,
                                stderr,
                            });

                            yield RestartProcessResponse {
                                response: Some(restart_process_response::Response::Process(failed_info)),
                            };
                        }
                        _ => {
                            // Map error to appropriate gRPC status
                            Err(mcprocd_error_to_status(&e))?;
                        }
                    };
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }

    pub(super) async fn get_process_impl(
        &self,
        request: Request<GetProcessRequest>,
    ) -> Result<Response<GetProcessResponse>, Status> {
        let req = request.into_inner();

        match self
            .process_manager
            .get_process_by_name_or_id_with_project(&req.name, Some(req.project.as_str()))
        {
            Some(process) => {
                // Create ProcessInfo using helper
                let info = create_process_info(
                    &process,
                    &self.config.paths.log_dir,
                    None,   // wait_timeout_occurred
                    vec![], // log_context
                    None,   // matched_line
                );

                Ok(Response::new(GetProcessResponse {
                    process: Some(info),
                }))
            }
            None => Err(Status::not_found("Process not found")),
        }
    }

    pub(super) async fn list_processes_impl(
        &self,
        request: Request<ListProcessesRequest>,
    ) -> Result<Response<ListProcessesResponse>, Status> {
        let req = request.into_inner();
        let mut processes = self.process_manager.get_all_processes();

        // Filter by project if specified
        if let Some(project_filter) = req.project_filter {
            processes.retain(|p| p.project == project_filter);
        }

        if let Some(status_filter) = req.status_filter {
            validate_status_filter(status_filter)?;
            processes.retain(|p| matches_status_filter(p.get_status(), status_filter));
        }

        let log_dir = self.config.paths.log_dir.clone();
        let process_infos: Vec<ProcessInfo> = processes
            .into_iter()
            .map(|process| {
                // Create ProcessInfo using helper
                create_process_info(
                    &process,
                    &log_dir,
                    None,   // wait_timeout_occurred
                    vec![], // log_context
                    None,   // matched_line
                )
            })
            .collect();

        Ok(Response::new(ListProcessesResponse {
            processes: process_infos,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        force_restart_stop_result, matches_status_filter, validate_status_filter,
        validate_wait_timeout,
    };
    use crate::daemon::api::grpc::test_support::{process_from_restart_stream, TestHarness};
    use crate::daemon::error::McprocdError;
    use crate::daemon::process::ProcessStatus;
    use proto::{
        GetProcessRequest, ListProcessesRequest, RestartProcessRequest, StopProcessRequest,
    };
    use tonic::{Code, Request};

    #[test]
    fn status_filter_selects_only_requested_status() {
        let statuses = [ProcessStatus::Running, ProcessStatus::Stopped];
        let running = statuses
            .into_iter()
            .filter(|status| matches_status_filter(*status, proto::ProcessStatus::Running as i32))
            .collect::<Vec<_>>();

        assert_eq!(running, vec![ProcessStatus::Running]);
    }

    #[test]
    fn wait_timeout_is_limited_to_one_hour() {
        assert!(validate_wait_timeout(Some(3600)).is_ok());
        assert_eq!(
            validate_wait_timeout(Some(3601)).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn undefined_status_filter_is_rejected() {
        assert_eq!(
            validate_status_filter(999).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn force_restart_propagates_stop_failure() {
        let result =
            force_restart_stop_result(Err(McprocdError::StopError("cannot stop".to_string())));
        assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_start_process_returns_running_process_info() {
        let harness = TestHarness::new();
        let process = harness.start("worker", "alpha").await.unwrap();
        harness.cleanup().await;

        assert_eq!(process.name, "worker");
        assert_eq!(process.project, "alpha");
        assert!(process.pid.is_some_and(|pid| pid > 0));
        assert!(matches!(
            proto::ProcessStatus::try_from(process.status).unwrap(),
            proto::ProcessStatus::Starting | proto::ProcessStatus::Running
        ));
    }

    #[tokio::test]
    async fn grpc_rpc_start_process_rejects_invalid_process_name() {
        let harness = TestHarness::new();
        let mut request = TestHarness::start_request("a/b", "alpha");
        request.args.clear();
        let result = harness
            .service
            .start_process_impl(Request::new(request))
            .await;
        harness.cleanup().await;

        assert_eq!(result.err().unwrap().code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn grpc_rpc_start_process_rejects_wait_timeout_over_one_hour() {
        let harness = TestHarness::new();
        let mut request = TestHarness::start_request("worker", "alpha");
        request.wait_timeout = Some(3601);
        let result = harness
            .service
            .start_process_impl(Request::new(request))
            .await;
        harness.cleanup().await;

        assert_eq!(result.err().unwrap().code(), Code::InvalidArgument);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_start_process_rejects_duplicate_name_in_project() {
        let harness = TestHarness::new();
        harness.start("worker", "alpha").await.unwrap();
        let duplicate = harness.start("worker", "alpha").await;
        harness.cleanup().await;

        assert_eq!(duplicate.unwrap_err().code(), Code::AlreadyExists);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_start_process_reports_missing_command_as_failed_process() {
        let harness = TestHarness::new();
        let mut request = TestHarness::start_request("missing-command", "alpha");
        request.args.clear();
        request.cmd = Some("definitely-not-a-command-xyz".to_string());
        let process = harness.start_with_request(request).await.unwrap();
        harness.cleanup().await;

        assert_eq!(
            process.status,
            proto::ProcessStatus::Failed as i32,
            "a command lookup failure must be represented in ProcessInfo"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_start_process_force_restart_replaces_running_process() {
        let harness = TestHarness::new();
        let first = harness.start("worker", "alpha").await.unwrap();
        let mut request = TestHarness::start_request("worker", "alpha");
        request.force_restart = Some(true);
        let replacement = harness.start_with_request(request).await.unwrap();
        harness.cleanup().await;

        assert_ne!(first.pid, replacement.pid);
        assert_eq!(replacement.status, proto::ProcessStatus::Running as i32);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_stop_process_handles_existing_and_missing_processes() {
        let harness = TestHarness::new();
        harness.start("worker", "alpha").await.unwrap();
        let stopped = harness
            .service
            .stop_process_impl(Request::new(StopProcessRequest {
                name: "worker".to_string(),
                project: "alpha".to_string(),
                force: Some(true),
            }))
            .await
            .unwrap()
            .into_inner();
        let removed = harness
            .service
            .process_manager
            .get_process_by_name_or_id_with_project("worker", Some("alpha"))
            .is_none();
        let missing = harness
            .service
            .stop_process_impl(Request::new(StopProcessRequest {
                name: "absent".to_string(),
                project: "alpha".to_string(),
                force: Some(true),
            }))
            .await
            .unwrap()
            .into_inner();
        harness.cleanup().await;

        assert!(stopped.success);
        assert!(removed);
        assert!(!missing.success);
        assert!(missing.message.unwrap().contains("not found"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_restart_process_replaces_pid_and_missing_process_is_not_found() {
        let harness = TestHarness::new();
        let first = harness.start("worker", "alpha").await.unwrap();
        let response = harness
            .service
            .restart_process_impl(Request::new(RestartProcessRequest {
                name: "worker".to_string(),
                project: "alpha".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap();
        let restarted = process_from_restart_stream(response.into_inner())
            .await
            .unwrap();
        let missing_response = harness
            .service
            .restart_process_impl(Request::new(RestartProcessRequest {
                name: "absent".to_string(),
                project: "alpha".to_string(),
                ..Default::default()
            }))
            .await
            .unwrap();
        let missing = process_from_restart_stream(missing_response.into_inner()).await;
        harness.cleanup().await;

        assert_ne!(first.pid, restarted.pid);
        assert_eq!(restarted.status, proto::ProcessStatus::Running as i32);
        assert_eq!(missing.unwrap_err().code(), Code::NotFound);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_get_process_returns_metadata_and_not_found() {
        let harness = TestHarness::new();
        let started = harness.start("worker", "alpha").await.unwrap();
        let found = harness
            .service
            .get_process_impl(Request::new(GetProcessRequest {
                name: "worker".to_string(),
                project: "alpha".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .process
            .unwrap();
        let missing = harness
            .service
            .get_process_impl(Request::new(GetProcessRequest {
                name: "absent".to_string(),
                project: "alpha".to_string(),
            }))
            .await;
        harness.cleanup().await;

        assert_eq!(found.name, "worker");
        assert_eq!(found.project, "alpha");
        assert_eq!(found.pid, started.pid);
        assert_eq!(found.status, proto::ProcessStatus::Running as i32);
        assert_eq!(missing.unwrap_err().code(), Code::NotFound);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn grpc_rpc_list_processes_applies_project_and_status_filters() {
        let harness = TestHarness::new();
        harness.start("alpha-worker", "alpha").await.unwrap();
        harness.start("beta-worker", "beta").await.unwrap();
        let by_project = harness
            .service
            .list_processes_impl(Request::new(ListProcessesRequest {
                project_filter: Some("alpha".to_string()),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let running = harness
            .service
            .list_processes_impl(Request::new(ListProcessesRequest {
                status_filter: Some(proto::ProcessStatus::Running as i32),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        let invalid = harness
            .service
            .list_processes_impl(Request::new(ListProcessesRequest {
                status_filter: Some(999),
                ..Default::default()
            }))
            .await;
        harness.cleanup().await;

        assert_eq!(by_project.processes.len(), 1);
        assert_eq!(by_project.processes[0].project, "alpha");
        assert_eq!(running.processes.len(), 2);
        assert!(running
            .processes
            .iter()
            .all(|process| process.status == proto::ProcessStatus::Running as i32));
        assert_eq!(invalid.unwrap_err().code(), Code::InvalidArgument);
    }
}

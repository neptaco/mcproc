use super::helpers::{
    create_failed_process_info, create_process_info, create_timestamp, FailedProcessParams,
};
use super::service::GrpcService;
use proto::process_manager_server::ProcessManager as ProcessManagerService;
use proto::*;
use tonic::{Request, Response, Status};
use tracing::error;

impl GrpcService {
    pub(super) async fn start_process_impl(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<<Self as ProcessManagerService>::StartProcessStream>, Status> {
        let req = request.into_inner();

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
                let _ = process_manager
                    .stop_process(&existing.id, Some(project.as_str()), true) // Use force=true
                    .await;

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
                            // Other errors still return as status errors
                            let status = Status::internal(e.to_string());
                            Err(status)?;
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
                            // Other errors still return as status errors
                            let status = Status::internal(e.to_string());
                            Err(status)?;
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
        let mut processes = self.process_manager.list_processes();

        // Filter by project if specified
        if let Some(project_filter) = req.project_filter {
            processes.retain(|p| p.project == project_filter);
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

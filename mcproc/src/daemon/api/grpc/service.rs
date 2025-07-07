use crate::common::config::Config;
use crate::common::version::VERSION;
use crate::daemon::log::LogHub;
use crate::daemon::process::ProcessManager;
use crate::daemon::stream::SharedStreamEventHub;
use chrono::{DateTime, Utc};
use proto::*;
use std::sync::Arc;
use tonic::{Request, Response, Status};

pub struct GrpcService {
    pub(super) process_manager: Arc<ProcessManager>,
    pub(super) log_hub: Arc<LogHub>,
    pub(super) config: Arc<Config>,
    pub(super) event_hub: SharedStreamEventHub,
    pub(super) start_time: DateTime<Utc>,
}

impl GrpcService {
    pub fn new(
        process_manager: Arc<ProcessManager>,
        log_hub: Arc<LogHub>,
        config: Arc<Config>,
        event_hub: SharedStreamEventHub,
    ) -> Self {
        Self {
            process_manager,
            log_hub,
            config,
            event_hub,
            start_time: Utc::now(),
        }
    }

    pub(super) async fn clean_project_impl(
        &self,
        request: Request<CleanProjectRequest>,
    ) -> Result<Response<CleanProjectResponse>, Status> {
        let req = request.into_inner();

        if req.all_projects {
            // Clean all projects
            let results = self
                .process_manager
                .clean_all_projects(req.force)
                .await
                .map_err(|e| Status::internal(format!("Failed to clean all projects: {}", e)))?;

            let project_results: Vec<_> = results
                .into_iter()
                .map(
                    |(project, stopped_names)| proto::clean_project_response::ProjectCleanResult {
                        project,
                        processes_stopped: stopped_names.len() as u32,
                        logs_deleted: 0,
                        stopped_process_names: stopped_names,
                        deleted_log_files: vec![],
                    },
                )
                .collect();

            Ok(Response::new(CleanProjectResponse {
                processes_stopped: 0,
                logs_deleted: 0,
                stopped_process_names: vec![],
                deleted_log_files: vec![],
                project_results,
            }))
        } else {
            // Clean single project
            let project = req.project.as_deref().unwrap_or("default");
            let stopped_names = self
                .process_manager
                .clean_project(project, req.force)
                .await
                .map_err(|e| {
                    Status::internal(format!("Failed to clean project {}: {}", project, e))
                })?;

            Ok(Response::new(CleanProjectResponse {
                processes_stopped: stopped_names.len() as u32,
                logs_deleted: 0,
                stopped_process_names: stopped_names,
                deleted_log_files: vec![],
                project_results: vec![],
            }))
        }
    }

    pub(super) async fn get_daemon_status_impl(
        &self,
        _request: Request<GetDaemonStatusRequest>,
    ) -> Result<Response<GetDaemonStatusResponse>, Status> {
        let pid = std::process::id();
        let now = Utc::now();
        let uptime_seconds = (now - self.start_time).num_seconds() as u64;

        let active_processes = self.process_manager.get_all_processes().len() as u32;

        let response = GetDaemonStatusResponse {
            version: VERSION.to_string(),
            pid,
            start_time: Some(prost_types::Timestamp {
                seconds: self.start_time.timestamp(),
                nanos: self.start_time.timestamp_subsec_nanos() as i32,
            }),
            uptime_seconds,
            data_dir: self.config.paths.data_dir.to_string_lossy().to_string(),
            active_processes,
        };

        Ok(Response::new(response))
    }
}

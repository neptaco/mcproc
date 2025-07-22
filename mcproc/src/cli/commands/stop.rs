use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use clap::Args;
use colored::*;
use proto::StopProcessRequest;
use std::time::Duration;
use tonic::Request;

#[derive(Debug, Args)]
pub struct StopCommand {
    /// Process name or ID
    name: String,

    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,

    /// Force stop (SIGKILL)
    #[arg(short, long)]
    force: bool,
}

impl StopCommand {
    pub async fn execute(self, mut client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        let grpc_request = StopProcessRequest {
            name: self.name.clone(),
            force: Some(self.force),
            project: resolve_project_name(self.project)?,
        };

        // Load config to get timeout settings
        let config = crate::common::config::Config::load()?;
        // Set timeout based on config: process_stop_timeout + grpc_request_buffer
        let timeout = Duration::from_millis(
            config.process.restart.process_stop_timeout_ms
                + config.api.grpc_request_buffer_secs * 1000,
        );
        let mut request = Request::new(grpc_request);
        request.set_timeout(timeout);

        let response = client.inner().stop_process(request).await?;
        let result = response.into_inner();

        if result.success {
            println!(
                "{} Process '{}' stopped successfully",
                "✓".green(),
                self.name
            );
        } else {
            println!(
                "{} Failed to stop process '{}': {}",
                "✗".red(),
                self.name,
                result
                    .message
                    .unwrap_or_else(|| "Unknown error".to_string())
            );
        }

        Ok(())
    }
}

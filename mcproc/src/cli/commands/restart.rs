use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use clap::Args;
use colored::*;
use proto::RestartProcessRequest;
use std::time::Duration;
use tonic::Request;

#[derive(Debug, Args)]
pub struct RestartCommand {
    /// Process name or ID
    name: String,

    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,
}

impl RestartCommand {
    pub async fn execute(self, mut client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        let grpc_request = RestartProcessRequest {
            name: self.name.clone(),
            project: resolve_project_name(self.project)?,
            wait_for_log: None,
            wait_timeout: None,
        };

        println!("Restarting process '{}'...", self.name);

        // Load config to get timeout settings
        let config = crate::common::config::Config::load()?;
        // Set timeout based on config: process_stop_timeout + grpc_request_buffer
        // Restart needs more time: stop + start
        let timeout = Duration::from_millis(
            (config.process.restart.process_stop_timeout_ms * 2)
                + config.api.grpc_request_buffer_secs * 1000,
        );
        let mut request = Request::new(grpc_request);
        request.set_timeout(timeout);

        match client.inner().restart_process(request).await {
            Ok(response) => {
                let mut stream = response.into_inner();
                let mut process_info = None;

                // Process streaming responses
                while let Ok(Some(msg)) = stream.message().await {
                    match msg.response {
                        Some(proto::restart_process_response::Response::LogEntry(entry)) => {
                            // Print log entries as they arrive
                            println!("  {}", entry.content.dimmed());
                        }
                        Some(proto::restart_process_response::Response::Process(info)) => {
                            process_info = Some(info);
                        }
                        None => {}
                    }
                }

                let process = match process_info {
                    Some(p) => p,
                    None => {
                        println!("{} Failed to restart process", "✗".red());
                        return Err(
                            "No process info received - process may have failed to restart".into(),
                        );
                    }
                };

                // Check if process failed to restart
                if process.status == proto::ProcessStatus::Failed as i32 {
                    println!("{} Process '{}' failed to restart", "✗".red(), process.name);
                    if let (Some(exit_code), Some(exit_reason)) =
                        (process.exit_code, process.exit_reason)
                    {
                        println!("  Exit code: {}", exit_code);
                        println!("  Reason: {}", exit_reason);
                    }
                    if let Some(stderr) = process.stderr_tail {
                        if !stderr.is_empty() {
                            println!("  Recent logs:");
                            for line in stderr.lines() {
                                println!("    {}", line.dimmed());
                            }
                        }
                    }
                    return Err(format!(
                        "Process failed with exit code: {}",
                        process.exit_code.unwrap_or(-1)
                    )
                    .into());
                }

                println!("{} Process restarted successfully", "✓".green());
                println!("  Name: {}", process.name.bright_white());
                println!("  ID: {}", process.id);
                println!(
                    "  PID: {}",
                    process
                        .pid
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "N/A".to_string())
                );

                Ok(())
            }
            Err(e) => {
                println!("{} Failed to restart process: {}", "✗".red(), e);
                Err(e.into())
            }
        }
    }
}

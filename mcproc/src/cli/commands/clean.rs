use clap::Args;
use colored::Colorize;
use tonic::Request;

use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use proto::CleanProjectRequest;

/// Clean up processes and logs for projects
///
/// This command stops all processes and deletes log files for specified projects.
/// When verbose mode is enabled, it displays the names of stopped processes and
/// deleted log files.
#[derive(Args)]
pub struct CleanCommand {
    /// Clean all projects
    #[arg(long)]
    all_projects: bool,

    /// Project name to clean (defaults to current directory name)
    #[arg(short, long)]
    project: Option<String>,

    /// Verbose output (set from global flag)
    #[arg(skip)]
    pub verbose: bool,
}

impl CleanCommand {
    pub async fn execute(
        &self,
        mut client: DaemonClient,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Resolve project name from current directory if not specified
        let resolved_project = if self.all_projects {
            None
        } else {
            Some(resolve_project_name(self.project.clone())?)
        };

        let request = Request::new(CleanProjectRequest {
            project: resolved_project,
            all_projects: self.all_projects,
        });

        let response = client.inner().clean_project(request).await?;
        let response = response.into_inner();

        if self.all_projects {
            // Display results for all projects
            if response.project_results.is_empty() {
                println!("{}", "No projects to clean".dimmed());
            } else {
                println!("{}", "Cleaning all projects:".bold());
                let mut total_processes = 0;
                let mut total_logs = 0;

                for result in &response.project_results {
                    println!(
                        "  {} {}: {} processes stopped, {} logs deleted",
                        "•".cyan(),
                        result.project.bold(),
                        result.processes_stopped.to_string().yellow(),
                        result.logs_deleted.to_string().red()
                    );

                    if self.verbose {
                        if !result.stopped_process_names.is_empty() {
                            println!("      Stopped processes:");
                            for name in &result.stopped_process_names {
                                println!("        - {}", name.green());
                            }
                        }
                        if !result.deleted_log_files.is_empty() {
                            println!("      Deleted logs:");
                            for file in &result.deleted_log_files {
                                println!("        - {}", file.red());
                            }
                        }
                    }

                    total_processes += result.processes_stopped;
                    total_logs += result.logs_deleted;
                }

                println!("\n{}", "Summary:".bold());
                println!(
                    "  Total: {} processes stopped, {} logs deleted",
                    total_processes.to_string().yellow(),
                    total_logs.to_string().red()
                );
            }
        } else {
            // Display results for single project (use the resolved project name)
            let project_name = resolve_project_name(self.project.clone())?;

            if response.processes_stopped == 0 && response.logs_deleted == 0 {
                println!(
                    "{} {}",
                    "No processes or logs found for project:".dimmed(),
                    project_name.dimmed()
                );
            } else {
                println!(
                    "{} {}: {} processes stopped, {} logs deleted",
                    "Cleaned project".green(),
                    project_name.bold(),
                    response.processes_stopped.to_string().yellow(),
                    response.logs_deleted.to_string().red()
                );

                if self.verbose {
                    if !response.stopped_process_names.is_empty() {
                        println!("  Stopped processes:");
                        for name in &response.stopped_process_names {
                            println!("    - {}", name.green());
                        }
                    }
                    if !response.deleted_log_files.is_empty() {
                        println!("  Deleted logs:");
                        for file in &response.deleted_log_files {
                            println!("    - {}", file.red());
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

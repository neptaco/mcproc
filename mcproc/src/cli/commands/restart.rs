use crate::client::DaemonClient;
use crate::cli::utils::resolve_project_name_optional;
use clap::Args;
use colored::*;
use proto::RestartProcessRequest;

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
        let request = RestartProcessRequest {
            name: self.name.clone(),
            project: resolve_project_name_optional(self.project),
        };
        
        println!("Restarting process '{}'...", self.name);
        
        let response = client.inner().restart_process(request).await?;
        let process = response.into_inner().process.unwrap();
        
        println!("{} Process restarted successfully", "✓".green());
        println!("  Name: {}", process.name.bright_white());
        println!("  ID: {}", process.id);
        println!("  PID: {}", process.pid.map(|p| p.to_string()).unwrap_or_else(|| "N/A".to_string()));
        
        Ok(())
    }
}
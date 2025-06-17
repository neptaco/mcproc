use crate::client::McpClient;
use crate::cli::utils::resolve_project_name_optional;
use clap::Args;
use colored::*;
use proto::StopProcessRequest;

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
    pub async fn execute(self, mut client: McpClient) -> Result<(), Box<dyn std::error::Error>> {
        let request = StopProcessRequest {
            name: self.name.clone(),
            force: Some(self.force),
            project: resolve_project_name_optional(self.project),
        };
        
        let response = client.inner().stop_process(request).await?;
        let result = response.into_inner();
        
        if result.success {
            println!("{} Process '{}' stopped successfully", "✓".green(), self.name);
        } else {
            println!("{} Failed to stop process '{}': {}", 
                "✗".red(), 
                self.name,
                result.message.unwrap_or_else(|| "Unknown error".to_string())
            );
        }
        
        Ok(())
    }
}
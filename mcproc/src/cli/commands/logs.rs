use crate::client::DaemonClient;
use crate::cli::utils::resolve_project_name_optional;
use chrono;
use clap::Args;
use colored::*;
use proto::GetLogsRequest;
use tokio_stream::StreamExt;

#[derive(Debug, Args)]
pub struct LogsCommand {
    /// Process name
    name: String,
    
    /// Follow log output
    #[arg(short, long)]
    follow: bool,
    
    /// Number of lines to show from the end
    #[arg(short, long, default_value = "100")]
    tail: u32,
    
    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,
}

impl LogsCommand {
    pub async fn execute(self, mut client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        // Determine project name if not provided (use current working directory where mcproc is run)
        let project = resolve_project_name_optional(self.project);
        
        let request = GetLogsRequest {
            name: self.name.clone(),
            tail: Some(self.tail),
            follow: Some(self.follow),
            project,
        };
        
        let mut stream = client.inner().get_logs(request).await?.into_inner();
        
        while let Some(response) = stream.next().await {
            match response {
                Ok(logs_response) => {
                    // Display entries immediately
                    for entry in logs_response.entries {
                        print_log_entry(&entry);
                    }
                }
                Err(e) => {
                    eprintln!("{} Error receiving logs: {}", "✗".red(), e);
                    break;
                }
            }
        }
        
        Ok(())
    }
}

fn print_log_entry(entry: &proto::LogEntry) {
    let timestamp = entry.timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_else(chrono::Utc::now);
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_default();
    
    let level_indicator = match entry.level {
        2 => "E".red().bold(),
        _ => "I".green(),
    };
    
    println!("{} {} {}", 
        timestamp.dimmed(),
        level_indicator,
        entry.content
    );
}
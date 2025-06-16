use crate::client::McpClient;
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
    
    /// Line range (e.g., 100:200)
    #[arg(short, long)]
    range: Option<String>,
    
    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,
}

impl LogsCommand {
    pub async fn execute(self, mut client: McpClient) -> Result<(), Box<dyn std::error::Error>> {
        let (from_line, to_line) = if let Some(ref range) = self.range {
            parse_range(range)?
        } else {
            // For tail functionality, we'll get all lines and filter client-side
            (None, None)
        };
        
        let request = GetLogsRequest {
            name: self.name.clone(),
            from_line,
            to_line,
            follow: Some(self.follow),
            project: self.project,
        };
        
        let mut stream = client.inner().get_logs(request).await?.into_inner();
        
        // Collect all log entries if we need to apply tail
        let show_tail = !self.follow && self.range.is_none();
        let mut all_entries = Vec::new();
        
        while let Some(response) = stream.next().await {
            match response {
                Ok(logs_response) => {
                    if show_tail {
                        // Collect entries for tail processing
                        all_entries.extend(logs_response.entries);
                    } else {
                        // Display entries immediately
                        for entry in logs_response.entries {
                            print_log_entry(&entry);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Error receiving logs: {}", "✗".red(), e);
                    break;
                }
            }
        }
        
        // If we collected entries for tail, show only the last N
        if show_tail && !all_entries.is_empty() {
            let start_idx = all_entries.len().saturating_sub(self.tail as usize);
            for entry in &all_entries[start_idx..] {
                print_log_entry(entry);
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
        .unwrap_or_else(|| "".to_string());
    
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

fn parse_range(range: &str) -> Result<(Option<u32>, Option<u32>), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = range.split(':').collect();
    
    if parts.len() != 2 {
        return Err("Invalid range format. Use 'from:to'".into());
    }
    
    let from = if parts[0].is_empty() { None } else { Some(parts[0].parse()?) };
    let to = if parts[1].is_empty() { None } else { Some(parts[1].parse()?) };
    
    Ok((from, to))
}
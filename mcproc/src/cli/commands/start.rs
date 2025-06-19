use crate::client::McpClient;
use crate::cli::utils::resolve_project_name;
use clap::Args;
use colored::*;
use proto::StartProcessRequest;
use tonic::Request;
use std::time::Duration;

#[derive(Debug, Args)]
pub struct StartCommand {
    /// Process name
    name: String,
    
    /// Command to execute (use with shell)
    #[arg(short, long, conflicts_with = "args")]
    cmd: Option<String>,
    
    /// Command and arguments (direct execution)
    #[arg(short, long, conflicts_with = "cmd", num_args = 1..)]
    args: Option<Vec<String>>,
    
    /// Working directory
    #[arg(short = 'd', long)]
    cwd: Option<String>,
    
    /// Project name (defaults to directory name)
    #[arg(short, long)]
    project: Option<String>,
    
    /// Environment variables (KEY=VALUE)
    #[arg(short, long)]
    env: Vec<String>,
    
    /// Wait for this log pattern before considering the process ready (regex)
    #[arg(long)]
    wait_for_log: Option<String>,
    
    /// Timeout for log wait in seconds (default: 30)
    #[arg(long, default_value = "30")]
    wait_timeout: u32,
}

impl StartCommand {
    pub async fn execute(self, mut client: McpClient) -> Result<(), Box<dyn std::error::Error>> {
        let mut env_map = std::collections::HashMap::new();
        
        for env_str in self.env {
            if let Some((key, value)) = env_str.split_once('=') {
                env_map.insert(key.to_string(), value.to_string());
            }
        }
        
        // Check that either cmd or args is provided
        if self.cmd.is_none() && self.args.is_none() {
            return Err("Must provide either --cmd or --args".into());
        }
        
        // Determine project name if not provided (use current working directory where mcproc is run)
        let project = resolve_project_name(self.project);
        
        let grpc_request = StartProcessRequest {
            name: self.name.clone(),
            cmd: self.cmd,
            args: self.args.unwrap_or_default(),
            cwd: self.cwd,
            project: Some(project.clone()),
            env: env_map,
            wait_for_log: self.wait_for_log.clone(),
            wait_timeout: Some(self.wait_timeout),
        };
        
        // Set timeout to wait_timeout + 5 seconds to allow for process startup
        let timeout = Duration::from_secs((self.wait_timeout + 5) as u64);
        let mut request = Request::new(grpc_request);
        request.set_timeout(timeout);
        
        match client.inner().start_process(request).await {
            Ok(response) => {
                let mut stream = response.into_inner();
                let mut process_info = None;
                
                // Process streaming responses
                while let Ok(Some(msg)) = stream.message().await {
                    match msg.response {
                        Some(proto::start_process_response::Response::LogEntry(entry)) => {
                            // Print log entries as they arrive if wait_for_log is enabled
                            if self.wait_for_log.is_some() {
                                println!("  {}", entry.content.dimmed());
                            }
                        }
                        Some(proto::start_process_response::Response::Process(info)) => {
                            process_info = Some(info);
                        }
                        None => {}
                    }
                }
                
                let process = process_info.expect("No process info received");
                
                println!("{} Process started successfully", "✓".green());
                println!("  Project: {}", project.bright_white());
                println!("  Name: {}", process.name.bright_white());
                println!("  ID: {}", process.id);
                println!("  PID: {}", process.pid.map(|p| p.to_string()).unwrap_or_else(|| "N/A".to_string()));
                println!("  Status: {}", format_status(process.status));
                println!("  Log file: {}", process.log_file.dimmed());
                
                if self.wait_for_log.is_some() {
                    println!("  {} Process is ready (log pattern matched)", "✓".green());
                }
            }
            Err(e) => {
                if e.code() == tonic::Code::AlreadyExists {
                    println!("{} Process '{}' is already running", "!".yellow(), self.name);
                    println!("  Use 'mcproc ps' to see the running process");
                    println!("  Use 'mcproc restart {}' to restart it", self.name);
                } else {
                    println!("{} Failed to start process: {}", "✗".red(), e.message());
                }
                return Err(e.into());
            }
        }
        
        Ok(())
    }
}

fn format_status(status: i32) -> ColoredString {
    match status {
        1 => "Starting".yellow(),
        2 => "Running".green(),
        3 => "Stopping".yellow(),
        4 => "Stopped".red(),
        5 => "Failed".red().bold(),
        _ => "Unknown".white(),
    }
}
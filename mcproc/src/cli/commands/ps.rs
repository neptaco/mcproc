use crate::client::DaemonClient;
use crate::common::status::format_status;
use clap::Args;
use proto::ListProcessesRequest;
use tabled::{Table, Tabled};

#[derive(Debug, Args)]
pub struct PsCommand {
    /// Filter by status
    #[arg(short, long)]
    status: Option<String>,
}

#[derive(Tabled)]
struct ProcessRow {
    #[tabled(rename = "PROJECT")]
    project: String,
    
    #[tabled(rename = "NAME")]
    name: String,
    
    #[tabled(rename = "PID")]
    pid: String,
    
    #[tabled(rename = "STATUS")]
    status: String,
    
    #[tabled(rename = "PORTS")]
    ports: String,
    
    #[tabled(rename = "COMMAND")]
    cmd: String,
}

impl PsCommand {
    pub async fn execute(self, mut client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        let request = ListProcessesRequest {
            status_filter: None, // TODO: Parse status filter
            project_filter: None,
        };
        
        let response = client.inner().list_processes(request).await?;
        let processes = response.into_inner().processes;
        
        if processes.is_empty() {
            println!("No processes running");
            return Ok(());
        }
        
        let rows: Vec<ProcessRow> = processes.into_iter().map(|p| {
            ProcessRow {
                project: p.project,
                name: p.name,
                pid: p.pid.map(|pid| pid.to_string()).unwrap_or_else(|| "-".to_string()),
                status: format_status(p.status),
                ports: format_ports(&p.ports),
                cmd: truncate(&p.cmd, 40),
            }
        }).collect();
        
        let table = Table::new(rows);
        println!("{}", table);
        
        Ok(())
    }
}


fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len-3])
    }
}

fn format_ports(ports: &[u32]) -> String {
    if ports.is_empty() {
        "-".to_string()
    } else {
        ports.iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
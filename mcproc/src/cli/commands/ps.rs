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
            status_filter: self.status.as_deref().map(parse_status).transpose()?,
            project_filter: None,
        };

        let response = client.inner().list_processes(request).await?;
        let processes = response.into_inner().processes;

        if processes.is_empty() {
            println!("No processes running");
            return Ok(());
        }

        let rows: Vec<ProcessRow> = processes
            .into_iter()
            .map(|p| ProcessRow {
                project: p.project,
                name: p.name,
                pid: p
                    .pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                status: format_status(p.status),
                ports: format_ports(&p.ports),
                cmd: truncate(&p.cmd, 40),
            })
            .collect();

        let table = Table::new(rows);
        println!("{}", table);

        Ok(())
    }
}

fn parse_status(status: &str) -> Result<i32, String> {
    let status = match status.to_ascii_lowercase().as_str() {
        "unknown" => proto::ProcessStatus::Unknown,
        "starting" => proto::ProcessStatus::Starting,
        "running" => proto::ProcessStatus::Running,
        "stopping" => proto::ProcessStatus::Stopping,
        "stopped" => proto::ProcessStatus::Stopped,
        "failed" => proto::ProcessStatus::Failed,
        _ => return Err(format!("Invalid process status: {status}")),
    };
    Ok(status as i32)
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        ".".repeat(max_len)
    } else {
        let cutoff = s
            .char_indices()
            .nth(max_len - 3)
            .map_or(s.len(), |(index, _)| index);
        format!("{}...", &s[..cutoff])
    }
}

fn format_ports(ports: &[u32]) -> String {
    if ports.is_empty() {
        "-".to_string()
    } else {
        ports
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_handles_multibyte_characters() {
        let truncated = truncate("日本語のとても長いコマンドライン文字列テスト", 10);

        assert_eq!(truncated.chars().count(), 10);
        assert!(truncated.ends_with("..."));
    }
}

use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use chrono;
use clap::Args;
use colored::*;
use proto::{GetLogsRequest, ListProcessesRequest};
use strip_ansi_escapes::strip;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

#[derive(Debug, Clone)]
struct ColorOptions {
    raw_color: bool,
    no_color: bool,
    smart_color: bool,
}

#[derive(Debug, Args)]
pub struct LogsCommand {
    /// Process name (omit to show logs from all processes)
    name: Option<String>,

    /// Follow log output
    #[arg(short, long)]
    follow: bool,

    /// Number of lines to show from the end
    #[arg(short, long, default_value = "100")]
    tail: u32,

    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,

    /// Show process output colors only (disable mcproc colors)
    #[arg(long)]
    raw_color: bool,

    /// Disable all colors
    #[arg(long)]
    no_color: bool,

    /// Smart color mode: auto-adjust colors based on process output
    #[arg(long)]
    smart_color: bool,
}

impl LogsCommand {
    pub async fn execute(
        mut self,
        mut client: DaemonClient,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check NO_COLOR environment variable
        if std::env::var("NO_COLOR").is_ok() {
            self.no_color = true;
        }

        let color_opts = ColorOptions {
            raw_color: self.raw_color,
            no_color: self.no_color,
            smart_color: self.smart_color,
        };

        // Determine project name if not provided (use current working directory where mcproc is run)
        let project = resolve_project_name(self.project)?;

        match self.name {
            Some(name) => {
                // Single process logs
                let request = GetLogsRequest {
                    name: name.clone(),
                    tail: Some(self.tail),
                    follow: Some(self.follow),
                    project: project.clone(),
                };

                let mut stream = client.inner().get_logs(request).await?.into_inner();

                while let Some(response) = stream.next().await {
                    match response {
                        Ok(logs_response) => {
                            // Display entries immediately
                            for entry in logs_response.entries {
                                print_log_entry(&entry, &color_opts);
                            }
                        }
                        Err(e) => {
                            eprintln!("{} Error receiving logs: {}", "✗".red(), e);
                            break;
                        }
                    }
                }
            }
            None => {
                // All processes in project
                // Get list of processes in the current project
                let list_request = ListProcessesRequest {
                    project_filter: Some(project.clone()),
                    status_filter: None,
                };

                let response = client.inner().list_processes(list_request).await?;
                let processes = response.into_inner().processes;

                if processes.is_empty() && !self.follow {
                    eprintln!("{} No processes found in project", "✗".red());
                    return Ok(());
                }

                if !self.follow {
                    // For non-follow mode, get and merge all logs
                    let mut all_entries = Vec::new();

                    for process in &processes {
                        let request = GetLogsRequest {
                            name: process.name.clone(),
                            tail: Some(self.tail),
                            follow: Some(false),
                            project: project.clone(),
                        };

                        let mut stream = client.inner().get_logs(request).await?.into_inner();

                        while let Some(response) = stream.next().await {
                            if let Ok(logs_response) = response {
                                for mut entry in logs_response.entries {
                                    // Add process name to the entry
                                    entry.process_name = Some(process.name.clone());
                                    all_entries.push(entry);
                                }
                            }
                        }
                    }

                    // Sort by timestamp
                    all_entries.sort_by(|a, b| {
                        let a_ts = a
                            .timestamp
                            .as_ref()
                            .map(|t| (t.seconds, t.nanos))
                            .unwrap_or((0, 0));
                        let b_ts = b
                            .timestamp
                            .as_ref()
                            .map(|t| (t.seconds, t.nanos))
                            .unwrap_or((0, 0));
                        a_ts.cmp(&b_ts)
                    });

                    // Take only the last N entries
                    let start = all_entries.len().saturating_sub(self.tail as usize);
                    for entry in &all_entries[start..] {
                        print_log_entry_with_process(entry, &color_opts);
                    }
                } else {
                    // For follow mode, stream from all processes concurrently
                    // Calculate max name length for padding
                    let max_name_len = processes
                        .iter()
                        .map(|p| p.name.len())
                        .max()
                        .unwrap_or(10)
                        .max(10); // Minimum 10 characters

                    stream_multiple_logs(
                        client,
                        processes,
                        Some(project),
                        self.tail,
                        max_name_len,
                        &color_opts,
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }
}

fn contains_ansi_escape(text: &str) -> bool {
    text.contains("\x1b[")
}

fn strip_ansi_escapes_str(text: &str) -> String {
    let stripped = strip(text);
    String::from_utf8_lossy(&stripped).to_string()
}

fn print_log_entry(entry: &proto::LogEntry, color_opts: &ColorOptions) {
    let timestamp = entry
        .timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_else(chrono::Utc::now);
            let local_dt: chrono::DateTime<chrono::Local> = dt.into();
            local_dt.format("%H:%M:%S").to_string()
        })
        .unwrap_or_default();

    let content = if color_opts.no_color {
        strip_ansi_escapes_str(&entry.content)
    } else {
        entry.content.clone()
    };

    if color_opts.raw_color {
        // Raw color mode: no mcproc colors, just content
        println!("{}", content);
    } else if color_opts.no_color {
        // No color mode: plain text
        println!(
            "{} {} {}",
            timestamp,
            if entry.level == 2 { "E" } else { "I" },
            content
        );
    } else if color_opts.smart_color && contains_ansi_escape(&entry.content) {
        // Smart color mode: minimal mcproc colors when content has colors
        println!(
            "{} {} {}",
            timestamp.dimmed(),
            if entry.level == 2 { "E" } else { "I" },
            content
        );
    } else {
        // Default mode: full mcproc colors
        let level_indicator = match entry.level {
            2 => "E".red().bold(),
            _ => "I".dimmed(),
        };
        println!("{} {} {}", timestamp.dimmed(), level_indicator, content);
    }
}

fn print_log_entry_with_process(entry: &proto::LogEntry, color_opts: &ColorOptions) {
    print_log_entry_with_process_padded(entry, 10, color_opts);
}

fn print_log_entry_with_process_padded(
    entry: &proto::LogEntry,
    max_name_len: usize,
    color_opts: &ColorOptions,
) {
    let timestamp = entry
        .timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_else(chrono::Utc::now);
            let local_dt: chrono::DateTime<chrono::Local> = dt.into();
            local_dt.format("%H:%M:%S").to_string()
        })
        .unwrap_or_default();

    let process_name = entry.process_name.as_deref().unwrap_or("unknown");
    let padded_name = format!("{:width$}", process_name, width = max_name_len);

    let content = if color_opts.no_color {
        strip_ansi_escapes_str(&entry.content)
    } else {
        entry.content.clone()
    };

    if color_opts.raw_color {
        // Raw color mode: minimal formatting, preserve content colors
        println!("{} {} | {}", timestamp, padded_name, content);
    } else if color_opts.no_color {
        // No color mode: plain text
        println!(
            "{} {} | {} {}",
            timestamp,
            padded_name,
            if entry.level == 2 { "E" } else { "I" },
            content
        );
    } else if color_opts.smart_color && contains_ansi_escape(&entry.content) {
        // Smart color mode: reduced mcproc colors when content has colors
        let colored_padded_name = match process_name
            .chars()
            .fold(0u8, |acc, c| acc.wrapping_add(c as u8))
            % 5
        {
            0 => padded_name.green(),
            1 => padded_name.blue(),
            2 => padded_name.cyan(),
            3 => padded_name.magenta(),
            _ => padded_name.bright_blue(),
        };

        println!(
            "{} {} | {} {}",
            timestamp.dimmed(),
            colored_padded_name,
            if entry.level == 2 { "E" } else { "I" },
            content
        );
    } else {
        // Default mode: full mcproc colors
        let level_indicator = match entry.level {
            2 => "E".red().bold(),
            _ => "I".dimmed(),
        };

        let colored_padded_name = match process_name
            .chars()
            .fold(0u8, |acc, c| acc.wrapping_add(c as u8))
            % 5
        {
            0 => padded_name.green(),
            1 => padded_name.blue(),
            2 => padded_name.cyan(),
            3 => padded_name.magenta(),
            _ => padded_name.bright_blue(),
        };

        println!(
            "{} {} | {} {}",
            timestamp.dimmed(),
            colored_padded_name.bold(),
            level_indicator,
            content
        );
    }
}

async fn stream_multiple_logs(
    client: DaemonClient,
    initial_processes: Vec<proto::ProcessInfo>,
    project: Option<String>,
    tail: u32,
    max_name_len: usize,
    color_opts: &ColorOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    // Create a channel to collect log entries
    let (tx, mut rx) = mpsc::channel::<proto::LogEntry>(100);

    // Keep track of processes we're monitoring
    let mut monitored_processes: HashSet<String> = HashSet::new();
    let mut tasks = JoinSet::new();

    // Shared max name length that can be updated
    let shared_max_name_len = Arc::new(Mutex::new(max_name_len));

    // Spawn initial log streaming tasks
    for process in initial_processes {
        spawn_log_stream_task(&mut tasks, &tx, &client, &project, &process, tail);
        monitored_processes.insert(process.id.clone());
    }

    // If no initial processes, show waiting message
    if monitored_processes.is_empty() {
        match &project {
            Some(p) => {
                eprintln!(
                    "{} Waiting for processes to start in project: {}",
                    "→".yellow(),
                    p.cyan().bold()
                );
            }
            None => {
                eprintln!(
                    "{} Waiting for processes to start (no project context)",
                    "→".yellow()
                );
            }
        }
    }

    // Clone for the monitoring task
    let tx_monitor = tx.clone();
    let mut client_monitor = client.clone();
    let project_monitor = project.clone();
    let shared_max_name_len_monitor = shared_max_name_len.clone();

    // Spawn a task to periodically check for new processes
    tasks.spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
        let mut local_monitored = monitored_processes.clone();

        loop {
            interval.tick().await;

            // Get current processes
            let list_request = ListProcessesRequest {
                project_filter: project_monitor.clone(),
                status_filter: None,
            };

            match client_monitor.inner().list_processes(list_request).await {
                Ok(response) => {
                    let current_processes = response.into_inner().processes;

                    // Check for new processes
                    for process in current_processes {
                        if !local_monitored.contains(&process.id) {
                            eprintln!(
                                "{} New process started: {}",
                                "→".green(),
                                process.name.green().bold()
                            );

                            // Update max name length if needed
                            {
                                let mut max_len = shared_max_name_len_monitor.lock().unwrap();
                                if process.name.len() > *max_len {
                                    *max_len = process.name.len();
                                }
                            }

                            // Spawn new log stream task
                            let mut client_clone = client_monitor.clone();
                            let tx_clone = tx_monitor.clone();
                            let project_clone = project_monitor.clone();
                            let process_name = process.name.clone();
                            let process_id = process.id.clone();

                            tokio::spawn(async move {
                                let request = GetLogsRequest {
                                    name: process_name.clone(),
                                    tail: Some(tail),
                                    follow: Some(true),
                                    project: project_clone.unwrap_or_else(|| "default".to_string()),
                                };

                                match client_clone.inner().get_logs(request).await {
                                    Ok(stream) => {
                                        let mut stream = stream.into_inner();

                                        while let Some(response) = stream.next().await {
                                            if let Ok(logs_response) = response {
                                                for mut entry in logs_response.entries {
                                                    entry.process_name = Some(process_name.clone());
                                                    if tx_clone.send(entry).await.is_err() {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "{} Error streaming logs for {}: {}",
                                            "✗".red(),
                                            process_name,
                                            e
                                        );
                                    }
                                }
                            });

                            local_monitored.insert(process_id);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Error checking for new processes: {}", "✗".red(), e);
                }
            }
        }
    });

    // Drop the original sender so the channel closes when all tasks complete
    drop(tx);

    // Print logs as they arrive
    let shared_max_name_len_print = shared_max_name_len.clone();
    while let Some(entry) = rx.recv().await {
        let current_max_len = *shared_max_name_len_print.lock().unwrap();
        print_log_entry_with_process_padded(&entry, current_max_len, color_opts);
    }

    // Wait for all tasks to complete
    while tasks.join_next().await.is_some() {}

    Ok(())
}

fn spawn_log_stream_task(
    tasks: &mut JoinSet<()>,
    tx: &mpsc::Sender<proto::LogEntry>,
    client: &DaemonClient,
    project: &Option<String>,
    process: &proto::ProcessInfo,
    tail: u32,
) {
    let mut client_clone = client.clone();
    let tx_clone = tx.clone();
    let project_clone = project.clone();
    let process_name = process.name.clone();

    tasks.spawn(async move {
        let request = GetLogsRequest {
            name: process_name.clone(),
            tail: Some(tail),
            follow: Some(true),
            project: project_clone.unwrap_or_else(|| "default".to_string()),
        };

        match client_clone.inner().get_logs(request).await {
            Ok(stream) => {
                let mut stream = stream.into_inner();

                while let Some(response) = stream.next().await {
                    if let Ok(logs_response) = response {
                        for mut entry in logs_response.entries {
                            entry.process_name = Some(process_name.clone());
                            if tx_clone.send(entry).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "{} Error streaming logs for {}: {}",
                    "✗".red(),
                    process_name,
                    e
                );
            }
        }
    });
}

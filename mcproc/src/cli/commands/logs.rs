use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use chrono;
use clap::Args;
use colored::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use strip_ansi_escapes::strip;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::StreamExt;

/// Target specification for log streaming
#[derive(Debug, Clone)]
enum LogTarget {
    /// Multiple specific processes (including single process as vec with one element)
    Multiple { names: Vec<String>, project: String },
    /// All processes in a project (wildcard)
    All { project: String },
}

#[derive(Debug, Clone)]
struct ColorOptions {
    raw_color: bool,
    no_color: bool,
    smart_color: bool,
}

#[derive(Debug, Args)]
pub struct LogsCommand {
    /// Process names to monitor (omit to show logs from all processes)
    #[arg(help = "Process names to monitor (e.g., 'frontend backend' or leave empty for all)")]
    names: Vec<String>,

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
    pub async fn execute(mut self, client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        // Check NO_COLOR environment variable
        if std::env::var("NO_COLOR").is_ok() {
            self.no_color = true;
        }

        let color_opts = ColorOptions {
            raw_color: self.raw_color,
            no_color: self.no_color,
            smart_color: self.smart_color,
        };

        // Determine project name
        let project = resolve_project_name(self.project.clone())?;

        // Create shutdown flag
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        // Set up Ctrl+C handler
        let shutdown_flag_ctrl_c = shutdown_flag.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            shutdown_flag_ctrl_c.store(true, Ordering::Relaxed);
        });

        // Determine log target
        let target = if self.names.is_empty() {
            LogTarget::All {
                project: project.clone(),
            }
        } else {
            LogTarget::Multiple {
                names: self.names.clone(),
                project: project.clone(),
            }
        };

        // Start streaming
        self.stream_logs(client, shutdown_flag, target, color_opts)
            .await
    }

    async fn stream_logs(
        &self,
        mut client: DaemonClient,
        shutdown_flag: Arc<AtomicBool>,
        target: LogTarget,
        color_opts: ColorOptions,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Create channel for log entries
        let (tx, mut rx) = mpsc::channel::<proto::LogEntry>(100);
        let mut tasks: JoinSet<()> = JoinSet::new();

        // Unified streaming for all target types
        let process_names = match &target {
            LogTarget::Multiple { names, .. } => names.clone(),
            LogTarget::All { .. } => vec![], // Empty vec means all processes
        };

        let project = match &target {
            LogTarget::Multiple { project, .. } => project.clone(),
            LogTarget::All { project } => project.clone(),
        };

        // Display what we're following
        if self.follow {
            match &target {
                LogTarget::Multiple { names, project } => {
                    eprintln!(
                        "{} Following logs for processes: {} in project: {}",
                        "→".yellow(),
                        names.join(", ").cyan().bold(),
                        project.cyan().bold()
                    );
                }
                LogTarget::All { project } => {
                    eprintln!(
                        "{} Following logs for all processes in project: {}",
                        "→".yellow(),
                        project.cyan().bold()
                    );
                }
            }
        }

        // Spawn unified gRPC stream task
        let tx_clone = tx.clone();
        let shutdown_flag_clone = shutdown_flag.clone();
        let tail = self.tail;
        let follow = self.follow;
        tasks.spawn(async move {
            let request = proto::GetLogsRequest {
                process_names,
                tail: Some(tail),
                follow: Some(follow),
                project,
                include_events: Some(false),
            };

            // Start single gRPC stream
            match client.inner().get_logs(request).await {
                Ok(response) => {
                    let mut stream = response.into_inner();

                    // Use select! to handle shutdown signals immediately
                    loop {
                        tokio::select! {
                            // Wait for next stream message with timeout
                            stream_result = tokio::time::timeout(
                                tokio::time::Duration::from_secs(30),
                                stream.next()
                            ) => {
                                match stream_result {
                                    Ok(Some(response)) => {
                                        match response {
                                            Ok(logs_response) => {
                                                if let Some(content) = logs_response.content {
                                                    match content {
                                                        proto::get_logs_response::Content::LogEntry(entry) => {
                                                            if tx_clone.send(entry).await.is_err() {
                                                                return; // Channel closed
                                                            }
                                                        }
                                                        proto::get_logs_response::Content::Event(_) => {
                                                            // Ignore events in logs command
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("{} Error receiving logs: {}", "✗".red(), e);
                                                return;
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        // Stream ended normally
                                        return;
                                    }
                                    Err(_) => {
                                        // Timeout occurred - check shutdown flag and continue
                                        if shutdown_flag_clone.load(Ordering::Relaxed) {
                                            return;
                                        }
                                        continue;
                                    }
                                }
                            }
                            // Check for shutdown signal every 100ms
                            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                                if shutdown_flag_clone.load(Ordering::Relaxed) {
                                    return;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to start log stream: {}", "✗".red(), e);
                }
            }
        });

        // Drop original tx to ensure channel closes when task completes
        drop(tx);

        // Process and display logs with shutdown handling
        loop {
            tokio::select! {
                entry = rx.recv() => {
                    match entry {
                        Some(entry) => {
                            print_log_entry(&entry, &color_opts);
                        }
                        None => {
                            // Channel closed, all streams finished
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    if shutdown_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
        }

        // Wait for all tasks to complete
        while tasks.join_next().await.is_some() {}

        Ok(())
    }
}

// Helper functions

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

    // Format based on whether we have a process name
    if let Some(process_name) = &entry.process_name {
        print_log_entry_with_process(entry, process_name, &timestamp, &content, color_opts);
    } else {
        print_log_entry_simple(&timestamp, &content, entry.level, color_opts);
    }
}

fn print_log_entry_simple(timestamp: &str, content: &str, level: i32, color_opts: &ColorOptions) {
    if color_opts.raw_color {
        println!("{}", content);
    } else if color_opts.no_color {
        println!(
            "{} {} {}",
            timestamp,
            if level == 2 { "E" } else { "I" },
            content
        );
    } else if color_opts.smart_color && contains_ansi_escape(content) {
        println!(
            "{} {} {}",
            timestamp.dimmed(),
            if level == 2 { "E" } else { "I" },
            content
        );
    } else {
        let level_indicator = match level {
            2 => "E".red().bold(),
            _ => "I".dimmed(),
        };
        println!("{} {} {}", timestamp.dimmed(), level_indicator, content);
    }
}

fn print_log_entry_with_process(
    entry: &proto::LogEntry,
    process_name: &str,
    timestamp: &str,
    content: &str,
    color_opts: &ColorOptions,
) {
    let padded_name = format!("{:15}", process_name);

    if color_opts.raw_color {
        println!("{} {} | {}", timestamp, padded_name, content);
    } else if color_opts.no_color {
        println!(
            "{} {} | {} {}",
            timestamp,
            padded_name,
            if entry.level == 2 { "E" } else { "I" },
            content
        );
    } else {
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

        if color_opts.smart_color && contains_ansi_escape(content) {
            println!(
                "{} {} | {} {}",
                timestamp.dimmed(),
                colored_padded_name,
                if entry.level == 2 { "E" } else { "I" },
                content
            );
        } else {
            let level_indicator = match entry.level {
                2 => "E".red().bold(),
                _ => "I".dimmed(),
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
}

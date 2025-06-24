use crate::cli::utils::resolve_project_name;
use crate::client::DaemonClient;
use chrono;
use clap::Args;
use colored::*;
use proto::GrepLogsRequest;

#[derive(Debug, Args)]
pub struct GrepCommand {
    /// Process name
    name: String,

    /// Pattern to search for
    pattern: String,

    /// Project name (optional, helps disambiguate)
    #[arg(short, long)]
    project: Option<String>,

    /// Lines of context around matches (before and after)
    #[arg(short = 'C', long, default_value = "3")]
    context: u32,

    /// Lines of context before matches
    #[arg(short = 'B', long)]
    before: Option<u32>,

    /// Lines of context after matches
    #[arg(short = 'A', long)]
    after: Option<u32>,

    /// Show logs since this time (e.g., "2025-06-17 10:30", "10:30")
    #[arg(long)]
    since: Option<String>,

    /// Show logs until this time (e.g., "2025-06-17 12:00", "12:00")
    #[arg(long)]
    until: Option<String>,

    /// Show logs from the last duration (e.g., "1h", "30m", "2d")
    #[arg(long)]
    last: Option<String>,
}

impl GrepCommand {
    pub async fn execute(self, mut client: DaemonClient) -> Result<(), Box<dyn std::error::Error>> {
        // Determine project name if not provided (should always succeed)
        let project = resolve_project_name(self.project)?;

        let request = GrepLogsRequest {
            name: self.name.clone(),
            pattern: self.pattern.clone(),
            project,
            context: Some(self.context),
            before: self.before,
            after: self.after,
            since: self.since,
            until: self.until,
            last: self.last,
        };

        match client.inner().grep_logs(request).await {
            Ok(response) => {
                let grep_response = response.into_inner();

                if grep_response.matches.is_empty() {
                    println!(
                        "{} No matches found for pattern: {}",
                        "!".yellow(),
                        self.pattern.bright_white()
                    );
                    return Ok(());
                }

                println!(
                    "{} Found {} matches for pattern: {}",
                    "✓".green(),
                    grep_response.matches.len(),
                    self.pattern.bright_white()
                );
                println!();

                for (match_idx, grep_match) in grep_response.matches.iter().enumerate() {
                    if match_idx > 0 {
                        println!("{}", "--".dimmed());
                    }

                    // Print context before
                    for entry in &grep_match.context_before {
                        print_log_entry(entry, false);
                    }

                    // Print matched line (highlighted)
                    if let Some(ref matched_line) = grep_match.matched_line {
                        print_log_entry(matched_line, true);
                    }

                    // Print context after
                    for entry in &grep_match.context_after {
                        print_log_entry(entry, false);
                    }
                }
            }
            Err(e) => {
                println!("{} Failed to grep logs: {}", "✗".red(), e.message());
                return Err(e.into());
            }
        }

        Ok(())
    }
}

fn print_log_entry(entry: &proto::LogEntry, is_match: bool) {
    let timestamp = entry
        .timestamp
        .as_ref()
        .map(|ts| {
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                .unwrap_or_else(chrono::Utc::now);
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        })
        .unwrap_or_default();

    let level_indicator = match entry.level {
        2 => "E".red().bold(),
        _ => "I".dimmed(),
    };

    let line_number = format!("{:>6}", entry.line_number);

    if is_match {
        // Highlight the entire matched line
        println!(
            "{} {} {} {}",
            line_number.bright_yellow(),
            timestamp.dimmed(),
            level_indicator,
            entry.content.on_bright_black()
        );
    } else {
        // Normal context line
        println!(
            "{} {} {} {}",
            line_number.dimmed(),
            timestamp.dimmed(),
            level_indicator,
            entry.content
        );
    }
}

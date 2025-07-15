use super::helpers::create_timestamp;
use super::service::GrpcService;
use crate::daemon::stream::{StreamEvent, StreamFilter};
use proto::process_manager_server::ProcessManager as ProcessManagerService;
use proto::*;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

impl GrpcService {
    pub(super) async fn get_logs_impl(
        &self,
        request: Request<GetLogsRequest>,
    ) -> Result<Response<<Self as ProcessManagerService>::GetLogsStream>, Status> {
        let req = request.into_inner();

        let project = req.project.clone();
        let process_names = req.process_names.clone();
        let tail = req.tail.unwrap_or(100) as usize;
        let follow = req.follow.unwrap_or(false);
        let include_events = req.include_events.unwrap_or(false);

        // Log the request details for debugging
        info!(
            "get_logs request: project={}, process_names={:?}, tail={}, follow={}, include_events={}",
            project, process_names, tail, follow, include_events
        );

        // Create filter based on request
        let filter = StreamFilter {
            project: Some(project.clone()),
            process_names: process_names.clone(),
            include_events,
        };

        // Subscribe to event hub
        let mut event_receiver = self.event_hub.subscribe();

        // For tail functionality, read existing logs from files first
        let log_hub = self.log_hub.clone();
        let process_manager = self.process_manager.clone();

        // Create stream
        let stream = async_stream::try_stream! {
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::fs::File;
            use crate::common::process_key::ProcessKey;

            // First, send existing logs if tail is requested
            if tail > 0 {
                info!("Reading tail logs (tail={})", tail);
                // Get list of processes matching the filter
                let processes = process_manager.list_processes();
                info!("Found {} total processes", processes.len());
                let matching_processes: Vec<_> = processes
                    .into_iter()
                    .filter(|p| filter.matches_process(&p.project, &p.name))
                    .collect();
                info!("Found {} matching processes", matching_processes.len());

                // Read tail lines from each matching process's log file
                for process_info in matching_processes {
                    let key = ProcessKey {
                        name: process_info.name.clone(),
                        project: process_info.project.clone(),
                    };
                    let log_file = log_hub.get_log_file_path_for_key(&key);

                    if log_file.exists() {
                        match File::open(&log_file).await {
                            Ok(file) => {
                                let reader = BufReader::new(file);
                                let mut lines = reader.lines();
                                let mut all_lines = Vec::new();

                                // Read all lines
                                while let Ok(Some(line)) = lines.next_line().await {
                                    all_lines.push(line);
                                }

                                // Get tail lines
                                let start_idx = all_lines.len().saturating_sub(tail);
                                let mut line_num = start_idx as u32;

                                for line in &all_lines[start_idx..] {
                                    line_num += 1;
                                    let (timestamp, level, content) = parse_log_line(line);

                                    let log_entry = LogEntry {
                                        line_number: line_num,
                                        content,
                                        timestamp,
                                        level: level as i32,
                                        process_name: Some(process_info.name.clone()),
                                    };

                                    yield GetLogsResponse {
                                        content: Some(proto::get_logs_response::Content::LogEntry(log_entry)),
                                    };
                                }
                            }
                            Err(e) => {
                                error!("Failed to open log file for {}/{}: {}",
                                    process_info.project, process_info.name, e);
                            }
                        }
                    }
                }
            }

            // If follow mode, subscribe to event hub for new logs
            if follow {
                info!("Starting follow mode for filter: project={:?}, process_names={:?}",
                    filter.project, filter.process_names);
                loop {
                    tokio::select! {
                        event = event_receiver.recv() => {
                            match event {
                                Ok(stream_event) => {
                                    // Check if event matches filter
                                    if !filter.matches(&stream_event) {
                                        continue;
                                    }
                                    debug!("Received matching event: {:?}", stream_event);

                                    match stream_event {
                                        StreamEvent::Log { process_name, entry, .. } => {
                                            // Set process_name in log entry
                                            let mut log_entry = entry;
                                            log_entry.process_name = Some(process_name);

                                            yield GetLogsResponse {
                                                content: Some(proto::get_logs_response::Content::LogEntry(log_entry)),
                                            };
                                        }
                                        StreamEvent::Process(event) => {
                                            if include_events {
                                                // Convert ProcessEvent to ProcessLifecycleEvent
                                                let lifecycle_event = match event {
                                                    crate::daemon::process::event::ProcessEvent::Starting { process_id, name, project } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Starting as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: create_timestamp(chrono::Utc::now()),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Started { process_id, name, project, pid } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Started as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: Some(pid),
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: create_timestamp(chrono::Utc::now()),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Stopping { process_id, name, project } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Stopping as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: None,
                                                            timestamp: create_timestamp(chrono::Utc::now()),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Stopped { process_id, name, project, exit_code } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Stopped as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code,
                                                            error: None,
                                                            timestamp: create_timestamp(chrono::Utc::now()),
                                                        }
                                                    }
                                                    crate::daemon::process::event::ProcessEvent::Failed { process_id, name, project, error } => {
                                                        ProcessLifecycleEvent {
                                                            event_type: proto::process_lifecycle_event::EventType::Failed as i32,
                                                            process_id,
                                                            name,
                                                            project,
                                                            pid: None,
                                                            exit_code: None,
                                                            error: Some(error),
                                                            timestamp: create_timestamp(chrono::Utc::now()),
                                                        }
                                                    }
                                                };

                                                yield GetLogsResponse {
                                                    content: Some(proto::get_logs_response::Content::Event(lifecycle_event)),
                                                };
                                            }
                                        }
                                    }
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                                    // We missed some events due to lag
                                    error!("Event receiver lagged by {} events", count);
                                    // Continue processing
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    // Event hub closed, exit
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

// Helper function to parse log lines
fn parse_log_line(line: &str) -> (Option<prost_types::Timestamp>, log_entry::LogLevel, String) {
    // Expected format: "2025-07-15T03:13:12.375+00:00 [INFO] Log message"
    // Find the first space (after the timestamp)

    if let Some(space_pos) = line.find(' ') {
        let timestamp_str = &line[..space_pos];
        let rest = &line[space_pos + 1..];

        // Parse RFC 3339 timestamp directly
        let timestamp = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp_str) {
            let dt_utc = dt.with_timezone(&chrono::Utc);
            Some(prost_types::Timestamp {
                seconds: dt_utc.timestamp(),
                nanos: dt_utc.timestamp_subsec_nanos() as i32,
            })
        } else {
            None
        };

        // Parse level and content
        if let Some(content) = rest.strip_prefix("[ERROR] ") {
            return (timestamp, log_entry::LogLevel::Stderr, content.to_string());
        } else if let Some(content) = rest.strip_prefix("[INFO] ") {
            return (timestamp, log_entry::LogLevel::Stdout, content.to_string());
        }

        // If we have a timestamp but no recognized level, return with default level
        (timestamp, log_entry::LogLevel::Stdout, rest.to_string())
    } else {
        // No space found, treat entire line as content
        (None, log_entry::LogLevel::Stdout, line.to_string())
    }
}

impl GrpcService {
    pub(super) async fn grep_logs_impl(
        &self,
        request: Request<GrepLogsRequest>,
    ) -> Result<Response<GrepLogsResponse>, Status> {
        let req = request.into_inner();

        // Construct the log file path
        let project = req.project.clone();

        // Use project-based directory structure
        let log_file = self
            .log_hub
            .config
            .paths
            .log_dir
            .join(&project)
            .join(format!("{}.log", req.name));

        // Try to get process first for memory-based search
        let process = self
            .process_manager
            .get_process_by_name_or_id_with_project(&req.name, Some(&req.project));

        // If file doesn't exist and process doesn't exist, return error
        if !log_file.exists() && process.is_none() {
            return Err(Status::not_found(format!(
                "Process '{}' not found in project '{}'",
                req.name, req.project
            )));
        }

        // Parse time filters
        let (since_time, until_time) =
            parse_time_filters(&req.since, &req.until, &req.last).map_err(|e| *e)?;

        // Compile regex pattern
        let pattern = regex::Regex::new(&req.pattern)
            .map_err(|e| Status::invalid_argument(format!("Invalid regex pattern: {}", e)))?;

        // Determine context settings
        let context = req.context.unwrap_or(3) as usize;
        let before = req.before.map(|b| b as usize).unwrap_or(context);
        let after = req.after.map(|a| a as usize).unwrap_or(context);

        // Read and process logs (from file or memory)
        let matches = if log_file.exists() {
            // Read from file
            info!(
                "grep_logs: Using file-based search for {}",
                log_file.display()
            );
            grep_log_file(&log_file, &pattern, before, after, since_time, until_time)
                .await
                .map_err(|e| Status::internal(format!("Failed to grep log file: {}", e)))?
        } else if let Some(process) = process {
            // Read from memory (ring buffer)
            info!(
                "grep_logs: Using memory-based search for {}/{}",
                req.project, req.name
            );
            grep_from_memory(
                &process, &pattern, before, after, since_time, until_time, &req.name,
            )
            .map_err(|e| Status::internal(format!("Failed to grep memory: {}", e)))?
        } else {
            info!(
                "grep_logs: No file and no process found for {}/{}",
                req.project, req.name
            );
            Vec::new()
        };

        Ok(Response::new(GrepLogsResponse { matches }))
    }
}

// Type alias for time range
type TimeRange = (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

// Helper function to parse time filters
fn parse_time_filters(
    since: &Option<String>,
    until: &Option<String>,
    last: &Option<String>,
) -> Result<TimeRange, Box<Status>> {
    let now = chrono::Utc::now();

    let since_time = if let Some(last_str) = last {
        // Parse "last" duration (e.g., "1h", "30m", "2d")
        let duration = parse_duration(last_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid duration '{}': {}",
                last_str, e
            )))
        })?;
        Some(now - duration)
    } else if let Some(since_str) = since {
        Some(parse_time_string(since_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid since time '{}': {}",
                since_str, e
            )))
        })?)
    } else {
        None
    };

    let until_time = if let Some(until_str) = until {
        Some(parse_time_string(until_str).map_err(|e| {
            Box::new(Status::invalid_argument(format!(
                "Invalid until time '{}': {}",
                until_str, e
            )))
        })?)
    } else {
        None
    };

    Ok((since_time, until_time))
}

// Helper function to parse duration strings
fn parse_duration(duration_str: &str) -> Result<chrono::Duration, String> {
    let duration_str = duration_str.trim();

    if duration_str.is_empty() {
        return Err("Empty duration".to_string());
    }

    let (num_str, unit) = if let Some(pos) = duration_str.rfind(char::is_alphabetic) {
        duration_str.split_at(pos)
    } else {
        return Err("No time unit specified".to_string());
    };

    let number: i64 = num_str
        .parse()
        .map_err(|_| format!("Invalid number: {}", num_str))?;

    match unit {
        "s" => Ok(chrono::Duration::seconds(number)),
        "m" => Ok(chrono::Duration::minutes(number)),
        "h" => Ok(chrono::Duration::hours(number)),
        "d" => Ok(chrono::Duration::days(number)),
        _ => Err(format!("Unknown time unit: {}", unit)),
    }
}

// Helper function to parse time strings
fn parse_time_string(time_str: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let time_str = time_str.trim();

    // Try different time formats
    let formats = [
        "%Y-%m-%d %H:%M:%S", // 2025-06-17 10:30:00
        "%Y-%m-%d %H:%M",    // 2025-06-17 10:30
        "%H:%M:%S",          // 10:30:00 (today)
        "%H:%M",             // 10:30 (today)
    ];

    for format in &formats {
        if let Ok(naive_time) = chrono::NaiveDateTime::parse_from_str(time_str, format) {
            return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                naive_time,
                chrono::Utc,
            ));
        }

        // For time-only formats, combine with today's date
        if format.starts_with("%H") {
            if let Ok(naive_time) = chrono::NaiveTime::parse_from_str(time_str, format) {
                let today = chrono::Utc::now().date_naive();
                let naive_datetime = today.and_time(naive_time);
                return Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    naive_datetime,
                    chrono::Utc,
                ));
            }
        }
    }

    Err(format!("Could not parse time: {}", time_str))
}

// Helper function to grep log file
async fn grep_log_file(
    log_file: &std::path::Path,
    pattern: &regex::Regex,
    before: usize,
    after: usize,
    since_time: Option<chrono::DateTime<chrono::Utc>>,
    until_time: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Vec<GrepMatch>, std::io::Error> {
    use tokio::fs::File;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let file = File::open(log_file).await?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let mut all_lines = Vec::new();
    let mut line_num = 0u32;

    // Read all lines and parse them
    while let Ok(Some(line)) = lines.next_line().await {
        line_num += 1;
        let (timestamp, level, content) = parse_log_line(&line);

        // Apply time filters
        if let Some(ts) = &timestamp {
            let log_time =
                chrono::DateTime::<chrono::Utc>::from_timestamp(ts.seconds, ts.nanos as u32)
                    .unwrap_or_else(chrono::Utc::now);

            if let Some(since) = since_time {
                if log_time < since {
                    continue;
                }
            }

            if let Some(until) = until_time {
                if log_time > until {
                    continue;
                }
            }
        }

        all_lines.push(LogEntry {
            line_number: line_num,
            content,
            timestamp,
            level: level as i32,
            process_name: None,
        });
    }

    let mut matches = Vec::new();

    // Find matches and collect context
    for (idx, entry) in all_lines.iter().enumerate() {
        if pattern.is_match(&entry.content) {
            let context_before = if before > 0 && idx >= before {
                all_lines[idx.saturating_sub(before)..idx].to_vec()
            } else {
                all_lines[0..idx].to_vec()
            };

            let context_after = if after > 0 && idx + 1 + after <= all_lines.len() {
                all_lines[idx + 1..idx + 1 + after].to_vec()
            } else {
                all_lines[idx + 1..].to_vec()
            };

            matches.push(GrepMatch {
                matched_line: Some(entry.clone()),
                context_before,
                context_after,
            });
        }
    }

    Ok(matches)
}

// Helper function to grep from memory (ring buffer)
fn grep_from_memory(
    process: &std::sync::Arc<crate::daemon::process::proxy::ProxyInfo>,
    pattern: &regex::Regex,
    before: usize,
    after: usize,
    since_time: Option<chrono::DateTime<chrono::Utc>>,
    until_time: Option<chrono::DateTime<chrono::Utc>>,
    process_name: &str,
) -> Result<Vec<proto::GrepMatch>, String> {
    use ringbuf::traits::{Consumer, Observer};

    let mut all_lines = Vec::new();

    // Get logs from ring buffer
    if let Ok(ring) = process.ring.lock() {
        let chunk_count = ring.occupied_len();
        info!(
            "DEBUG grep_from_memory: Found {} chunks in ring buffer for process {}",
            chunk_count, process_name
        );

        // Convert chunks to lines with timestamps (without cloning)
        for log_chunk in ring.iter() {
            if let Ok(text) = std::str::from_utf8(&log_chunk.data) {
                for line in text.lines() {
                    // Apply time filter if specified
                    if let Some(since) = since_time {
                        if log_chunk.timestamp < since {
                            continue;
                        }
                    }
                    if let Some(until) = until_time {
                        if log_chunk.timestamp > until {
                            continue;
                        }
                    }

                    let log_entry = proto::LogEntry {
                        line_number: (all_lines.len() + 1) as u32,
                        content: line.to_string(),
                        timestamp: Some(prost_types::Timestamp {
                            seconds: log_chunk.timestamp.timestamp(),
                            nanos: log_chunk.timestamp.timestamp_subsec_nanos() as i32,
                        }),
                        level: 1, // Default to INFO
                        process_name: Some(process_name.to_string()),
                    };
                    all_lines.push(log_entry);
                }
            }
        }
    }

    let mut matches = Vec::new();

    // Find matches and collect context
    for (idx, entry) in all_lines.iter().enumerate() {
        if pattern.is_match(&entry.content) {
            let context_before = if before > 0 && idx >= before {
                all_lines[idx.saturating_sub(before)..idx].to_vec()
            } else {
                all_lines[0..idx].to_vec()
            };

            let context_after = if after > 0 && idx + 1 + after <= all_lines.len() {
                all_lines[idx + 1..idx + 1 + after].to_vec()
            } else {
                all_lines[idx + 1..].to_vec()
            };

            matches.push(proto::GrepMatch {
                matched_line: Some(entry.clone()),
                context_before,
                context_after,
            });
        }
    }

    Ok(matches)
}

use crate::common::exit_code::format_exit_reason;
use crate::daemon::process::proxy::{LogChunk, ProxyInfo};
use crate::daemon::process::ProcessStatus;
use chrono::{DateTime, Utc};
use proto::ProcessInfo;
use ringbuf::traits::Consumer;
use ringbuf::HeapRb;
use std::path::Path;
use std::sync::Mutex;
use tracing::debug;

/// Extract port information from a process
pub fn extract_ports(process: &ProxyInfo) -> Vec<u32> {
    if let Some(port) = process.port {
        vec![port as u32]
    } else if let Ok(detected) = process.detected_port.try_lock() {
        detected.map(|p| vec![p as u32]).unwrap_or_default()
    } else {
        Vec::new()
    }
}

/// Create a prost timestamp from a chrono DateTime
pub fn create_timestamp(datetime: DateTime<Utc>) -> Option<prost_types::Timestamp> {
    Some(prost_types::Timestamp {
        seconds: datetime.timestamp(),
        nanos: datetime.timestamp_subsec_nanos() as i32,
    })
}

/// Extract exit details from a process (exit code, reason, stderr tail)
pub fn extract_exit_details(process: &ProxyInfo) -> (Option<i32>, Option<String>, Option<String>) {
    match process.exit_code.try_lock() {
        Ok(code_guard) => {
            if let Some(code) = *code_guard {
                let reason = Some(format_exit_reason(code));
                let stderr = extract_stderr_tail(&process.ring);
                (Some(code), reason, Some(stderr))
            } else {
                (None, None, None)
            }
        }
        Err(_) => (None, None, None),
    }
}

/// Extract the last 200 lines from stderr ring buffer
pub fn extract_stderr_tail(ring: &Mutex<HeapRb<LogChunk>>) -> String {
    ring.try_lock()
        .ok()
        .map(|ring| {
            let all_chunks: Vec<_> = ring.iter().collect();
            let start_idx = all_chunks.len().saturating_sub(200);
            all_chunks[start_idx..]
                .iter()
                .map(|chunk| String::from_utf8_lossy(&chunk.data).to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Convert ProcessStatus to proto representation
pub fn convert_process_status(status: ProcessStatus) -> i32 {
    match status {
        ProcessStatus::Starting => proto::ProcessStatus::Starting as i32,
        ProcessStatus::Running => proto::ProcessStatus::Running as i32,
        ProcessStatus::Stopping => proto::ProcessStatus::Stopping as i32,
        ProcessStatus::Stopped => proto::ProcessStatus::Stopped as i32,
        ProcessStatus::Failed => proto::ProcessStatus::Failed as i32,
    }
}

/// Create a ProcessInfo from ProxyInfo
pub fn create_process_info(
    process: &ProxyInfo,
    log_dir: &Path,
    timeout_occurred: Option<bool>,
    log_context: Vec<String>,
    matched_line: Option<String>,
) -> ProcessInfo {
    let current_status = process.get_status();
    let (exit_code, exit_reason, stderr_tail) = if matches!(current_status, ProcessStatus::Failed) {
        extract_exit_details(process)
    } else {
        (None, None, None)
    };

    ProcessInfo {
        id: process.id.clone(),
        name: process.name.clone(),
        cmd: process.cmd.clone().unwrap_or_default(),
        cwd: process
            .cwd
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        status: convert_process_status(current_status),
        start_time: create_timestamp(process.start_time),
        pid: Some(process.pid),
        log_file: {
            let path = log_dir
                .join(&process.project)
                .join(format!("{}.log", process.name.replace('/', "_")));
            debug!("Generated log file path: {:?}", path);
            path.to_string_lossy().to_string()
        },
        project: process.project.clone(),
        ports: extract_ports(process),
        wait_timeout_occurred: if process.wait_for_log.is_some() {
            timeout_occurred
        } else {
            None
        },
        exit_code,
        exit_reason,
        stderr_tail,
        log_context,
        matched_line,
    }
}

/// Parameters for creating a failed process info
pub struct FailedProcessParams<'a> {
    pub name: &'a str,
    pub project: &'a str,
    pub cmd: Option<String>,
    pub cwd: Option<&'a Path>,
    pub log_dir: &'a Path,
    pub exit_code: i32,
    pub exit_reason: &'a str,
    pub stderr: &'a str,
}

/// Create a failed ProcessInfo for error cases
pub fn create_failed_process_info(params: FailedProcessParams) -> ProcessInfo {
    ProcessInfo {
        id: uuid::Uuid::new_v4().to_string(),
        name: params.name.to_string(),
        cmd: params.cmd.unwrap_or_default(),
        cwd: params
            .cwd
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        status: proto::ProcessStatus::Failed as i32,
        start_time: create_timestamp(Utc::now()),
        pid: None,
        log_file: params
            .log_dir
            .join(params.project)
            .join(format!("{}.log", params.name.replace('/', "_")))
            .to_string_lossy()
            .to_string(),
        project: params.project.to_string(),
        ports: vec![],
        wait_timeout_occurred: None,
        exit_code: Some(params.exit_code),
        exit_reason: Some(params.exit_reason.to_string()),
        stderr_tail: Some(params.stderr.to_string()),
        log_context: vec![],
        matched_line: None,
    }
}

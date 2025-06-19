use std::process::Command;
use tracing::{debug, warn};

/// Detect listening ports for a given PID using lsof
pub fn detect_ports(pid: u32) -> Vec<u32> {
    // First, get all child PIDs
    let all_pids = get_process_tree(pid);
    debug!("Process tree for PID {}: {:?}", pid, all_pids);
    
    let mut all_ports = Vec::new();
    
    // Check ports for each PID in the process tree
    for check_pid in all_pids {
        // Use lsof to find listening TCP ports for the process
        let output = match Command::new("lsof")
            .args([
                "-Pan",           // No name resolution, all network files
                "-p", &check_pid.to_string(),  // Process ID
                "-iTCP",          // TCP connections only
                "-sTCP:LISTEN",   // Only listening state
            ])
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                debug!("Failed to run lsof for PID {}: {}", check_pid, e);
                continue;
            }
        };

        if !output.status.success() {
            // lsof returns non-zero when no files found, which is normal
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse lsof output
        // Format: COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME
        for line in stdout.lines().skip(1) {  // Skip header
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 9 {
                continue;
            }

            // NAME field contains address:port
            let name = parts[8];
            if let Some(port_str) = extract_port(name) {
                if let Ok(port) = port_str.parse::<u32>() {
                    if !all_ports.contains(&port) {
                        all_ports.push(port);
                    }
                }
            }
        }
    }

    debug!("Detected ports for PID {} and children: {:?}", pid, all_ports);
    all_ports
}

/// Get process tree - parent PID and all its children
fn get_process_tree(pid: u32) -> Vec<u32> {
    let mut pids = vec![pid];
    
    // Use pgrep to find child processes
    if let Ok(output) = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Ok(child_pid) = line.trim().parse::<u32>() {
                    // Recursively get children of children
                    let child_tree = get_process_tree(child_pid);
                    for cpid in child_tree {
                        if !pids.contains(&cpid) {
                            pids.push(cpid);
                        }
                    }
                }
            }
        }
    }
    
    pids
}

/// Extract port from lsof NAME field
/// Examples:
/// - *:3000 -> 3000
/// - 127.0.0.1:8080 -> 8080
/// - [::]:3000 -> 3000
fn extract_port(name: &str) -> Option<&str> {
    // Handle IPv6 format [::]:port
    if name.contains("]:") {
        return name.split("]:").nth(1);
    }
    
    // Handle IPv4 and wildcard format
    name.split(':').nth(1)
}

/// Alternative implementation using netstat (for systems without lsof)
#[allow(dead_code)]
pub fn detect_ports_netstat(pid: u32) -> Vec<u32> {
    // Try netstat with different options based on OS
    let output = if cfg!(target_os = "macos") {
        Command::new("netstat")
            .args(["-anv", "-p", "tcp"])
            .output()
    } else {
        // Linux
        Command::new("netstat")
            .args(["-tlnp"])
            .output()
    };

    let output = match output {
        Ok(output) => output,
        Err(e) => {
            warn!("Failed to run netstat: {}", e);
            return Vec::new();
        }
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut ports = Vec::new();

    // Parse netstat output (format varies by OS)
    for line in stdout.lines() {
        if !line.contains("LISTEN") {
            continue;
        }

        // Check if line contains our PID
        if !line.contains(&pid.to_string()) {
            continue;
        }

        // Extract port from local address
        if let Some(addr) = line.split_whitespace().nth(3) {
            if let Some(port_str) = extract_port(addr) {
                if let Ok(port) = port_str.parse::<u32>() {
                    if !ports.contains(&port) {
                        ports.push(port);
                    }
                }
            }
        }
    }

    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_port() {
        assert_eq!(extract_port("*:3000"), Some("3000"));
        assert_eq!(extract_port("127.0.0.1:8080"), Some("8080"));
        assert_eq!(extract_port("[::]:3000"), Some("3000"));
        assert_eq!(extract_port("[::1]:8080"), Some("8080"));
        assert_eq!(extract_port("localhost"), None);
    }
}
use proto::process_manager_client::ProcessManagerClient;
use tonic::transport::{Channel, Endpoint};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone)]
pub struct McpClient {
    client: ProcessManagerClient<Channel>,
}

impl McpClient {
    pub async fn connect(socket_path: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let data_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".mcproc");
        
        let _socket_path = socket_path.unwrap_or_else(|| {
            data_dir.join("mcprocd.sock")
        });
        
        // Check if daemon is running by checking PID file
        let pid_file = data_dir.join("mcprocd.pid");
        let daemon_running = if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // Check if process is actually running
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
            } else {
                false
            }
        } else {
            false
        };
        
        if !daemon_running {
            eprintln!("mcprocd daemon is not running. Starting it automatically...");
            
            // Start daemon in background
            let mcprocd_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .map(|p| p.join("mcprocd"))
                .unwrap_or_else(|| PathBuf::from("mcprocd"));
            
            let mut cmd = std::process::Command::new(&mcprocd_path);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            
            // Detach from parent process
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                unsafe {
                    cmd.pre_exec(|| {
                        // Create new session
                        nix::unistd::setsid()?;
                        Ok(())
                    });
                }
            }
            
            match cmd.spawn() {
                Ok(_) => {
                    eprintln!("Started mcprocd daemon");
                    // Wait a bit for daemon to start
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(e) => {
                    return Err(format!("Failed to start mcprocd daemon: {}. Please start it manually.", e).into());
                }
            }
        }
        
        // Read the port from file
        let port_file = data_dir.join("mcprocd.port");
        let port = if port_file.exists() {
            std::fs::read_to_string(&port_file)
                .ok()
                .and_then(|s| s.trim().parse::<u16>().ok())
                .unwrap_or(50051)
        } else {
            50051
        };
        
        let endpoint = Endpoint::from_shared(format!("http://127.0.0.1:{}", port))?
            .timeout(Duration::from_secs(2))
            .connect_timeout(Duration::from_secs(2));
            
        let client = ProcessManagerClient::connect(endpoint).await
            .map_err(|e| format!("Failed to connect to mcprocd on port {}: {}", port, e))?;
            
        Ok(Self { client })
    }
    
    pub async fn connect_remote(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let client = ProcessManagerClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }
    
    pub fn inner(&mut self) -> &mut ProcessManagerClient<Channel> {
        &mut self.client
    }
}
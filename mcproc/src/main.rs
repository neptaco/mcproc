mod client;
mod commands;

use clap::{Parser, Subcommand};
use client::McpClient;
use commands::*;

#[derive(Parser)]
#[command(name = "mcproc")]
#[command(about = "CLI tool for managing development processes via mcprocd", long_about = None)]
struct Cli {
    /// Remote mcprocd address (default: local unix socket)
    #[arg(short, long)]
    remote: Option<String>,
    
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start or attach to a process
    Start(StartCommand),
    
    /// Stop a running process
    Stop(StopCommand),
    
    /// Restart a process
    Restart(RestartCommand),
    
    /// List running processes
    Ps(PsCommand),
    
    /// View process logs
    Logs(LogsCommand),
    
    /// Get path to process log file
    Logfile {
        /// Process name
        name: String,
    },
    
    /// MCP server management
    Mcp(MpcCommand),
    
    /// Manage mcprocd daemon
    Daemon(DaemonCommand),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    
    // Handle daemon command separately (doesn't need client connection)
    if let Commands::Daemon(cmd) = cli.command {
        return cmd.execute().await;
    }
    
    // Connect to mcprocd
    let client = if let Some(remote) = cli.remote {
        McpClient::connect_remote(&remote).await?
    } else {
        McpClient::connect(None).await?
    };
    
    // Execute command
    match cli.command {
        Commands::Start(cmd) => cmd.execute(client).await?,
        Commands::Stop(cmd) => cmd.execute(client).await?,
        Commands::Restart(cmd) => cmd.execute(client).await?,
        Commands::Ps(cmd) => cmd.execute(client).await?,
        Commands::Logs(cmd) => cmd.execute(client).await?,
        Commands::Logfile { name } => {
            // For now, just print expected path
            let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
            let log_path = home.join(".mcproc").join("log").join(format!("{}_{}.log", 
                name, 
                chrono::Utc::now().format("%Y%m%d")
            ));
            println!("{}", log_path.display());
        }
        Commands::Mcp(cmd) => cmd.execute(client).await?,
        Commands::Daemon(_) => unreachable!(), // Already handled above
    }
    
    Ok(())
}
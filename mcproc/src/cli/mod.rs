pub mod commands;
pub mod utils;

use crate::client::DaemonClient;
use crate::common::config::Config;
use clap::{Parser, Subcommand};
use commands::*;

#[derive(Parser)]
#[command(name = "mcproc")]
#[command(about = "CLI tool for managing development processes via mcprocd", long_about = None)]
pub struct Cli {
    /// Run as daemon
    #[arg(long, hide = true)]
    daemon: bool,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
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

    /// Search process logs
    Grep(GrepCommand),

    /// Get path to process log file
    Logfile {
        /// Process name
        name: String,
    },

    /// Clean project (stop processes and delete logs)
    Clean(CleanCommand),

    /// MCP server management
    Mcp(McpCommand),

    /// Manage mcprocd daemon
    Daemon(DaemonCommand),
}

pub async fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Check if --daemon flag is set
    if cli.daemon {
        return crate::daemon::run_daemon().await;
    }

    // If no command, show help
    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            return Ok(());
        }
    };

    // Handle daemon command separately (doesn't need client connection)
    if let Commands::Daemon(cmd) = command {
        return cmd.execute().await;
    }

    // Connect to mcprocd
    let client = DaemonClient::connect(None).await?;

    // Execute command
    match command {
        Commands::Start(cmd) => cmd.execute(client).await?,
        Commands::Stop(cmd) => cmd.execute(client).await?,
        Commands::Restart(cmd) => cmd.execute(client).await?,
        Commands::Ps(cmd) => cmd.execute(client).await?,
        Commands::Logs(cmd) => cmd.execute(client).await?,
        Commands::Grep(cmd) => cmd.execute(client).await?,
        Commands::Logfile { name } => {
            let config = Config::for_client();
            let log_path = config.process_log_file(&name);
            println!("{}", log_path.display());
        }
        Commands::Clean(mut cmd) => {
            cmd.verbose = cli.verbose;
            cmd.execute(client).await?
        }
        Commands::Mcp(cmd) => cmd.execute(client).await?,
        Commands::Daemon(_) => unreachable!(), // Already handled above
    }

    Ok(())
}

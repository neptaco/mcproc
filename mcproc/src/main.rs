mod cli;
mod client;
mod common;
mod daemon;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if --daemon flag is present in args
    let args: Vec<String> = std::env::args().collect();

    if args.contains(&"--daemon".to_string()) {
        // Run in daemon mode
        daemon::run_daemon().await
    } else {
        // Run in CLI mode
        cli::run_cli().await
    }
}

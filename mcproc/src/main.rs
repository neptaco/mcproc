mod cli;
mod client;
mod common;
mod daemon;

fn is_daemon_invocation(args: &[String]) -> bool {
    args.get(1).is_some_and(|arg| arg == "--daemon")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if --daemon flag is present in args
    let args: Vec<String> = std::env::args().collect();

    if is_daemon_invocation(&args) {
        // Run in daemon mode
        daemon::run_daemon().await
    } else {
        // Run in CLI mode
        match cli::run_cli().await {
            Err(error)
                if error
                    .downcast_ref::<client::DaemonRestartedForUpgrade>()
                    .is_some() =>
            {
                eprintln!("Daemon restarted successfully. Please run your command again.");
                Ok(())
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_daemon_invocation;

    #[test]
    fn daemon_flag_is_only_recognized_as_first_argument() {
        let direct = vec!["mcproc".to_string(), "--daemon".to_string()];
        let command_argument = vec![
            "mcproc".to_string(),
            "start".to_string(),
            "x".to_string(),
            "--cmd".to_string(),
            "--daemon".to_string(),
        ];

        assert!(is_daemon_invocation(&direct));
        assert!(!is_daemon_invocation(&command_argument));
    }
}

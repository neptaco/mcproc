use super::service::GrpcService;
use crate::common::config::Config;
use crate::daemon::log::LogHub;
use crate::daemon::process::ProcessManager;
use crate::daemon::stream::SharedStreamEventHub;
use proto::process_manager_server::ProcessManagerServer;
use std::sync::Arc;
use tonic::transport::Server;
use tracing::{error, info};

pub async fn start_grpc_server(
    config: Arc<Config>,
    process_manager: Arc<ProcessManager>,
    log_hub: Arc<LogHub>,
    event_hub: SharedStreamEventHub,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = GrpcService::new(process_manager, log_hub, config.clone(), event_hub);

    // Remove old socket file if it exists
    if config.paths.socket_path.exists() {
        std::fs::remove_file(&config.paths.socket_path)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = config.paths.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use tokio::net::UnixListener;
        use tokio_stream::wrappers::UnixListenerStream;

        // Create Unix socket
        let uds = UnixListener::bind(&config.paths.socket_path)?;
        let uds_stream = UnixListenerStream::new(uds);

        // Set permissions
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(config.api.unix_socket_permissions);
        std::fs::set_permissions(&config.paths.socket_path, permissions)?;

        info!(
            "Starting gRPC server on Unix socket: {:?}",
            config.paths.socket_path
        );

        let result = Server::builder()
            .add_service(ProcessManagerServer::new(service))
            .serve_with_incoming(uds_stream)
            .await;

        if let Err(e) = &result {
            error!("gRPC server failed: {}", e);
        } else {
            info!("gRPC server shutdown gracefully");
        }
        result?;
    }

    #[cfg(not(unix))]
    {
        return Err("Unix sockets are not supported on this platform".into());
    }

    Ok(())
}

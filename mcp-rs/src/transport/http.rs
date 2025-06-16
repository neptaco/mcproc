use crate::error::Result;
use crate::transport::Transport;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use warp::{Filter, Rejection, Reply};

#[derive(Clone)]
pub struct HttpTransportConfig {
    pub addr: SocketAddr,
    pub path: String,
}

impl Default for HttpTransportConfig {
    fn default() -> Self {
        Self {
            addr: ([127, 0, 0, 1], 3434).into(),
            path: "/mcp".to_string(),
        }
    }
}

/// HTTP+SSE transport for MCP communication
pub struct HttpTransport {
    config: HttpTransportConfig,
    tx: mpsc::Sender<JsonRpcMessage>,
    rx: mpsc::Receiver<JsonRpcMessage>,
    sse_clients: Arc<RwLock<Vec<mpsc::UnboundedSender<JsonRpcMessage>>>>,
    shutdown_tx: mpsc::Sender<()>,
}

impl HttpTransport {
    pub fn new(config: HttpTransportConfig) -> Self {
        let (tx, rx) = mpsc::channel(100);
        let (shutdown_tx, _) = mpsc::channel(1);
        
        Self {
            config,
            tx,
            rx,
            sse_clients: Arc::new(RwLock::new(Vec::new())),
            shutdown_tx,
        }
    }
    
    fn routes(
        tx: mpsc::Sender<JsonRpcMessage>,
        sse_clients: Arc<RwLock<Vec<mpsc::UnboundedSender<JsonRpcMessage>>>>,
    ) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
        let json_rpc = warp::path("jsonrpc")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |message: JsonRpcMessage| {
                let tx = tx.clone();
                async move {
                    if tx.send(message).await.is_err() {
                        return Err(warp::reject::reject());
                    }
                    Ok::<_, Rejection>(warp::reply::json(&serde_json::json!({})))
                }
            });
        
        let sse = warp::path("sse")
            .and(warp::get())
            .and_then(move || {
                let clients = sse_clients.clone();
                async move {
                    let (client_tx, client_rx) = mpsc::unbounded_channel();
                    
                    // Add client to list
                    {
                        let mut clients = clients.write().await;
                        clients.push(client_tx);
                    }
                    
                    // Convert to SSE stream
                    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(client_rx)
                        .map(|msg| {
                            let data = serde_json::to_string(&msg).unwrap_or_default();
                            Ok::<_, warp::Error>(
                                warp::sse::Event::default()
                                    .event("message")
                                    .data(data)
                            )
                        });
                    
                    Ok::<_, Rejection>(warp::sse::reply(stream))
                }
            });
        
        json_rpc.or(sse)
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn start(&mut self) -> Result<()> {
        let tx = self.tx.clone();
        let sse_clients = self.sse_clients.clone();
        let addr = self.config.addr;
        let _path = self.config.path.clone();
        
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = shutdown_tx;
        
        // Create routes
        let routes = Self::routes(tx, sse_clients);
        
        // Spawn HTTP server
        tokio::spawn(async move {
            let (_, server) = warp::serve(routes)
                .bind_with_graceful_shutdown(addr, async move {
                    let _ = shutdown_rx.recv().await;
                });
            
            server.await;
        });
        
        Ok(())
    }
    
    async fn send(&mut self, message: JsonRpcMessage) -> Result<()> {
        // Send to all SSE clients
        let clients = self.sse_clients.read().await;
        let mut disconnected = Vec::new();
        
        for (i, client) in clients.iter().enumerate() {
            if client.send(message.clone()).is_err() {
                disconnected.push(i);
            }
        }
        
        // Remove disconnected clients
        if !disconnected.is_empty() {
            drop(clients);
            let mut clients = self.sse_clients.write().await;
            for i in disconnected.into_iter().rev() {
                clients.swap_remove(i);
            }
        }
        
        Ok(())
    }
    
    async fn receive(&mut self) -> Result<Option<JsonRpcMessage>> {
        Ok(self.rx.recv().await)
    }
    
    async fn close(&mut self) -> Result<()> {
        let _ = self.shutdown_tx.send(()).await;
        Ok(())
    }
}
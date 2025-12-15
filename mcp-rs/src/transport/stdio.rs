use crate::error::Result;
use crate::transport::Transport;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Stdio transport for MCP communication
pub struct StdioTransport {
    /// Sender for incoming messages. Wrapped in Option so we can take ownership
    /// and move it into the spawned task, ensuring EOF properly closes the channel.
    tx: Option<mpsc::Sender<JsonRpcMessage>>,
    rx: mpsc::Receiver<JsonRpcMessage>,
    shutdown_tx: mpsc::Sender<()>,
}

impl StdioTransport {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(100);
        let (shutdown_tx, _) = mpsc::channel(1);

        Self {
            tx: Some(tx),
            rx,
            shutdown_tx,
        }
    }
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn start(&mut self) -> Result<()> {
        // Take ownership of tx so it's the only sender.
        // When the spawned task exits (on EOF), tx is dropped and rx.recv() returns None.
        let tx = self.tx.take().ok_or_else(|| {
            crate::error::Error::Internal("Transport already started".to_string())
        })?;

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = shutdown_tx;

        // Spawn stdin reader
        tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let reader = BufReader::new(stdin);
            let mut lines = reader.lines();

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    line = lines.next_line() => {
                        match line {
                            Ok(Some(line)) => {
                                if let Ok(message) = serde_json::from_str::<JsonRpcMessage>(&line) {
                                    if tx.send(message).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Ok(None) => break, // EOF - tx will be dropped, causing rx.recv() to return None
                            Err(_) => break,
                        }
                    }
                }
            }
            // tx is dropped here, which closes the channel
        });

        Ok(())
    }

    async fn send(&mut self, message: JsonRpcMessage) -> Result<()> {
        let json = serde_json::to_string(&message)?;
        let mut stdout = tokio::io::stdout();
        stdout.write_all(json.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
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

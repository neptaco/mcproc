use crate::error::Result;
use crate::transport::Transport;
use crate::types::JsonRpcMessage;
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// Stdio transport for MCP communication
pub struct StdioTransport {
    tx: mpsc::Sender<JsonRpcMessage>,
    rx: mpsc::Receiver<JsonRpcMessage>,
    shutdown_tx: mpsc::Sender<()>,
}

impl StdioTransport {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(100);
        let (shutdown_tx, _) = mpsc::channel(1);

        Self {
            tx,
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
        let tx = self.tx.clone();
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
                            Ok(None) => break, // EOF
                            Err(_) => break,
                        }
                    }
                }
            }
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

use std::sync::{Arc, Mutex}; // Use std Mutex for synchronous access to Option
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, broadcast};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{info, warn, error, debug};
use url::Url;

use crate::error::{AppError, Result};
use crate::transport::Transport;

// Keepalive settings
const PING_INTERVAL: Duration = Duration::from_secs(30);
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub struct WebSocketManager {
    url: String,
    // Channel to send detected signatures to the processor
    signature_tx: mpsc::UnboundedSender<String>,
    // We keep the receiver in an Option inside a Mutex to hand it out once
    // Using std::sync::Mutex to allow synchronous get_signature_receiver
    signature_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    // Track current subscription to resubscribe on reconnect
    // Using tokio::sync::Mutex here is fine as it's accessed in async tasks,
    // but std::sync::Mutex is also fine if contention is low.
    // Let's stick to tokio Mutex for subscription as it might be held across awaits?
    // No, string cloning is fast. Let's use std Mutex for simplicity and consistency unless await is needed while holding lock.
    current_subscription: Arc<Mutex<Option<String>>>,
}

impl WebSocketManager {
    pub fn new(url: String) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            url,
            signature_tx: tx,
            signature_rx: Arc::new(Mutex::new(Some(rx))),
            current_subscription: Arc::new(Mutex::new(None)),
        }
    }

    async fn handle_connection(&self, target_wallet: Option<String>) -> Result<()> {
        let url = Url::parse(&self.url)
            .map_err(|e| AppError::Init(format!("Invalid WebSocket URL: {}", e)))?;

        info!("Connecting to WebSocket: {}", url);
        let (ws_stream, _) = connect_async(url).await?;
        info!("WebSocket connected");

        let (mut write, mut read) = ws_stream.split();

        // 1. Send Subscription if we have a target
        if let Some(wallet) = target_wallet {
            let subscribe_msg = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "logsSubscribe",
                "params": [
                    { "mentions": [wallet] },
                    { "commitment": "processed" }
                ]
            });
            write.send(Message::Text(subscribe_msg.to_string())).await?;
            info!("Subscribed to logs for {}", wallet);
        }

        // 2. Heartbeat task
        let mut ping_interval = tokio::time::interval(PING_INTERVAL);

        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    if let Err(e) = write.send(Message::Ping(vec![])).await {
                        warn!("Failed to send ping: {}", e);
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(message)) => {
                            match message {
                                Message::Text(text) => self.process_message(&text).await,
                                Message::Binary(_) => {},
                                Message::Ping(_) => {},
                                Message::Pong(_) => {},
                                Message::Close(_) => {
                                    warn!("WebSocket closed by server");
                                    break;
                                }
                                Message::Frame(_) => {}
                            }
                        }
                        Some(Err(e)) => {
                            error!("WebSocket stream error: {}", e);
                            break;
                        }
                        None => {
                            warn!("WebSocket stream ended");
                            break;
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    async fn process_message(&self, text: &str) {
        if !text.contains("logsNotification") {
            return;
        }

        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(json) => {
                if let Some(params) = json.get("params") {
                    if let Some(result) = params.get("result") {
                        if let Some(value) = result.get("value") {
                            if let Some(sig) = value.get("signature").and_then(|s| s.as_str()) {
                                if let Err(e) = self.signature_tx.send(sig.to_string()) {
                                    error!("Failed to send signature to channel: {}", e);
                                } else {
                                    debug!("Received signature: {}", sig);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => error!("Failed to parse WS message: {}", e),
        }
    }

    /// Run the connection loop forever.
    pub async fn run(&self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        loop {
            let target = {
                let lock = self.current_subscription.lock().unwrap();
                lock.clone()
            };

            // Race connection handling with shutdown signal
            tokio::select! {
                result = self.handle_connection(target) => {
                    if let Err(e) = result {
                        error!("WebSocket connection failed: {}. Retrying in {}s...", e, RECONNECT_DELAY.as_secs());
                    } else {
                        warn!("WebSocket connection dropped. Retrying in {}s...", RECONNECT_DELAY.as_secs());
                    }
                }
                _ = shutdown.recv() => {
                    info!("WebSocket Manager shutting down...");
                    break;
                }
            }

            // Race retry sleep with shutdown
            tokio::select! {
                _ = sleep(RECONNECT_DELAY) => {}
                _ = shutdown.recv() => {
                    info!("WebSocket Manager shutting down...");
                    break;
                }
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Transport for WebSocketManager {
    async fn connect(&self) -> Result<()> {
        // Since run() is the main loop, connect() here is ambiguous.
        // We can just return Ok() and let main call run().
        Ok(())
    }

    async fn subscribe_logs(&self, mention: &str) -> Result<()> {
        let mut sub = self.current_subscription.lock().unwrap();
        *sub = Some(mention.to_string());
        Ok(())
    }

    fn get_signature_receiver(&self) -> mpsc::UnboundedReceiver<String> {
        self.signature_rx.lock().unwrap().take().expect("Receiver already taken")
    }

    async fn reconnect(&self) -> Result<()> {
        Ok(())
    }
}

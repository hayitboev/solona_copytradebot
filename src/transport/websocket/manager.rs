use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, Mutex};
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
    signature_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    // Track current subscription to resubscribe on reconnect
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
            .map_err(|e| AppError::Config(config::ConfigError::Message(e.to_string())))?;

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
                    { "commitment": "processed" } // "processed" is fastest for speed
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
                    // Send ping to keep connection alive
                    if let Err(e) = write.send(Message::Ping(vec![])).await {
                        warn!("Failed to send ping: {}", e);
                        break; // Reconnect
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(message)) => {
                            match message {
                                Message::Text(text) => self.process_message(&text).await,
                                Message::Binary(_) => {}, // Ignore binary for JSON-RPC
                                Message::Ping(_) => {
                                    // Tungstenite handles pong automatically usually, 
                                    // but explicitly sending Pong doesn't hurt if needed.
                                }, 
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

    // Zero-copy optimization: parsing directly from slice if possible, 
    // but serde_json typically works on str/bytes.
    async fn process_message(&self, text: &str) {
        // Fast path: check if it's a notification before full parsing
        if !text.contains("logsNotification") {
            return;
        }

        // Parse minimal needed structure
        // {"method": "logsNotification", "params": { "result": { "value": { "signature": "..." } } }}
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(json) => {
                if let Some(params) = json.get("params") {
                    if let Some(result) = params.get("result") {
                        if let Some(value) = result.get("value") {
                            if let Some(sig) = value.get("signature").and_then(|s| s.as_str()) {
                                // Send to processor (non-blocking)
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
}

#[async_trait::async_trait]
impl Transport for WebSocketManager {
    async fn connect(&self) -> Result<()> {
        // We spawn the connection loop in the background
        let this_manager = self_clone_helper(self); // Need a way to clone inside async block if needed, 
                                                    // but Transport trait & self is usually Arc'd in main.
                                                    // For now, we assume the caller calls connect() and we run the loop.
                                                    
        // Ideally, connect() starts the loop.
        // We need a persistent loop that handles reconnects.
        panic!("Use run() loop instead for now, or implement spawned actor pattern");
    }

    // Changing signature slightly to fit the implementation pattern better:
    // The Trait usually implies "start working". 
    // Let's implement a specific `run` method for the manager, 
    // and the trait simply exposes capabilities.
    
    async fn subscribe_logs(&self, mention: &str) -> Result<()> {
        let mut sub = self.current_subscription.lock().await;
        *sub = Some(mention.to_string());
        Ok(())
    }

    fn get_signature_receiver(&self) -> mpsc::UnboundedReceiver<String> {
        self.signature_rx.lock().await.take().expect("Receiver already taken")
    }

    async fn reconnect(&self) -> Result<()> {
        // Handled by the run loop
        Ok(())
    }
}

// Extension to run the loop
impl WebSocketManager {
    pub async fn run(&self) {
        loop {
            let target = self.current_subscription.lock().await.clone();
            
            if let Err(e) = self.handle_connection(target).await {
                error!("WebSocket connection failed: {}. Retrying in {}s...", e, RECONNECT_DELAY.as_secs());
            } else {
                warn!("WebSocket connection dropped. Retrying in {}s...", RECONNECT_DELAY.as_secs());
            }
            
            sleep(RECONNECT_DELAY).await;
        }
    }
}

// Helper for cloning
fn self_clone_helper(manager: &WebSocketManager) -> WebSocketManager {
    // This struct isn't natively cloneable easily due to channels, 
    // In main logic we wrap WebSocketManager in Arc.
    // This is a placeholder for logic separation.
    unimplemented!()
}
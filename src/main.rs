// In src/main.rs

// ... existing imports ...
mod transport;

use crate::transport::websocket::manager::WebSocketManager;
use crate::transport::TransportMode;

#[tokio::main]
async fn main() -> Result<()> {
    // ... config loading ...

    info!("Initializing Transport Layer...");

    // Create channel for signatures
    // The Manager creates the channel internally, we just need to access the receiver later.
    
    // Select Transport
    let transport: Arc<WebSocketManager> = match config.transport_mode {
        TransportMode::Grpc => {
            if let Some(endpoint) = config.grpc_endpoint {
                info!("gRPC mode selected (Implementation pending proto files). Fallback to WS.");
                // For now, force WS because gRPC code is a stub
                Arc::new(WebSocketManager::new(config.ws_url.clone()))
            } else {
                error!("gRPC selected but no endpoint provided. Using WebSocket.");
                Arc::new(WebSocketManager::new(config.ws_url.clone()))
            }
        },
        _ => {
            info!("WebSocket mode selected.");
            Arc::new(WebSocketManager::new(config.ws_url.clone()))
        }
    };

    // Configure subscription
    transport.subscribe_logs(&config.wallet_address).await?;
    
    // Spawn Transport Loop
    let transport_clone = transport.clone();
    tokio::spawn(async move {
        transport_clone.run().await;
    });

    info!("Transport layer running. Monitoring {}", config.wallet_address);

    // ... continue to next phases ...
    
    Ok(())
}
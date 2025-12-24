mod config;
mod error;
mod http;
mod transport;
mod processor;
mod trading;

use std::sync::Arc;
use tracing::{info, error, Level};
use crate::error::Result;
use crate::transport::websocket::manager::WebSocketManager;
use crate::transport::Transport;
use crate::config::Config; // Assuming Config struct exists
use crate::processor::worker::Worker;
use crate::http::race_client::RaceClient;
use crate::trading::engine::TradingEngine;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // Load Config
    let config = Config::load()?;
    
    info!("Starting Solana Copy-Trading Bot...");
    info!("Configuration loaded. Monitoring: {}", config.wallet_address);
    info!("Trading Mode: Enabled. Risk limits: Min {} SOL, Max {} SOL", config.min_trade_amount_sol, config.max_trade_amount_sol);

    // Phase 1: Infrastructure

    // 1. Race Client
    let race_client = RaceClient::new(config.rpc_endpoints.clone())?;

    // 2. Transport (WebSocket)
    // We default to WebSocket for now as per previous code snippet logic
    let transport = Arc::new(WebSocketManager::new(config.ws_url.clone()));

    // Subscribe
    transport.subscribe_logs(&config.wallet_address).await?;
    
    // Get the signature receiver
    // The WebSocketManager should expose a way to get the receiver.
    // Looking at the errors: `self.signature_rx.lock().await.take()` in `manager.rs`.
    // I need to check `src/transport/websocket/manager.rs` to see how to get the receiver.
    let rx_signatures = transport.get_signature_receiver();

    // Start Transport Loop
    let transport_clone = transport.clone();
    tokio::spawn(async move {
        if let Err(e) = transport_clone.run().await {
            error!("Transport error: {}", e);
        }
    });

    info!("Transport layer running.");

    // Phase 2: Transaction Processing
    
    // Create channel for SwapEvents (Output of Phase 2)
    let (tx_swaps, rx_swaps) = tokio::sync::mpsc::channel(100);

    // Start Worker
    let rx_sigs = rx_signatures;
    let worker = Worker::new(race_client.clone(), rx_sigs, tx_swaps, config.wallet_address.clone());
    tokio::spawn(async move {
        worker.run().await;
    });
    info!("Worker started.");

    // Phase 3: Trading Engine
    let trading_engine = TradingEngine::new(config.clone(), race_client.clone(), rx_swaps)?;
    tokio::spawn(async move {
        trading_engine.run().await;
    });

    // Keep main alive
    tokio::signal::ctrl_c().await.expect("Failed to listen for event");
    info!("Shutting down...");

    Ok(())
}

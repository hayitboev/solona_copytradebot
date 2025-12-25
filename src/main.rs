use std::sync::Arc;
use tracing::{info, error, Level};
use std::time::Duration;

use solana_wallet_monitor::error::Result;
use solana_wallet_monitor::transport::websocket::manager::WebSocketManager;
use solana_wallet_monitor::transport::Transport;
use solana_wallet_monitor::config::Config;
use solana_wallet_monitor::processor::worker::Worker;
use solana_wallet_monitor::http::race_client::RaceClient;
use solana_wallet_monitor::trading::engine::TradingEngine;
use solana_wallet_monitor::analytics::stats::Stats;

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

    // Initialize Analytics
    let stats = Arc::new(Stats::new());

    // Shutdown Signal
    let (shutdown_tx, _shutdown_rx) = tokio::sync::broadcast::channel(1);

    // Spawn Stats Logger
    let stats_clone = stats.clone();
    let mut stats_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => stats_clone.log_stats(),
                _ = stats_shutdown_rx.recv() => break,
            }
        }
    });

    // Phase 1: Infrastructure

    // 1. Race Client
    let race_client = RaceClient::new(config.rpc_endpoints.clone())?;

    // 2. Transport (WebSocket)
    let transport = Arc::new(WebSocketManager::new(config.ws_url.clone()));

    // Subscribe
    transport.subscribe_logs(&config.wallet_address).await?;
    
    let rx_signatures = transport.get_signature_receiver();

    // Start Transport Loop
    let transport_clone = transport.clone();
    let transport_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        if let Err(e) = transport_clone.run(transport_shutdown_rx).await {
            error!("Transport error: {}", e);
        }
    });

    info!("Transport layer running.");

    // Phase 2: Transaction Processing

    // Create channel for SwapEvents (Output of Phase 2)
    let (tx_swaps, rx_swaps) = tokio::sync::mpsc::channel(100);

    // Start Worker
    let rx_sigs = rx_signatures;
    let worker = Worker::new(
        race_client.clone(),
        rx_sigs,
        tx_swaps,
        config.wallet_address.clone(),
        stats.clone(),
        config.max_workers
    );
    let worker_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        worker.run(worker_shutdown_rx).await;
    });
    info!("Worker started.");

    // Phase 3: Trading Engine
    let trading_engine = TradingEngine::new(
        config.clone(),
        race_client.clone(),
        rx_swaps,
        stats.clone()
    )?;
    let engine_shutdown_rx = shutdown_tx.subscribe();
    tokio::spawn(async move {
        trading_engine.run(engine_shutdown_rx).await;
    });

    // Keep main alive
    tokio::signal::ctrl_c().await.expect("Failed to listen for event");
    info!("Shutdown signal received.");

    // Graceful shutdown steps
    // Broadcast shutdown signal
    let _ = shutdown_tx.send(());

    info!("Waiting for pending operations...");
    // Give tasks some time to finish (e.g. 5 seconds)
    // Pending tokio tasks (like trades) will continue running if they were spawned.
    // The shutdown signal stops new work from being picked up.
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Log final stats
    stats.log_stats();

    info!("Bot stopped cleanly.");

    Ok(())
}

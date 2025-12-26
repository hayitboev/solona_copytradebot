use std::sync::Arc;
use tracing::{info, error, Level};
use std::time::Duration;
use std::io::{self, Write};

use solana_wallet_monitor::error::Result;
use solana_wallet_monitor::transport::websocket::manager::WebSocketManager;
use solana_wallet_monitor::transport::Transport;
use solana_wallet_monitor::config::Config;
use solana_wallet_monitor::processor::worker::Worker;
use solana_wallet_monitor::http::race_client::RaceClient;
use solana_wallet_monitor::trading::engine::TradingEngine;
use solana_wallet_monitor::analytics::stats::Stats;

enum UserChoice {
    PrimaryQuickNode,
    PublicSolana,
    Custom(String),
    Exit,
}

fn read_user_selection() -> UserChoice {
    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        match input {
            "1" => return UserChoice::PrimaryQuickNode,
            "2" => return UserChoice::PublicSolana,
            "3" => {
                print!("Enter Custom URL: ");
                io::stdout().flush().unwrap();
                let mut url = String::new();
                io::stdin().read_line(&mut url).unwrap();
                return UserChoice::Custom(url.trim().to_string());
            },
            "4" => return UserChoice::Exit,
            _ => println!("Invalid selection. Please try again."),
        }
    }
}

async fn run_session(config: Config) -> Result<()> {
    info!("Starting session with WebSocket: {}", config.ws_url);
    info!("Monitoring Wallet: {}", config.wallet_address);

    // Initialize Analytics
    let stats = Arc::new(Stats::new());

    // Shutdown Signal Channel
    // We use this to signal components to stop if Transport fails OR user hits Ctrl+C (handled in wrapper?)
    // Actually, handling Ctrl+C here is good, but if we want to return to menu, maybe Ctrl+C should exit app?
    // Let's assume typical CLI behavior: Ctrl+C kills app.
    // But if Transport fails, we return Err and go back to menu.

    let (shutdown_tx, _shutdown_rx) = tokio::sync::broadcast::channel(1);

    // Phase 1: Infrastructure
    // 1. Race Client
    let race_client = RaceClient::new(config.rpc_endpoints.clone())?;

    // 2. Transport (WebSocket)
    // Pass max_retries = 5 (hardcoded or from config if added later)
    let transport = Arc::new(WebSocketManager::new(config.ws_url.clone(), 5));

    transport.subscribe_logs(&config.wallet_address).await?;
    let rx_signatures = transport.get_signature_receiver();

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

    // Start Transport Loop
    // We await this task in a select! block later to catch failures
    let transport_clone = transport.clone();
    let transport_shutdown_rx = shutdown_tx.subscribe();
    let transport_handle = tokio::spawn(async move {
        transport_clone.run(transport_shutdown_rx).await
    });

    info!("Transport layer running.");

    // Phase 2: Transaction Processing
    let (tx_swaps, rx_swaps) = tokio::sync::mpsc::channel(100);

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

    // Wait for critical failure or interrupt
    tokio::select! {
        res = transport_handle => {
            // Transport task finished (likely error or disconnect)
            match res {
                Ok(inner_res) => {
                    if let Err(e) = inner_res {
                        error!("Transport Critical Error: {}", e);
                        // Signal shutdown to others
                        let _ = shutdown_tx.send(());
                        return Err(e);
                    }
                },
                Err(e) => {
                    error!("Transport Task Panicked: {}", e);
                    let _ = shutdown_tx.send(());
                    return Err(solana_wallet_monitor::error::AppError::Transport("Transport task panicked".into()));
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received (Ctrl+C). Exiting application.");
            // We want to exit completely on Ctrl+C usually, not just return to menu.
            // But to return to menu, user can force fail or select exit.
            // If user presses Ctrl+C, usually they want to kill the process.
            // Let's exit process here.
            let _ = shutdown_tx.send(());
            std::process::exit(0);
        }
    }

    // Graceful cleanup if we got here via non-fatal path (unlikely for infinite loop) or error handled above
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // Load Initial Config
    let mut config = Config::load()?;

    loop {
        println!("\n=== Solana Copy-Trade Bot ===");
        println!("Select Data Source:");
        println!("1. Primary (From .env: {})", config.ws_url);
        println!("2. Public Fallback (From .env: {})", config.fallback_ws_url);
        println!("3. Custom URL");
        println!("4. Exit");

        match read_user_selection() {
            UserChoice::Exit => {
                info!("Exiting...");
                break;
            },
            UserChoice::PrimaryQuickNode => {
                // Reload or just use current if it matches?
                // Actually if user selected Custom before, we want to revert.
                // Re-loading from env might be safest or using stored val.
                // But config.ws_url might have been mutated.
                // We should probably keep the "Original" values stored somewhere or reload config.
                // Since Config::load is cheap (env vars), let's reload base config to be sure.
                let base = Config::load()?;
                config.ws_url = base.ws_url;
            },
            UserChoice::PublicSolana => {
                config.ws_url = config.fallback_ws_url.clone();
            },
            UserChoice::Custom(url) => {
                config.ws_url = url;
            }
        }

        println!("Starting engine with: {}", config.ws_url);

        match run_session(config.clone()).await {
            Ok(_) => {
                // Normal exit (unlikely given loop)
                info!("Session ended normally.");
            },
            Err(e) => {
                error!("Session crashed: {}. Returning to menu in 2s...", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }

    Ok(())
}

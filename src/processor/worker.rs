use std::sync::Arc;
use tokio::sync::{mpsc::{UnboundedReceiver, Sender}, broadcast, Semaphore};
use tracing::{info, debug, error, warn};

use crate::http::race_client::RaceClient;
use crate::processor::transaction::parse_transaction;
use crate::processor::swap_detector::{detect_swap, SwapEvent};
use crate::processor::cache::DedupCache;
use crate::error::Result;
use crate::analytics::stats::Stats;
use crate::utils::time::{now_instant, elapsed_ms};

pub struct Worker {
    race_client: RaceClient,
    cache: DedupCache,
    rx_signatures: UnboundedReceiver<String>,
    tx_swaps: Sender<SwapEvent>,
    target_wallet: String,
    stats: Arc<Stats>,
    semaphore: Arc<Semaphore>,
}

impl Worker {
    pub fn new(
        race_client: RaceClient,
        rx_signatures: UnboundedReceiver<String>,
        tx_swaps: Sender<SwapEvent>,
        target_wallet: String,
        stats: Arc<Stats>,
        max_workers: usize,
    ) -> Self {
        Self {
            race_client,
            cache: DedupCache::new(60_000), // 1 minute deduplication window
            rx_signatures,
            tx_swaps,
            target_wallet,
            stats,
            semaphore: Arc::new(Semaphore::new(max_workers)),
        }
    }

    pub async fn run(mut self, mut shutdown: broadcast::Receiver<()>) {
        info!("Worker started. Waiting for signatures...");

        // Background cleanup task for cache
        let cache_clone = self.cache.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                cache_clone.cleanup();
            }
        });

        loop {
            tokio::select! {
                signature_opt = self.rx_signatures.recv() => {
                    match signature_opt {
                        Some(signature) => {
                            let client = self.race_client.clone();
                            let tx_swaps = self.tx_swaps.clone();
                            let cache = self.cache.clone();
                            let target_wallet = self.target_wallet.clone();
                            let stats = self.stats.clone();

                            // Acquire permit
                            let permit = match self.semaphore.clone().acquire_owned().await {
                                Ok(p) => p,
                                Err(_) => {
                                    error!("Semaphore closed");
                                    break;
                                }
                            };

                            // Spawn task for signature processing
                            tokio::spawn(async move {
                                // Permit is held until this task completes and permit is dropped
                                let _permit = permit;
                                let _start_time = now_instant();
                                if let Err(e) = process_signature(client, cache, signature, tx_swaps, target_wallet, stats.clone()).await {
                                    warn!("Error processing signature: {}", e);
                                }
                            });
                        }
                        None => {
                            info!("Signature channel closed.");
                            break;
                        }
                    }
                }
                _ = shutdown.recv() => {
                    info!("Worker shutting down...");
                    break;
                }
            }
        }

        info!("Worker stopped.");
    }
}

async fn process_signature(
    client: RaceClient,
    cache: DedupCache,
    signature: String,
    tx_swaps: Sender<SwapEvent>,
    target_wallet: String,
    stats: Arc<Stats>,
) -> Result<()> {
    // 1. Deduplication
    if !cache.check_and_insert(&signature) {
        debug!("Signature {} already processed (cache hit)", signature);
        return Ok(());
    }

    let start_time = now_instant();
    debug!("Processing signature: {}", signature);

    // 2. Fetch Transaction with Retry (to handle race where signature appears before index)
    let mut tx_value = serde_json::Value::Null;
    let mut attempts = 0;
    const MAX_RETRIES: u32 = 10;

    while attempts < MAX_RETRIES {
        match client.get_transaction(&signature).await {
            Ok(val) => {
                // If val is null, it means RPC returned success but no data (transaction not found yet)
                if !val.is_null() {
                    tx_value = val;
                    break;
                }
                debug!("Transaction {} not found yet (attempt {}/{})", signature, attempts + 1, MAX_RETRIES);
            }
            Err(e) => {
                debug!("Failed to fetch transaction {} (attempt {}/{}): {}", signature, attempts + 1, MAX_RETRIES, e);
            }
        }

        attempts += 1;
        if attempts < MAX_RETRIES {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }

    if tx_value.is_null() {
        return Err(crate::error::AppError::Parse(format!("Transaction {} not found after {} retries", signature, MAX_RETRIES)));
    }

    // 3. Parse Transaction
    let parsed_tx = parse_transaction(&signature, &tx_value)?;

    // 4. Detect Swap
    if let Some(swap) = detect_swap(&parsed_tx, &target_wallet)? {
        stats.inc_swaps_detected();
        info!("Swap detected: {} {} {} for {} SOL (Price: {})",
             if matches!(swap.direction, crate::processor::swap_detector::SwapDirection::Buy) { "Bought" } else { "Sold" },
             swap.amount_out, // Amount of token/SOL depending on direction
             swap.mint,
             swap.amount_in, // This logic in formatting might be confusing, let's just log struct
             swap.price
        );
        info!("Swap details: {:?}", swap);

        // 5. Send to output
        if let Err(e) = tx_swaps.send(swap).await {
            error!("Failed to send swap event: {}", e);
        }
    } else {
        debug!("No swap detected for {}", signature);
    }

    stats.update_processing_latency(elapsed_ms(start_time));

    Ok(())
}

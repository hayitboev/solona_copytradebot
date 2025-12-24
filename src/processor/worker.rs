use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, Sender};
use tracing::{info, debug, error, warn};

use crate::http::race_client::RaceClient;
use crate::processor::transaction::parse_transaction;
use crate::processor::swap_detector::{detect_swap, SwapEvent};
use crate::processor::cache::DedupCache;
use crate::error::Result;

pub struct Worker {
    race_client: RaceClient,
    cache: DedupCache,
    rx_signatures: UnboundedReceiver<String>,
    tx_swaps: Sender<SwapEvent>,
    target_wallet: String,
}

impl Worker {
    pub fn new(
        race_client: RaceClient,
        rx_signatures: UnboundedReceiver<String>,
        tx_swaps: Sender<SwapEvent>,
        target_wallet: String,
    ) -> Self {
        Self {
            race_client,
            cache: DedupCache::new(60_000), // 1 minute deduplication window
            rx_signatures,
            tx_swaps,
            target_wallet,
        }
    }

    pub async fn run(mut self) {
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

        while let Some(signature) = self.rx_signatures.recv().await {
            let client = self.race_client.clone();
            let tx_swaps = self.tx_swaps.clone();
            let cache = self.cache.clone();
            let target_wallet = self.target_wallet.clone();

            // Spawn a task for each signature to handle concurrency
            tokio::spawn(async move {
                if let Err(e) = process_signature(client, cache, signature, tx_swaps, target_wallet).await {
                    // Only log errors that are not "already processed" if we treated that as error
                    // But we handle cache check inside.
                    warn!("Error processing signature: {}", e);
                }
            });
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
) -> Result<()> {
    // 1. Deduplication
    if !cache.check_and_insert(&signature) {
        debug!("Signature {} already processed (cache hit)", signature);
        return Ok(());
    }

    debug!("Processing signature: {}", signature);

    // 2. Fetch Transaction
    // Phase 1 RaceClient::get_transaction returns serde_json::Value
    let tx_value = client.get_transaction(&signature).await?;

    // 3. Parse Transaction
    let parsed_tx = parse_transaction(&signature, &tx_value)?;

    // 4. Detect Swap
    if let Some(swap) = detect_swap(&parsed_tx, &target_wallet)? {
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

    Ok(())
}

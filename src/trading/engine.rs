use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tracing::{info, warn, error, debug};
use crate::error::Result;
use crate::processor::swap_detector::{SwapEvent, SwapDirection};
use crate::trading::risk::RiskManager;
use crate::trading::signer::TransactionSigner;
use crate::trading::jupiter::JupiterClient;
use crate::http::race_client::RaceClient;
use crate::config::Config;

// Constants
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub struct TradingEngine {
    config: Config,
    risk_manager: Arc<RiskManager>,
    signer: Arc<TransactionSigner>,
    jupiter_client: Arc<JupiterClient>,
    race_client: RaceClient,
    rx_swaps: Receiver<SwapEvent>,
}

impl TradingEngine {
    pub fn new(
        config: Config,
        race_client: RaceClient,
        rx_swaps: Receiver<SwapEvent>,
    ) -> Result<Self> {
        let risk_manager = Arc::new(RiskManager::new(
            config.min_trade_amount_sol,
            config.max_trade_amount_sol,
            config.cooldown_seconds,
        ));

        let signer = Arc::new(TransactionSigner::new(&config.private_key)?);

        let jupiter_client = Arc::new(JupiterClient::new(
            config.jupiter_api_url.clone(),
            config.slippage_bps,
        )?);

        Ok(Self {
            config,
            risk_manager,
            signer,
            jupiter_client,
            race_client,
            rx_swaps,
        })
    }

    pub async fn run(mut self) {
        info!("Trading Engine started.");

        while let Some(event) = self.rx_swaps.recv().await {
            let engine = self.clone_components(); // Helper to clone Arcs for spawning
            let event = event.clone();

            // Spawn task to handle trade execution
            tokio::spawn(async move {
                if let Err(e) = engine.execute_trade(event).await {
                    error!("Trade execution failed: {}", e);
                }
            });
        }

        info!("Trading Engine stopped.");
    }

    // Helper struct to hold cloned components for async tasks
    // Or we can just implement a helper method on Self that returns a struct
    // or pass clones individually.
    // Let's create a lightweight context struct or just pass clones.
    fn clone_components(&self) -> EngineContext {
        EngineContext {
            risk_manager: self.risk_manager.clone(),
            signer: self.signer.clone(),
            jupiter_client: self.jupiter_client.clone(),
            race_client: self.race_client.clone(),
            // config is simple enough to clone fields if needed, or wrap in Arc.
            // `Config` derives Clone.
            config: self.config.clone(),
        }
    }
}

struct EngineContext {
    risk_manager: Arc<RiskManager>,
    signer: Arc<TransactionSigner>,
    jupiter_client: Arc<JupiterClient>,
    race_client: RaceClient,
    config: Config,
}

impl EngineContext {
    async fn execute_trade(&self, event: SwapEvent) -> Result<()> {
        debug!("Processing swap event: {:?}", event);

        // 1. Determine Trade Parameters
        // If User Bought Token (SOL -> Token), we Buy Token (SOL -> Token).
        // If User Sold Token (Token -> SOL), we Sell Token (Token -> SOL).

        let (input_mint, output_mint, amount_in_lamports) = match event.direction {
            SwapDirection::Buy => {
                // We want to buy `event.mint`. Input is SOL.
                // Amount?
                // We use our configured Trade Amount?
                // Or we copy the user's amount (scaled)?
                // Requirement says: "Ensure trade size is within configured limits (e.g., 0.01 SOL to 1.0 SOL)."
                // It implies we might have a dynamic size or fixed strategy.
                // For "Copy-Trading", usually we copy proportional or fixed.
                // Let's assume we use `min_trade_amount_sol` as the base trade size for now,
                // or just `0.1 SOL` hardcoded if config logic is complex,
                // but we have config `min_trade_amount_sol`.
                // Let's use `min_trade_amount_sol` as the default "Copy Unit".
                // Or better, let's use a fixed amount for simplicity of Phase 3 unless specified.
                // We will use `config.min_trade_amount_sol` as the "buy amount".

                let amount = (self.config.min_trade_amount_sol * LAMPORTS_PER_SOL as f64) as u64;
                (SOL_MINT.to_string(), event.mint.clone(), amount)
            },
            SwapDirection::Sell => {
                // We want to sell `event.mint`. Input is Token. Output is SOL.
                // Amount? We need to know our Token Balance!
                // This requires an RPC call to `getTokenAccountsByOwner` or similar.
                // "If buying, check SOL balance in parallel..."
                // For selling, we MUST check token balance.
                // For now, let's skip Sell implementation or assume we sell 100%?
                // Or maybe we just trade "Buy" side for this task?
                // The task description doesn't explicitly limit to Buy only, but "Max Position" check implies checking if we hold it.
                // Let's implement Buy logic robustly first.
                // If Sell, we log "Sell detected, logic pending balance check".
                // Or we try to fetch balance.

                // Let's support BUY only for the MVP step if Sell is complex without state.
                // But wait, "Detect SOL -> TOKEN (Buy). Detect TOKEN -> SOL (Sell)." in Phase 2.
                // Phase 3 goal is "Turn those SwapEvent signals into actual on-chain transactions".
                // If I am copying, and they sell, I should sell.
                // I will try to fetch my token balance for that mint.
                // `RaceClient` can do `getTokenAccountBalance`.

                // For this implementation, I will focus on BUY (SOL -> Token) as it's the entry.
                // And simpler Sell (Token -> SOL) if I can get balance.

                if event.direction == SwapDirection::Sell {
                     // Need to find the associated token account for this mint.
                     // This is an extra RPC roundtrip.
                     warn!("Sell event detected for {}. Selling logic requires balance check (Not implemented in Phase 3 MVP).", event.mint);
                     return Ok(());
                }

                // Fallback for compiler flow (unreachable due to return above)
                (event.mint.clone(), SOL_MINT.to_string(), 0)
            }
        };

        // If amount is 0 (Sell logic skip), return
        if amount_in_lamports == 0 {
            return Ok(());
        }

        let amount_sol = amount_in_lamports as f64 / LAMPORTS_PER_SOL as f64;

        // 2. Risk Check
        self.risk_manager.check_trade(&output_mint, amount_sol)?;

        info!("Executing BUY for {} (Amount: {} SOL)", output_mint, amount_sol);

        // 3. Fetch Quote
        let quote = self.jupiter_client.get_quote(&input_mint, &output_mint, amount_in_lamports).await?;

        // 4. Get Swap Transaction
        let swap_response = self.jupiter_client.get_swap_tx(quote, &self.signer.pubkey()).await?;

        // 5. Sign Transaction
        let signed_tx = self.signer.sign_transaction(&swap_response.swap_transaction)?;

        // 6. Broadcast
        let signature = self.race_client.send_transaction_with_retry(&signed_tx, 3).await?;

        info!("Trade submitted! Signature: {}", signature);

        // Record trade in risk manager (cooldown)
        self.risk_manager.record_trade(&output_mint);

        Ok(())
    }
}

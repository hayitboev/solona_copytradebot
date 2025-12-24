use std::sync::Arc;
use tokio::sync::{mpsc::Receiver, broadcast};
use tracing::{info, warn, error, debug};
use crate::error::Result;
use crate::processor::swap_detector::{SwapEvent, SwapDirection};
use crate::trading::risk::RiskManager;
use crate::trading::signer::TransactionSigner;
use crate::trading::jupiter::JupiterClient;
use crate::http::race_client::RaceClient;
use crate::config::Config;
use crate::analytics::stats::Stats;
use crate::utils::time::{now_instant, elapsed_ms};
use crate::utils::token::{get_token_balance, get_decimals};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

// Constants
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub struct TradingEngine {
    config: Config,
    risk_manager: Arc<RiskManager>,
    signer: Arc<TransactionSigner>,
    jupiter_client: Arc<JupiterClient>,
    race_client: RaceClient,
    rpc_client: Arc<RpcClient>,
    rx_swaps: Receiver<SwapEvent>,
    stats: Arc<Stats>,
}

impl TradingEngine {
    pub fn new(
        config: Config,
        race_client: RaceClient,
        rx_swaps: Receiver<SwapEvent>,
        stats: Arc<Stats>,
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

        // Reuse one of the RPC endpoints for the RpcClient
        let rpc_url = config.rpc_endpoints.first()
            .ok_or_else(|| crate::error::AppError::Config(config::ConfigError::Message("No RPC endpoints".into())))?
            .clone();
        let rpc_client = Arc::new(RpcClient::new(rpc_url));

        Ok(Self {
            config,
            risk_manager,
            signer,
            jupiter_client,
            race_client,
            rpc_client,
            rx_swaps,
            stats,
        })
    }

    pub async fn run(mut self, mut shutdown: broadcast::Receiver<()>) {
        info!("Trading Engine started.");

        loop {
            tokio::select! {
                event_opt = self.rx_swaps.recv() => {
                    match event_opt {
                        Some(event) => {
                            let engine = self.clone_components(); // Helper to clone Arcs for spawning
                            let event = event.clone();

                            // Spawn task to handle trade execution
                            tokio::spawn(async move {
                                if let Err(e) = engine.execute_trade(event).await {
                                    engine.stats.inc_failed_trades();
                                    error!("Trade execution failed: {}", e);
                                }
                            });
                        },
                        None => {
                            info!("Swap event channel closed.");
                            break;
                        }
                    }
                }
                _ = shutdown.recv() => {
                    info!("Trading Engine shutting down...");
                    break;
                }
            }
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
            rpc_client: self.rpc_client.clone(),
            // config is simple enough to clone fields if needed, or wrap in Arc.
            // `Config` derives Clone.
            config: self.config.clone(),
            stats: self.stats.clone(),
        }
    }
}

struct EngineContext {
    risk_manager: Arc<RiskManager>,
    signer: Arc<TransactionSigner>,
    jupiter_client: Arc<JupiterClient>,
    race_client: RaceClient,
    rpc_client: Arc<RpcClient>,
    config: Config,
    stats: Arc<Stats>,
}

impl EngineContext {
    async fn execute_trade(&self, event: SwapEvent) -> Result<()> {
        let start_time = now_instant();
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
                // Determine our Token Balance
                let wallet_pubkey = Pubkey::from_str(&self.signer.pubkey())
                    .map_err(|e| crate::error::AppError::Parse(format!("Invalid wallet pubkey: {}", e)))?;
                let mint_pubkey = Pubkey::from_str(&event.mint)
                    .map_err(|e| crate::error::AppError::Parse(format!("Invalid mint pubkey: {}", e)))?;

                let balance = get_token_balance(&self.rpc_client, &wallet_pubkey, &mint_pubkey).await?;

                if balance == 0 {
                    warn!("Target sold {}, but our balance is 0. Skipping.", event.mint);
                    return Ok(());
                }

                // Sell 100%
                (event.mint.clone(), SOL_MINT.to_string(), balance)
            }
        };

        // If amount is 0 (Sell logic skip), return
        if amount_in_lamports == 0 {
            return Ok(());
        }

        // Calculate approximate SOL value for risk check
        let amount_sol_risk = if input_mint == SOL_MINT {
            // Buying with SOL
            amount_in_lamports as f64 / LAMPORTS_PER_SOL as f64
        } else {
            // Selling Token for SOL
            // We need to normalize token amount and estimated price
            // Price from event is SOL/Token
            let mint_pubkey = Pubkey::from_str(&input_mint)
                .map_err(|e| crate::error::AppError::Parse(format!("Invalid mint pubkey: {}", e)))?;
            let decimals = get_decimals(&self.rpc_client, &mint_pubkey).await?;
            let token_amount_norm = amount_in_lamports as f64 / 10f64.powi(decimals as i32);
            token_amount_norm * event.price
        };

        // 2. Risk Check
        self.risk_manager.check_trade(&output_mint, amount_sol_risk)?;

        info!("Executing BUY for {} (Approx Value: {} SOL)", output_mint, amount_sol_risk);

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
        // Always record the Token Mint involved (Buy: output, Sell: input/event.mint)
        // to prevent immediate re-entry/spam.
        self.risk_manager.record_trade(&event.mint);

        self.stats.inc_successful_trades();
        self.stats.update_trade_latency(elapsed_ms(start_time));

        Ok(())
    }
}

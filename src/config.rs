use serde::Deserialize;
use crate::error::{Result, AppError};
use std::env;
use std::sync::Arc;
use config::{Config as ConfigLoader, File, Environment};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    WebSocket,
    Grpc,
    Auto,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    // General
    pub log_level: String,
    
    // Wallet
    pub wallet_address: String,
    pub private_key: String, // Can be Base58 string

    // Transport
    pub transport_mode: TransportMode,
    pub ws_url: String, // Mapped from WEBSOCKET_URL or FAST_WS_ENDPOINT
    pub grpc_endpoint: Option<String>,

    // RPCs (Used for race client)
    pub rpc_endpoints: Vec<String>,
    
    // Jupiter
    pub jupiter_quote_url: String, // JUPITER_QUOTE_URL_PRIMARY
    pub jupiter_swap_url: String,  // JUPITER_SWAP_URL_PRIMARY
    pub jupiter_timeout: f64,
    pub jup_priority_level: String,
    pub jup_priority_max_lamports: u64,

    // Performance
    pub max_workers: usize,
    pub fast_mode: bool,
    pub http_rate_limit_max: u32,
    pub signature_poll_enabled: bool,
    pub signature_poll_interval: f64,

    // Trading & Risk
    pub buy_amount_sol: f64,
    pub mirror_buy_mode: bool,
    pub min_trade_amount_sol: f64, // Still used for clamping, mapped to MIRROR_MIN_SOL if mirror mode?
                                   // Or keep distinct. The .env has MIRROR_MIN_SOL.
                                   // Let's map .env MIRROR_MIN_SOL to this or add new fields.
    pub mirror_min_sol: f64,
    pub mirror_max_sol: f64,

    // Keep legacy for compatibility or mapping
    pub max_trade_amount_sol: f64, // Mapped to MIRROR_MAX_SOL or independent?
    pub slippage_bps: u16,
    pub cooldown_seconds: u64,

    pub auto_trade_enabled: bool,
    pub confirm_commitment: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        // Load .env file if it exists
        dotenv::dotenv().ok();

        // 1. Manually collect RPCs from new ENV pattern
        let mut collected_rpcs = Vec::new();
        let rpc_keys = [
            "RPC_URL", "FAST_RPC_ENDPOINT",
            "HELIUS_HTTP", "SYNDICA_HTTP", "ALCHEMY_SOL_HTTP", "QN_HTTP",
            "RPC_URL_FALLBACK1", "RPC_URL_FALLBACK2", "RPC_URL_FALLBACK3"
        ];
        
        for key in rpc_keys {
            if let Ok(val) = env::var(key) {
                if !val.trim().is_empty() {
                    collected_rpcs.push(val.trim().to_string());
                }
            }
        }
        
        // 2. Determine WebSocket URL (Prefer FAST_WS_ENDPOINT, then WEBSOCKET_URL)
        let ws_url = env::var("FAST_WS_ENDPOINT")
            .or_else(|_| env::var("WEBSOCKET_URL"))
            .unwrap_or_else(|_| "wss://api.mainnet-beta.solana.com".to_string());

        // 3. Build Config using `config` crate for standard loading,
        // but we might need to manually map some env vars to struct fields
        // if names don't match exactly.
        
        // Let's use manual construction for clarity given the specific .env mapping requirement
        // or helper builder.
        
        let wallet_address = env::var("WALLET_ADDRESS").expect("WALLET_ADDRESS must be set");
        // PRIVATE_KEY_BYTES from env is Base58 string
        let private_key = env::var("PRIVATE_KEY_BYTES").expect("PRIVATE_KEY_BYTES must be set");

        let jupiter_quote_url = env::var("JUPITER_QUOTE_URL_PRIMARY").unwrap_or_else(|_| "https://api.jup.ag/swap/v1/quote".to_string());
        let jupiter_swap_url = env::var("JUPITER_SWAP_URL_PRIMARY").unwrap_or_else(|_| "https://api.jup.ag/swap/v1/swap".to_string());
        let jupiter_timeout = env::var("JUPITER_TIMEOUT").unwrap_or("1.0".to_string()).parse().unwrap_or(1.0);
        let jup_priority_level = env::var("JUP_PRIORITY_LEVEL").unwrap_or_else(|_| "veryHigh".to_string());
        let jup_priority_max_lamports = env::var("JUP_PRIORITY_MAX_LAMPORTS").unwrap_or("10000000".to_string()).parse().unwrap_or(10_000_000);

        let max_workers = env::var("MAX_WORKERS").unwrap_or("4".to_string()).parse().unwrap_or(4);
        let fast_mode = env::var("FAST_MODE").unwrap_or("false".to_string()).parse().unwrap_or(false);
        let http_rate_limit_max = env::var("HTTP_RATE_LIMIT_MAX").unwrap_or("100".to_string()).parse().unwrap_or(100);
        let signature_poll_enabled = env::var("SIGNATURE_POLL_ENABLED").unwrap_or("false".to_string()).parse().unwrap_or(false);
        let signature_poll_interval = env::var("SIGNATURE_POLL_INTERVAL").unwrap_or("0.1".to_string()).parse().unwrap_or(0.1);

        let buy_amount_sol = env::var("BUY_AMOUNT_SOL").unwrap_or("0.01".to_string()).parse().unwrap_or(0.01);
        let mirror_buy_mode = env::var("MIRROR_BUY_MODE").unwrap_or("false".to_string()).parse().unwrap_or(false);
        let mirror_min_sol = env::var("MIRROR_MIN_SOL").unwrap_or("0.001".to_string()).parse().unwrap_or(0.001);
        let mirror_max_sol = env::var("MIRROR_MAX_SOL").unwrap_or("1.0".to_string()).parse().unwrap_or(1.0);
        let auto_trade_enabled = env::var("AUTO_TRADE_ENABLED").unwrap_or("true".to_string()).parse().unwrap_or(true);
        let confirm_commitment = env::var("CONFIRM_COMMITMENT").unwrap_or("confirmed".to_string());
        
        let slippage_bps = 50; // Default or add to env if needed (not in provided list)
        let cooldown_seconds = 60; // Default

        Ok(Self {
            log_level: "info".to_string(),
            wallet_address,
            private_key,
            transport_mode: TransportMode::Auto,
            ws_url,
            grpc_endpoint: None,
            rpc_endpoints: collected_rpcs,
            jupiter_quote_url,
            jupiter_swap_url,
            // jupiter_api_url removed, ensure no other file uses it (already updated engine.rs)
            jupiter_timeout,
            jup_priority_level,
            jup_priority_max_lamports,
            max_workers,
            fast_mode,
            http_rate_limit_max,
            signature_poll_enabled,
            signature_poll_interval,
            buy_amount_sol,
            mirror_buy_mode,
            min_trade_amount_sol: mirror_min_sol, // Mapping for compatibility
            max_trade_amount_sol: mirror_max_sol, // Mapping for compatibility
            mirror_min_sol,
            mirror_max_sol,
            slippage_bps,
            cooldown_seconds,
            auto_trade_enabled,
            confirm_commitment,
        })
    }
}

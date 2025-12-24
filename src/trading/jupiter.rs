use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use crate::error::{Result, AppError};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct JupiterClient {
    client: Client,
    base_url: String,
    slippage_bps: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub input_mint: String,
    pub output_mint: String,
    pub amount: u64, // Lamports
    pub slippage_bps: u16,
    pub only_direct_routes: bool,
    pub as_legacy_transaction: bool, // We want versioned if possible, but default is false (Versioned) usually?
                                     // Actually Jupiter v6 defaults to whatever.
                                     // Let's stick to defaults or specify if needed.
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    pub input_mint: String,
    pub in_amount: String,
    pub output_mint: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub swap_mode: String,
    pub slippage_bps: u64,
    pub price_impact_pct: String,
    pub route_plan: Vec<serde_json::Value>,
    // There are more fields but we just need the ID/Response to pass to Swap
    // Actually Swap API takes the entire Quote Response object or parts of it?
    // Jupiter v6 /swap takes:
    // {
    //   "userPublicKey": "...",
    //   "quoteResponse": { ... }
    // }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest<'a> {
    pub user_public_key: &'a str,
    pub quote_response: QuoteResponse,
    // Optional config
    pub wrap_and_unwrap_sol: bool,
    // pub use_shared_accounts: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapResponse {
    pub swap_transaction: String, // Base64 encoded transaction
    pub last_valid_block_height: u64,
}

impl JupiterClient {
    pub fn new(base_url: String, slippage_bps: u16) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(5000)) // 5s timeout
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(AppError::Http)?;

        Ok(Self {
            client,
            base_url,
            slippage_bps,
        })
    }

    pub async fn get_quote(&self, input_mint: &str, output_mint: &str, amount: u64) -> Result<QuoteResponse> {
        let url = format!("{}/quote", self.base_url);

        // Construct query params manually or use serde
        let params = [
            ("inputMint", input_mint),
            ("outputMint", output_mint),
            ("amount", &amount.to_string()),
            ("slippageBps", &self.slippage_bps.to_string()),
        ];

        let start = std::time::Instant::now();
        let response = self.client.get(&url)
            .query(&params)
            .send()
            .await
            .map_err(AppError::Http)?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Trading(format!("Jupiter Quote API error: {}", error_text)));
        }

        let quote: QuoteResponse = response.json().await.map_err(AppError::Http)?;
        debug!("Fetched quote in {:?}ms", start.elapsed().as_millis());

        Ok(quote)
    }

    pub async fn get_swap_tx(&self, quote: QuoteResponse, user_public_key: &str) -> Result<SwapResponse> {
        let url = format!("{}/swap", self.base_url);

        let request = SwapRequest {
            user_public_key,
            quote_response: quote,
            wrap_and_unwrap_sol: true,
        };

        let start = std::time::Instant::now();
        let response = self.client.post(&url)
            .json(&request)
            .send()
            .await
            .map_err(AppError::Http)?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(AppError::Trading(format!("Jupiter Swap API error: {}", error_text)));
        }

        let swap_response: SwapResponse = response.json().await.map_err(AppError::Http)?;
        debug!("Fetched swap tx in {:?}ms", start.elapsed().as_millis());

        Ok(swap_response)
    }
}

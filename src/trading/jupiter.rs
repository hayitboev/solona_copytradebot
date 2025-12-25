use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use crate::error::{Result, AppError};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct JupiterClient {
    client: Client,
    quote_url: String,
    swap_url: String,
    slippage_bps: u16,
    priority_level: String, // "veryHigh", "high", etc.
    priority_max_lamports: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub input_mint: String,
    pub output_mint: String,
    pub amount: u64, // Lamports
    pub slippage_bps: u16,
    pub only_direct_routes: bool,
    pub as_legacy_transaction: bool,
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
    // For V1/V6 compatibility, this struct usually works for V6.
    // V1 might differ. If V1 is actually V6 (which is common for "v1/quote" endpoint urls in aggregators), this is fine.
    // If it's a completely different schema, this might break.
    // Given Jupiter V6 is the standard, we assume the URL just points to a V6-compatible endpoint
    // or the user calls it V1 but it returns standard Quote response.
    // To be safe, we allow extra fields to be ignored (serde default behavior unless deny_unknown_fields).
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest<'a> {
    pub user_public_key: &'a str,
    pub quote_response: QuoteResponse,
    pub wrap_and_unwrap_sol: bool,
    // Priority Fee configuration
    // Jupiter API allows `computeUnitPriceMicroLamports` (int or "auto") OR `prioritizationFeeLamports` ("auto" or int)
    // We will use `prioritizationFeeLamports` for V1/V6 compatibility if instructed,
    // but the prompt says: "ensure the prioritizationFeeLamports or computeUnitPriceMicroLamports is set according to JUP_PRIORITY_LEVEL=veryHigh or JUP_PRIORITY_MAX_LAMPORTS."
    // We will use `dynamicComputeUnitLimit: true` and `prioritizationFeeLamports`.

    // Actually, recent Jupiter API uses `computeUnitPriceMicroLamports`.
    // If level is provided, we might need a specific structure:
    // "prioritizationFeeLamports": { "priorityLevelWithMaxLamports": { "priorityLevel": "veryHigh", "maxLamports": 10000000 } }
    // OR "auto".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prioritization_fee_lamports: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_unit_price_micro_lamports: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapResponse {
    pub swap_transaction: String, // Base64 encoded transaction
    pub last_valid_block_height: u64,
}

impl JupiterClient {
    pub fn new(
        quote_url: String,
        swap_url: String,
        slippage_bps: u16,
        priority_level: String,
        priority_max_lamports: u64,
        timeout_secs: f64
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis((timeout_secs * 1000.0) as u64))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(20)
            .build()
            .map_err(AppError::Http)?;

        Ok(Self {
            client,
            quote_url,
            swap_url,
            slippage_bps,
            priority_level,
            priority_max_lamports,
        })
    }

    pub async fn get_quote(&self, input_mint: &str, output_mint: &str, amount: u64) -> Result<QuoteResponse> {
        let url = &self.quote_url;

        // Construct query params
        // V1/V6 common params
        let params = [
            ("inputMint", input_mint),
            ("outputMint", output_mint),
            ("amount", &amount.to_string()),
            ("slippageBps", &self.slippage_bps.to_string()),
            // Add maxAccounts if needed for V1 compatibility? usually not required for basic swap
        ];

        let start = std::time::Instant::now();
        let response = self.client.get(url)
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
        let url = &self.swap_url;

        // Construct Priority Fee Config
        // Strategy: Use `prioritizationFeeLamports` object with `priorityLevelWithMaxLamports` if level is set.
        // Otherwise default to something else.

        let priority_config = serde_json::json!({
            "priorityLevelWithMaxLamports": {
                "priorityLevel": self.priority_level,
                "maxLamports": self.priority_max_lamports
            }
        });

        let request = SwapRequest {
            user_public_key,
            quote_response: quote,
            wrap_and_unwrap_sol: true,
            // We use prioritizationFeeLamports for the sophisticated strategy
            prioritization_fee_lamports: Some(priority_config),
            compute_unit_price_micro_lamports: None,
        };

        let start = std::time::Instant::now();
        let response = self.client.post(url)
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

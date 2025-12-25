use std::time::Duration;
use futures_util::future::select_ok;
use futures_util::FutureExt;
use reqwest::Client;
use serde_json::Value;
use tracing::{warn, error};
use std::future::Future;

use crate::error::{AppError, Result};
use crate::http::pool::create_http_client;
use crate::http::rate_limiter::RateLimiter;

#[derive(Clone)]
pub struct RaceClient {
    client: Client,
    rpc_endpoints: Vec<String>,
    limiter: RateLimiter,
}

impl RaceClient {
    pub fn new(rpc_endpoints: Vec<String>) -> Result<Self> {
        if rpc_endpoints.is_empty() {
            return Err(AppError::Init("No RPC endpoints provided".into()));
        }

        let client = create_http_client()?;
        // Allow 50 concurrent requests globally for now
        let limiter = RateLimiter::new(50); 

        Ok(Self {
            client,
            rpc_endpoints,
            limiter,
        })
    }

    /// Race a specific logic closure against all endpoints.
    /// The closure `f` receives (client, url) and returns a Future.
    async fn race<F, Fut, T>(&self, f: F) -> Result<T> 
    where
        F: Fn(Client, String) -> Fut + Send + Sync,
        Fut: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let mut futures = Vec::with_capacity(self.rpc_endpoints.len());
        
        // Prepare futures
        for url in &self.rpc_endpoints {
            let client = self.client.clone();
            let url = url.clone();
            // We need to reference f, but f is a closure that returns a future.
            // Since f is Fn (not FnOnce), we can call it multiple times.
            // But we need to call it inside the loop to get the future.

            // However, `async move` block captures `f`.
            // If we move `f` into the async block, we can only do it once if `f` is not Copy/Clone.
            // But we don't need to move `f` into the async block if we call `f` HERE (synchronously) and await the result inside?
            // `f` returns `Fut`. `Fut` is a Future.

            let fut = f(client, url);
            
            // We pin the future box to satisfy select_ok requirements
            let future = async move {
                fut.await
            }.boxed();
            
            futures.push(future);
        }

        // Run the race
        match select_ok(futures).await {
            Ok((result, _remaining)) => {
                // We could cancel remaining here, strictly they are dropped.
                Ok(result)
            }
            Err(e) => {
                // e is the last error, assuming all failed
                error!("All RPC endpoints failed. Last error: {}", e);
                Err(e)
            }
        }
    }

    /// Generic RPC JSON-RPC 2.0 Call
    pub async fn rpc_call(&self, method: &str, params: Value) -> Result<Value> {
        let _params_str = params.to_string(); // serialization for potential debug
        let method = method.to_string();

        let _permit = self.limiter.acquire().await;

        self.race(move |client, url| {
            let method = method.clone();
            let params = params.clone();
            
            async move {
                let request_body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": method,
                    "params": params
                });

                let response = client.post(&url)
                    .json(&request_body)
                    .send()
                    .await
                    .map_err(|e| AppError::Rpc(format!("Reqwest error: {}", e)))?;

                let status = response.status();
                if !status.is_success() {
                    return Err(AppError::Rpc(format!("HTTP Error: {}", status)));
                }

                let bytes = response.bytes().await
                    .map_err(|e| AppError::Rpc(format!("Body error: {}", e)))?;
                
                // Zero-copy optimization candidates later, for now parse Value
                let json: Value = serde_json::from_slice(&bytes)
                    .map_err(|e| AppError::Parse(format!("JSON error: {}", e)))?;

                if let Some(error) = json.get("error") {
                    return Err(AppError::Rpc(format!("RPC Error: {}", error)));
                }

                Ok(json["result"].clone())
            }
        }).await
    }

    /// Optimized for sending transactions (Base64 encoded)
    pub async fn send_transaction(&self, base64_tx: &str) -> Result<String> {
        let params = serde_json::json!([
            base64_tx,
            {
                "encoding": "base64",
                "skipPreflight": true, // Critical for speed
                "maxRetries": 0        // We handle retries or let the network handle it
            }
        ]);

        // Returns the signature string
        let result = self.rpc_call("sendTransaction", params).await?;
        
        result.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::Parse("sendTransaction result is not a string".into()))
    }

    /// Fetch transaction details (for verification/parsing)
    pub async fn get_transaction(&self, signature: &str) -> Result<Value> {
        let params = serde_json::json!([
            signature,
            {
                "encoding": "jsonParsed",
                "maxSupportedTransactionVersion": 0
            }
        ]);

        self.rpc_call("getTransaction", params).await
    }
    
    // Retry wrapper
    pub async fn send_transaction_with_retry(&self, base64_tx: &str, retries: u32) -> Result<String> {
        let mut attempt = 0;
        loop {
            match self.send_transaction(base64_tx).await {
                Ok(sig) => return Ok(sig),
                Err(e) => {
                    attempt += 1;
                    if attempt >= retries {
                        return Err(e);
                    }
                    warn!("Send tx failed, retrying ({}/{}): {}", attempt, retries, e);
                    // Exponential backoff: 50ms, 100ms, 200ms...
                    tokio::time::sleep(Duration::from_millis(50 * 2u64.pow(attempt - 1))).await;
                }
            }
        }
    }
}
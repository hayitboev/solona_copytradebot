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
    pub private_key: String, // Kept as string here, parsed when needed

    // Transport
    pub transport_mode: TransportMode,
    pub ws_url: String,
    pub grpc_endpoint: Option<String>,

    // RPCs (Comma separated list for race client)
    pub rpc_endpoints: Vec<String>,
    
    // Trading
    pub jupiter_api_url: String,
    pub max_workers: usize,
}

impl Config {
    pub fn load() -> Result<Self> {
        // Load .env file if it exists
        dotenv::dotenv().ok();

        let builder = ConfigLoader::builder()
            // Set defaults
            .set_default("log_level", "info")?
            .set_default("transport_mode", "auto")?
            .set_default("max_workers", 4)?
            .set_default("jupiter_api_url", "https://quote-api.jup.ag/v6")?
            
            // Load from environment variables (prefixed with SOL_ if needed, or direct)
            .add_source(Environment::default().separator("__"));

        // We can look for a config file, but strictly using ENV as per spec
        
        let config_store = builder.build()?;
        
        // Manual extraction to handle complex types like Vec<String> from env string if needed
        // But 'config' crate handles array in env vars usually via specific syntax (RPC_ENDPOINTS__0)
        // For simplicity in .env, we often use a single string and split it. 
        // Let's assume standard deserialization first.
        
        // Helper to parse comma-separated RPCs if simple env var is used
        let rpc_str = env::var("RPC_ENDPOINTS").unwrap_or_default();
        let rpc_endpoints: Vec<String> = rpc_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        
        if rpc_endpoints.is_empty() {
            return Err(AppError::Init("No RPC_ENDPOINTS provided".to_string()));
        }

        // Re-construct with overrides if necessary, or just deserialize what we can
        // and manually fill the rest. 
        // For strictness, let's try direct deserialization with a fallback for the vector.

        let mut s: Config = config_store.try_deserialize().map_err(|e| 
             AppError::Config(e)
        )?;
        
        // Override the empty vec with our manually parsed one if the automatic one failed 
        // or if we rely on the CSV string approach common in .env
        if s.rpc_endpoints.is_empty() {
             s.rpc_endpoints = rpc_endpoints;
        }

        Ok(s)
    }
}
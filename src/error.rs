use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] config::ConfigError),

    #[error("Environment variable error: {0}")]
    Env(#[from] std::env::VarError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Parsing error: {0}")]
    Parse(String),

    #[error("Solana SDK error: {0}")]
    Solana(#[from] solana_sdk::pubkey::ParsePubkeyError),

    #[error("Trading error: {0}")]
    Trading(String),
    
    #[error("Initialization error: {0}")]
    Init(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::error::Result;

#[async_trait]
pub trait Transport: Send + Sync {
    /// Connect and start the background event loop
    async fn connect(&self) -> Result<()>;

    /// Subscribe to logs for a specific target (usually wallet address)
    async fn subscribe_logs(&self, mention: &str) -> Result<()>;

    /// Get the channel receiver for transaction signatures
    /// Returns a broadcast or mpsc receiver
    fn get_signature_receiver(&self) -> mpsc::UnboundedReceiver<String>;

    /// Force a reconnection logic
    async fn reconnect(&self) -> Result<()>;
}
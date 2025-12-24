use reqwest::Client;
use std::time::Duration;
use crate::error::Result;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500); // 500ms strict timeout
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

pub fn create_http_client() -> Result<Client> {
    let client = Client::builder()
        .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
        .http2_prior_knowledge() // Assume HTTP/2 if possible (optional, depends on RPC)
        .https_only(true)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .pool_max_idle_per_host(10)
        .connect_timeout(CONNECTION_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    Ok(client)
}
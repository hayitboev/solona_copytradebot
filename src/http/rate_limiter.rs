use std::sync::Arc;
use tokio::sync::Semaphore;

/// A simple concurrency limiter to prevent flooding RPCs
#[derive(Debug, Clone)]
pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
}

impl RateLimiter {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    pub async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        // In a real high-perf scenario, we might want to handle the error or timeout 
        // acquiring a permit, but for now we wait.
        self.semaphore.acquire().await.expect("Semaphore closed")
    }
}
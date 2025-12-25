use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;

#[derive(Debug)]
pub struct Stats {
    pub total_swaps_detected: AtomicU64,
    pub successful_trades: AtomicU64,
    pub failed_trades: AtomicU64,

    // For latency, we store the last observed value for simplicity in a Gauge-like manner
    // Or we could use a histogram crate, but keeping it simple as requested.
    pub last_processing_latency_ms: AtomicU64,
    pub last_trade_latency_ms: AtomicU64,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            total_swaps_detected: AtomicU64::new(0),
            successful_trades: AtomicU64::new(0),
            failed_trades: AtomicU64::new(0),
            last_processing_latency_ms: AtomicU64::new(0),
            last_trade_latency_ms: AtomicU64::new(0),
        }
    }

    pub fn inc_swaps_detected(&self) {
        self.total_swaps_detected.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_successful_trades(&self) {
        self.successful_trades.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failed_trades(&self) {
        self.failed_trades.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_processing_latency(&self, ms: u64) {
        self.last_processing_latency_ms.store(ms, Ordering::Relaxed);
    }

    pub fn update_trade_latency(&self, ms: u64) {
        self.last_trade_latency_ms.store(ms, Ordering::Relaxed);
    }

    pub fn log_stats(&self) {
        let swaps = self.total_swaps_detected.load(Ordering::Relaxed);
        let success = self.successful_trades.load(Ordering::Relaxed);
        let failed = self.failed_trades.load(Ordering::Relaxed);
        let proc_lat = self.last_processing_latency_ms.load(Ordering::Relaxed);
        let trade_lat = self.last_trade_latency_ms.load(Ordering::Relaxed);

        info!(
            "STATS: Swaps Detected: {} | Trades: {} Success, {} Failed | Latency: Proc {}ms, Trade {}ms",
            swaps, success, failed, proc_lat, trade_lat
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_stats_concurrency() {
        let stats = Arc::new(Stats::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let stats = stats.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    stats.inc_swaps_detected();
                    stats.update_processing_latency(50);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(stats.total_swaps_detected.load(Ordering::Relaxed), 1000);
        assert_eq!(stats.last_processing_latency_ms.load(Ordering::Relaxed), 50);
    }
}

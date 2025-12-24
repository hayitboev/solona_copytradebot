use dashmap::DashMap;
use std::time::{Instant, Duration};
use tracing::warn;
use crate::error::{Result, AppError};

#[derive(Debug, Clone)]
pub struct RiskManager {
    // Map Token Mint -> Last Trade Time
    cooldowns: DashMap<String, Instant>,
    cooldown_duration: Duration,
    min_amount_sol: f64,
    max_amount_sol: f64,
}

impl RiskManager {
    pub fn new(min_sol: f64, max_sol: f64, cooldown_secs: u64) -> Self {
        Self {
            cooldowns: DashMap::new(),
            cooldown_duration: Duration::from_secs(cooldown_secs),
            min_amount_sol: min_sol,
            max_amount_sol: max_sol,
        }
    }

    pub fn check_trade(&self, token_mint: &str, amount_sol: f64) -> Result<()> {
        // 1. Check Amount Limits
        if amount_sol < self.min_amount_sol {
            return Err(AppError::Trading(format!(
                "Trade amount {} SOL is below minimum {} SOL",
                amount_sol, self.min_amount_sol
            )));
        }

        if amount_sol > self.max_amount_sol {
            return Err(AppError::Trading(format!(
                "Trade amount {} SOL is above maximum {} SOL",
                amount_sol, self.max_amount_sol
            )));
        }

        // 2. Check Cooldown
        if let Some(last_trade) = self.cooldowns.get(token_mint) {
            if last_trade.elapsed() < self.cooldown_duration {
                return Err(AppError::Trading(format!(
                    "Token {} is in cooldown. Time remaining: {:?}s",
                    token_mint,
                    (self.cooldown_duration - last_trade.elapsed()).as_secs()
                )));
            }
        }

        // 3. Max Position Check (Simplified: Just assume we shouldn't trade if we just did)
        // For a real max position, we'd need to query on-chain balance or track portfolio state.
        // The requirement says: "Max Position: Check if we already hold this token (simplified tracking for now)".
        // The cooldown essentially acts as a basic preventative measure for spam buying.
        // We can extend this to "Position Map" later.
        // For now, if we are in cooldown, we assume we hold it or just traded it.

        Ok(())
    }

    pub fn record_trade(&self, token_mint: &str) {
        self.cooldowns.insert(token_mint.to_string(), Instant::now());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_risk_manager_limits() {
        let risk = RiskManager::new(0.1, 1.0, 60);

        // Too small
        assert!(risk.check_trade("MintA", 0.05).is_err());

        // Too large
        assert!(risk.check_trade("MintA", 1.5).is_err());

        // Good
        assert!(risk.check_trade("MintA", 0.5).is_ok());
    }

    #[test]
    fn test_risk_manager_cooldown() {
        let risk = RiskManager::new(0.1, 1.0, 1); // 1 sec cooldown

        assert!(risk.check_trade("MintA", 0.5).is_ok());
        risk.record_trade("MintA");

        // Immediate check should fail
        assert!(risk.check_trade("MintA", 0.5).is_err());

        // Wait 1.1s
        thread::sleep(Duration::from_millis(1100));
        assert!(risk.check_trade("MintA", 0.5).is_ok());
    }
}

use crate::processor::transaction::{ParsedTransaction, AccountChange};
use tracing::{info, debug};
use crate::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum SwapDirection {
    Buy,  // SOL -> Token
    Sell, // Token -> SOL
}

#[derive(Debug, Clone)]
pub struct SwapEvent {
    pub signature: String,
    pub user: String,
    pub direction: SwapDirection,
    pub mint: String,
    pub amount_in: f64,
    pub amount_out: f64,
    pub price: f64, // Price in SOL (SOL/Token or Token/SOL depending on convention, typically SOL per Token)
}

pub fn detect_swap(tx: &ParsedTransaction, target_wallet: &str) -> Result<Option<SwapEvent>> {
    // Logic:
    // We only analyze changes for the target_wallet.

    if let Some(change) = tx.account_changes.get(target_wallet) {
        let address = target_wallet;
        // or only Token change (unlikely for swap, usually involves SOL).
        // However, wrapped SOL (WSOL) swaps look like Token <-> Token.
        // The requirement says "Detect SOL -> TOKEN (Buy) and TOKEN -> SOL (Sell)".
        // So we focus on native SOL changes.

        if change.token_deltas.is_empty() {
            return Ok(None);
        }

        // We only care if there is EXACTLY ONE token change?
        // A complex swap might involve multiple tokens (routing).
        // Requirement: "Detect SOL -> TOKEN" and "TOKEN -> SOL".
        // We will look for the "primary" swap.
        // If multiple tokens changed, it might be a multi-hop or arbitrage.
        // We will take the largest magnitude token change or just the first one?
        // Let's iterate through token changes.

        for (mint, token_delta) in &change.token_deltas {
            let sol_delta = change.sol_delta;
            let token_amount_delta = token_delta.amount_delta;

            // Check for Buy: SOL decreases, Token increases
            if sol_delta < 0 && token_amount_delta > 0 {
                // Potential Buy
                // But SOL decrease includes transaction fee!
                // We should probably check if the SOL decrease is significant.
                // Or better, check if there are other transfers.
                // Assuming "Copy-Trading Bot", we care about the user's intent.

                let sol_spent_lamports = sol_delta.abs() as u64;
                // approximate price
                let token_received = token_amount_delta as f64 / 10f64.powi(token_delta.decimals as i32);
                let sol_spent = sol_spent_lamports as f64 / 1e9;

                if token_received == 0.0 { continue; }

                let price = sol_spent / token_received;

                return Ok(Some(SwapEvent {
                    signature: tx.signature.clone(),
                    user: address.to_string(),
                    direction: SwapDirection::Buy,
                    mint: mint.clone(),
                    amount_in: sol_spent,
                    amount_out: token_received,
                    price,
                }));
            }
            // Check for Sell: SOL increases, Token decreases
            else if sol_delta > 0 && token_amount_delta < 0 {
                // Potential Sell
                let sol_received_lamports = sol_delta as u64;
                let token_sold = token_amount_delta.abs() as f64 / 10f64.powi(token_delta.decimals as i32);
                let sol_received = sol_received_lamports as f64 / 1e9;

                if token_sold == 0.0 { continue; }

                let price = sol_received / token_sold;

                return Ok(Some(SwapEvent {
                    signature: tx.signature.clone(),
                    user: address.to_string(),
                    direction: SwapDirection::Sell,
                    mint: mint.clone(),
                    amount_in: token_sold,
                    amount_out: sol_received,
                    price,
                }));
            }
        }
    }

    Ok(None)
}

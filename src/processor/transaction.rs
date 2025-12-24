use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct TokenDelta {
    pub mint: String,
    pub amount_delta: i128,
    pub decimals: u8,
}

#[derive(Debug, Clone, Default)]
pub struct AccountChange {
    pub sol_delta: i64,
    pub token_deltas: HashMap<String, TokenDelta>,
}

#[derive(Debug, Clone)]
pub struct ParsedTransaction {
    pub signature: String,
    pub account_changes: HashMap<String, AccountChange>,
}

pub fn parse_transaction(signature: &str, value: &Value) -> Result<ParsedTransaction> {
    // Check if value is null (transaction not found)
    if value.is_null() {
        return Err(AppError::Parse(format!("Transaction {} not found or pending", signature)));
    }

    let transaction = value.get("transaction")
        .ok_or_else(|| AppError::Parse("Missing transaction field".to_string()))?;

    let meta = value.get("meta")
        .ok_or_else(|| AppError::Parse("Missing meta field".to_string()))?;

    // 1. Build Account Map (Index -> Address)
    let message = transaction.get("message")
        .ok_or_else(|| AppError::Parse("Missing message field".to_string()))?;

    let mut account_keys: Vec<String> = Vec::new();

    // Handle "accountKeys"
    if let Some(keys) = message.get("accountKeys") {
        if let Some(arr) = keys.as_array() {
            for k in arr {
                // In jsonParsed, accountKeys can be an array of strings OR array of objects {pubkey, ...}
                if let Some(s) = k.as_str() {
                    account_keys.push(s.to_string());
                } else if let Some(obj) = k.as_object() {
                    if let Some(pk) = obj.get("pubkey").and_then(|v| v.as_str()) {
                        account_keys.push(pk.to_string());
                    }
                }
            }
        }
    }

    // Handle "loadedAddresses" (for versioned transactions)
    if let Some(loaded) = meta.get("loadedAddresses") {
        if let Some(writable) = loaded.get("writable").and_then(|v| v.as_array()) {
            for k in writable {
                if let Some(s) = k.as_str() {
                    account_keys.push(s.to_string());
                }
            }
        }
        if let Some(readonly) = loaded.get("readonly").and_then(|v| v.as_array()) {
            for k in readonly {
                if let Some(s) = k.as_str() {
                    account_keys.push(s.to_string());
                }
            }
        }
    }

    let mut changes: HashMap<String, AccountChange> = HashMap::new();

    // 2. SOL Balances
    let pre_balances = meta.get("preBalances").and_then(|v| v.as_array());
    let post_balances = meta.get("postBalances").and_then(|v| v.as_array());

    if let (Some(pre), Some(post)) = (pre_balances, post_balances) {
        for (i, (pre_val, post_val)) in pre.iter().zip(post.iter()).enumerate() {
            if i >= account_keys.len() {
                continue; // Should not happen if RPC is correct
            }
            let address = &account_keys[i];

            let pre_u64 = pre_val.as_u64().unwrap_or(0);
            let post_u64 = post_val.as_u64().unwrap_or(0);

            if pre_u64 != post_u64 {
                let delta = (post_u64 as i64) - (pre_u64 as i64);
                changes.entry(address.clone()).or_default().sol_delta = delta;
            }
        }
    }

    // 3. Token Balances
    // Helper to process token balances
    let process_token_balances = |key: &str| -> Result<HashMap<String, HashMap<String, (u64, u8)>>> {
        let mut map: HashMap<String, HashMap<String, (u64, u8)>> = HashMap::new(); // Address -> Mint -> (Amount, Decimals)

        if let Some(balances) = meta.get(key).and_then(|v| v.as_array()) {
            for b in balances {
                let index = b.get("accountIndex").and_then(|v| v.as_u64());
                let mint = b.get("mint").and_then(|v| v.as_str());
                let ui_token_amount = b.get("uiTokenAmount");

                if let (Some(idx), Some(mint_str), Some(amount_obj)) = (index, mint, ui_token_amount) {
                     if (idx as usize) < account_keys.len() {
                        let address = &account_keys[idx as usize];
                        let amount = amount_obj.get("amount").and_then(|v| v.as_str()).unwrap_or("0");
                        let decimals = amount_obj.get("decimals").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

                        let amount_u64 = amount.parse::<u64>().unwrap_or(0);

                        map.entry(address.clone())
                           .or_default()
                           .insert(mint_str.to_string(), (amount_u64, decimals));
                     }
                }
            }
        }
        Ok(map)
    };

    let pre_tokens = process_token_balances("preTokenBalances")?;
    let post_tokens = process_token_balances("postTokenBalances")?;

    // Calculate Token Deltas
    // Union of addresses involved
    let mut all_token_addresses: Vec<String> = pre_tokens.keys().cloned().collect();
    for k in post_tokens.keys() {
        if !pre_tokens.contains_key(k) {
            all_token_addresses.push(k.clone());
        }
    }

    for address in all_token_addresses {
        let empty_map = HashMap::new();
        let pre_map = pre_tokens.get(&address).unwrap_or(&empty_map);
        let post_map = post_tokens.get(&address).unwrap_or(&empty_map);

        // Union of mints for this address
        let mut all_mints: Vec<String> = pre_map.keys().cloned().collect();
        for k in post_map.keys() {
            if !pre_map.contains_key(k) {
                all_mints.push(k.clone());
            }
        }

        for mint in all_mints {
            let (pre_amt, pre_dec) = pre_map.get(&mint).copied().unwrap_or((0, 0));
            let (post_amt, post_dec) = post_map.get(&mint).copied().unwrap_or((0, 0));

            // Decimals should match, but if one is missing (0 balance), take the other.
            let decimals = if pre_dec != 0 { pre_dec } else { post_dec };

            if pre_amt != post_amt {
                let delta = (post_amt as i128) - (pre_amt as i128);

                changes.entry(address.clone())
                    .or_default()
                    .token_deltas
                    .insert(mint.clone(), TokenDelta {
                        mint, // move mint here
                        amount_delta: delta,
                        decimals,
                    });
            }
        }
    }

    Ok(ParsedTransaction {
        signature: signature.to_string(),
        account_changes: changes,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_transaction;
    use serde_json::json;

    #[test]
    fn test_parse_transaction_buy_sol_to_token() {
        // Mock a Buy Transaction (SOL -> USDC)
        // User (Account 0) pays SOL, receives USDC.
        // Pool (Account 1) receives SOL, pays USDC.

        let tx_json = json!({
            "transaction": {
                "message": {
                    "accountKeys": [
                        {"pubkey": "User111111111111111111111111111111111111111"},
                        {"pubkey": "Pool111111111111111111111111111111111111111"},
                        {"pubkey": "MintUSDC11111111111111111111111111111111111"}
                    ]
                }
            },
            "meta": {
                "preBalances": [1000000000u64, 5000000000u64, 0], // User has 1 SOL
                "postBalances": [ 900000000u64, 5100000000u64, 0], // User spent 0.1 SOL (ignoring fees for simplicity of test)

                "preTokenBalances": [
                    {
                        "accountIndex": 0,
                        "mint": "MintUSDC11111111111111111111111111111111111",
                        "uiTokenAmount": { "amount": "0", "decimals": 6 }
                    }
                ],
                "postTokenBalances": [
                    {
                        "accountIndex": 0,
                        "mint": "MintUSDC11111111111111111111111111111111111",
                        "uiTokenAmount": { "amount": "1000000", "decimals": 6 } // Received 1 USDC
                    }
                ]
            }
        });

        let parsed = parse_transaction("sig1", &tx_json).expect("Parse failed");

        let user = "User111111111111111111111111111111111111111";
        let change = parsed.account_changes.get(user).expect("User change not found");

        // SOL Change: 900M - 1000M = -100M
        assert_eq!(change.sol_delta, -100_000_000);

        // Token Change: 1M - 0 = +1M
        let token_delta = change.token_deltas.get("MintUSDC11111111111111111111111111111111111").expect("Token delta not found");
        assert_eq!(token_delta.amount_delta, 1_000_000);
        assert_eq!(token_delta.decimals, 6);
    }
}

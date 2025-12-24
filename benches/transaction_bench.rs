use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use solana_wallet_monitor::processor::transaction::{parse_transaction, ParsedTransaction};
use solana_wallet_monitor::processor::swap_detector::detect_swap;
use serde_json::json;

fn bench_parse_transaction(c: &mut Criterion) {
    let tx_json = json!({
        "transaction": {
            "message": {
                "accountKeys": [
                    "User111111111111111111111111111111111111111",
                    "Pool111111111111111111111111111111111111111",
                    "MintUSDC11111111111111111111111111111111111",
                    "SystemProgram111111111111111111111111111111",
                    "TokenProgram1111111111111111111111111111111"
                ]
            }
        },
        "meta": {
            "preBalances": [1000000000u64, 5000000000u64, 0, 0, 0],
            "postBalances": [ 900000000u64, 5100000000u64, 0, 0, 0],
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
                    "uiTokenAmount": { "amount": "1000000", "decimals": 6 }
                }
            ],
            "loadedAddresses": {
                "writable": [],
                "readonly": []
            }
        }
    });

    // Convert to Value ahead of time if we want to measure just parse_transaction,
    // but RaceClient returns Value, so that's fair.
    // However, RaceClient does serde_json::from_slice.
    // Let's benchmark the `parse_transaction` function which takes `&Value`.

    c.bench_function("parse_transaction", |b| b.iter(|| {
        parse_transaction(black_box("sig1"), black_box(&tx_json))
    }));
}

fn bench_detect_swap(c: &mut Criterion) {
    // Setup parsed transaction
    let mut account_changes = std::collections::HashMap::new();
    let mut token_deltas = std::collections::HashMap::new();
    token_deltas.insert("MintUSDC".to_string(), solana_wallet_monitor::processor::transaction::TokenDelta {
        mint: "MintUSDC".to_string(),
        amount_delta: 1000000,
        decimals: 6,
    });

    account_changes.insert("User1".to_string(), solana_wallet_monitor::processor::transaction::AccountChange {
        sol_delta: -100_000_000,
        token_deltas,
    });

    let parsed_tx = ParsedTransaction {
        signature: "sig1".to_string(),
        account_changes,
    };

    let target = "User1";

    c.bench_function("detect_swap", |b| b.iter(|| {
        detect_swap(black_box(&parsed_tx), black_box(target))
    }));
}

criterion_group!(benches, bench_parse_transaction, bench_detect_swap);
criterion_main!(benches);

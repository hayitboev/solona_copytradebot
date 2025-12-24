use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use solana_wallet_monitor::trading::risk::RiskManager;
use solana_wallet_monitor::trading::signer::TransactionSigner;
use solana_sdk::signature::Keypair;
use bs58;

fn bench_risk_check(c: &mut Criterion) {
    let risk = RiskManager::new(0.01, 1.0, 60);

    c.bench_function("risk_check", |b| b.iter(|| {
        risk.check_trade(black_box("MintA"), black_box(0.1))
    }));
}

fn bench_sign_transaction(c: &mut Criterion) {
    // Setup signer
    let keypair = Keypair::new();
    let priv_key = bs58::encode(keypair.to_bytes()).into_string();
    let signer = TransactionSigner::new(&priv_key).unwrap();

    // Create a dummy raw tx (base64 of serialized VersionedTransaction)
    // We need a valid serialized transaction structure.
    // Let's create one using solana-sdk.
    use solana_sdk::transaction::VersionedTransaction;
    use solana_sdk::message::VersionedMessage;
    use solana_sdk::message::v0::Message;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::instruction::Instruction;
    use solana_sdk::signer::Signer;

    let instructions = vec![Instruction::new_with_bytes(
        Pubkey::new_unique(),
        &[],
        vec![],
    )];

    let message = VersionedMessage::V0(Message::try_compile(
        &keypair.pubkey(),
        &instructions,
        &[],
        solana_sdk::hash::Hash::default()
    ).unwrap());

    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message,
    };

    let tx_bytes = bincode::serialize(&tx).unwrap();
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let tx_base64 = STANDARD.encode(tx_bytes);

    c.bench_function("sign_transaction", |b| b.iter(|| {
        signer.sign_transaction(black_box(&tx_base64))
    }));
}

criterion_group!(benches, bench_risk_check, bench_sign_transaction);
criterion_main!(benches);

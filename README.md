# Solana Wallet Monitor (copy-trading prototype)

A Rust prototype that monitors a Solana wallet, fetches transactions through HTTP RPC, detects simple SOL-to-token buys and token-to-SOL sells, and calculates parameters for a potential mirrored trade. The project uses Tokio for asynchronous processing and a WebSocket subscription for wallet activity.

**Current status:** The Jupiter quote, swap transaction, signing, and broadcast calls in `src/trading/engine.rs` are commented out. This version **does not submit copy trades**. Its "successful trades" counter records processed candidate events, not confirmed on-chain trades. The gRPC client is a stub; `src/main.rs` starts the WebSocket transport regardless of the `TRANSPORT_MODE` field.

## How it is organized

1. `src/transport/websocket/` subscribes to Solana `logsSubscribe` notifications for the monitored wallet and reconnects after disconnections.
2. `src/processor/` fetches and parses notified transactions, then detects basic buy or sell events from wallet balance changes.
3. `src/trading/` checks trade size and cooldown rules and calculates fixed or mirrored buy amounts. Network submission is currently disabled in source.
4. `src/analytics/` records processing metrics; `src/http/` provides HTTP RPC clients and rate limiting.

The swap detector uses a simplified balance-delta heuristic. It may miss complex or routed swaps and can mistake other balance changes for trades. The performance goals in [`техзадание.md`](техзадание.md) are design targets, **not measured results**.

## Configuration

The current loader in `src/config.rs` requires `WALLET_ADDRESS`, `PRIVATE_KEY_BYTES` (a Base58-encoded keypair), and at least one HTTP RPC endpoint such as `RPC_URL`. It reads `FAST_WS_ENDPOINT` or `WEBSOCKET_URL` for the WebSocket connection. See [`.env.example`](.env.example) for the variable names the current code recognizes.

> **Security notice:** An earlier commit contained a populated `.env` with a private key. The file has been removed from the current branch, but the secret remains exposed in Git history and may have been copied. **Never reuse that key.** If it controls a funded wallet, move assets to a newly generated wallet and rotate any exposed provider credentials. Rewriting Git history alone cannot make an exposed key safe.

## Run locally

Use a new, disposable wallet and a trusted RPC provider. Running the current code requires a private key even though transaction submission is commented out.

```bash
git clone https://github.com/hayitboev/solona_copytradebot.git
cd solona_copytradebot
cp .env.example .env
# Edit .env: set a monitored public wallet, a NEW test keypair, and an HTTP RPC URL.
cargo run
```

On Windows PowerShell, copy the file with `Copy-Item .env.example .env`. The CLI offers the configured WebSocket URL, a public fallback, or a custom URL. Press Ctrl+C to stop. The `.env` file is ignored by Git.

The commands above describe how the repository is configured; they have not been verified against a live Solana endpoint here. Do not treat `AUTO_TRADE_ENABLED=false` as a security control: the current engine parses that variable but does not use it to gate execution. Future transaction submission code must add and test an explicit gate before live use.

## Development

```bash
cargo test
cargo fmt --check
```

The repository also contains benchmark source files in `benches/`. There is no verified latency or profitability result documented here. Review the source and test behavior before treating the prototype as a trading system.

# roko-chain

On-chain client abstractions and blockchain primitives for Roko.

## What it does

Provides backend-agnostic traits (`ChainClient`, `ChainWallet`) for reading chain state and
submitting transactions. Ships mock test doubles and an optional Alloy-backed JSON-RPC backend.
Also includes higher-level constructs: agent registry (soulbound ERC-721), reputation scoring,
job marketplace with escrow, futures markets, MEV detection gates, and micropayment state channels.

## Key types and modules

- `ChainClient` / `ChainWallet` -- core read/write traits (backend-agnostic)
- `MockChainClient` / `MockChainWallet` -- in-memory test doubles via `paired_mocks()`
- `AgentRegistry` -- soulbound ERC-721 passport registry (CHAIN-02)
- `ReputationRegistry` -- 7-domain EMA reputation scoring (CHAIN-03)
- `KoraiToken` -- KORAI token with lazy demurrage (CHAIN-01)
- `Marketplace` -- job marketplace with escrow and 3 hiring models (CHAIN-04)
- `X402Manager` -- HTTP 402 micropayment protocol with state channels (CHAIN-08)
- `FuturesMarket` -- prediction/futures market primitives
- `MevGate` / `TxSimGate` / `WalletGate` -- transaction safety gates
- `BlockObserver` -- real-time block event filtering and tracking
- `ChainWitnessEngine` -- on-chain attestation via `witness_on_chain` / `verify_on_chain`
- `TraceRank` -- PageRank-style reputation propagation (P1-02)
- `NelsonSiegel` -- yield curve model for DeFi oracle rates (P2-09)
- `collusion` -- clique-based collusion ring detection (P2-11)

## Feature flags

- `alloy-backend` -- enables Alloy JSON-RPC implementation (requires rustc 1.91+)

## Usage

```rust
use roko_chain::{paired_mocks, ChainClient, ChainWallet};

let (client, wallet) = paired_mocks();
let balance = client.balance("0xdead...").await?;
wallet.send_tx(tx_request).await?;
```

## Architecture

Sits below the orchestrator and gate pipeline. Chain gates (`MevGate`, `TxSimGate`) plug into
the standard gate pipeline to validate transactions before submission. The witness engine
provides on-chain attestation for completed plan steps. Higher-level modules (marketplace,
reputation, token) are used by the identity/economy layer.

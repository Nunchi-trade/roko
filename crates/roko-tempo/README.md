# roko-tempo

Light-client trait + real Tempo testnet ("Moderato") backend for Roko agents.

## What this crate gives you

- A chain-agnostic [`LightClient`] trait — `await_next_header`, `read_account_at`, `verify_account`.
- A `TempoLightClient` façade with a default constructor that connects to Tempo's public testnet (`https://rpc.moderato.tempo.xyz`, chain id `42431`) and verifies account state via EIP-1186 Merkle Patricia trie proofs against each block's `stateRoot`.
- A `TempoRpcBackend` you can point at any Tempo-compatible RPC (devnet, alternate provider, future mainnet).
- A runnable example: `cargo run -p roko-tempo --example tempo_tail`.

## Why it exists

From the 2026-05-01 Jacob × Jae × JD Tempo+Oracle call (Nunchi-trade/collaboration PR #144 §3): the highest-leverage Tempo partnership artifact is a Roko agent that consumes verified Tempo state — *"when they consume data, that data is proven. There's no potential for fraud."* — without bridging or deploying our core on Tempo. This crate is the agent-side surface for that demo.

## Trust model

State reads are cryptographically verified against the block's `stateRoot`. The header chain is anchored at trust-on-first-use (TOFU) at construction time and parent-hash-chained from there — the RPC operator cannot alter past state without producing a hash-mismatched chain. The remaining gap is the consensus signature on each header: Tempo does not yet expose a public consensus-peer surface, so we cannot verify the validator BLS aggregate signature directly. That gap is closed by the `commonware-backend` cargo feature (Phase 2) when Tempo opens its surface.

The backend marks itself with `attestation.quorum_id == "tempo-rpc-tofu"` so consumers can tell it apart from a fully-consensus-verified backend.

## Backends

| Backend | Network | Default? | Cargo feature |
|---|---|---|---|
| `TempoRpcBackend` | Tempo testnet "Moderato" via JSON-RPC + EIP-1186 proofs | yes | always-on |
| `MockLightClient` | none — deterministic in-memory chain | no — tests/demos only | `mock` |
| commonware-p2p follower socket | Tempo consensus peer (when Tempo opens it) | no — Phase 2 | `commonware-backend` |

The trait surface is stable across backends.

## Quick start

```rust
use roko_tempo::{LightClient, TempoLightClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let lc = TempoLightClient::testnet().await?;
    let header = lc.await_next_header().await?;
    let proof = lc.read_account_at(&some_address, header.height).await?;
    lc.verify_account(&proof)?;
    println!("verified at {} state_root={}", header.height, header.state_root);
    Ok(())
}
```

## Running the demo

```bash
cargo run -p roko-tempo --example tempo_tail
# or override the endpoint:
cargo run -p roko-tempo --example tempo_tail -- --rpc <url> --chain-id <id>
# or track a specific account:
cargo run -p roko-tempo --example tempo_tail -- --address 0x...
```

The example tails the chain for 5 headers by default, fetching an EIP-1186 account proof at each block and verifying it locally.

## Tests

Unit tests are available behind the `mock` feature and run with no network:

```bash
cargo test -p roko-tempo --features mock
```

Integration tests against the live testnet:

```bash
ROKO_TEST_RPC_URL=https://rpc.moderato.tempo.xyz cargo test -p roko-tempo --test tempo_live
```

The live suite no-ops if the endpoint is unreachable.

## Cross-references

- [`docs/08-chain/25-tempo-light-client.md`](../../docs/08-chain/25-tempo-light-client.md) — design doc.
- Nunchi-trade/collaboration PR #144 — Tempo Integration Workstreams (this crate is the Workstream A artifact).
- Nunchi-trade/collaboration PR #143 §0 + #145 §0 — the source-of-truth architecture this trait mirrors on the consumer side.

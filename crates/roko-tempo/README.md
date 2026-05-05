# roko-tempo

Light-client trait + Tempo-shaped backend for Roko agents.

## What this crate gives you

- A chain-agnostic [`LightClient`] trait — `await_next_header`, `read_account_at`, `verify_account`.
- A `TempoLightClient` façade that today wraps an in-memory mock and in Phase 1 will wrap a real commonware-p2p subscription to a Tempo follower-node.
- A `MockLightClient` deterministic backend with a canned demo chain so tests, examples, and offline fundraising demos run with no network.
- A runnable example: `cargo run -p roko-tempo --example tempo_tail`.

## Why it exists

From the 2026-05-01 Jacob × Jae × JD Tempo+Oracle call (Nunchi-trade/collaboration PR #144 §3): the highest-leverage Tempo partnership artifact is a Roko agent that consumes verified Tempo state — *"when they consume data, that data is proven. There's no potential for fraud."* — without bridging or deploying our core on Tempo. This crate is the agent-side surface for that demo.

## Phase model

| Phase | Backend | Network dep | Default? |
|---|---|---|---|
| **0** | `MockLightClient` (this PR) | none | yes |
| 1 | commonware-p2p follower-node anchor | follower socket | gated on `commonware-backend` feature, not yet wired |
| 2 | commonware-p2p LC peer protocol | LC peer | gated on Tempo's LC protocol shipping |

The trait surface is stable across phases; only the backing changes.

## Cross-references

- [`docs/08-chain/25-tempo-light-client.md`](../../docs/08-chain/25-tempo-light-client.md) — design doc.
- Nunchi-trade/collaboration PR #144 — Tempo Integration Workstreams (this crate is the Workstream A artifact).
- Nunchi-trade/collaboration PR #143 §0 + #145 §0 — the source-of-truth architecture this trait mirrors on the consumer side.

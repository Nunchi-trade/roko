---
id: P08
title: Trading surface — agent-cli / offchainservices-agent full port
status: in_progress
owner: jl
created: 2026-04-20
---

# P08 — Trading Surface

## Intent

Make the `roko` CLI a complete superset of `agent-cli` /
`offchainservices-agent`: every feature — 14 strategies, APEX orchestrator,
REFLECT review, MCP server, Perpetual-Agent-Jobs keeper/operator/cooperative/
managed engines, Guard custody, CLI commands — accessible from `roko ...`
with both Hyperliquid and Nunchi venue backends.

The Python stack at `Nunchi-trade/offchainservices-agent` stays the source
of truth for strategy logic until this Rust port is validated in
production. This PR is additive; no existing roko crates have breaking API
changes.

## Motivation

- Trading agents are the demo/GTM surface for Nunchi. Having them live in
  roko — alongside the self-building-code agent primitives — collapses two
  separate CLIs into one and lets trading composition reuse roko's
  Gate/Policy/Conductor infrastructure.
- `contracts-core/docs/agent_cli_jobs_spec.tex` is the canonical spec for
  the Perpetual-Agent-Jobs layer. This plan implements it in Rust.
- `Nunchi-trade/offchainservices-agent#2` (shipping in parallel) adds the
  Python Nunchi venue adapter. `roko-venue::NunchiVenue` ports that.

## Deliverables

9 new crates under `crates/`:

| Crate | Role | Mirrors (Python) |
|---|---|---|
| `roko-venue` | `VenueAdapter` trait + `HLVenue` + `NunchiVenue` | `common/venue_adapter.py`, `adapters/{hl,nunchi}_adapter.py`, `parent/hl_proxy.py` |
| `roko-strategy` | `Strategy` trait + 14 strategy impls + loader/registry | `sdk/strategy_sdk/`, `strategies/` |
| `roko-quoting` | Wave-based quoting engine | `quoting_engine/` |
| `roko-execution` | Routing, TWAP, parent-order splitter, portfolio risk | `execution/` |
| `roko-custody` | CustodyPolicy + CustodyGuard (rate/dest/selector/value caps) | `cli/jobs/custody.py` + spec §Custody |
| `roko-jobs` | JobRegistry, 4 engines, EventSubscriber, StatusTracker | `cli/jobs/`, spec §Architecture/Engines/Events |
| `roko-apex` | APEX multi-slot orchestrator | `cli/commands/apex.py` + APEX modules |
| `roko-reflect` | Nightly review (wired as a `Policy`) | `cli/commands/reflect.py` + REFLECT tasks |
| `roko-trading-agent` | `Agent` impl that composes the rest | glue |

Plus:

- `roko-gate` gets `CustodyGate` + `RiskLimitGate` (additive; existing
  gates untouched).
- `roko-cli` gets `roko trading {trade,run,apex,reflect,...}` and
  `roko jobs {list,info,register,run,status,stop}` subcommands.
- `roko-mcp-stdio` gets a `trading` tool namespace (16 tools parity).
- `examples/trading/` ships `simple_mm_nunchi.rs`, `apex_hl.rs`,
  `keeper_job.rs`.

## Reuse, don't reinvent

| Existing | Reused for |
|---|---|
| `roko-chain::ChainClient` / `ChainWallet` | `NunchiVenue` RPC + signing (no second web3 stack) |
| `roko-runtime::event_bus::EventBus<T>` | `roko-jobs::EventSubscriber` with replay ring |
| `roko-gate::PropertyTestGate` pattern | Template for `CustodyGate` / `RiskLimitGate` |
| `roko-conductor` Policy watchers | `roko-reflect` nightly review schedule |
| `roko-agent::Agent` trait | `roko-trading-agent::TradingAgent` |

## Tasks

- [x] T1 Scaffold 9 new crates + workspace Cargo.toml edits + this PRD
- [ ] T2 `roko-venue` — trait + HL impl + Nunchi impl + unit tests
- [ ] T3 `roko-strategy` — trait + 14 strategy impls + loader/registry
- [ ] T4 `roko-quoting` — wave quoting port
- [ ] T5 `roko-execution` — routing / TWAP / portfolio risk
- [ ] T6 `roko-custody` — policy + guard
- [ ] T7 `roko-jobs` — registry + 4 engines + event subscriber + heartbeats
- [ ] T8 `roko-apex` — orchestrator port
- [ ] T9 `roko-reflect` — conductor policy + report generator
- [ ] T10 `roko-trading-agent` — glue `Agent` impl
- [ ] T11 `roko-gate` `CustodyGate` + `RiskLimitGate`
- [ ] T12 `roko-cli` `trading` + `jobs` subcommand surface
- [ ] T13 `roko-mcp-stdio` `trading` tool namespace
- [ ] T14 `examples/trading/` + README "Trading" section
- [ ] T15 Verification per plan file

## Verification

- `cargo check --workspace --all-features` clean.
- `cargo test --workspace` green — ported strategy tests mirror Python.
- HL: `ROKO_HL_TESTNET=1 cargo run -- trading run --venue hl --config examples/trading/simple_mm_hl.toml` 60s OK.
- Nunchi: `ROKO_NUNCHI_RPC=... cargo run -- trading run --venue nunchi --config examples/trading/simple_mm_nunchi.toml` 60s OK against anvil fork.
- Jobs: `cargo run -- jobs register ... && cargo run -- jobs run ...` keeper 5 min with heartbeats + one claim.
- MCP: `cargo run --bin roko-mcp-stdio -- --trading` parity with Python MCP golden test.
- Dashboard: `roko dashboard` Trading tab live.

## Non-goals

- Deleting/deprecating `agent-cli` or `offchainservices-agent` (follow-up RFC).
- Cross-venue arbitrage (future `roko-arb`).
- Public API changes to existing roko crates.
- Mainnet Nunchi — devnet1 only until production deployment exists.

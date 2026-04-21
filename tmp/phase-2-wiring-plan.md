# Phase 2 Wiring Plan — May 7 Demo

**Status:** Proposal, awaiting review
**Author:** @JaeLeex — for @wpank to push back on, then execute
**Parent PR:** #24 (landed Phase 1 on 2026-04-21)
**Target:** 2026-05-07 demo
**Total scope:** ~3,400 LoC across 6 stacked PRs in 16 days

---

## Why this doc exists

PR #24 took roko from type stubs to a working self-hosting system. It explicitly left the chain / reputation / marketplace layer as "types + algorithms, not yet connected to a blockchain — no code path from the plan executor calls into it yet."

The May 7 demo needs Phase 2 visibly alive: agents competing for a real job, reputation-weighted selection, on-chain settlement, and (stretch) collusion detection + slashing. This doc proposes the specific wiring work to get there.

**It also proposes a PR-stacking discipline.** PR #24 was 661 files, ~150k LoC, zero reviewers — unreviewable at that size. This plan breaks Phase 2 into 6 PRs of ≤1k LoC each, landing in dependency order, each with at least one reviewer. That's the real lesson from #24 and should become the pattern going forward.

---

## What we want the demo to prove

> **Roko is an agent runtime with economic memory and self-regulation.**
> Every plan leaves a gate verdict, an episode, and a chain receipt. Every agent has a reputation. Every marketplace job settles on-chain. Nobody else in the agent-framework space has wired economic primitives into the runtime.

Three-act arc (15 min total):

- **Act 1 — "Roko builds itself" (5 min, works today post-#24).** Self-hosting loop: `prd idea → draft → plan → run → episode persisted`. `git diff` proves a real change shipped.
- **Act 2 — "Roko runs an agent economy" (7 min, needs wiring).** `roko demo up job-board`. 5 workers compete for a real task with real LLM bid decisions. Reputation-weighted VRF picks winner. Winner executes via the same Act-1 infrastructure (the aha moment). `BountyMarket.resolve` tx settles on mirage. Reputation EMA updates. TUI F8/F9 tabs show it live.
- **Act 3 — "Roko is self-regulating" (3 min, stretch).** Collusion ring: two workers post mutual-positive feedback. TraceRank + Bron-Kerbosch flag the clique. `WorkerRegistry.slash` burns stake. Reputation drops visible.

Act 1 proves *compounding memory*. Act 2 proves *economy*. Act 3 proves *immune response*. Same self-hosting infrastructure powers all three — that's the structural moat.

---

## Current state (what exists post-#24)

| Layer | Status | Location |
|---|---|---|
| Self-hosting loop (`roko init → prd → plan → run`) | Working | `crates/roko-cli/src/` |
| Mirage EVM fork + RPC | Working | `apps/mirage-rs/` |
| `roko_bridge` (SimulationGate + ChainSubstrate + HdcSubstrate + Buses) | Working, ~2,800 LoC | `apps/mirage-rs/src/roko_bridge/` |
| 10 Solidity contracts | Compile + deploy via scenario | `contracts/src/` (AgentRegistry, BountyMarket, WorkerRegistry, ReputationRegistry, MockERC20, ValidationRegistry, ConsortiumValidator, FeeDistributor, IdentityRegistry, InsightBoard) |
| `demo/scenarios/job-board.toml` | Runs as integration test (round-robin assignment, stub-always-yes bidding) | `crates/roko-demo/src/scenarios/job_board.rs` |
| `ChainClient` + `ChainWallet` traits + `AlloyChainWallet` RPC impl | Working | `crates/roko-chain/src/{client,wallet,alloy_impl,mock}.rs` |
| TUI — 7 tabs (Dashboard / Plans / Agents / Git / Logs / Config / Inspect) | fs-watch driven, reads `.roko/*.jsonl` | `crates/roko-cli/src/tui/` |

The wiring *substrate* is there. What's missing is the **orchestrator → chain → TUI feedback loop**.

---

## The 6 gaps

| # | Gap | Files touched | LoC |
|---|---|---|---|
| **A** | Orchestrator → chain. Gate verdict doesn't emit a tx. No ABI encoders for BountyMarket / ReputationRegistry / WorkerRegistry. | `crates/roko-chain/src/{marketplace,reputation_registry,worker_registry}.rs` (new), `crates/roko-orchestrator/src/executor/action.rs`, `crates/roko-cli/src/orchestrate.rs` | ~250 |
| **B** | Scenario runner is faux multi-agent. `scripted_actions` in TOML is unused. Assignment is round-robin, StubLlm always bids "yes". No real competition. | `crates/roko-demo/src/scenarios/{job_board,llm}.rs`, new `collusion_ring.rs` | ~400 |
| **C** | Chain-watcher is blind to contract events. Watcher polls pheromones/insights only — no `eth_getLogs` for `JobResolved` / `FeedbackRecorded` / `Slashed`. `alloy_impl::get_logs` is stubbed. | `apps/roko-chain-watcher/src/{watcher,reactions}.rs`, new `event_writer.rs`, `crates/roko-chain/src/alloy_impl.rs` | ~180 |
| **D** | No TUI tabs for Marketplace / Reputation. F8/F9 don't exist. `.roko/marketplace/` + `.roko/reputation/` dirs don't exist. | `crates/roko-cli/src/tui/{tabs,state,dashboard}.rs`, new `views/{marketplace_view,reputation_view}.rs` | ~1,800 |
| **E** | Demo scenario lacks pacing/visuals. Runs silently via `tracing::info!`. No rehearsable cadence. | `crates/roko-demo/src/scenarios/job_board.rs`, new `scripts/demo-preseed.sh` | ~300 |
| **F** | Act 3 (collusion + slashing). No collusion-scenario TOML, no slash tx path. | new `demo/scenarios/collusion-ring.toml`, `crates/roko-demo/src/scenarios/collusion_ring.rs`, slash encoder | ~500 |

Total: ~3,400 LoC.

---

## Target architecture

```
┌──────────────────────────────────────────────────────────┐
│ roko-cli (plan runner, TUI)                              │
└──────────┬─────────────────────────────────┬─────────────┘
           │                                 │
           │ gate verdict                    │ fs-watch
           ▼                                 │
┌──────────────────────────────────┐         │
│ roko-chain (ABI glue — NEW)      │         │
│  ├─ marketplace.rs               │         │
│  ├─ reputation_registry.rs       │         │
│  └─ worker_registry.rs           │         │
└──────────┬───────────────────────┘         │
           │ eth_sendTx / eth_call            │
           ▼                                  │
┌──────────────────────────────────┐         │
│ mirage-rs (EVM fork)             │         │
│  ├─ Contracts deployed:          │         │
│  │   BountyMarket, WorkerReg,    │         │
│  │   ReputationReg, MockERC20    │         │
│  └─ roko_bridge:                 │         │
│      SimulationGate (pre-val)    │         │
│      ChainSubstrate              │         │
│      HdcSubstrate                │         │
└──────────┬───────────────────────┘         │
           │ events (JobResolved, etc.)       │
           ▼                                  │
┌──────────────────────────────────┐         │
│ roko-chain-watcher               │         │
│   polls eth_getLogs              │         │
│   decodes events                 │         │
│   writes .roko/marketplace/      │         │
│           .roko/reputation/      │─────────┘
└──────────────────────────────────┘
```

---

## Stacked PR sequence

| # | Branch | Scope | LoC | Depends on | Target land |
|---|---|---|---|---|---|
| P2.1 | `will/phase2-abi-glue` | WS2 — ABI encoders in `roko-chain` (no wiring yet). Unit tests for round-trip + foundry selector parity. | ~250 | none | Day 4 |
| P2.2 | `will/phase2-orchestrator-hook` | WS3 — `EmitChainTransaction` action + `orchestrate.rs` dispatch + `--chain-*` CLI flags + `roko.toml` `[chain]` section. | ~150 | P2.1 | Day 6 |
| P2.3 | `will/phase2-chain-watcher-events` | WS4 — watcher polls `eth_getLogs`, decodes demo events, writes `.roko/{marketplace,reputation}/*.jsonl`. Fix `alloy_impl::get_logs`. | ~180 | P2.1 | Day 7 |
| P2.4 | `will/phase2-scenario-upgrade` | WS5 — real multi-agent bidding (concurrent `tokio::spawn`, real LLM bid decisions, reputation-weighted VRF winner selection). | ~400 | P2.2, P2.3 | Day 10 |
| P2.5 | `will/phase2-tui-marketplace-reputation` | WS6 — F8 Marketplace + F9 Reputation tabs with incremental cursors. Parallelizable with P2.4. | ~1,800 | P2.3 | Day 14 |
| P2.6 | `will/phase2-demo-polish` | WS7 — pacing, colorized output, `demo-preseed.sh`, 3 dress rehearsals. | ~300 | P2.4, P2.5 | Day 16 |
| P2.7 | `will/phase2-collusion-act3` (optional) | WS6/Act 3 — collusion scenario + slash path. Cut if P2.4/P2.5 slip. | ~500 | P2.4, P2.5 | Day 16 |

### Rules per stacked PR

- Commit tag convention mirrors #24's methodology: `ws2(ABI-01): ...`, `ws3(HOOK-01): ...`, etc.
- `cargo build --workspace && cargo clippy --workspace --no-deps -- -D warnings && cargo test --workspace` green before merge.
- **At least one reviewer before merge** — non-negotiable. #24's zero-reviewer solo-merge is the anti-pattern this plan is designed to correct.
- Squash-merge on land with `feat(phase2): ...` conventional commit title, preserving the `ws*(TAG-NN): ...` phase tags in the pre-squash commit history for audit.

### Why stacked, not mega

| Dimension | Mega PR (like #24) | Stacked PRs |
|---|---|---|
| Review load | 3,400 LoC one shot → no one reviews | 6 × ≤1k LoC → actual review possible |
| Scope creep risk | Act 3 blocks the whole PR | Act 3 isolated, cuttable |
| Incremental demo value | Nothing demos until all lands | Act 2 partial demo available after P2.4 |
| Rollback blast radius | Revert loses everything | Revert one PR, keep the rest |
| Parallelism | Single critical path | P2.3 and P2.4/P2.5 parallelizable |
| Audit trail | One giant squash | 6 named PRs in history |

---

## Timeline (Gantt-ish)

```
Day:  1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16
WS1:  █   (sync branches)
WS2:      █ █ █
WS3:            █ █ █
WS4:            █ █ █
WS5:              █ █ █ █ █
WS6:                  █ █ █ █ █ █ █
WS7:                              █ █ █ █
```

WS2 + WS5 on critical path for Act 2. Others parallelizable.

---

## Out of scope for May 7

| Deferred | Why |
|---|---|
| ISFR clearing demo | 6-phase cycle too complex for 3 min — save for a focused "oracle" demo later |
| Passport tier progression demo | Needs long time horizon to feel real — show as TUI chrome in Reputation tab, don't demo progression |
| x402 HTTP payment demo | Orthogonal to the "agent economy" story — separate demo |
| KORAI demurrage | Slow-burn mechanic, not visually impressive |
| Real Chainlink VRF | Use deterministic seeded pseudo-VRF for demo repeatability |
| Gate-failure → re-plan loop | `gate_failure_replan_enabled()` returns `false` in #24 — leave it false for May 7 |
| Knowledge-informed routing | Phase 2.5 — outside this demo arc |

---

## Risks + mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| Claude API rate limits during live demo | Medium | Ollama as fallback; pre-cache LLM responses for deterministic replay |
| Mirage crash or RPC hang mid-demo | Medium | Pre-recorded 4K fallback video; rehearse recovery |
| ABI encoding bug shipping late | Low | Use `alloy::sol!` macro — auto-generated from `.sol` files |
| Act 2 pacing drags | High | Tight scenario scripting + deterministic timing; no open-ended agent freedom live |
| Act 3 collusion scenario fragile | High | Act 3 is stretch-only — cut if P2.4 slips |
| Will is pulled to other work | High | Protect demo calendar; demo is the Apr/May primary deliverable |
| CI billing still red post-merge | Low (cosmetic) | Confirm billing fix before May 7 so green sig is visible |

---

## Verification per PR

- **P2.1:** `cargo test -p roko-chain --test abi_roundtrip` — round-trip tests pass; selectors match `forge inspect <Contract> methodIdentifiers`
- **P2.2:** `cargo test -p roko-cli --test chain_integration` — plan task completes, `JobResolved` event appears in mirage logs
- **P2.3:** Run `roko demo up job-board`, tail `.roko/marketplace/jobs.jsonl` — entries appear within 2s of contract event emission
- **P2.4:** Run `roko demo up job-board --llm-backend claude` 10× with different seeds — all complete with varied (non-identical) bid values
- **P2.5:** Run `roko dashboard` against a live demo — F8/F9 tabs show live data with <500ms lag behind `.roko/` updates
- **P2.6:** Full dress rehearsal — 15-min run completes without human intervention, 3 runs in a row

---

## Decisions I need @wpank to weigh in on

1. **Act 3 go/no-go** — attempt 3-act (stretch, riskier) or commit to 2-act (safer)? Your read on P2.4/P2.5 slip risk drives this.
2. **Ownership split** — solo-own all 6 PRs, or hand P2.5 (TUI — biggest and orthogonal to chain work) to someone else to parallelize? Sam? Bharav? Others?
3. **Live vs pre-recorded** — commit to live Act 1+2 with 60-sec video fallback, or pre-record everything + live Q&A only?
4. **Ollama primary or Claude primary** for demo LLM? Claude wins on bid-reasoning quality; Ollama wins on no-rate-limits. Fallback either way.

## Decisions already made (push back if you disagree)

- **Audience:** VC frame, lead with moat + compounding
- **Length:** 15 min, 3 acts (Q&A after, not inside)
- **Review hygiene:** at least one reviewer per PR, non-negotiable (I'll review for narrative/scope; need a Rust-senior co-reviewer named by you)

---

## Post-demo follow-up PRs

After May 7 the following naturally fall out:

- Gate-failure → re-plan feedback loop (flip the flag, add signal wiring)
- ISFR clearing demo + TUI tab
- Real Chainlink VRF integration
- x402 HTTP payment demo
- Passport tier progression + governance gate
- Knowledge-informed agent routing

---

## References

- Phase 1 PR: https://github.com/Nunchi-trade/roko/pull/24
- `roko_bridge` module: `apps/mirage-rs/src/roko_bridge/mod.rs`
- Job-board scenario: `demo/scenarios/job-board.toml`, `crates/roko-demo/src/scenarios/job_board.rs`
- Contracts: `contracts/src/`
- Chain trait layer: `crates/roko-chain/src/{client,wallet,alloy_impl}.rs`

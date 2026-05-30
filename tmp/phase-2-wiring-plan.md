# Phase 2 Wiring Plan — May 7 Demo

**Status:** Proposal, awaiting review
**Author:** @JaeLeex — for @wpank to push back on, then execute
**Parent PR:** #24 (landed Phase 1 on 2026-04-21)
**Target:** 2026-05-07 demo
**Total scope:** ~3,610 LoC across 7 stacked PRs in 17 days
**Contracts SoT:** [`Nunchi-trade/contracts-core`](https://github.com/Nunchi-trade/contracts-core) — `packages/agents/` (pinned to commit `a818863`)

---

## Why this doc exists

PR #24 took roko from type stubs to a working self-hosting system. It explicitly left the chain / reputation / marketplace layer as "types + algorithms, not yet connected to a blockchain — no code path from the plan executor calls into it yet."

The May 7 demo needs Phase 2 visibly alive: agents competing for a real job, reputation-weighted selection, on-chain settlement, and (stretch) collusion detection + slashing. This doc proposes the specific wiring work to get there.

**Two disciplines this plan installs:**

1. **Stacked PRs at ≤1k LoC each, every PR gets a reviewer.** PR #24 was 661 files, ~150k LoC, zero reviewers — unreviewable at that size. Phase 2 is broken into 7 PRs in dependency order to make review actually happen.
2. **Contracts-core is the source of truth.** Roko's local `contracts/src/` has 10 duplicates that have already diverged from `Nunchi-trade/contracts-core/packages/agents/`. Keeping two copies in sync across dashboard, off-chain services, and roko is a lost battle. Phase 2 consumes contracts-core ABIs directly; the roko-local copies get deprecated in a separate post-demo cleanup PR.

---

## Source of truth: contracts-core

### What contracts-core is

`Nunchi-trade/contracts-core` is the canonical home for all Nunchi Solidity since PR #100 (agents consolidation, merged 2026-04-15). Dashboard, off-chain services, exchange contracts, and now roko all target it. Package layout:

```
contracts-core/packages/
├── agents/        ← Phase 2 demo lives here
├── exchange/      ← perps, order book, fee module
├── nhype/         ← native staking, vaults, lockers
├── shared/        ← common libs
├── sy-tokens/     ← staking-yield tokens
└── vaults/        ← vault infra
```

`packages/agents/src/` (19 contracts): AgentRegistry, BountyMarket, CompletionProof, ConsortiumValidator, DisputeResolver, FeeDistributor, FundingRateKeeperJob, HDCClient, IHDCPrecompile, IISFROracle, InsightBoard, ISFRMinimal, JobTypeRegistry, MockERC20, NotificationRegistry, OracleUpdaterJob, PerpsLiquidatorJob, RoleRegistry, WorkerRegistry.

### Contracts roko uses for Phase 2 demo

| Contract | Role in demo |
|---|---|
| `BountyMarket` | Job posting, assign, submit, resolve (via ConsortiumValidator) |
| `WorkerRegistry` | Register, bond, slash, 7-domain reputation EMA (reputation is built-in here, NOT a separate contract) |
| `ConsortiumValidator` | Committee assembly + attestation on job resolution |
| `RoleRegistry` | `MANAGER_ROLE` auth for admin ops |
| `FeeDistributor` | Route validator fees on resolve |
| `JobTypeRegistry` | Declares job types (can use minimal custom type or one of the existing ERC-8183 wrappers) |
| `MockERC20` | DAEJI token (stake + bounty) |

**Not used for May 7** (kept in contracts-core for later phases): CompletionProof, DisputeResolver, HDCClient, IHDCPrecompile, IISFROracle, InsightBoard, ISFRMinimal, NotificationRegistry, PerpsLiquidatorJob, OracleUpdaterJob, FundingRateKeeperJob (unless a job type ends up demo-useful).

### Integration method: submodule + forge ABIs

Add contracts-core as a git submodule at `roko/contracts-core/` pinned to commit `a818863` (current main — "feat(agents): HDC precompile Solidity surface"). Roko's Rust bindings are generated from `contracts-core/packages/agents/out/*.json` (forge-compiled ABI artifacts) via `alloy::sol!` with JSON sourcing.

**Why submodule vs. published cargo crate:**
- Submodule makes ABI pins explicit + commit-level. Re-pin is a reviewable action.
- No dependency on contracts-core publishing a separate bindings crate (it doesn't today).
- Matches how foundry projects consume external Solidity (lib/ + soldeer).
- Before writing bindings, **grep contracts-core for an existing Rust bindings crate** (per roko `CLAUDE.md`: "NEVER reimplement what already exists"). If one exists, consume it; skip WS2 ABI work.

**Re-pin policy:** only if a critical fix from the audit queue (4 must-fix items tracked separately — see `reviews/2026-04-20-pr-100-contracts-core-agents-consolidation.md`) lands during the sprint AND the fix touches an ABI roko uses. Otherwise stay on `a818863` through May 7 to avoid mid-sprint breakage.

### Roko-local contracts being deprecated

Roko's current `contracts/src/` (10 files — AgentRegistry, BountyMarket, ConsortiumValidator, FeeDistributor, IdentityRegistry, InsightBoard, MockERC20, ReputationRegistry, ValidationRegistry, WorkerRegistry) will be replaced by contracts-core versions. Deprecation is a **separate post-demo cleanup PR**, not part of Phase 2 — don't churn during the demo sprint. MockERC20 may stay as a lightweight local mock if avoiding the contracts-core dep is valuable for some test rigs.

**Gotcha:** roko's local `ReputationRegistry.sol` has no counterpart in contracts-core — contracts-core bakes 7-domain reputation into `WorkerRegistry` (EMA tier via `_effectiveReputation`). Our Rust `reputation_registry.rs` plan is replaced by `worker_registry.rs` bindings that cover both registration and reputation reads/writes.

---

## What we want the demo to prove

> **Roko is an agent runtime with economic memory and self-regulation.**
> Every plan leaves a gate verdict, an episode, and a chain receipt. Every agent has a reputation. Every marketplace job settles on-chain. Nobody else in the agent-framework space has wired economic primitives into the runtime.

Three-act arc (15 min total):

- **Act 1 — "Roko builds itself" (5 min, works today post-#24).** Self-hosting loop: `prd idea → draft → plan → run → episode persisted`. `git diff` proves a real change shipped.
- **Act 2 — "Roko runs an agent economy" (7 min, needs wiring).** `roko demo up job-board`. 5 workers compete for a real task with real LLM bid decisions. `BountyMarket.assign` picks a worker. Winner executes via the same Act-1 infrastructure. Worker submits → `ConsortiumValidator` assembles committee → attests → `BountyMarket.resolve` by validator contract. Payment settles via `FeeDistributor`. `WorkerRegistry` reputation EMA updates. TUI F8/F9 tabs show it live.
- **Act 3 — "Roko is self-regulating" (3 min, stretch).** Collusion ring: two workers post mutual-positive feedback. TraceRank + Bron-Kerbosch flag the clique. `WorkerRegistry.slash` burns stake. Reputation drops visible.

Act 1 proves *compounding memory*. Act 2 proves *economy*. Act 3 proves *immune response*. Same self-hosting infrastructure powers all three — that's the structural moat.

---

## Current state (what exists post-#24)

| Layer | Status | Location | Owner |
|---|---|---|---|
| Self-hosting loop | Working | `crates/roko-cli/src/` | roko |
| Mirage EVM fork + RPC | Working | `apps/mirage-rs/` | roko |
| `roko_bridge` (SimulationGate + ChainSubstrate + HdcSubstrate + Buses) | Working, ~2,800 LoC | `apps/mirage-rs/src/roko_bridge/` | roko |
| `ChainClient` + `ChainWallet` traits + `AlloyChainWallet` | Working | `crates/roko-chain/src/{client,wallet,alloy_impl}.rs` | roko |
| TUI — 7 tabs (Dashboard/Plans/Agents/Git/Logs/Config/Inspect), fs-watch driven | Working | `crates/roko-cli/src/tui/` | roko |
| Agents-package Solidity contracts | **Canonical** | `contracts-core/packages/agents/src/` | **contracts-core (SoT)** |
| Roko-local duplicate contracts (10 files) | Diverged from contracts-core | `roko/contracts/src/` | **roko (deprecate post-demo)** |
| `demo/scenarios/job-board.toml` | Deploys roko-local duplicates | roko | roko (migrate to contracts-core in P2.0) |

**Flagged during planning (re-verify, may save work):** The roko `CLAUDE.md` at the workspace root lists several items as "Wired" that PR #24's body called out as not-yet-wired — gate-failure replan, PRD auto-plan on publish, context bidders in heartbeat. If those are in fact wired post-#24, scope drops. First-day task: grep and confirm.

---

## The 7 gaps

| # | Gap | Files touched | LoC |
|---|---|---|---|
| **0** | **Contracts-core integration.** Add submodule, wire `forge build` into roko build, regenerate ABIs at build time, migrate `job-board.toml` deploy step to contracts-core contract names. | `.gitmodules`, `contracts-core/` (submodule), `foundry.toml`, `Cargo.toml`, `build.rs` (new or extend), `demo/scenarios/job-board.toml`, `crates/roko-demo/src/scenarios/job_board.rs` | ~150 |
| **A** | Orchestrator → chain. ABI encoders from contracts-core. Gate verdict emits submission or resolve tx. | `crates/roko-chain/src/{bounty_market,worker_registry,consortium_validator,role_registry,fee_distributor}.rs` (new), `crates/roko-orchestrator/src/executor/action.rs`, `crates/roko-cli/src/orchestrate.rs` | ~280 |
| **B** | Scenario runner is faux multi-agent. `scripted_actions` in TOML is unused. Round-robin assignment + StubLlm always bids "yes". No real competition. | `crates/roko-demo/src/scenarios/{job_board,llm}.rs`, new `collusion_ring.rs` | ~400 |
| **C** | Chain-watcher blind to contract events. Watcher polls pheromones/insights only — no `eth_getLogs` for `JobPosted` / `JobAssigned` / `JobResolved` / `Slashed` / `FeedbackRecorded`. `alloy_impl::get_logs` is stubbed. | `apps/roko-chain-watcher/src/{watcher,reactions}.rs`, new `event_writer.rs`, `crates/roko-chain/src/alloy_impl.rs` | ~180 |
| **D** | No TUI tabs for Marketplace / Reputation. F8/F9 don't exist. `.roko/marketplace/` + `.roko/reputation/` dirs don't exist. | `crates/roko-cli/src/tui/{tabs,state,dashboard}.rs`, new `views/{marketplace_view,reputation_view}.rs` | ~1,800 |
| **E** | Demo scenario lacks pacing/visuals. Runs silently via `tracing::info!`. No rehearsable cadence. | `crates/roko-demo/src/scenarios/job_board.rs`, new `scripts/demo-preseed.sh` | ~300 |
| **F** | Act 3 (collusion + slashing). No collusion-scenario TOML, no slash tx path. | new `demo/scenarios/collusion-ring.toml`, `crates/roko-demo/src/scenarios/collusion_ring.rs`, slash encoder | ~500 |

Total: ~3,610 LoC.

---

## Target architecture

```
┌────────────────────────────────────────────────────────────────────┐
│ Nunchi-trade/contracts-core (SoT)                                  │
│  packages/agents/src/*.sol → packages/agents/out/*.json (ABIs)     │
│  pinned at commit a818863                                          │
└────────────┬───────────────────────────────────────┬───────────────┘
             │ submodule checkout                    │ forge build
             ▼                                       ▼
┌────────────────────────────────┐       ┌────────────────────────────┐
│ roko (consumer)                │       │ roko (consumer, Rust)      │
│  contracts-core/ (submodule)   │──────►│  build.rs generates        │
│  deploys into mirage via       │       │  bindings from out/*.json  │
│  demo/scenarios/job-board.toml │       │  → alloy::sol! typed ABI   │
└────────────┬───────────────────┘       └────────────┬───────────────┘
             │                                        │
             │ deploy txs                             │ encode_* / decode_*
             ▼                                        ▼
┌─────────────────────────────────────────────────────────────────────┐
│ mirage-rs (EVM fork)                                                │
│  Deployed contracts (from contracts-core bytecode):                 │
│    RoleRegistry → WorkerRegistry → BountyMarket →                   │
│    ConsortiumValidator → FeeDistributor → MockERC20                 │
│  roko_bridge: SimulationGate / ChainSubstrate / HdcSubstrate        │
└────────────┬────────────────────────────────────────────────────────┘
             │ events: JobPosted, JobAssigned, JobResolved, Slashed,
             │         FeedbackRecorded
             ▼
┌─────────────────────────────────────────────────────────────────────┐
│ roko-chain-watcher                                                  │
│   polls eth_getLogs against mirage                                  │
│   decodes with contracts-core ABIs                                  │
│   writes .roko/marketplace/*.jsonl + .roko/reputation/*.jsonl       │
└────────────┬────────────────────────────────────────────────────────┘
             │ fs-watch (200ms debounce + 1s fallback poll)
             ▼
┌─────────────────────────────────────────────────────────────────────┐
│ roko-cli TUI (F8 Marketplace + F9 Reputation)                       │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Stacked PR sequence — 7 PRs

| # | Branch | Scope | LoC | Depends on | Target land |
|---|---|---|---|---|---|
| P2.0 | `will/phase2-contracts-core-integration` | WS0 — submodule, `forge build` wiring, scenario migration to contracts-core contract names. Verify no existing Rust bindings crate first. | ~150 | none | Day 2 |
| P2.1 | `will/phase2-abi-glue` | WS2 — ABI bindings sourced from contracts-core `out/*.json`: BountyMarket, WorkerRegistry, ConsortiumValidator, RoleRegistry, FeeDistributor. Unit tests: round-trip + selector parity with `forge inspect`. | ~280 | P2.0 | Day 5 |
| P2.2 | `will/phase2-orchestrator-hook` | WS3 — `EmitChainTransaction` action + `orchestrate.rs` dispatch + `--chain-*` CLI flags + `roko.toml` `[chain]` section. Submit / resolve paths. | ~150 | P2.1 | Day 7 |
| P2.3 | `will/phase2-chain-watcher-events` | WS4 — watcher polls `eth_getLogs`, decodes via contracts-core ABIs, writes `.roko/{marketplace,reputation}/*.jsonl`. Fix `alloy_impl::get_logs`. | ~180 | P2.1 | Day 8 |
| P2.4 | `will/phase2-scenario-upgrade` | WS5 — real multi-agent bidding (concurrent `tokio::spawn`, real LLM bid decisions). Committee attestation (auto-approve in demo). | ~400 | P2.2, P2.3 | Day 11 |
| P2.5 | `will/phase2-tui-marketplace-reputation` | WS6 — F8 Marketplace + F9 Reputation tabs, incremental cursors. Parallelizable with P2.4. | ~1,800 | P2.3 | Day 15 |
| P2.6 | `will/phase2-demo-polish` | WS7 — pacing, colorized output, `demo-preseed.sh`, 3 dress rehearsals. | ~300 | P2.4, P2.5 | Day 17 |
| P2.7 | `will/phase2-collusion-act3` (optional) | Act 3 — collusion scenario + slash path. Cut if P2.4/P2.5 slip. | ~500 | P2.4, P2.5 | Day 17 |

### Rules per stacked PR

- Commit tag convention mirrors #24's methodology: `ws2(ABI-01): ...`, `ws3(HOOK-01): ...`, etc.
- Pre-commit: `cargo +nightly fmt --all && cargo clippy --workspace --no-deps -- -D warnings && cargo test --workspace` — per roko `CLAUDE.md`.
- **At least one reviewer before merge** — non-negotiable. #24's zero-reviewer solo-merge is the anti-pattern this plan corrects.
- Squash-merge on land with `feat(phase2): ...` conventional commit title, preserving phase tags in pre-squash commit history for audit.

### Why stacked, not mega

| Dimension | Mega PR (like #24) | Stacked PRs |
|---|---|---|
| Review load | 3,610 LoC one shot → no one reviews | 7 × ≤1k LoC → actual review possible |
| Scope creep risk | Act 3 blocks the whole PR | Act 3 isolated, cuttable |
| Incremental demo value | Nothing demos until all lands | Act 2 partial demo available after P2.4 |
| Rollback blast radius | Revert loses everything | Revert one PR, keep the rest |
| Parallelism | Single critical path | P2.3 and P2.4/P2.5 parallelizable |
| Audit trail | One giant squash | 7 named PRs in history |
| Cross-repo coordination | Hidden | P2.0 explicitly pins contracts-core commit |

---

## Timeline (Gantt-ish, 17 days)

```
Day:  1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17
WS0:  █ █          (contracts-core integration)
WS2:      █ █ █
WS3:            █ █ █
WS4:            █ █ █
WS5:              █ █ █ █ █
WS6:                  █ █ █ █ █ █ █
WS7:                              █ █ █ █
```

Today is Day 0 (2026-04-21). Demo is Day 16 (2026-05-07). Plan has 1 day of buffer — tight. P2.0 on critical path for everything (blocks P2.1, which blocks P2.2–P2.7).

---

## Out of scope for May 7

| Deferred | Why |
|---|---|
| Roko-local contracts deprecation | Separate cleanup PR after May 7 — don't churn during demo sprint |
| ISFR clearing demo | 6-phase cycle too complex for 3 min — save for a focused "oracle" demo later |
| Passport tier progression demo | Needs long time horizon to feel real — show as TUI chrome in Reputation tab, don't demo progression |
| x402 HTTP payment demo | Orthogonal to the "agent economy" story — separate demo |
| KORAI demurrage | Slow-burn mechanic, not visually impressive |
| Real Chainlink VRF | `BountyMarket.assign` is currently permissionless (known audit finding) — accept for demo, don't try to fix the pseudo-VRF here |
| Applying contracts-core audit-fix PRs | 4 must-fix items open (see PR #100 review); demo pins pre-fix; fixes land in contracts-core on their own cadence |
| Gate-failure → re-plan loop | Already wired per roko CLAUDE.md (may contradict #24 body — verify Day 1) |
| Knowledge-informed routing | Phase 2.5 — outside this demo arc |
| Contracts-core packages outside `agents/` | Exchange, nhype, vaults, sy-tokens, shared — not used |

---

## Risks + mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| contracts-core audit-fix lands during build and breaks an ABI roko uses | Medium | Pin to commit `a818863`; re-pin is an explicit, reviewed action. Monitor contracts-core PR #91 |
| Claude API rate limits during live demo | Medium | Ollama as fallback; pre-cache LLM responses for deterministic replay |
| Mirage crash or RPC hang mid-demo | Medium | Pre-recorded 4K fallback video; rehearse recovery |
| `alloy::sol!` compile time explodes with 5 contracts | Medium | Generate bindings at build via `build.rs` + feature-gate heavy tests |
| ABI encoding bug shipping late | Low | Selector parity check against `forge inspect <Contract> methodIdentifiers` in unit tests |
| Act 2 pacing drags | High | Tight scenario scripting + deterministic timing; no open-ended agent freedom live |
| Act 3 collusion scenario fragile | High | Act 3 is stretch-only — cut if P2.4 slips |
| Will pulled to other work | High | Protect demo calendar; Phase 2 is the Apr/May primary deliverable |
| Contracts-core submodule discovery breaks fresh-clone CI | Low | `git clone --recurse-submodules` in CI; document in README |
| CI billing still red post-#24 merge | Low (cosmetic) | Confirm billing fix before May 7 |

---

## Verification per PR

- **P2.0:** `git submodule status` shows contracts-core pinned at `a818863`. `cd contracts-core/packages/agents && forge build` succeeds in roko's workspace. `roko demo up job-board` deploys from contracts-core bytecode (verify by checking deployed bytecode hash matches contracts-core `out/`).
- **P2.1:** `cargo test -p roko-chain --test abi_roundtrip` — round-trip pass for all 5 contracts. Selectors match `forge inspect` output. **First:** `grep -rn "sol!\|abi::" contracts-core/` — confirm no pre-existing Rust bindings crate to reuse.
- **P2.2:** `cargo test -p roko-cli --test chain_integration` — plan task completes, `BountyMarket.JobResolved` event appears in mirage logs.
- **P2.3:** Run `roko demo up job-board`, tail `.roko/marketplace/jobs.jsonl` — entries appear within 2s of contract event emission. Events decoded via contracts-core ABIs.
- **P2.4:** Run `roko demo up job-board --llm-backend claude` 10× with different seeds — all complete with varied (non-identical) bid values. Committee attestation auto-approves.
- **P2.5:** Run `roko dashboard` against a live demo — F8/F9 tabs show live data with <500ms lag behind `.roko/` updates.
- **P2.6:** Full dress rehearsal — 15-min run completes without human intervention, 3 runs in a row.

---

## Decisions I need @wpank to weigh in on

1. **contracts-core integration method** — git submodule at `roko/contracts-core/` (recommended, explicit pins) or cargo crate if contracts-core publishes one? First-step verification: grep contracts-core for existing Rust bindings crate.
2. **Roko-local contracts handling during P2.0** — ignore (keep as-is, deprecate post-demo) or delete the duplicate `.sol` files in P2.0 to force attention? I'd say ignore — no churn during the sprint.
3. **Act 3 go/no-go** — attempt 3-act (stretch, riskier) or commit to 2-act (safer)?
4. **Ownership split** — solo-own all 7 PRs, or hand P2.5 (TUI, orthogonal to chain work) to someone else to parallelize? Sam? Bharav?
5. **Live vs pre-recorded for Act 2** — live + 60-sec video fallback, or pre-record + live Q&A?
6. **Ollama primary or Claude primary** for demo LLM? Claude wins on bid-reasoning quality; Ollama wins on no-rate-limits.

## Decisions already made (push back if you disagree)

- **Audience:** VC frame, lead with moat + compounding
- **Length:** 15 min, 3 acts (Q&A after, not inside)
- **Review hygiene:** at least one reviewer per PR, non-negotiable
- **contracts-core pin for May 7:** commit `a818863` — re-pin only on critical break

---

## Post-demo follow-up PRs

After May 7 the following naturally fall out:

- **Roko-local contracts cleanup PR** — delete `roko/contracts/src/{AgentRegistry,BountyMarket,ConsortiumValidator,FeeDistributor,IdentityRegistry,InsightBoard,ReputationRegistry,ValidationRegistry,WorkerRegistry}.sol` (keep MockERC20 only if needed as mock). All callers now reference contracts-core.
- Gate-failure → re-plan feedback loop verification (or confirm already wired)
- ISFR clearing demo + TUI tab (uses `ISFRMinimal`, `IISFROracle` from contracts-core)
- Real Chainlink VRF integration for `BountyMarket.assign`
- x402 HTTP payment demo
- Passport tier progression + governance gate
- Knowledge-informed agent routing
- Re-pin contracts-core to absorb audit-fix PRs

---

## References

- Phase 1 PR: https://github.com/Nunchi-trade/roko/pull/24
- contracts-core: https://github.com/Nunchi-trade/contracts-core
- contracts-core PR #100 review (audit findings): `~/obsidian-vault/reviews/2026-04-20-pr-100-contracts-core-agents-consolidation.md`
- `roko_bridge` module: `apps/mirage-rs/src/roko_bridge/mod.rs`
- Job-board scenario: `demo/scenarios/job-board.toml`, `crates/roko-demo/src/scenarios/job_board.rs`
- Chain trait layer: `crates/roko-chain/src/{client,wallet,alloy_impl}.rs`
- Roko CLAUDE.md: `/CLAUDE.md` (workspace root — has per-crate status table)
- Phase 2+ chain architecture docs: `~/dev/nunchi/roko/bardo-backup/tmp/agent-chain/` (Will's local)

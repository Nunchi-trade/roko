# Codebase TODOs, Stubs, and Dormant Code

Structural gaps found by scanning the actual code, not just the docs.

---

## 1. roko-serve: Former Route Stubs Are Mostly Closed

The old route-stub list is stale. `prds.rs`, `plans.rs`, and `research.rs` now run background work through the shared runtime (`runtime.run_once(...)`) rather than placeholder no-ops.

Remaining server/runtime gaps are higher-level:
- observability/learning endpoints
- richer progress broadcasting
- deeper runtime safety unification below provider-backed paths

## 2. Safety Subsystems in roko-orchestrator: Built, Only Partially Called

6 modules in `roko-orchestrator/src/safety/` (~2,441 lines total, tested), but still not integrated as a universal orchestrator-level safety layer:

| Module | Lines | What |
|--------|-------|------|
| `audit_chain.rs` | 565 | Tamper-evident audit trail |
| `capability_tokens.rs` | ~200 | Privilege gating (OCaps-lite) |
| `loop_guard.rs` | 364 | Runaway loop detection |
| `permit.rs` | 452 | Scoped permission system |
| `sandboxing.rs` | 651 | Sandbox enforcement |
| `taint_propagation.rs` | 409 | Taint tracking through signals |

## 3. Safety Subsystems in roko-agent: Built, Only Partially Called

6 guards in `roko-agent/src/safety/` (~1,355 lines, 50+ tests) — fully implemented and now reached on shared ToolLoop paths for OpenAI-compatible providers, Gemini compat models, Anthropic API, and Perplexity tool-capable chat, but still not universal across all runtime backends:

| Guard | What |
|-------|------|
| `bash.rs` | Deny patterns (rm -rf, sudo, etc.), 8192 char limit |
| `git.rs` | Force push, hard reset, branch deletion blocks |
| `network.rs` | RFC1918, link-local, loopback denial |
| `path.rs` | Worktree sandbox via canonicalization |
| `scrub.rs` | 9 regex patterns (API keys, JWTs, etc.) |
| `rate_limit.rs` | Sliding-window per (role, tool) |

`ToolDispatcher` is no longer purely dormant, but backend-specific paths such as Claude CLI and Gemini-native still bypass the shared dispatcher/safety pipeline. Specialty non-chat endpoints such as embeddings and async deep-research also remain adapter-specific.

## 4. Conductor: Partially Wired, Still Not Universal

`Conductor` is no longer completely dormant in `orchestrate.rs`:
- `compile_fail_repeat`, `context_window_pressure`, `cost_overrun`, `ghost_turn`, `iteration_loop`, `review_loop`, `spec_drift`, `stuck_pattern`, `test_failure_budget`, `time_overrun`
- `CircuitBreaker::is_broken()` is now checked during dispatch
- `DiagnosisEngine::diagnose()` never called on failures

Remaining gap: richer watcher diagnosis and intervention still are not treated as a universal executor control loop.

## 5. Feature-Gated Scaffold Code (roko-golem)

Entire subsystems under `#[cfg(feature = "scaffold")]`:
- `chain_witness.rs` — "Placeholder API only"
- `daimon.rs` — "Construct a placeholder daimon engine"
- `dreams.rs` — "Placeholder API only"
- `grimoire.rs` — "Construct a placeholder grimoire engine"
- `hypnagogia.rs` — "Placeholder API only"
- `mortality.rs` — "Construct a placeholder mortality engine"

Enabled in `roko-dreams` and `roko-serve` Cargo.toml but never actually used.

## 6. HDC Features: Built, Never Enabled by Default

HDC code exists behind feature flags, never enabled:
- `roko-compose/context_assembler.rs` — `semantic_similarity()` behind `#[cfg(feature = "hdc")]`
- `roko-fs/file_substrate.rs` — HDC fingerprinting optional
- `roko-neuro/knowledge_store.rs` — Full HDC indexing behind `#[cfg(feature = "hdc")]`
- `bardo-primitives/hdc.rs` — Core HDC operations behind `#[cfg(feature = "rkyv")]`

## 7. Executor Config: More Wired Than Before

From tasks.toml, most fields are now used by the executor:
- `tier` — wired into task model selection and dispatch
- `model_hint` — wired into dispatch overrides
- `read_files` — injected into task prompts / agent context
- `verify` commands — run after task completion
- `depends_on_plan` — used for readiness and blocking checks
- `write_files` — now injected into dispatch conventions / prompts and post-run auditing, but still not a full hard filesystem sandbox

## 8. Phase Handlers: No Longer Broadly Missing

The old “phase handlers not implemented” claim is stale:
- `Enriching` exists and is dispatched
- `Verifying` runs task `[[task.verify]]` commands
- `Reviewing` exists and now avoids falling back to repo-root diffs when plan worktrees are unavailable
- `DocRevision` exists

Remaining gap: these phases still do not cover every PRD-specified auto-fix / regeneration policy, but they are no longer absent.

## 9. Re-Planning: Strategy Enum Exists, Never Used

`ReplanStrategy` enum in `roko-orchestrator/src/replan.rs`:
- `Decompose` — split failed task into subtasks → NOT IMPLEMENTED
- `RetryWithEscalation` — haiku→sonnet→opus on failure → NOT IMPLEMENTED
- `RegeneratePlan` — structural plan regeneration → NOT IMPLEMENTED
Only basic "retry with error context" works.

## 10. Skill Library & Playbook: Partially Wired

- `SkillLibrary` is now loaded into `PlanRunner`
- failure recording and skill extraction paths are called from `orchestrate.rs`
- skill search/query participates in prompt-time context selection
- `PlaybookStore` exists but:
  - Never added to `PlanRunner`
  - Playbooks never recorded or looked up

## 11. LinUCB Bandit: Partially Wired

- `CascadeRouter::observe()` is now called with runtime rewards in `orchestrate.rs`
- Context vector construction never wired
- Model selection never uses bandit recommendation
- Per-crate "familiarity score" never tracked

## 12. TUI Dashboard: Scaffold Only

- `ratatui` and `crossterm` in Cargo.toml deps
- Pages directory exists but all pages are placeholder scaffolds
- Widgets are stub declarations (31 lines total)
- 6 dashboard pages designed but not rendered
- No keyboard navigation implemented
- No live refresh

## 13. Missing HTTP API Endpoints

The route stubs above are no longer the main gap. Remaining missing HTTP endpoints are mostly observability/learning surfaces:
- `GET /api/gates/summary`
- `GET /api/gates/{name}/history`
- `GET /api/learn/cascade`
- `GET /api/learn/experiments`
- `GET /api/learn/efficiency`
- `GET /api/learn/adaptive-thresholds`
- `GET /api/health`
- `GET /api/metrics/summary`
- WebSocket progress events never broadcast

## 14. Observability: Minimal

- `tracing-subscriber` not wired (only `tracing` facade)
- No `#[instrument]` spans in orchestrate.rs
- `--log-format json` CLI flag not implemented
- No cost aggregation at plan/session level
- No `ROKO_LOG` env var support

## 15. Worktree Manager: Wired, With Residual Edge Cases

`roko-orchestrator/src/worktree.rs` is now called from the dispatch loop:
- tasks can execute in isolated task worktrees
- successful task worktrees merge back into the plan worktree
- terminal and interrupted plan paths now clean up tracked worktrees

Remaining gap: abnormal lowest-level subprocess fallback and broader crash-proof cleanup semantics still deserve hardening, but “worktree manager never called” is no longer accurate.

---

## Summary

| Category | Items | LOC Built | LOC Wired |
|----------|-------|-----------|-----------|
| Safety (orchestrator) | 6 modules | ~2,441 | 0 |
| Safety (agent) | 6 guards | ~1,355 | partial |
| Conductor watchers | 10 | ~2,000 | partial |
| Serve routes | 3 former stubs | ~300 | mostly wired |
| roko-golem scaffold | 6 placeholders | ~1,100 | 0 |
| HDC features | 4 modules | ~3,000 | 0 |
| Executor features | ~6 fields | mostly wired | `write_files` still partial |
| Learning feedback | 3 systems | ~1,500 | 0 |
| **Total dormant code** | | **~12,000** | **partial / uneven** |

The codebase still has a large amount of built-but-unevenly-wired code, but this document originally overstated several zero-wiring claims. The remaining gaps are now more about universal enforcement and higher-order integration than missing first-call wiring.

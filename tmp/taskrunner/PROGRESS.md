# Taskrunner Progress Report

**Last updated**: 2026-05-06T01:30Z
**Branch**: `wp-arch2`

## Summary

| Status | Count | % |
|--------|-------|-----|
| **Implemented** | 100 | 100% |
| **Pending** | 0 | 0% |
| **Total** | 100 | |

**All 100 tasks fully implemented.** 4 batches of parallel agents completed over 2026-05-05/06.

## Wave Progress

| Wave | Total | Done | Pending | % Done |
|------|-------|------|---------|--------|
| 0: Foundation | 4 | 4 | 0 | 100% |
| 1: Parallel Fixes | 37 | 37 | 0 | 100% |
| 2: V2 Core + CLI | 33 | 33 | 0 | 100% |
| 3: Graph + Engine | 10 | 10 | 0 | 100% |
| 4: Feeds + Grad + Cal | 4 | 4 | 0 | 100% |
| 5: Migration + Hot | 3 | 3 | 0 | 100% |
| 6: Arch Cleanup | 2 | 2 | 0 | 100% |

---

## Next Steps: Audit Batch 4

**All 28 tasks from batch 4 need auditing.** The previous audit (batch 3) caught 3 P0 and 4 P1 issues.
Batch 4 had more complex tasks (graph engine, streaming redesign, WAL, graduation policy) so thorough
review is critical before merging to main.

### Audit checklist

1. **`cargo test --workspace`** — full test suite (last known: 686/687 pass before batch 4 additions)
2. **`cargo clippy --workspace --no-deps -- -D warnings`** — already PASS after batch 4 cleanup commit
3. **Per-task code review** for all 28 batch 4 tasks (see "Recently Completed (batch 4)" below)
4. **Integration testing** — particularly:
   - Graph engine: plan-to-graph converter, hot graph, cognitive loop stubs
   - Streaming: `StreamEvent`/`stream_turn()` in tool_loop
   - WAL: crash recovery replay
   - State snapshots: SHA-256 checksum validation
   - Terminal: PTY grace period + scrollback reattach
   - Graduation: `GraduationCell` + `GraduationPolicy` + `Pulse::graduate()`
5. **Stub identification** — some implementations are intentionally stubbed:
   - `TaskExecutorCell` (dry-run only, live dispatch deferred)
   - 7 `PassthroughCell` stubs in cognitive loop
   - OTLP tracing layer (flag wired, layer installation is placeholder)
6. **P1 items from prior audit** (may still be open):
   - Config test isolation (`merge_global: false` in 5 tests)
   - `ApplyDagMutation` wildcard arm in orchestrate.rs
   - roko-graph dead files — batch 4 should have resolved most of these

### Files changed in batch 4

Key files to audit (all committed on `wp-arch2`, commit `1d6b1be17`):

| Category | Files |
|----------|-------|
| **Graph engine** | `convert.rs`, `hot.rs`, `cells/task_executor.rs`, `cells/stubs.rs`, `cells/graduation.rs`, `condition.rs`, `types.rs`, `cells/agent.rs`, `cells/compose.rs`, `tests/fanout_condition.rs` |
| **Streaming** | `tool_loop/mod.rs` (StreamEvent, TurnConfig, stream_turn()), `openai_compat_backend.rs` |
| **WAL + durability** | `learn/wal.rs`, `learn/runtime_feedback.rs`, `runtime/state_snapshot.rs` |
| **Observability** | `serve/routes/metrics.rs`, `serve/state.rs` |
| **Terminal** | `serve/terminal.rs` (PTY grace, scrollback, reattach) |
| **CLI** | `main.rs` (PlanEngine, --engine, feed cmd), `doctor.rs`, `runner/event_loop.rs`, `runner/sse_stream.rs`, `runner/output_sink.rs`, `commands/feed.rs` |
| **Config** | `config/graduation.rs`, `config/chain.rs`, `pulse.rs`, `runtime_event.rs` |
| **Conductor** | `watchers/context_window_pressure.rs` |
| **Prompt** | `dispatch/prompt_builder.rs` |
| **ACP** | `event_forward.rs`, `bridge_events.rs`, `config.rs`, `handler.rs`, `runner.rs` |

---

## Recently Completed (28 tasks, 2026-05-06 batch 4)

Finished by `claude-batch-4` agents (5 agents in sequence after rate limits on larger batches).
All P0 clippy/compile fixes applied. Build PASS, Clippy PASS.

| ID | Title | What was done |
|----|-------|---------------|
| 025 | Config unification | `load_config_unified()` fallback when `config.roko_config` is None in event_loop.rs |
| 036 | Gate Cell::execute() impls | Implemented `Cell::execute()` on CompileGate, TestGate, ClippyGate, DiffGate with verdict_to_engram pattern. 4 integration tests. |
| 042 | Phase 1 integration tests | 17 integration tests exercising Cell/Observe/Connect/Trigger/Pipeline traits (644 LOC). 3 CLI-level tests for `roko doctor`. |
| 056 | Workspace context in prompts | `generate_workspace_context()` (git branch, modified files, crate descriptions), `generate_cfactor_context()`. 6 tests. |
| 059 | SSE stream client + output sink | `SseStreamClient` (319 LOC) with reconnection/backoff. `FormattedStderrSink` with ANSI color/truncation. 20 tests total. |
| 066 | Graph foundation wiring | Wired existing Graph/Node/Edge types, fixed `NodeOutputStatus`, `GraphConfig`, execution error variants. |
| 067 | Graph engine wiring | Verified engine works, wired CLI commands. |
| 068 | Fan-out + conditional edges | Renamed `EdgeCondition` → `Condition` to resolve conflicts with live types. Rewrote fanout_condition.rs tests (26 tests). |
| 069 | Cell stubs (agent, compose) | Fixed `from_node_config()` to accept `&toml::Value`. |
| 070 | Conditional edge logic | Wired `Condition` evaluation into engine execution path. |
| 071 | Budget tracking | Wired `BudgetTracker` with resolved imports. |
| 082 | Streaming-first backend | `StreamEvent` (7 variants), `StreamEventKind`, `TurnConfig`, `stream_turn()` as primary LLM trait method. `collect_stream_to_response()`, `response_to_synthetic_stream()`. 13 tests. |
| 083 | RuntimeEvent progress | 6 new variants: `InferenceFirstToken`, `ToolCallStarted/Completed`, `TaskStarted/Completed`, `PipelinePhase`. Wired emission in event_loop.rs. |
| 086 | Terminal session reattach | PTY grace period (60s), scrollback ring buffer (512 chunks), `AttachResult` enum, `mark_disconnected()`, `reap_expired()`, `attach_session()`. |
| 091 | Doctor command enhancements | `check_v2_abstractions()`, `check_claude_cli()`, `check_anthropic_api_key()`, `check_rust_version()`, `check_node_version()`. Added `fix: Option<String>` field. |
| 092 | Write-Ahead Log | `WalEntry` (3 variants), `WalWriter`, `replay_wal()` (305 LOC). Integrated into runtime_feedback at 3 event sites. 5 tests. |
| 093 | Context window pressure watcher | Config-gated (`context_pressure_enabled`), lookback window (3), configured windows map from `ModelProfile`. 8 tests. |
| 094 | Atomic state snapshots | `StateSnapshot` with SHA-256 checksum, version validation. Rewrote `save_snapshot()` in event_loop.rs. 6 tests. |
| 095 | Prometheus metrics endpoint | `GET /metrics` (293 LOC), 7 new metric families registered in serve state. 3 tests. |
| 096 | Observability emission wiring | OTLP tracing flag wired (layer installation placeholder). Metrics emission integrated into serve startup. |
| 098 | Feed CLI commands | `roko feed list/status` (169 LOC). `ServeFeeds` struct for runtime feed management in serve state. |
| 099 | Graduation policy | `GraduationPolicy`, `GraduationConfig`, `should_graduate()` (382 LOC). `GraduationCell` implementing both Graph Cell and Core React traits (328 LOC). `Pulse::graduate()` with 6+17 tests. |
| 101 | Plan-to-graph converter | `plan_to_graph()`, `PlanTaskInfo` (387 LOC). Cycle detection, dependency validation, cross-plan dep warnings. 8 tests. |
| 102 | CLI plan engine flag | `PlanEngine` enum, `--engine` flag on `plan run`, `--quick`/`--effective` flags, `--follow` on show. |
| 103 | Hot graph engine | `HotPolicy`, `HotGraphHandle`, `start_hot()` (334 LOC). `PassthroughCell`, `COGNITIVE_LOOP_STUBS`. Cognitive loop TOML definition. 4 tests. |
| 104 | StateHub crate boundary | Moved StateHub from roko-core to roko-runtime. Added doc comments for clippy. |
| 105 | ACP event mapping tests | 20 ACP event mapping tests in `event_forward.rs`. Clippy fixes across 4 ACP files. |

### Clippy fixes applied in batch 4 cleanup

All of these were fixed in commit `1d6b1be17`:

- `unnecessary_literal_bound`: `&str` → `&'static str` in task_executor.rs
- `unused_imports`: `serde_json::json` moved to `#[cfg(test)]` in convert.rs
- `missing_doc_comments`: added doc comment on StateHub in state_hub.rs
- `too_long_first_doc_paragraph`: split doc paragraph in context_window_pressure.rs
- `too_many_arguments`: allowed in roko-acp bridge_events.rs
- `collapsible_if`: collapsed 5 instances across roko-acp
- `redundant_else`: removed in roko-serve terminal.rs
- `more_private_than_parent`: `pub(crate)` → `pub` on `AttachResult` in terminal.rs
- `dead_code`: removed `resolve_config`, `OneshotMode` import in roko-cli main.rs
- Feature-gated `preflight_gate_deps` behind `legacy-runner-v2`

---

## Recently Completed (20 tasks, 2026-05-05 batch 3)

Finished by parallel claude-batch-2 agents (20 opus agents in parallel):

| ID | Title | What was done |
|----|-------|---------------|
| 001 | Unified config loader | LayeredConfigLoader with source priority, env overlay, validation. |
| 004 | Workspace struct wiring | Workspace/RokoLayout struct with boundary detection + path helpers. |
| 006 | Output sink wiring | OutputSink trait + ProgressSink + StreamEvent enum wired into runner. |
| 035 | Cell execute trait | CellContext, TypeSchema, async execute() default impl. Merged from worktree. |
| 045 | Bound streaming channels | Replaced unbounded with capacity-limited channels + backpressure. |
| 049 | roko dev command | Starts serve + watcher + demo frontend in parallel. |
| 051 | Integration test suite | Test harness structure, fixtures, helper utilities. |
| 053 | Workspace persistence | State serialization/deserialization + recovery. |
| 055 | Docker multistage cleanup | Optimized Dockerfile layers, reduced image size. |
| 072 | CLI boot sequence | Phased init, lazy loading, fast path for simple commands. |
| 073 | ACP startup resilience | Graceful degradation, retry on init failure, health probes. |
| 074 | Claude CLI provider fixes | Usage extraction, finish reason, reasoning, session_id. Merged from worktree. |
| 075 | Provider translator parity | All backends produce consistent ChatResponse fields. |
| 076 | Tool dispatch safety | Path confinement, argument validation, permission checks. |
| 077 | Model identity redesign | ModelId type, profile registry, capability detection. |
| 078 | Learning loop completeness | Feedback channels, signal propagation, loop closure. |
| 081 | Error type hierarchy | RokoError enum, classify_error(), From impls for all crate errors. |
| 084 | Concurrency sweep | Arc/Mutex audit, Send+Sync bounds, deadlock prevention. |
| 089 | Orchestration cleanup | Dead code removal, module reorganization, pub visibility audit. |
| 090 | Provider UX redesign | Unified provider config, health dashboard, connection testing. |

## Implemented Tasks (26)

Code verified in codebase on `wp-arch2`:

| ID | Title | Source |
|----|-------|--------|
| 002 | IndexMap migration for provider/model ordering | codex/demo-running |
| 007 | Gate pipeline: TOML-configurable shell commands | codex/demo-running-B3 |
| 008 | AdaptiveBudget wiring to prompt assembly | codex/demo-running-B4 |
| 009 | SafetyLayer.check() wired to all backends | codex/demo-running-B6 |
| 010 | Playbook outcome recording in episodes | codex/demo-running |
| 012 | Wire validate_against_schema() into plan loading | codex/demo-running-B7 |
| 013 | SSE keepalive + bound replay buffer (.take(256)) | codex/demo-running |
| 017 | JSONL rotation for episode/efficiency logs | codex/demo-running |
| 018 | IDE/ACP SessionNewParams (model/provider/effort) | codex/demo-running |
| 020 | IDE/ACP command categories + bare_mode filtering | codex/demo-running |
| 023 | Health check degradation detection | codex/demo-running |
| 024 | Wire agents_instructions_section() for all 7 templates | codex/demo-running |
| 028 | Orchestrate feature gate cleanup (legacy-orchestrate) | codex/demo-running |
| 031 | Wire CalibrationPolicy to CascadeRouter | codex/demo-running |
| 037 | Rename Engram -> Signal (alias in core) | codex/demo-running |
| 039 | Observe trait definition | codex/demo-running |
| 040 | Connect trait definition | codex/demo-running |
| 041 | Trigger trait definition | codex/demo-running |
| 048 | CI pipeline hardening (rust-toolchain pinning) | codex/demo-running |
| 058 | roko show command (full topic dispatch) | codex/demo-running-C2 |
| 061 | IDE/ACP max_output surfacing | codex/demo-running |
| 062 | IDE/ACP provider readiness boolean | codex/demo-running |
| 063 | IDE/ACP MCP status notification + discovery timeout | codex/demo-running |
| 064 | IDE/ACP default model/provider fallback logic | codex/demo-running |
| 097 | Feed trait + FeedRegistry infrastructure | codex/demo-running |
| 100 | Predict-publish-correct calibration loop | codex/demo-running |

## Recently Completed (20 tasks, 2026-05-05 batch 2)

Finished by parallel claude-batch-1 agents (20 opus agents in parallel):

| ID | Title | What was done |
|----|-------|---------------|
| 014 | Clippy suppression removal | Removed blanket #![cfg_attr(clippy, allow(...))] from main.rs. Added targeted #[allow(clippy::large_enum_variant)]. |
| 015 | RunLedger wiring | Wired RunLedger into Runner v2 event_loop: task starts, completions, gate outcomes → .roko/state/run-ledger.jsonl. |
| 016 | Error enrichment wiring | build_gate_retry_context() helper using classify_gate_failure + render_failure_classification. 3 tests. |
| 019 | IDE MCP error accumulation | McpErrorAccumulator wired through McpHandlerResolver + ToolLoop. Errors drain to ToolLoopOutput.mcp_errors. 8 tests. |
| 026 | TopicFilter combinators | Added And/Or/Not variants to TopicFilter with matches() arms. 6 unit tests. |
| 027 | Engram balance field | balance: f64 field + serde default + touch() + EngramBuilder support. 5 tests. |
| 029 | Delete roko-calc | Removed empty skeleton crate (not in workspace, no dependents). |
| 030 | Tag floating code | Tagged 14 floating modules (6 runtime, 8 learn) with //! STATUS: NOT WIRED comments. |
| 032 | DemurrageConsumer wiring | start_demurrage_timer() in roko-serve background. 5-min interval + KnowledgeStore::apply_demurrage(). |
| 033 | PostGateReflection wiring | lessons_from_post_gate_reflections() loads past lessons into gate retry prompts. 6 tests. |
| 034 | SectionOutcome wiring | Tracks prompt sections per attempt, records to .roko/learn/section-outcomes.jsonl on gate terminal. |
| 038 | Signal rename propagation | Engram→Signal in 73 files across 5 crates (agent, gate, learn, compose, orchestrator). |
| 043 | Sync mutex audit | Verified no change needed: serve uses tokio::sync::Mutex, runtime_feedback locks are sync-correct. |
| 044 | MCP transport timeout | 5s stdin write + 30s response timeouts on StdioTransport::roundtrip(). |
| 046 | PRD promote atomicity | YAML frontmatter parser (serde_yaml_ng). Atomic promote + plan writes. Tests for colons/lists. |
| 047 | TOCTOU fixes | Fixed check-then-use patterns in 10 files (serve, cli dispatch, repo_context, repl). |
| 050 | Silent error swallowing | Added tracing::warn! to silent error paths in 8 route files + provider. |
| 052 | Atomic writes migration | PRD plan writes + cascade_router save → atomic_write_str. |
| 054 | Retry backoff consolidation | RetryPolicy::execute() async helper in roko-core. 3 tests. |
| 079 | Magic number centralization | 15 DEFAULT_* constants. Replaced literals in runner event_loop/persist/state/types + serve/deployments. |
| 080 | Unwrap elimination | Replaced .unwrap()/.expect() in serve/lib, agent/provider/*, chain/marketplace, cli/prd. |

## Previously Completed (6 tasks, 2026-05-05 batch 1)

| ID | Title | What was done |
|----|-------|---------------|
| 003 | TimeoutConfig wiring | Replaced hardcoded Duration::from_secs across chat.rs, evaluator.rs, event_loop.rs. 6 timeout helpers + 3 tests. |
| 021 | Demo scenario redesign | Created CostComparisonPanel, MemoryTransferPanel, OracleFlowPanel (all SSE-driven). Sidebar routing wired. e2e updated. |
| 057 | roko do command | Full classify→route→execute pipeline. Simple/standard/complex paths in do_cmd.rs. 30+28 tests. Dead code removed. |
| 085 | Config architecture | Fixed critical ArcSwap duplication bug in ConfigCache (watcher wrote to different swap than get() read from). Regression test. |
| 087 | Frontend architecture | TIMEOUTS/RECONNECT_BACKOFF in serve-url.ts. Polling→SSE. Stale closures fixed. modelColor(). ErrorBoundary wrapped. |
| 088 | ACP architecture sweep | configSources prefixes/ordering. Live-reload IDE notification. Implicit global watched. Effort docs. 7 tests. |

## Codex Batch Work (not in task list)

The `codex/demo-running-*` batch merged 25 branches into `wp-arch2`. Some covered
task-list items (counted above). Others delivered infrastructure without a direct task ID:

- **A5**: Event ingest endpoint (`/events/ingest` + `/events/ingest/batch`)
- **A6**: HttpEventSink (`roko-runtime/src/http_event_sink.rs`)
- **A7**: PTY env injection (ZDOTDIR, serve_url)
- **A8**: ACP event bridge (`AcpEventForwarder`)
- **B2**: Inline terminal (`InlineTerminal` + `RunnerInlineTerminal`)
- **C1**: roko do command (partial — counted under task 057)
- **C3**: roko think + tune commands
- **D1**: RuntimeEvent variants (InferenceStarted/Completed/Failed, AgentTrace)
- **D2**: SSE adapter variants (RuntimeEvent -> SSE bridge)
- **D3**: Inference tracking (model_call_service emits lifecycle events)
- **D4**: Agent trace events (AgentTracePayload)
- **E1**: Terminal session fundamentals (PTY structure)
- **E2**: Archive old scenarios (14->5 collapse)
- **E3**: SSE client hooks (EventStreamContext, useEventStream)
- **E5**: Pipeline demo scenario

## Batch History

| Batch | Date | Tasks | Agent Strategy | Result |
|-------|------|-------|----------------|--------|
| Codex | pre-2026-05-05 | 26 | Codex multi-branch merge | 26 tasks implemented |
| 1 | 2026-05-05 | 6 | 6 individual opus agents | 6 tasks implemented |
| 2 | 2026-05-05 | 20 | 20 opus agents in parallel (rate-limited) | 20 tasks implemented |
| 3 | 2026-05-05 | 20 | 20 opus agents in parallel | 20 tasks implemented |
| 4 | 2026-05-06 | 28 | 5 sequential agents (rate limits) | 28 tasks, build+clippy PASS |

## Audit History

### Audit 1 (2026-05-05, post-batch 3)

Full branch audit completed with 20 parallel agents. See `audits/2026-05-05-wp-arch2-audit.md`.

**Result**: Build PASS, Clippy PASS, 686/687 tests. 3 P0 + 4 P1 issues found.

P0 fixes (all applied in batch 4):
- roko-acp test compile: `IndexMap::new()` + `max_tool_iterations: None` in helpers.rs
- roko-serve block_in_place: `#[tokio::test(flavor = "multi_thread")]` on 2 tests
- IPv6 bracket in CORS: added `host == "[::1]"` check

P1 items (may still be open — verify in audit 2):
- Config test isolation (`merge_global: false` in 5 tests)
- `ApplyDagMutation` wildcard arm in orchestrate.rs
- roko-graph dead files (most should be resolved by batch 4 tasks 066-071)

### Audit 2 (PENDING — next step)

**All 28 batch 4 tasks need auditing.** See "Next Steps: Audit Batch 4" at top.

Priority areas:
1. Graph engine (tasks 066-071, 101-103) — most complex new code
2. Streaming redesign (082, 083) — trait changes affect all backends
3. WAL + state snapshots (092, 094) — durability correctness is critical
4. Graduation policy (099) — dual-trait implementation (Graph Cell + Core React)

## Git State

- Branch: `wp-arch2` (12 commits ahead of `origin/wp-arch2`)
- Latest commit: `1d6b1be17` — "Fix clippy lints + post-batch cleanup for all 28 task implementations"
- **Not yet pushed to origin** (needs explicit approval)
- Working tree: clean after batch 4 commit

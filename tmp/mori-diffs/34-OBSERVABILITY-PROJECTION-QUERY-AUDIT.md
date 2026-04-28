# 34 - Observability Projection Query Audit

Date: 2026-04-27

Purpose: this file documents the observability, projection, HTTP query, TUI state, and proof-surface architecture gaps that keep Roko from proving runtime behavior end to end. It complements [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md), [30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md](30-ARCHITECTURAL-SIDE-EFFECT-AUDIT.md), [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), and [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md).

### Architecture Runner Update (2026-04-28)
Observability infrastructure now in place:
- `RuntimeEvent` enum (P0A) defines the universal event vocabulary
- `EventBus` (P0C) provides fan-out to all consumers
- `RuntimeProjection` (P3C) provides materialized view for queries
- `SseAdapter` (P3B) streams events to web clients
- `JsonlLogger` (P3C) provides durable event journal for replay
- Remaining: REST query endpoints, TUI projection integration, full event catalog coverage

If an agent is assigned "make the UI/API/proof show what happened" or "fix observability end to end", this file is the implementation handoff.

## Executive Verdict

Roko has many observability components, but they are not one system. There is a runner projection facade, a core `StateHub`, a runtime event bus, a server event bus, a server projection contract, dashboard snapshots, TUI dashboard file readers, learning JSONL readers, and HTTP endpoints. The problem is that these are peers and bridges, not a single event store with materialized projections.

The current shape makes proof fragile. A run can emit events to the runner projection but not appear in server projection endpoints. A server route can publish `ServerEvent` and then mirror to `DashboardEvent`. A dashboard can recover by reading raw files instead of querying the projection service. HTTP endpoints can partly use `RuntimeProjectionSet` and partly read JSONL directly.

Initial self-grade after this pass: `9.84 / 10`.

Reason: this pass identifies the event/projection split, names the authoritative target architecture, lists concrete hot files and source evidence, and provides implementation checklists and grep gates. It is not a `10` because a full proof would include generated endpoint coverage and live runtime artifacts.

## Method

Commands used during this pass:

```bash
python3 - <<'PY'
from pathlib import Path
import re
root=Path('/Users/will/dev/nunchi/roko/roko')
files=sorted(root.glob('crates/**/*.rs'))
patterns={
  'event_bus': re.compile(r'EventBus|global_event_bus|PublishOrigin|RokoEvent|ServerEvent|LearningEventBus|AgentEvent|RunnerEvent|AgentRuntimeEvent'),
  'jsonl': re.compile(r'jsonl|JSONL|events\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl|engrams\\.jsonl|signals\\.jsonl|run-state\\.json|process-sessions\\.json'),
  'projection': re.compile(r'projection|Projection|Dashboard|dashboard|snapshot|Snapshot|truth_map|TruthSource'),
  'http_route': re.compile(r'\\.route\\(|Router::new|Json<|StatusCode|WebSocket|Sse|EventSource|/api/|/health|/metrics'),
  'raw_reads': re.compile(r'read_to_string|read_dir|File::open|OpenOptions|append_jsonl|write_jsonl|serde_json::from_str|serde_json::to_string'),
  'unbounded': re.compile(r'Vec<.*Event|VecDeque|push\\(|extend\\(|read_all_lossy|collect::<Vec|efficiency_events|episodes_cache'),
}
for path in files:
    text=path.read_text(errors='ignore')
    counts={k: len(p.findall(text)) for k,p in patterns.items()}
    if any(counts.values()):
        print(path.relative_to(root), counts)
PY
```

```bash
rg -n "EventBus|global_event_bus|PublishOrigin|RokoEvent|ServerEvent|LearningEventBus|AgentEvent|RunnerEvent|AgentRuntimeEvent|projection|Projection|Dashboard|dashboard|Snapshot|truth_map|TruthSource|events\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl|engrams\\.jsonl|signals\\.jsonl|run-state\\.json|process-sessions\\.json|read_all_lossy|append_jsonl|read_project_|efficiency_events|episodes_cache" \
  crates/roko-cli/src \
  crates/roko-serve/src \
  crates/roko-core/src \
  crates/roko-runtime/src \
  crates/roko-learn/src \
  crates/roko-neuro/src \
  crates/roko-fs/src \
  -g '*.rs'
```

## Current Scan Counts

| Crate | Event Refs | JSONL Refs | Projection Refs | HTTP Refs | Raw IO/Serde Refs | Potential Unbounded Refs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `roko-cli` | 341 | 399 | 2560 | 141 | 588 | 1567 |
| `roko-serve` | 264 | 138 | 1157 | 1479 | 187 | 277 |
| `roko-learn` | 63 | 248 | 429 | 128 | 181 | 362 |
| `roko-core` | 14 | 36 | 463 | 16 | 168 | 231 |
| `roko-agent` | 79 | 13 | 34 | 60 | 220 | 319 |
| `roko-orchestrator` | 0 | 0 | 375 | 4 | 29 | 98 |
| `roko-compose` | 0 | 0 | 53 | 0 | 28 | 324 |
| `roko-neuro` | 0 | 119 | 65 | 0 | 72 | 130 |
| `roko-fs` | 0 | 128 | 48 | 17 | 74 | 18 |
| `roko-dreams` | 0 | 47 | 41 | 0 | 40 | 99 |
| `roko-runtime` | 44 | 13 | 56 | 0 | 13 | 57 |

Interpretation:

- `roko-cli` is still the dominant observability owner, not just an adapter.
- `roko-serve` owns a large HTTP/projection surface and a parallel event bus.
- `roko-learn`, `roko-neuro`, and `roko-fs` still expose durable JSONL shapes directly.
- Projection and dashboard logic is spread across core, CLI, serve, and runtime.

## Hot Files

| File | Why It Matters |
| --- | --- |
| `crates/roko-cli/src/orchestrate.rs` | Legacy monolith still maps server events, dashboard events, learning events, JSONL, and snapshots. |
| `crates/roko-cli/src/tui/dashboard.rs` | TUI standalone mode reads `.roko` JSONL directly and materializes many dashboard views itself. |
| `crates/roko-serve/src/projection_contract.rs` | Server projection set joins StateHub, dashboard snapshot recovery, learning JSONL, provider health, and runtime feedback. |
| `crates/roko-core/src/state_hub.rs` | Core StateHub persists `DashboardEvent` to `.roko/events.jsonl` and materializes `DashboardSnapshot`. |
| `crates/roko-cli/src/runner/projection.rs` | Runner projection defines a normalized `ProjectionEvent`, but it is per-run and not the authoritative server/query projection engine. |
| `crates/roko-serve/src/lib.rs` | Bidirectional bridge maps `ServerEvent <-> DashboardEvent`; includes an explicit duplicate-event FIXME. |
| `crates/roko-serve/src/routes/projections.rs` | Exposes projection routes, but streams deltas from `StateHub` dashboard events, not from one durable runtime event store. |
| `crates/roko-serve/src/routes/status/gates.rs` | Uses `RuntimeProjectionSet` for public handlers but still contains direct `.roko/engrams.jsonl` and `.roko/events.jsonl` helpers. |
| `crates/roko-serve/src/routes/status/episodes.rs` | Episodes use canonical projections, while signals still read raw `engrams.jsonl`. |
| `crates/roko-serve/src/routes/status/metrics.rs` | Uses projections for some metrics and direct file/status logic for others. |
| `crates/roko-cli/src/tui/state.rs` | Keeps raw efficiency and episode caches in memory, including previously identified unbounded growth risk. |
| `crates/roko-learn/src/runtime_feedback.rs` | Learning feedback reads/writes durable JSONL and also acts as a projection-like layer. |

## Existing Pieces That Should Be Preserved

These pieces are useful, but they need to be subordinated to one runtime observability spine.

- `crates/roko-cli/src/runner/projection.rs` has a good provider-neutral `ProjectionEvent` concept, bounded dashboard buffer, dropped/coerced counters, and preview truncation.
- `crates/roko-core/src/state_hub.rs` has a useful watch/broadcast/replay model for dashboard consumers.
- `crates/roko-serve/src/projection_contract.rs` has useful projection catalog, query filters, evidence envelopes, and durable recovery metadata.
- `crates/roko-serve/src/routes/projections.rs` exposes a useful `/api/projections/{name}` and `/stream` shape.
- `crates/roko-serve/src/truth_map.rs` documents source ownership, but it is not yet an enforced runtime contract.
- `roko_learn::runtime_feedback` has useful durable learning readers, but they should become repositories/projection inputs instead of a second query layer.

## Target Design

The target is not "more events". The target is one event contract, one durable event store, one projection engine, and one query service.

| Layer | Component | Responsibility |
| --- | --- | --- |
| L1 event contract | `RuntimeEventEnvelope` | Versioned event id, run id, source, category, timestamp, correlation id, causation id, payload, redaction status |
| L2 event store | `RuntimeEventStore` | Append, replay, recover, truncate/compact, cursor assignment, redacted evidence storage |
| L3 projection engine | `ProjectionEngine` | Materialize dashboard, provider, gate, retry, cost, feedback, merge, worktree, resume, prompt, and background-task projections |
| L4 query service | `RuntimeQueryService` | Serve typed queries, list projections, explain freshness, return evidence and cursors |
| L5 adapters | CLI, HTTP, TUI, proof scripts | Subscribe/query/render only; never reconstruct raw runtime state from JSONL |

StateHub and dashboard snapshots should become projections of the runtime event store. They should not be the source of truth.

## P0 Findings

### P0-01 Event Vocabularies Are Parallel Instead Of One Contract

Problem:

The codebase has multiple event vocabularies that need bridge code: runner `RunnerEvent`, runner/inline `AgentEvent`, provider `AgentRuntimeEvent`, core `DashboardEvent`, serve `ServerEvent`, runtime `RokoEvent`, and learn `AgentEvent`.

Evidence:

```text
crates/roko-cli/src/runner/projection.rs: RawRuntimeEvent accepts RunnerEvent, AgentEventInput, and Custom.
crates/roko-core/src/state_hub.rs: StateHub materializes DashboardEvent.
crates/roko-serve/src/lib.rs: server_event_to_dashboard maps ServerEvent -> DashboardEvent.
crates/roko-serve/src/lib.rs: dashboard_event_to_server maps DashboardEvent -> ServerEvent.
crates/roko-runtime/src/event_bus.rs: RokoEvent process-local runtime bus.
crates/roko-learn/src/events.rs: learning AgentEvent.
crates/roko-cli/src/inline/agent_events.rs: inline AgentEvent stream.
```

Why it matters:

Every bridge loses information. The current server bridge explicitly drops some dashboard variants and warns about duplicate event loops. That makes proof difficult because the event a provider emitted may not be the event an HTTP endpoint exposes.

Target design:

All entrypoints should emit `RuntimeEventEnvelope`. Legacy event types should be adapter input or projection output, not authoritative runtime contracts.

Implementation checklist:

- [ ] Define `RuntimeEventEnvelope` with schema version, event id, cursor, run id, source, category, event type, timestamp, correlation id, causation id, redaction status, and payload.
- [ ] Define canonical categories for provider, prompt, task, plan, gate, retry, merge, worktree, resume, feedback, dream, knowledge, safety, background task, HTTP, and UI.
- [ ] Add adapters from `RunnerEvent`, `AgentRuntimeEvent`, `DashboardEvent`, `ServerEvent`, `RokoEvent`, and learning events into `RuntimeEventEnvelope`.
- [ ] Make adapters one-way into runtime events; remove bidirectional bridge loops after migration.
- [ ] Add schema tests for every canonical event type.
- [ ] Add grep gate: `rg "server_event_to_dashboard|dashboard_event_to_server|RawRuntimeEvent::Custom|LearningEventBus|global_event_bus" crates` requires an allowlist and retirement plan.

### P0-02 StateHub Is Acting As A Source Of Truth

Problem:

`StateHub` is useful for live dashboard delivery, but it currently persists `DashboardEvent` to `.roko/events.jsonl` and reconstructs snapshots by replaying dashboard events. Server projections then combine live StateHub state, recovered dashboard snapshots, and durable learning JSONL.

Evidence:

```text
crates/roko-core/src/state_hub.rs: publish broadcasts, updates DashboardSnapshot, and appends DashboardEvent to .roko/events.jsonl.
crates/roko-core/src/state_hub.rs: replay_from_log reads DashboardEvent JSONL into snapshot.
crates/roko-serve/src/projection_contract.rs: RuntimeProjectionSet::load uses live StateHub, DashboardSnapshot::load_from_workdir, and learning JSONL.
crates/roko-serve/src/routes/projections.rs: stream_projection subscribes to state.state_hub.subscribe_events().
```

Why it matters:

Dashboard events are view events, not complete runtime facts. If the durable store is a dashboard event log, later projections cannot recover provider lifecycle, prompt diagnostics, retry reasons, merge evidence, safety policy, or credential status unless those were manually mirrored into dashboard payloads.

Target design:

StateHub should be a materialized projection fed by `ProjectionEngine`. It can keep a watch channel and ring buffer, but the durable source must be `RuntimeEventStore`.

Implementation checklist:

- [ ] Create `RuntimeEventStore` with append/replay/cursor APIs.
- [ ] Make runner, server, dreams, provider dispatch, gates, merge, and feedback append runtime events.
- [ ] Make `StateHub` subscribe to `ProjectionEngine` dashboard projection instead of persisting dashboard events as source facts.
- [ ] Keep `.roko/events.jsonl` only for `RuntimeEventEnvelope`, not `DashboardEvent`.
- [ ] Add migration reader for old dashboard-event logs.
- [ ] Add proof that a run can be recovered from runtime events into dashboard, provider, gate, retry, and proof projections.

### P0-03 HTTP Endpoints Do Not All Query The Same Projection Service

Problem:

Some endpoints use `RuntimeProjectionSet`; others read raw files, inspect state maps, proxy sidecars, or compute status locally. This means the API surface is not a single query contract.

Evidence:

```text
crates/roko-serve/src/routes/projections.rs: uses RuntimeProjectionSet.
crates/roko-serve/src/routes/status/gates.rs: public handlers use RuntimeProjectionSet but helper code still reads .roko/engrams.jsonl and .roko/events.jsonl.
crates/roko-serve/src/routes/status/episodes.rs: episodes use RuntimeProjectionSet, signals reads .roko/engrams.jsonl directly.
crates/roko-serve/src/routes/status/metrics.rs: mixes RuntimeProjectionSet with endpoint-specific metric computation.
crates/roko-serve/src/routes/providers.rs: computes provider health and direct test responses locally.
crates/roko-serve/src/routes/agents.rs: computes agent dashboard payloads and provider inference locally.
```

Why it matters:

The user needs to query proof that a provider run, retry, merge, crash/resume, and HTTP/UI state all agree. That cannot be definitive while endpoints use different source paths.

Target design:

All read endpoints should call `RuntimeQueryService`. Route handlers should not read `.roko` files directly or reconstruct projection rows.

Implementation checklist:

- [ ] Define `RuntimeQueryService` with typed queries for runs, tasks, agents, providers, prompts, gates, retries, merges, worktrees, knowledge, dreams, costs, feedback, background tasks, and proof bundles.
- [ ] Make `/api/projections/{name}` the generic projection endpoint backed by `RuntimeQueryService`.
- [ ] Migrate `/api/status`, `/api/metrics/*`, `/api/gates/*`, `/api/episodes`, `/api/signals`, `/api/providers/*`, and `/api/agents/*` to query service methods.
- [ ] Remove route-local JSONL readers except migration or download endpoints.
- [ ] Add endpoint coverage proof that all runtime-observability endpoints include `projection_name`, `cursor`, `computed_at`, `source_evidence`, and `freshness`.
- [ ] Add grep gate: `rg "read_jsonl_entries|read_to_string|engrams\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl|events\\.jsonl" crates/roko-serve/src/routes` returns only repository/query adapters.

### P0-04 Projection Streams Emit Deltas From Dashboard Events, Not Runtime Events

Problem:

`/api/projections/{name}/stream` sends an initial projection state from `RuntimeProjectionSet`, then streams deltas from `StateHub` `DashboardEvent`s. The initial state and delta stream are not materialized by the same engine.

Evidence:

```text
crates/roko-serve/src/routes/projections.rs: initial state uses RuntimeProjectionSet::load.
crates/roko-serve/src/routes/projections.rs: delta stream uses state.state_hub.subscribe_events().
crates/roko-serve/src/projection_contract.rs: projection_delta_frame receives DashboardEvent.
```

Why it matters:

Consumers cannot safely apply deltas if the deltas are not in the same schema as the projection state. This also prevents durable replay of projection deltas after reconnect.

Target design:

Projection streams should stream projection frames produced by `ProjectionEngine`, keyed by projection name and runtime cursor. Deltas should include enough data to update the projection or should force a reload with a cursor.

Implementation checklist:

- [ ] Define `ProjectionFrame` with `projection_name`, `version`, `cursor`, `frame_type`, `data`, `invalidates`, and `source_event_id`.
- [ ] Make projection streams subscribe to `ProjectionEngine`, not `StateHub`.
- [ ] Ensure initial and delta frames use the same schema and cursor space.
- [ ] Add lag handling that emits `resync_required` when a client misses frames.
- [ ] Add proof that a stream reconnect can resume from cursor or request a full state reload.

### P0-05 Proof Artifacts Are Not First-Class Projections

Problem:

Provider matrix, gate retry, merge success/conflict, crash/resume, prompt diagnostics, and HTTP query proof are expected, but proof is not one projection family with standard evidence rows.

Evidence:

```text
tests/proof/mori-diffs/prove-runtime-end-to-end.sh exists as a proof harness.
crates/roko-serve/src/projection_contract.rs exposes execution_trace, provider_state, retry_state, cost_state, runtime_feedback.
Runtime proof requirements in 29 and 33 still require live evidence.
```

Why it matters:

Proof should not scrape logs differently per feature. It should query a stable proof projection and archive redacted evidence.

Target design:

Add `proof_state` and `proof_bundle` projections with rows for each required behavior and source artifact.

Implementation checklist:

- [ ] Define `ProofEvidenceRow` with proof id, feature id, status, event ids, artifact paths, redaction status, command, provider/model, run id, and failure class.
- [ ] Add projections for provider matrix, HTTP query, retry/replan, resume, merge success, merge conflict, prompt diagnostics, and safety policy.
- [ ] Make proof harness query projection endpoints instead of grepping raw files.
- [ ] Add proof bundle export that copies redacted events, projections, command output, and config provenance.
- [ ] Add endpoint `/api/projections/proof_state` or equivalent.

## P1 Findings

### P1-01 TUI Still Materializes Runtime State From Files

Problem:

The TUI dashboard has push-based pieces, but standalone dashboard paths still read `.roko` files directly and cache raw event lists. This repeats server projection logic in the CLI.

Evidence:

```text
crates/roko-cli/src/tui/dashboard.rs: DashboardData reads .roko/engrams.jsonl, .roko/episodes.jsonl, .roko/learn/efficiency.jsonl, cascade snapshots, provider health, and git diff.
crates/roko-cli/src/tui/dashboard.rs: DashboardSnapshot::load reads many durable files directly.
crates/roko-cli/src/tui/state.rs: stores efficiency_events and episodes_cache.
```

Why it matters:

The TUI can disagree with HTTP projection endpoints. It also risks unbounded memory use when raw event vectors grow.

Target design:

TUI should use `RuntimeQueryService` snapshots and subscriptions. Standalone mode should start or connect to a local query service, not rebuild projections itself.

Implementation checklist:

- [ ] Replace TUI raw JSONL loaders with query-service calls.
- [ ] Replace raw `efficiency_events` and `episodes_cache` with bounded projection windows.
- [ ] Make standalone `roko dashboard --text` query the same projection path as HTTP `/api/dashboard`.
- [ ] Keep direct file readers only as offline migration/debug commands.
- [ ] Add proof that TUI text output and HTTP projection output share the same cursor and evidence.

### P1-02 Learning Feedback Is Also A Query Layer

Problem:

`roko-learn::runtime_feedback` reads project episodes, efficiency events, and learning snapshots. Server projections call into these readers directly. This creates a second projection layer below server projections.

Evidence:

```text
crates/roko-learn/src/runtime_feedback.rs: reads/writes durable project learning data.
crates/roko-serve/src/projection_contract.rs: imports read_project_efficiency_events and read_project_episodes_lossy.
crates/roko-serve/src/routes/learning/mod.rs: learning endpoints project some data directly from RuntimeProjectionSet.
```

Why it matters:

Learning data is domain state, but runtime observability needs uniform event/projection semantics. If learning readers return raw domain rows, runtime proof must know learning internals.

Target design:

Learning should expose repositories and event sinks. ProjectionEngine should own the observability materialization.

Implementation checklist:

- [ ] Split learning persistence readers into repository traits.
- [ ] Make `RuntimeProjectionSet` depend on repositories, not direct JSONL helpers.
- [ ] Emit learning feedback events into `RuntimeEventStore`.
- [ ] Materialize learning projections from runtime events plus repositories.
- [ ] Add proof that learning, provider, retry, and cost projections can be rebuilt from a clean event/repository snapshot.

### P1-03 Event Retention And Query Windows Are Not Uniform

Problem:

Different components impose different windows: runner projection keeps 200 dashboard events, StateHub ring defaults to 1024, status helpers use `MAX_JSONL_RESULTS`, TUI caches raw lists, and projection endpoints use caller limits inconsistently.

Evidence:

```text
crates/roko-cli/src/runner/projection.rs: DASHBOARD_MAX_EVENTS = 200 and channel capacity = 1024.
crates/roko-core/src/state_hub.rs: default ring capacity = 1024.
crates/roko-serve/src/routes/status/helpers.rs: MAX_JSONL_RESULTS.
crates/roko-cli/src/tui/dashboard.rs: reads raw efficiency events and episodes.
```

Why it matters:

Inconsistent windows cause API/TUI/proof mismatches. A proof can fail because one endpoint truncated evidence another endpoint still sees.

Target design:

Projection windows should be query parameters with defaults declared in projection policy. Durable proof queries should request explicit windows or evidence ids.

Implementation checklist:

- [ ] Add `default_limit`, `max_limit`, and `retention_policy` to each projection catalog entry.
- [ ] Apply limits centrally in `RuntimeQueryService`.
- [ ] Emit `truncated: true` and `next_cursor` or `next_offset` when results are limited.
- [ ] Make proof queries request explicit feature-specific evidence ids instead of relying on "recent N".
- [ ] Add tests for consistent limits across HTTP, TUI, and CLI text output.

### P1-04 Bridge Loops Create Duplicate And Dropped Events

Problem:

The server bridge maps `ServerEvent -> DashboardEvent` and `DashboardEvent -> ServerEvent`. The reverse bridge drops unmapped variants and has an explicit duplicate-event FIXME.

Evidence:

```text
crates/roko-serve/src/lib.rs: start_state_hub_bridge maps server event bus to StateHub.
crates/roko-serve/src/lib.rs: start_orchestrator_event_bridge maps StateHub to server event bus.
crates/roko-serve/src/lib.rs: FIXME: bridge loop - REST-originated events appear twice on EventBus.
crates/roko-serve/src/lib.rs: unmapped variants such as Diagnosis, ExperimentWinnersUpdated, CFactorTrendUpdated, CascadeRouterUpdated, GateThresholdsUpdated are dropped.
```

Why it matters:

Duplicate events corrupt counts. Dropped events make UI/API incomplete. Both are unacceptable for proof.

Target design:

Remove bridge loops. All producers append `RuntimeEventEnvelope`. ProjectionEngine fans out to dashboard and server streams.

Implementation checklist:

- [ ] Add event ids and source ids before bridge migration.
- [ ] Add duplicate detection in current bridges as a temporary guard.
- [ ] Convert server routes to append runtime events instead of publishing server events directly.
- [ ] Convert dashboard subscribers to consume projection frames.
- [ ] Remove reverse bridge after server streams use projection frames.
- [ ] Add proof that one REST-originated action produces exactly one runtime event and one projection update.

## P2 Findings

### P2-01 Truth Map Is Documentation, Not Enforcement

Problem:

`roko-serve/src/truth_map.rs` documents authoritative sources, but runtime code does not enforce that reads go through those sources.

Evidence:

```text
crates/roko-serve/src/truth_map.rs: documents truth sources and projection paths.
Route handlers and TUI files still read raw source files directly.
```

Target design:

Truth map should become a machine-readable ownership registry used by query services and grep gates.

Implementation checklist:

- [ ] Convert truth map entries into `ProjectionSourceSpec` entries.
- [ ] Use specs to generate projection catalog and docs.
- [ ] Add CI that flags direct reads of truth-owned files outside repositories.
- [ ] Add `/api/truth_map` evidence showing owner, repository, projection, and allowed readers.

### P2-02 Route Surface Is Not Generated From Projection Catalog

Problem:

There is a projection catalog, OpenAPI docs, and many route modules, but endpoint coverage is not generated from the same projection definitions.

Evidence:

```text
crates/roko-serve/src/projection_contract.rs: projection_policies returns catalog.
crates/roko-serve/src/openapi.rs: separately documents many status, provider, and projection endpoints.
crates/roko-serve/src/routes/status/mod.rs: separately declares status and metrics routes.
```

Target design:

Projection endpoints should be catalog-driven where possible. Specialized routes should be aliases or views over projection queries.

Implementation checklist:

- [ ] Add endpoint registry generated from projection catalog.
- [ ] Add aliases for legacy routes that call projection queries.
- [ ] Add docs generation from projection catalog.
- [ ] Add coverage test that every projection has HTTP read, optional stream, schema version, and proof sample.

## Implementation Order

Implement in this order:

1. [ ] Define `RuntimeEventEnvelope` and event categories.
2. [ ] Define `RuntimeEventStore` and migrate `.roko/events.jsonl` to runtime events.
3. [ ] Build `ProjectionEngine` with materializers for dashboard, provider, gate, retry, cost, feedback, prompt, merge, resume, safety, background tasks, and proof.
4. [ ] Define `RuntimeQueryService` and route `/api/projections/*` through it.
5. [ ] Convert StateHub into a dashboard projection subscriber.
6. [ ] Convert server streams to projection frames and remove bidirectional bridge loops.
7. [ ] Convert TUI dashboard and CLI text dashboard to query-service snapshots.
8. [ ] Convert learning feedback readers into repositories/projection inputs.
9. [ ] Add proof-state projections and update proof harnesses to query them.
10. [ ] Add grep gates and generated endpoint/projection coverage.

## Grep Gates

These commands should eventually pass with zero output or a documented allowlist.

```bash
rg "server_event_to_dashboard|dashboard_event_to_server|FIXME: bridge loop" crates/roko-serve/src crates/roko-cli/src
```

```bash
rg "read_jsonl_entries|read_to_string|engrams\\.jsonl|signals\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl|events\\.jsonl" crates/roko-serve/src/routes crates/roko-cli/src/tui -g '*.rs'
```

```bash
rg "DashboardEvent|ServerEvent|RunnerEvent|LearningEventBus|AgentRuntimeEvent|RokoEvent" crates -g '*.rs'
```

```bash
rg "efficiency_events: Vec|episodes_cache: Vec|read_all_lossy|collect::<Vec" crates/roko-cli/src/tui crates/roko-serve/src crates/roko-learn/src -g '*.rs'
```

```bash
rg "ProjectionEvent|RuntimeProjectionSet|StateHub|RuntimeEventEnvelope|RuntimeQueryService|ProjectionEngine" crates -g '*.rs'
```

## Proof Requirements

Observability work is complete only when all of these are proven:

- [ ] A real provider run emits runtime events with provider lifecycle, prompt diagnostics, policy, cost, and usage.
- [ ] `/api/projections/execution_trace` shows the same run id, task id, provider, model, gate, retry, and cost evidence.
- [ ] `/api/projections/provider_state` reports provider status and source evidence.
- [ ] `/api/projections/retry_state` reports retry decisions and gate failure context.
- [ ] `/api/projections/proof_state` or equivalent reports provider matrix, HTTP query, resume, merge success, merge conflict, prompt, and safety proof rows.
- [ ] TUI text dashboard and HTTP projection show the same cursor and evidence for a run.
- [ ] Restart recovery rebuilds projections from durable runtime events without duplicate completion.
- [ ] Stream reconnect either resumes from cursor or emits `resync_required`.
- [ ] Redaction canary is absent from runtime events, projection responses, streams, proof bundles, and logs.
- [ ] A bridge-loop proof shows one source action produces one runtime event and no duplicate server/dashboard event.

## Agent Handoff Checklist

Use this checklist when assigning observability work to an agent with no other context.

- [ ] Read this file, [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md), and [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md).
- [ ] Run the scan commands from the Method section and save before counts.
- [ ] Pick one P0 finding.
- [ ] Implement the target event/projection/query seam without adding another bridge.
- [ ] Add or update grep gates for the bypass being removed.
- [ ] Run the grep gate before and after the change.
- [ ] Run the smallest relevant cargo check.
- [ ] Add proof through HTTP projection or proof-state output.
- [ ] Update this file and [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) with evidence.

## What Not To Do

- [ ] Do not add another route-local JSONL reader for runtime state.
- [ ] Do not add another bridge between event vocabularies without an event id, source id, and retirement condition.
- [ ] Do not make `DashboardEvent` the durable source of truth.
- [ ] Do not make `ServerEvent` the durable source of truth.
- [ ] Do not treat a live TUI update as proof unless the durable projection also shows it.
- [ ] Do not archive observability docs until active runtime proof goes through HTTP/query projections.

## 2026-04-27 Deepening Pass - Event Store, Projection Cursors, And Proof Bundles

This pass upgrades the observability handoff from "clean up many events" to a concrete implementation design. The core diagnosis is that Roko is missing an event identity and cursor contract. Without that, runner, HTTP, TUI, provider, retry, merge, cognitive, and proof features can all appear to work locally while still failing to prove the same facts end to end.

The target is a Mori-like runtime evidence spine:

```text
producer effect
  -> RuntimeEventEnvelope
  -> RuntimeEventStore append with durable cursor
  -> ProjectionEngine materializers
  -> RuntimeQueryService and ProjectionStreamService
  -> HTTP, TUI, CLI, proof harness
```

Everything else is an adapter. `RunnerEvent`, `AgentRuntimeEvent`, `DashboardEvent`, `ServerEvent`, `FeedbackEvent`, `Episode`, and raw JSONL files are either producer inputs or projection outputs. None of them should be the durable observability source of truth.

Updated self-grade after this deepening pass: `9.91 / 10`.

Reason: this pass now gives a file-by-file, source-verified design that an implementation agent can execute without additional context. It defines the contracts, events, migration batches, proof requirements, and grep gates. It is not a `10` until the implementation exists and proof artifacts from real provider runs are attached.

### Additional Source Evidence

These source references were checked on 2026-04-27 and should be re-run before implementation:

```text
crates/roko-cli/src/runner/event_loop.rs:1139 appends runner events directly to paths.events_jsonl.
crates/roko-cli/src/runner/event_loop.rs:1265 fans runner events out to feedback after separate persistence/projection handling.
crates/roko-cli/src/runner/event_loop.rs:1285 translates RunnerEvent into FeedbackEvent.
crates/roko-cli/src/runner/event_loop.rs:2307 appends episodes JSONL directly.
crates/roko-cli/src/runner/event_loop.rs:2318 appends efficiency JSONL directly.
crates/roko-cli/src/runner/event_loop.rs:2503 appends bandit candidates directly.
crates/roko-cli/src/runner/projection.rs defines the per-run runner projection facade and raw runtime event inputs.
crates/roko-serve/src/projection_contract.rs:42 defines ProjectionEnvelope with cursor/staleness metadata.
crates/roko-serve/src/projection_contract.rs:467 defines RuntimeProjectionSet.
crates/roko-serve/src/projection_contract.rs:528 loads live StateHub snapshots.
crates/roko-serve/src/projection_contract.rs:539 uses StateHub total_published as cursor.
crates/roko-serve/src/projection_contract.rs:543 loads runtime feedback from workdir.
crates/roko-serve/src/projection_contract.rs:967 materializes execution_trace from mixed sources.
crates/roko-serve/src/projection_contract.rs:1536 reads .roko/events.jsonl for runtime feedback.
crates/roko-serve/src/projection_contract.rs:1605 builds projection stream delta frames from DashboardEvent.
crates/roko-serve/src/projection_contract.rs:1615 filters StateHub dashboard events for projection streams.
crates/roko-serve/src/routes/projections.rs:39 loads RuntimeProjectionSet for one-shot reads.
crates/roko-serve/src/routes/projections.rs:58 streams by subscribing to state.state_hub.subscribe_events().
crates/roko-serve/src/routes/status/gates.rs:75 reads .roko/events.jsonl directly.
crates/roko-serve/src/routes/status/gates.rs:81 names .roko/engrams.jsonl and .roko/events.jsonl as gate sources.
crates/roko-serve/src/routes/ws.rs:78 replays from the server EventBus ring.
crates/roko-serve/src/routes/ws.rs:89 streams from the server EventBus subscription.
crates/roko-serve/src/routes/ws.rs:109 catches up with EventBus replay_from.
crates/roko-serve/src/lib.rs:436 documents an EventBus to StateHub to EventBus feedback loop.
crates/roko-serve/src/lib.rs:750 starts the ServerEvent to DashboardEvent bridge.
crates/roko-serve/src/lib.rs:770 maps ServerEvent to DashboardEvent.
crates/roko-serve/src/lib.rs:918 starts the reverse StateHub to EventBus bridge.
crates/roko-serve/src/lib.rs:945 maps DashboardEvent to ServerEvent.
crates/roko-serve/src/lib.rs:1061 keeps a FIXME for bridge-loop duplicate events.
crates/roko-serve/src/events.rs:87 defines the parallel ServerEvent vocabulary.
crates/roko-core/src/state_hub.rs:134 publishes DashboardEvent into snapshot/broadcast/log machinery.
crates/roko-core/src/state_hub.rs:179 exposes StateHub dashboard event subscriptions.
crates/roko-core/src/state_hub.rs:190 replays DashboardEvent logs into a dashboard snapshot.
```

### Architectural Invariants

These invariants are the acceptance criteria for the redesign:

- [ ] Every runtime fact has exactly one canonical `RuntimeEventEnvelope`.
- [ ] Every runtime event has a durable monotonic cursor assigned by `RuntimeEventStore`.
- [ ] Every live stream frame includes a cursor that can be used to resume from the durable store.
- [ ] Every HTTP projection response includes `source_cursor`, `materialized_at`, `staleness_ms`, `truncated`, and `evidence_event_ids`.
- [ ] Every proof script queries `RuntimeQueryService`; it does not scrape route-local logs or TUI text.
- [ ] Every bridge from a legacy event type to runtime events is one-way and has a retirement checklist.
- [ ] Every projection materializer is deterministic from runtime events plus declared external repositories.
- [ ] Every direct read of `.roko/events.jsonl`, `.roko/engrams.jsonl`, `.roko/episodes.jsonl`, `.roko/learn/efficiency.jsonl`, and `.roko/learn/*.json` is behind a repository or query service.
- [ ] Every dropped, lagged, redacted, truncated, unsupported, missing-credential, auth-failed, rate-limited, and conflict state is itself observable.
- [ ] Every UI/API count can be traced to one or more event ids, not just a current in-memory snapshot.

### Required Runtime Event Contract

Create this contract in a shared crate that both `roko-cli` and `roko-serve` can depend on without circular coupling. Prefer `roko-runtime` if it remains a small runtime-contract crate; otherwise create `roko-observe` and have both CLI and server depend on it.

```rust
pub struct RuntimeEventEnvelope {
    pub schema_version: u16,
    pub event_id: EventId,
    pub cursor: Option<RuntimeCursor>,
    pub run_id: Option<String>,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub operation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<EventId>,
    pub source: RuntimeEventSource,
    pub category: RuntimeEventCategory,
    pub event_type: String,
    pub timestamp_ms: i64,
    pub redaction: RedactionSummary,
    pub payload: serde_json::Value,
}
```

```rust
pub enum RuntimeEventCategory {
    Operation,
    Workflow,
    Task,
    Process,
    Service,
    Provider,
    ModelCall,
    Prompt,
    Agent,
    Gate,
    Retry,
    Merge,
    Artifact,
    Workspace,
    Cognitive,
    Knowledge,
    Dream,
    Safety,
    Config,
    Http,
    Tui,
    Proof,
}
```

Implementation checklist:

- [ ] Add `EventId` as a stable ULID/UUID wrapper that serializes to a lowercase string.
- [ ] Add `RuntimeCursor` as a monotonic durable integer assigned only by the store.
- [ ] Add `RuntimeEventSource` with `component`, `crate_name`, `entrypoint`, `host_pid`, and optional `provider`.
- [ ] Add `RedactionSummary` with `status`, `policy_version`, `redacted_fields`, and `canary_detected`.
- [ ] Require every event type to be lowercase dotted names, for example `provider.started`, `prompt.assembled`, `gate.completed`, `merge.conflict_detected`.
- [ ] Add schema tests that serialize and deserialize every event category and representative event type.
- [ ] Add a compatibility adapter from current `RunnerEvent` into `RuntimeEventEnvelope`.
- [ ] Add a compatibility adapter from current `AgentRuntimeEvent` into `RuntimeEventEnvelope`.
- [ ] Add a compatibility adapter from current `DashboardEvent` into `RuntimeEventEnvelope` only for migration/import, not for live production.
- [ ] Add a compatibility adapter from current `ServerEvent` into `RuntimeEventEnvelope` only at route boundaries during migration.
- [ ] Add a compatibility adapter from current learning `FeedbackEvent`/episode records into `RuntimeEventEnvelope` only at repository boundaries.
- [ ] Add compile-time docs that state no new event vocabulary may become authoritative.

### Required Runtime Event Store

The event store is the only place that assigns durable cursors. JSONL is acceptable as the first backend if the abstraction is clean, but JSONL must store `RuntimeEventEnvelope`, not raw `DashboardEvent` or ad-hoc runner payloads.

```rust
#[async_trait]
pub trait RuntimeEventStore: Send + Sync {
    async fn append(&self, event: RuntimeEventEnvelope) -> Result<StoredRuntimeEvent>;
    async fn append_batch(&self, events: Vec<RuntimeEventEnvelope>) -> Result<Vec<StoredRuntimeEvent>>;
    async fn replay(&self, query: EventReplayQuery) -> Result<RuntimeEventReplay>;
    async fn latest_cursor(&self) -> Result<Option<RuntimeCursor>>;
    async fn get(&self, event_id: &EventId) -> Result<Option<StoredRuntimeEvent>>;
}
```

```rust
pub struct EventReplayQuery {
    pub after_cursor: Option<RuntimeCursor>,
    pub limit: usize,
    pub run_id: Option<String>,
    pub operation_id: Option<String>,
    pub categories: Vec<RuntimeEventCategory>,
    pub event_types: Vec<String>,
}
```

Implementation checklist:

- [ ] Add `JsonlRuntimeEventStore` backed by `.roko/runtime/events.jsonl` or `.roko/events.jsonl` after migration.
- [ ] Add a lock/append strategy that avoids corrupt JSONL when CLI and server append concurrently.
- [ ] Add idempotent append behavior keyed by `event_id`.
- [ ] Add replay by cursor and limit.
- [ ] Add import of legacy `.roko/events.jsonl` dashboard records into runtime envelopes with category `Proof` or `Workflow` and `source.component = "legacy_import"`.
- [ ] Add import of legacy runner event records if they differ from dashboard event records.
- [ ] Add redaction before append, not during query.
- [ ] Add corruption recovery that returns partial replay plus a diagnostic event.
- [ ] Add a repository-level test for append, replay, idempotency, corrupted trailing line recovery, and cursor monotonicity.
- [ ] Add a proof fixture that writes events, restarts the store, replays from cursor, and verifies no duplicate completion.

### Required Projection Engine

The projection engine owns materializers. `StateHub`, `RuntimeProjectionSet`, server streams, and TUI views become consumers of these materializers.

```rust
#[async_trait]
pub trait ProjectionMaterializer: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema_version(&self) -> u16;
    fn accepts(&self, event: &StoredRuntimeEvent) -> bool;
    async fn apply(&mut self, event: &StoredRuntimeEvent) -> Result<Vec<ProjectionDelta>>;
    async fn snapshot(&self, query: ProjectionQuery) -> Result<ProjectionSnapshot>;
}
```

Required projections:

- [ ] `dashboard_state`: materializes the current TUI/dashboard snapshot.
- [ ] `execution_trace`: materializes run, plan, task, agent, gate, retry, merge, and artifact timelines.
- [ ] `provider_state`: materializes provider/model availability, lifecycle, auth state, usage, latency, and errors.
- [ ] `prompt_diagnostics`: materializes prompt sections, dropped sections, token estimates, redaction, cache hints, and assembler policy.
- [ ] `gate_pipeline`: materializes per-task gate rungs, skipped gates, failed gates, retries, and terminal outcomes.
- [ ] `retry_state`: materializes retry decisions, backoff, cause, attempts, and replan triggers.
- [ ] `merge_state`: materializes queue, backend command, success, conflict files, conflict hunks, and abort evidence.
- [ ] `workspace_state`: materializes worktree, branch, artifact, file-change, and checkout state.
- [ ] `process_lifecycle`: materializes PID/process group, command, exit, timeout, termination, and orphan evidence.
- [ ] `cognitive_loop`: materializes feedback ingestion, knowledge writes, dream jobs, policy updates, and prompt influence.
- [ ] `config_effective`: materializes config sources, provider credentials status, model resolution, and policy versions.
- [ ] `proof_state`: materializes proof scenarios and their required evidence ids.

Implementation checklist:

- [ ] Move the useful `ProjectionEvent` ideas from `crates/roko-cli/src/runner/projection.rs` into the shared projection layer or adapt them below the event store.
- [ ] Keep the runner projection facade as a thin compatibility subscriber during migration.
- [ ] Replace `RuntimeProjectionSet::load(&state)` internals with `RuntimeQueryService` calls backed by materialized projections.
- [ ] Replace StateHub snapshot rebuilding from `DashboardEvent` logs with `dashboard_state` materialization from runtime events.
- [ ] Make projection windows and retention policy catalog-driven.
- [ ] Add deterministic replay tests: same events in the same order produce byte-identical projection snapshots.
- [ ] Add materializer lag metrics and expose them through `observability_health`.

### Required Query And Stream Services

HTTP routes, TUI, CLI proof scripts, and remote dashboards should use the same query service. A route handler may transform the response for legacy compatibility, but it must not compute runtime truth directly.

```rust
#[async_trait]
pub trait RuntimeQueryService: Send + Sync {
    async fn get_projection(&self, name: &str, query: ProjectionQuery) -> Result<ProjectionEnvelope<serde_json::Value>>;
    async fn list_projections(&self) -> Result<Vec<ProjectionCatalogEntry>>;
    async fn get_run_trace(&self, run_id: &str) -> Result<ProjectionEnvelope<serde_json::Value>>;
    async fn get_proof_bundle(&self, query: ProofBundleQuery) -> Result<ProofBundle>;
    async fn explain_evidence(&self, ids: &[EventId]) -> Result<EvidenceExplanation>;
}
```

```rust
#[async_trait]
pub trait ProjectionStreamService: Send + Sync {
    async fn subscribe(&self, request: ProjectionStreamRequest) -> Result<ProjectionStream>;
    async fn resume(&self, cursor: RuntimeCursor, request: ProjectionStreamRequest) -> Result<ProjectionStream>;
}
```

Implementation checklist:

- [ ] Make `/api/projections/{name}` call `RuntimeQueryService::get_projection`.
- [ ] Make `/api/projections/{name}/stream` call `ProjectionStreamService`, not `state.state_hub.subscribe_events()`.
- [ ] Make `/ws` replay and stream projection frames from durable runtime cursors, not server `EventBus` ring cursors.
- [ ] Make `/api/status/gates`, `/api/status/episodes`, `/api/status/metrics`, `/api/providers`, `/api/agents`, and `/api/run` status views call query service methods.
- [ ] Keep legacy response shapes by adding projection adapters, not new file readers.
- [ ] Add `resync_required` frames when a client resumes from a cursor older than retained events.
- [ ] Add `lagged` frames with the last applied cursor when a subscriber falls behind.
- [ ] Add endpoint tests that assert every response includes source cursor/evidence metadata.
- [ ] Add a route grep gate that fails direct `.roko` runtime file reads outside repository modules.

### Required Proof Bundle

The proof bundle is the missing user-visible answer to "prove this fully works." It should be queryable through HTTP and generatable from CLI. It must not be a pile of screenshots, TUI logs, or bespoke shell checks.

```rust
pub struct ProofBundle {
    pub schema_version: u16,
    pub proof_id: String,
    pub generated_at_ms: i64,
    pub query: ProofBundleQuery,
    pub status: ProofStatus,
    pub run_ids: Vec<String>,
    pub source_cursor: RuntimeCursor,
    pub projections: Vec<ProjectionProof>,
    pub required_evidence: Vec<RequiredEvidenceResult>,
    pub artifacts: Vec<ProofArtifactRef>,
    pub redaction: RedactionSummary,
}
```

Required evidence rows:

- [ ] `real_provider_run`: provider started, prompt assembled, model call started, model call completed, agent output received, gate completed, task completed.
- [ ] `provider_matrix`: Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI statuses are one of `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, or `unsupported`.
- [ ] `prompt_diagnostics`: assembled prompt includes section list, token estimate, dropped sections, knowledge/playbook inserts, and redaction summary.
- [ ] `retry_after_gate_failure`: gate failed, retry decision emitted, retry started, second gate result emitted, final task state emitted.
- [ ] `http_query_projection`: `/api/projections/execution_trace`, `/api/projections/provider_state`, and `/api/projections/proof_state` agree on run id and cursor.
- [ ] `resume_after_crash`: snapshot persisted, process killed or interrupted, runner resumed, no duplicate completion event.
- [ ] `merge_success`: merge backend started, command recorded, merge succeeded, branch/head evidence captured.
- [ ] `merge_conflict_failure`: merge backend started, conflict detected, conflict file list captured, failure is terminal or awaits operator decision.
- [ ] `tui_consistency`: TUI/dashboard projection cursor equals or trails HTTP cursor by an explained lag.
- [ ] `redaction_canary`: secret canary absent from events, projections, streams, and proof bundle payload.

Implementation checklist:

- [ ] Add `ProofBundleService` backed only by `RuntimeQueryService`.
- [ ] Add `GET /api/proof/bundles/{proof_id}` and `POST /api/proof/bundles`.
- [ ] Add `roko proof run --scenario <name>` that executes or locates a scenario and emits a proof bundle path plus HTTP URL.
- [ ] Add `roko proof provider-matrix` that runs all configured providers through the same dispatch path and emits one proof bundle.
- [ ] Add durable proof artifacts under `.roko/proof/<proof_id>/`.
- [ ] Add JSON schemas for proof bundle responses.
- [ ] Add a redaction canary test that fails if the canary appears in stored events, projections, streams, or proof artifacts.

### Drift Finding O1 - Runner Fanout Is Still Side-Effect First

Problem:

The runner currently persists, projects, and feeds learning through separate paths. `event_loop.rs` appends runner events, episodes, efficiency events, and bandit candidates directly. It also translates `RunnerEvent` to `FeedbackEvent` after separate persistence/projection handling. That means the feedback store, projection stream, and event log can disagree.

Implementation checklist:

- [ ] Add `RuntimeEventWriter` to the runner context.
- [ ] Replace direct runner `append_jsonl(&paths.events_jsonl, ...)` with `RuntimeEventStore::append`.
- [ ] Emit runner lifecycle events only as `RuntimeEventEnvelope`.
- [ ] Move `runner_event_to_feedback` behind a `FeedbackMaterializer` or `FeedbackSink` subscribed to stored runtime events.
- [ ] Replace direct episode append with an episode projection/repository fed from `task.completed`, `gate.completed`, `prompt.assembled`, and `model_call.completed` events.
- [ ] Replace direct efficiency append with a cost/efficiency materializer fed from usage/cost runtime events.
- [ ] Replace direct bandit append with `policy.candidate_recorded` runtime events or a policy repository called by a projection sink.
- [ ] Add a test where feedback append fails and the runtime event is still durable with an observable `feedback.write_failed` event.
- [ ] Add a test where projection subscribers are absent and the durable event is still queryable.

Done criteria:

- [ ] `rg -n "append_jsonl\\(&paths\\.events_jsonl|append_jsonl\\(&paths\\.episodes_jsonl|append_jsonl\\(&paths\\.efficiency_jsonl|bandit_log" crates/roko-cli/src/runner -g '*.rs'` has zero output or only allowlisted migration code.
- [ ] `roko plan run` creates a runtime event log whose latest cursor is returned by `/api/projections/execution_trace`.
- [ ] Feedback, episodes, efficiency, and proof projections all cite the same event ids for a completed task.

### Drift Finding O2 - Runner Projection Is Useful But Not Durable

Problem:

`crates/roko-cli/src/runner/projection.rs` has useful concepts: provider-neutral categories, dropped/coerced counters, preview truncation, and event mapping. But it is per-run and broadcast oriented. A broadcast facade cannot be the proof source because no-subscriber and lagged conditions are treated as live delivery concerns rather than durable evidence.

Implementation checklist:

- [ ] Keep runner projection as `RunnerProjectionAdapter` during migration.
- [ ] Move event category definitions into the shared runtime event/projection crate.
- [ ] Make runner projection publish from `StoredRuntimeEvent`, not raw `RunnerEvent`.
- [ ] Make no-subscriber states observable as stream delivery state, not event loss.
- [ ] Make dropped/coerced counters projection metadata on `observability_health`.
- [ ] Remove `RawRuntimeEvent::Custom` as an untyped bypass or wrap it in `RuntimeEventEnvelope` with schema validation.
- [ ] Add a replay test that rebuilds runner progress UI from event store after process restart.

Done criteria:

- [ ] Runner inline output can be rendered from a projection snapshot.
- [ ] A run with no live TUI subscriber still has complete provider, prompt, gate, retry, and merge evidence via query service.
- [ ] A lagged TUI subscriber receives `resync_required` and catches up from store cursor.

### Drift Finding O3 - Server Projections Mix Live State, Recovery State, And Raw Files

Problem:

`RuntimeProjectionSet::load` currently joins live `StateHub`, recovered dashboard snapshots, provider health, runtime feedback loaded from workdir, and ad-hoc projection logic. This is valuable as a transitional facade, but it is not a clean source-of-truth boundary.

Implementation checklist:

- [ ] Introduce `ProjectionEngine` as the owner of materialized projection state.
- [ ] Convert `RuntimeProjectionSet` into a response composer over `RuntimeQueryService`.
- [ ] Remove direct `state.state_hub.current_snapshot()` from projection loading after `dashboard_state` exists.
- [ ] Remove direct `RuntimeFeedbackProjection::load(&state.workdir)` from projection loading after feedback projection exists.
- [ ] Replace `cursor: state.state_hub.total_published()` with event-store/projection cursor.
- [ ] Make each projection response list its materializers and input event categories.
- [ ] Add stale/recovered indicators per projection, not just per whole response.

Done criteria:

- [ ] Restarting `roko serve` and querying `/api/projections/execution_trace` returns the same run evidence from durable events.
- [ ] `state_hub` appears only as a dashboard delivery adapter in projection code.
- [ ] Projection response cursors correspond to event-store cursors, not StateHub sequence numbers.

### Drift Finding O4 - HTTP Streams And WebSockets Are Live Rings, Not Durable Projection Streams

Problem:

`/api/projections/{name}/stream` subscribes to `StateHub` events and `/ws` replays from a server `EventBus` ring. Those are useful live channels, but they cannot prove resume/crash behavior because the cursor belongs to a live ring rather than a durable event store.

Implementation checklist:

- [ ] Replace projection stream source with `ProjectionStreamService`.
- [ ] Replace `/ws` catchup with event-store cursor replay filtered through projection subscriptions.
- [ ] Add stream frames: `snapshot`, `delta`, `heartbeat`, `lagged`, `resync_required`, `error`.
- [ ] Add `last_event_id` support for SSE and cursor support for WebSocket.
- [ ] Add a retention policy that explicitly reports when a cursor is too old.
- [ ] Add a stream proof that disconnects, emits events, reconnects from cursor, and receives exactly the missed deltas.
- [ ] Add a WebSocket proof that a live REST-originated action emits one event and one projection delta.

Done criteria:

- [ ] `rg -n "state_hub\\.subscribe_events\\(|event_bus\\.replay_from\\(|event_bus\\.subscribe\\(" crates/roko-serve/src/routes -g '*.rs'` has zero output or only legacy-compatibility wrappers.
- [ ] Projection stream frames include durable event cursors.
- [ ] Reconnect proof passes without relying on in-memory StateHub/EventBus history.

### Drift Finding O5 - Bidirectional Bridges Create Duplicates And Information Loss

Problem:

The server has both `ServerEvent -> DashboardEvent` and `DashboardEvent -> ServerEvent` bridges. The code explicitly documents the feedback-loop risk and duplicate REST-originated events. The reverse bridge also cannot preserve variants that have no `ServerEvent` equivalent.

Implementation checklist:

- [ ] Add runtime event ids before touching bridges.
- [ ] Add source ids to bridge-produced events while migration is in progress.
- [ ] Add a duplicate detector that records `observability.duplicate_suppressed` events.
- [ ] Convert REST routes to append runtime events instead of publishing `ServerEvent` directly.
- [ ] Convert dashboard consumers to query/subscribe to projections instead of receiving `DashboardEvent` as truth.
- [ ] Remove `dashboard_event_to_server` after `/ws` streams projection frames.
- [ ] Remove `server_event_to_dashboard` after StateHub consumes `dashboard_state` projection deltas.
- [ ] Keep `ServerEvent` only as a legacy API compatibility enum if needed, not a runtime bus contract.

Done criteria:

- [ ] `rg -n "server_event_to_dashboard|dashboard_event_to_server|FIXME: bridge loop" crates/roko-serve/src -g '*.rs'` has zero output.
- [ ] A REST operation produces exactly one stored runtime event and one projection delta.
- [ ] Dropped/unmapped dashboard variants are no longer possible because projection materializers own view translation.

### Drift Finding O6 - Route-Local Runtime Readers Still Bypass Projections

Problem:

Some routes already use `RuntimeProjectionSet`, but helper code still reads `.roko/events.jsonl` and `.roko/engrams.jsonl`. This creates two incompatible truth paths: "projection route" and "helper route."

Implementation checklist:

- [ ] Create repository modules for any unavoidable legacy file import.
- [ ] Move all `.roko` file reads out of route handlers.
- [ ] Make gate summary/history/signals call `RuntimeQueryService`.
- [ ] Make episode/status routes call `RuntimeQueryService`.
- [ ] Make metrics routes call `RuntimeQueryService`.
- [ ] Make provider health/test results append/query provider runtime events.
- [ ] Make agent routes query `execution_trace`, `provider_state`, and `process_lifecycle` projections.
- [ ] Add endpoint-level tests that fail if the same run has mismatched counts between status/projection endpoints.

Done criteria:

- [ ] `rg -n "read_to_string|read_jsonl_entries|engrams\\.jsonl|events\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl" crates/roko-serve/src/routes crates/roko-cli/src/tui -g '*.rs'` has zero output or only route-independent repository calls.
- [ ] `/api/status/gates`, `/api/projections/gate_pipeline`, and proof bundle gate evidence agree on event ids.
- [ ] `/api/status/metrics`, `/api/projections/provider_state`, and proof bundle provider evidence agree on usage/cost ids.

### Drift Finding O7 - Truth Map And OpenAPI Are Not Enforcement

Problem:

The repo has `truth_map`, projection catalog ideas, and OpenAPI definitions, but they are not generated from one runtime query contract. That means docs can say one source is authoritative while code reads another source.

Implementation checklist:

- [ ] Convert truth map entries into machine-readable `ProjectionSourceSpec` values.
- [ ] Generate projection catalog entries from materializer metadata plus `ProjectionSourceSpec`.
- [ ] Generate OpenAPI projection schemas from the same catalog where possible.
- [ ] Add CI/proof gate that every projection has a query endpoint, stream policy, schema version, source cursor, and proof sample.
- [ ] Add `/api/observability/truth-map` that returns current authoritative sources and allowed readers.
- [ ] Add `/api/observability/health` that reports materializer lag, stream lag, dropped live deliveries, event-store cursor, redaction failures, and repository import failures.

Done criteria:

- [ ] Adding a new projection requires one catalog/materializer entry and automatically updates docs/OpenAPI tests.
- [ ] A route cannot claim projection ownership without a `ProjectionSourceSpec`.
- [ ] Runtime proof output includes truth-map version and projection catalog version.

## End-To-End Implementation Plan

### Batch O1 - Shared Runtime Event Contract

- [ ] Pick the crate location: `roko-runtime` if dependency direction stays clean, otherwise new `roko-observe`.
- [ ] Add `RuntimeEventEnvelope`, `StoredRuntimeEvent`, `RuntimeCursor`, `EventId`, `RuntimeEventCategory`, `RuntimeEventSource`, and `RedactionSummary`.
- [ ] Add JSON schemas or snapshot tests for canonical event examples.
- [ ] Add conversion traits: `TryFrom<RunnerEvent>`, `TryFrom<AgentRuntimeEvent>`, `TryFrom<DashboardEvent>`, `TryFrom<ServerEvent>`, and learning feedback import.
- [ ] Add schema-version migration hook even if version `1` is the only version.
- [ ] Add docs in the crate explaining that these types are authoritative runtime evidence.

Proof for batch:

- [ ] Unit test serializes and deserializes one event per category.
- [ ] Unit test rejects unredacted secret canary payloads.
- [ ] Unit test confirms event ids are stable and cursors are store-assigned.

### Batch O2 - Durable Event Store

- [ ] Implement `RuntimeEventStore` trait.
- [ ] Implement JSONL store with atomic append and replay by cursor.
- [ ] Add legacy import readers for dashboard/runner event logs behind explicit import APIs.
- [ ] Add corruption recovery and diagnostic events.
- [ ] Add idempotency by event id.
- [ ] Wire CLI runner context to receive a `RuntimeEventStore`.
- [ ] Wire server app state to receive a `RuntimeEventStore`.

Proof for batch:

- [ ] Append/replay test survives process restart.
- [ ] Concurrent append test does not corrupt JSONL.
- [ ] Duplicate append test returns the original stored event or a documented idempotency result.
- [ ] Corrupted trailing line test reports partial recovery and diagnostic evidence.

### Batch O3 - Runner Event Migration

- [ ] Replace runner direct event appends with `RuntimeEventStore`.
- [ ] Emit provider lifecycle, prompt diagnostics, model call, agent output, task, gate, retry, merge, and artifact events.
- [ ] Move feedback fanout behind an event-store subscriber or projection sink.
- [ ] Move episode and efficiency writes behind projection/repository sinks.
- [ ] Ensure gate skip is represented as `gate.skipped`, not pass.
- [ ] Ensure retry decisions include cause, attempt, max attempts, backoff, and source gate event id.
- [ ] Ensure merge decisions include backend name, command, cwd, exit status, stdout/stderr artifact ids, and conflict files.

Proof for batch:

- [ ] Run one plan through a real provider and query the event store for every required event category.
- [ ] Kill and resume the runner and verify no duplicate terminal task event.
- [ ] Force a gate failure and verify retry events cite the failed gate event id.
- [ ] Force merge success and conflict failure and verify conflict evidence is durable.

### Batch O4 - Projection Engine And Query Service

- [ ] Implement materializer trait and projection registry.
- [ ] Port existing `RuntimeProjectionSet` projection names into materializers.
- [ ] Implement `RuntimeQueryService`.
- [ ] Replace `/api/projections/{name}` internals with query service calls.
- [ ] Replace status/provider/agent/metrics route internals with query service calls or legacy adapters over query service.
- [ ] Replace TUI dashboard direct reads with query snapshots and projection stream deltas.
- [ ] Add projection catalog metadata for limits, retention, streamability, and proof relevance.

Proof for batch:

- [ ] Query `execution_trace`, `provider_state`, `gate_pipeline`, `retry_state`, `merge_state`, `prompt_diagnostics`, and `proof_state` for the same run id and verify cursors/evidence ids align.
- [ ] Restart server and confirm projections rebuild from event store without live runner state.
- [ ] Compare TUI snapshot cursor with HTTP projection cursor and report expected lag if any.

### Batch O5 - Durable Streams And Bridge Retirement

- [ ] Implement `ProjectionStreamService`.
- [ ] Migrate `/api/projections/{name}/stream`.
- [ ] Migrate `/ws`.
- [ ] Add stream frame schema.
- [ ] Add resume and retention behavior.
- [ ] Remove reverse dashboard/server bridge.
- [ ] Remove forward server/dashboard bridge after route producers append runtime events.
- [ ] Keep live in-memory buses only as delivery optimizations below the durable cursor contract.

Proof for batch:

- [ ] Disconnect/reconnect SSE from cursor and receive exactly missed deltas.
- [ ] Disconnect/reconnect WebSocket from cursor and receive exactly missed deltas.
- [ ] Trigger one REST operation and prove one stored event, one projection delta, and no duplicate bridge event.
- [ ] Simulate subscriber lag and prove `resync_required` behavior.

### Batch O6 - Proof Bundle Service

- [ ] Implement `ProofBundleService`.
- [ ] Add HTTP endpoints for proof bundles.
- [ ] Add CLI commands for provider matrix and runtime end-to-end proof.
- [ ] Update tracked proof scripts to call proof commands and HTTP query endpoints.
- [ ] Add provider status taxonomy: `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`.
- [ ] Store proof artifacts under `.roko/proof/<proof_id>/`.
- [ ] Add proof bundle schemas and example outputs.

Proof for batch:

- [ ] Provider matrix proof covers Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Runtime proof covers provider run, retry, HTTP query/projection, resume, merge success, merge conflict, prompt diagnostics, and redaction canary.
- [ ] Proof bundle can be regenerated from durable event store after server restart.

### Batch O7 - Cleanup And Enforcement

- [ ] Turn grep gates below into tracked scripts or CI checks.
- [ ] Archive this doc only after all done criteria are checked and proof bundle paths are linked.
- [ ] Remove dead helper functions that reconstruct runtime truth from raw files.
- [ ] Remove legacy event bridge code.
- [ ] Remove or rename `RuntimeProjectionSet` if it remains only a compatibility facade.
- [ ] Update `29-CURRENT-RUNTIME-GAP-LEDGER.md` with closed items and proof bundle ids.
- [ ] Update OpenAPI and truth-map docs from the projection catalog.

Proof for batch:

- [ ] All grep gates pass or have a documented allowlist with owner and retirement date.
- [ ] Clean clone can run proof scripts without ignored dependencies.
- [ ] A fresh agent can implement the next checklist item from this file alone.

## Expanded Grep Gates

These commands are the acceptance checks for removing ad-hoc observability.

```bash
rg -n "append_jsonl\\(|events_jsonl|episodes_jsonl|efficiency_jsonl|bandit_log" \
  crates/roko-cli/src/runner crates/roko-serve/src -g '*.rs'
```

Expected end state:

- [ ] Zero direct runtime event/episode/efficiency appends outside event-store repositories or projection sinks.

```bash
rg -n "RawRuntimeEvent|ProjectionEvent|Projection::new|broadcast::channel|NoSubscribers" \
  crates/roko-cli/src crates/roko-serve/src -g '*.rs'
```

Expected end state:

- [ ] Runner projection remains only as a compatibility adapter or is replaced by projection engine materializers.

```bash
rg -n "RuntimeProjectionSet::load|state_hub\\.current_snapshot|state_hub\\.subscribe_events|event_bus\\.replay_from|event_bus\\.subscribe" \
  crates/roko-serve/src -g '*.rs'
```

Expected end state:

- [ ] Routes do not use StateHub/EventBus as projection truth.

```bash
rg -n "server_event_to_dashboard|dashboard_event_to_server|ServerEvent|DashboardEvent|bridge loop" \
  crates/roko-serve/src crates/roko-cli/src -g '*.rs'
```

Expected end state:

- [ ] `ServerEvent`/`DashboardEvent` are view compatibility types only, not live runtime truth.

```bash
rg -n "read_to_string|read_jsonl_entries|engrams\\.jsonl|events\\.jsonl|episodes\\.jsonl|efficiency\\.jsonl" \
  crates/roko-serve/src/routes crates/roko-cli/src/tui -g '*.rs'
```

Expected end state:

- [ ] Route and TUI layers do not read runtime files directly.

```bash
rg -n "RuntimeEventEnvelope|RuntimeEventStore|ProjectionEngine|RuntimeQueryService|ProofBundleService|ProjectionStreamService" \
  crates -g '*.rs'
```

Expected end state:

- [ ] All core observability contracts exist and are used by CLI, server, TUI, and proof commands.

## Definition Of Complete

Observability can be considered complete only when this checklist is fully checked:

- [ ] Runtime event contract exists in shared code and every runtime producer uses it.
- [ ] Runtime event store is durable, replayable, cursor-based, idempotent, redacted, and recoverable.
- [ ] Projection engine materializes every user-facing runtime view from stored events.
- [ ] Runtime query service backs projection, status, provider, agent, metrics, TUI, and proof surfaces.
- [ ] Projection stream service backs SSE and WebSocket with durable cursor resume.
- [ ] Legacy bridge loops are removed or downgraded to explicitly allowlisted compatibility adapters.
- [ ] Direct route/TUI reads of runtime JSONL files are removed.
- [ ] Proof bundle service can prove real provider run, provider matrix, retry, HTTP query, resume, merge success, merge conflict, prompt diagnostics, TUI consistency, and redaction canary.
- [ ] Grep gates pass from a clean clone.
- [ ] `29-CURRENT-RUNTIME-GAP-LEDGER.md` links the proof bundle ids for every closed observability issue.

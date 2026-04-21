# Observability Gaps — Dashboard Surfaces & Metrics Aggregation

> **New file** added 2026-04-16 during post-PR-13 audit. Items here cluster
> around "data is being captured but never read back" — the inverse of the
> partially-wired-subsystems file. The data exists; the consumer is missing.
>
> **Re-audit 2026-04-20**: 5 items closed (83, 84, 85, 86, 88). 1 item still open (87).

## Summary

Six observability gaps where roko already records the data the user wants but
no widget, endpoint, or aggregation surfaces it. Each is high-leverage: a
day or two of consumer-side wiring unlocks visibility into a subsystem that's
already paying its instrumentation cost.

## Items

### 83. [DONE] Verdicts logged to substrate but never read back for trend widgets

**Resolved in**: `crates/roko-cli/src/tui/verdicts.rs` implements a full `VerdictsAggregator`
that incrementally reads `Kind::GateVerdict` engrams from the `FileSubstrate` using a
`SubstrateCursor` (line ~17). Per-gate `GateStats` (line ~64) track rolling 24x1h pass/fail
buckets via `TrendBuckets`. The aggregator is consumed by `app.rs` (line ~80:
`verdict_aggregator`) which populates `tui_state.gate_trends` (line ~2327) for the Gate tab.
The dashboard view renders a gate trend grid via `gate_trend_rows()` (dashboard_view.rs
line ~1509) and `render_gate_trend_grid()` (line ~1564). Multiple consumers now read
`Kind::GateVerdict`: the TUI verdict reader, `roko-serve/routes/status.rs` (line ~1828),
`roko-conductor` watchers (stuck_pattern, test_failure_budget, ghost_turn), and the
`roko-learn/verdict_scorer.rs`. Cross-ref item 35c.

**Status**: DONE.

---

### 84. [DONE] Conductor diagnoses computed but no TUI panel or HTTP endpoint

**Resolved in**: Both consumers now exist:
- **TUI**: `dashboard_view.rs` renders a Diagnosis panel (line ~1020) with severity-colored
  rows via `diagnosis_rows()` (line ~2075). State tracks `diagnoses: Vec<DiagnosisSummary>`
  (tui/state.rs line ~784).
- **HTTP**: `crates/roko-serve/src/routes/diagnosis.rs` exposes `GET /api/diagnosis/recent`
  (line ~17). OpenAPI tags include "diagnosis" (openapi.rs line ~51).
Cross-ref items 10 and 31 now also DONE.

**Status**: DONE.

---

### 85. [DONE] Efficiency trends not aggregated for dashboard charts

**Resolved in**: `roko-learn/src/aggregate.rs` now provides `efficiency_trend()` (line ~201)
and `efficiency_trend_with_cursor()` (line ~236) returning `Vec<EfficiencyBucket>` with
hourly/daily bucketing. `EfficiencyBucket` (line ~137) carries per-bucket aggregates
(tokens, cost, latency, task count). The dashboard loads this at
`dashboard.rs:load_efficiency_trend()` (line ~2576) with 24 hourly buckets. The Learning tab
renders trend sparklines via `render_learning_trend_lines()` (line ~4400). The HTTP surface
at `roko-serve/routes/learning.rs` line ~43 also aggregates efficiency data.
`DashboardSnapshot::efficiency_trend` (dashboard_snapshot.rs line ~588) carries the data
through the state hub. Tests at aggregate.rs lines ~657-738 verify bucketing.

**Status**: DONE.

---

### 86. [DONE] Metrics schema divergence between `roko-core::obs::metrics` and `roko-agent-server::state`

**Resolved in**: `crates/roko-core/src/obs/schema.rs` defines a canonical `MetricSchema` trait
(line ~63) and `CanonicalMetricSchema` implementation (line ~72) with `SCHEMA_VERSION`.
`crates/roko-agent-server/src/state.rs` imports and uses the same canonical schema (line ~24).
`crates/roko-core/tests/metric_schema.rs` has an explicit test
`agent_server_metrics_use_canonical_schema_constants` (line ~78) that prevents drift.
Cross-ref item 35 now also DONE.

**Status**: DONE.

---

### 87. No per-gate pass/fail rate timeline

**Evidence**: `.roko/learn/gate-thresholds.json` stores the EMA per rung.
That EMA hides per-gate (compile / test / clippy / …) breakdowns. Dashboard
Gate tab shows a single rolling number, not a timeline per gate.

**Current state**: Operators see "rung 1 EMA = 0.92"; cannot tell if the
test gate or the clippy gate is dragging the number down.

**Gap**: Compute per-gate rolling pass-rate from the verdicts substrate (see
item 83) and render as a multi-line sparkline on the Gate tab.

**Fix scope**: 2 days. Depends on item 83 reader.

**Priority**: P1.

---

### 88. [DONE] Prompt experiment winners not rendered on Learning tab

**Resolved in**: The Learning tab now renders concluded experiment winners:
`dashboard.rs` computes `experiment_winners` from `experiment_store.winner_summaries()`
(line ~459). `dashboard_view.rs` has `render_concluded_experiments_panel()` (line ~1208)
showing ID, winner variant, sample size, and confidence bars. State carries
`experiment_winners: Vec<ExperimentWinnerSummary>` (state.rs line ~786). Test at
dashboard_view.rs line ~2270 verifies rendering. Cross-ref item 20 now also DONE.

**Status**: DONE.

# 14 — Builtin Cell Catalog

> Every Cell that ships with Roko, organized by protocol conformance. All Cells participate in predict-publish-correct ([doc-02](02-CELL.md)): each publishes predictions as Pulses, subscribes to calibration error topics, and updates. 46 protocol Cells, 28+ domain Cells, 70+ total.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality, demurrage, Kind system), [02-CELL](02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign, EFE routing), [15-TELEMETRY](15-TELEMETRY.md) (Lens system, StateHub, c-factor), [11-CONNECTIVITY](11-CONNECTIVITY.md) (Connect protocol, exoskeleton)

---

## 1. Overview

Roko ships with **45+ built-in Cells** covering all nine protocols. Each Cell declares typed I/O, capabilities, cost estimates, and protocol conformance. Cells compose into Graphs ([doc-03](03-GRAPH.md)) — the catalog is deliberately large because more composable pieces yield more emergent value (ERC-20 precedent: combinatorial explosion of compositions).

Naming convention: kebab-case noun-or-verb-phrase. Cells describe operations; Graphs describe outcomes.

**Every Cell is a learner.** Through predict-publish-correct ([doc-02](02-CELL.md)), each Cell publishes its prediction as a Pulse on `prediction.{name}`, receives calibration updates on `calibration.{name}.updated`, and adjusts. This is structural — not a separate subsystem bolted on. The calibration pattern for each protocol is documented in [doc-07](07-LEARNING.md).

### Catalog summary

| Protocol | Built-in Cells | Count | Primary domain |
|---|---|---|---|
| Store | file-store, memory-store, chain-store | 3 | Signal persistence |
| Score | llm-scorer, rule-scorer, hdc-scorer | 3 | Signal quality rating |
| Verify | compile-gate, test-gate, clippy-gate, diff-gate, llm-judge-gate, consensus-gate | 6 | Truth checking, gates |
| Route | cascade-router, rule-router, cost-router | 3 | Candidate selection |
| Compose | prompt-composer, vcg-composer, greedy-composer | 3 | Signal combination |
| React | safety-reactor, budget-reactor, escalation-reactor, calibration-policy | 4 | Policy enforcement |
| Observe | 11 Lenses (see section 8) | 11 | Telemetry |
| Connect | chain-rpc, mcp, database, webhook, api | 5 | External I/O |
| Trigger | cron, webhook, file-watch, bus, chain-event, manual, hdc-bus, signal-pattern | 8 | Event-driven Graph firing |
| Domain | 28 domain-specific Cells (see section 11) | 28+ | Authoring, research, execution, deploy, ops, comms, code-intel |

Total: **46 built-in protocol Cells** at launch. Domain-specific Cells bring the shipped catalog above 70.

---

## 2. Store Cells

Cells implementing the Store protocol: `put / get / query / query_similar / prune` Signals ([doc-01](01-SIGNAL.md)).

### `file-store`

Persists Signals as JSONL on the local filesystem.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Store |
| Input | `Signal` |
| Output | `SignalRef` |
| Capabilities | `FsRead`, `FsWrite` |
| Description | Default Store for local development. Append-only JSONL with content-addressed IDs. Supports query by Kind, time range, and HDC similarity (`query_similar` via brute-force SIMD, <1ms for 800K entries). Prune removes entries below demurrage balance threshold. |

```toml
[[nodes]]
id = "persist"
cell = "file-store@^1"
[nodes.params]
path = ".roko/signals.jsonl"
```

### `memory-store`

In-memory Store for ephemeral Flows and testing.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Store |
| Input | `Signal` |
| Output | `SignalRef` |
| Capabilities | (none) |
| Description | Fast, volatile Store for unit tests and ephemeral Flows. All data lost on process exit. Same query interface as file-store including `query_similar`. |

```toml
[[nodes]]
id = "test-store"
cell = "memory-store@^1"
```

### `chain-store`

Persists Signal commitments on-chain for tamper-evident audit.

| Field | Value |
|---|---|
| Version | 0.1.0 |
| Protocols | Store |
| Input | `Signal` |
| Output | `SignalRef` (with tx hash) |
| Capabilities | `Chain { read: true, write: true }` |
| Description | Writes content hashes (not full content) to an on-chain registry ([doc-22](22-REGISTRIES.md)). Used for custody proofs and cross-agent attestation. Phase 2+. |

```toml
[[nodes]]
id = "anchor"
cell = "chain-store@^0.1"
[nodes.params]
network = "base-sepolia"
```

---

## 3. Score Cells

Cells implementing the Score protocol: rate Signals along 5 dimensions (relevance, quality, confidence, novelty, utility) ([doc-01, section 5](01-SIGNAL.md)).

Score Cells predict 5-axis quality, publish the prediction as a Pulse, and receive corrections from gate verdicts and episode rewards via the calibration loop. Per-axis weights update via online least-squares.

### `llm-scorer`

Model-based Signal scoring.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Score |
| Input | `Signal` |
| Output | `ScoreResult { relevance, quality, confidence, novelty, utility }` |
| Capabilities | `Llm` |
| Description | Sends the Signal to an LLM with a scoring rubric. Returns five-dimensional score. Model selected via CascadeRouter (EFE-based, [doc-02](02-CELL.md)) unless overridden. Predicts score before scoring, publishes prediction, corrects from gate verdicts. |

```toml
[[nodes]]
id = "score"
cell = "llm-scorer@^1"
[nodes.params]
rubric = "code-quality"
model = "claude-haiku-4-5"
```

### `rule-scorer`

Rule-based Signal scoring. Zero LLM cost.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Score |
| Input | `Signal` |
| Output | `ScoreResult` |
| Capabilities | (none) |
| Description | Evaluates Signals against declarative rules (regex matches, field presence, length thresholds, keyword density). Pure Rust pattern matching. T0-eligible (~80% of scoring ticks cost $0). |

```toml
[[nodes]]
id = "filter-score"
cell = "rule-scorer@^1"
[nodes.params]
rules = [
  { field = "content.length", op = "gte", value = 100, dimension = "quality", weight = 0.3 },
  { field = "kind", op = "eq", value = "code", dimension = "relevance", weight = 0.5 },
]
```

### `hdc-scorer`

HDC vector similarity scoring.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Score |
| Input | `Signal` (with HDC fingerprint, [doc-01, section 9](01-SIGNAL.md)) |
| Output | `ScoreResult` (similarity-weighted) |
| Capabilities | (none) |
| Description | Computes HDC Hamming similarity between the input Signal and a reference set. Returns similarity as the relevance dimension. Sub-microsecond per comparison via POPCNT. Used for knowledge retrieval ranking. Cross-domain resonance gives 15% additive bonus when domains differ. |

```toml
[[nodes]]
id = "similarity"
cell = "hdc-scorer@^1"
[nodes.params]
reference_set = "knowledge"
top_k = 20
```

---

## 4. Verify Cells

Cells implementing the Verify protocol: check Signals against truth criteria, produce Verdicts.

The Verify protocol is load-bearing ([doc-02](02-CELL.md)): it is the reward function (continuous `Verdict.reward`), the relabeling oracle (hindsight on failed trajectories, [doc-07](07-LEARNING.md)), the safety boundary (pre-action `verify_pre`), and the economic attestation (reputation flows from verified work via ERC-8004). All four learning loops depend on it.

**Key design decisions** carried by every Verify Cell:

1. **Pre-action and post-action.** Every Verify Cell implements `verify_pre()` (can veto execution) and `verify_post()` (evaluates results). Pre-action provides safety boundary; post-action provides reward signal.
2. **Continuous reward.** `Verdict.reward: f64` is a domain-specific learning signal alongside binary pass/fail. Feeds L1 parameter tuning and L2 strategy routing.
3. **Evidence typing.** `EvidenceCollector` is separate from `Criterion`. Evidence is collected by typed collectors (19 evidence kinds) and evaluated by criteria independently.
4. **Conjunctive hard + Pareto soft.** Hard criteria are AND — all must pass. Soft criteria are multi-objective Pareto — no weighted sum (Goodhart-resistant).

### `compile-gate`

Checks that code compiles.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Verify |
| Input | `Signal { kind: Code }` |
| Output | `Verdict { passed, reward, confidence, findings, evidence, hard_criteria, soft_criteria }` |
| Capabilities | `Shell { commands: ["cargo", "rustc", "tsc", "go"] }`, `FsRead` |
| Description | Runs the language-appropriate compiler. `verify_pre` checks source files exist and are well-formed. `verify_post` captures stderr as Finding Signals and compiler output as Evidence (kind: ProcessOutput). Reward: binary (1.0 on pass, 0.0 on fail). Hard criterion: zero compiler errors. Gate rung: 1. |

```toml
[[nodes]]
id = "compile"
cell = "compile-gate@^1"
[nodes.params]
command = "cargo check --workspace"
```

### `test-gate`

Runs test suites and checks for regressions.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Verify |
| Input | `Signal { kind: Code }` |
| Output | `Verdict` |
| Capabilities | `Shell { commands: ["cargo", "npm", "pytest", "jest"] }`, `FsRead` |
| Description | `verify_pre` checks test runner available and workspace compilable. `verify_post` runs the test runner, parses results, emits per-test Findings. Evidence kind: TestResult. Reward = pass_rate (continuous 0.0..=1.0). Hard criterion: no regressions vs baseline. Soft criterion: coverage delta. Gate rung: 2. |

```toml
[[nodes]]
id = "test"
cell = "test-gate@^1"
[nodes.params]
command = "cargo test --workspace"
target = "all"
```

### `clippy-gate`

Static analysis and lint checks.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Verify |
| Input | `Signal { kind: Code }` |
| Output | `Verdict` |
| Capabilities | `Shell { commands: ["cargo", "clippy", "eslint", "ruff"] }`, `FsRead` |
| Description | `verify_pre` checks linter installed and workspace compiles. `verify_post` runs linter with `-D warnings`. Findings are lint violations with location and severity. Evidence kind: LintReport. Reward = 1.0 - (violation_count / max_acceptable). Hard criterion: zero errors. Soft criterion: zero warnings. Gate rung: 3. |

```toml
[[nodes]]
id = "lint"
cell = "clippy-gate@^1"
[nodes.params]
command = "cargo clippy --workspace --no-deps -- -D warnings"
```

### `diff-gate`

Validates that a diff matches expected patterns or constraints.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Verify |
| Input | `Signal { kind: Diff }` |
| Output | `Verdict` |
| Capabilities | `FsRead`, `Shell { commands: ["git"] }` |
| Description | `verify_pre` checks diff is well-formed and within size bounds. `verify_post` checks diff against constraints: max lines changed, no secret patterns, no binary files, restricted paths respected. Evidence kind: DiffAnalysis. Hard criteria: no secrets, paths allowed. Soft criteria: diff size, file count. Gate rung: 4. |

```toml
[[nodes]]
id = "diff-check"
cell = "diff-gate@^1"
[nodes.params]
max_lines = 500
forbidden_patterns = ["API_KEY", "SECRET", "password"]
```

### `llm-judge-gate`

LLM-based quality evaluation producing a Verdict.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Score, Verify |
| Input | `Signal` |
| Output | `Verdict` (with ScoreResult embedded) |
| Capabilities | `Llm` |
| Description | Multi-protocol: Score + Verify. `verify_pre` checks input is scorable (non-empty, within token limit). `verify_post` sends Signal to LLM with evaluation criteria. Evidence kind: LlmJudgement. Reward = mean criterion score (continuous). Hard criterion: all criteria above threshold. Pairwise Bradley-Terry judges for inter-model comparison. Respects the Variance Inequality: verifier model must be spectrally cleaner than generator model. Gate rung: 5. |

```toml
[[nodes]]
id = "judge"
cell = "llm-judge-gate@^1"
[nodes.params]
criteria = ["correctness", "completeness", "clarity"]
threshold = 0.7
model = "claude-sonnet-4-6"
```

### `consensus-gate`

Multi-evaluator consensus verification.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Verify |
| Input | `Vec<Verdict>` (from multiple upstream Verify Cells) |
| Output | `Verdict` (aggregate) |
| Capabilities | (none) |
| Description | Takes N Verdicts, applies consensus strategy (majority, unanimous, weighted, quorum). `verify_pre` checks minimum voter count met. `verify_post` aggregates: reward = mean of upstream rewards. Evidence = union of upstream evidence sets. Hard criteria: consensus threshold met. Soft criteria: voter agreement spread. Gate rung: 6. |

```toml
[[nodes]]
id = "consensus"
cell = "consensus-gate@^1"
[nodes.params]
strategy = "majority"
min_voters = 3
```

---

## 5. Route Cells

Cells implementing the Route protocol: select among candidates, learn from outcomes.

Route Cells use **Expected Free Energy (EFE)** for selection (Friston 2006; [doc-02](02-CELL.md)). EFE naturally balances exploration (epistemic value) and exploitation (pragmatic value) while being cost-aware. Each cognitive timescale (T0/T1/T2 gating, L2 routing) uses a different free-energy lower bound. Route receives `regime: Signal` for context-aware selection — Calm regime favors exploration, Crisis regime favors cheapest reliable option.

### `cascade-router`

EFE-based model routing with learning. **Uses EFE, not LinUCB.**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Route, Observe |
| Input | `Vec<Signal>` (candidates) |
| Output | `RouteResult { selected, confidence, reason, efe_score }` |
| Capabilities | (none) |
| Description | Selects best candidate (typically a model) using Expected Free Energy. Balances exploration (epistemic value from uncertain arms) against exploitation (pragmatic value from known-good arms), conditioned on current regime. Calm -> explore more. Crisis -> exploit cheapest reliable. Observes outcomes via predict-publish-correct: predicts selection will succeed, receives gate verdict as outcome, updates EFE posteriors. Persists state to `.roko/learn/cascade-router.json`. Multi-protocol: Route + Observe. |

```toml
[[nodes]]
id = "model-select"
cell = "cascade-router@^1"
[nodes.params]
candidates = ["claude-opus-4-6", "claude-sonnet-4-6", "claude-haiku-4-5"]
context_features = ["task_complexity", "domain", "budget_remaining"]
```

### `rule-router`

Deterministic rule-based routing.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Route |
| Input | `Vec<Signal>` (candidates) |
| Output | `RouteResult` |
| Capabilities | (none) |
| Description | Evaluates candidates against declarative rules. No learning — deterministic selection. Used for fixed routing policies (e.g., "always use Opus for security reviews"). Still publishes predictions for calibration tracking. |

```toml
[[nodes]]
id = "fixed-route"
cell = "rule-router@^1"
[nodes.params]
rules = [
  { condition = "task.domain == 'security'", select = "claude-opus-4-6" },
  { condition = "task.budget_usd < 0.10", select = "claude-haiku-4-5" },
  { condition = "true", select = "claude-sonnet-4-6" },
]
```

### `cost-router`

Cheapest-viable candidate selection.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Route |
| Input | `Vec<Signal>` (candidates with cost estimates) |
| Output | `RouteResult` |
| Capabilities | (none) |
| Description | Selects cheapest candidate meeting minimum quality threshold. Useful for cost-sensitive Graphs and Conservation/Declining vitality phases ([doc-05](05-AGENT.md)) where budget pressure favors cheaper options. |

```toml
[[nodes]]
id = "cheap-route"
cell = "cost-router@^1"
[nodes.params]
min_quality = 0.6
prefer = "cheapest"
```

---

## 6. Compose Cells

Cells implementing the Compose protocol: combine Signals under budget into one Signal.

Compose Cells predict prompt-fits-budget-and-wins-gate, publish the prediction as a Pulse, and receive corrections from token count and gate verdict. Section effect beta-distributions track which context sections correlate with gate success, making context assembly learnable over time ([doc-07](07-LEARNING.md)).

### `prompt-composer`

9-layer system prompt assembly.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Compose |
| Input | `Vec<Signal>` (context Signals: role, task, knowledge, history, constraints) |
| Output | `Signal { kind: Text }` (assembled prompt) |
| Capabilities | (none) |
| Description | Assembles system prompt from up to 9 layers (role, domain, task, context, knowledge, history, constraints, tools, format). Budget-aware: truncates lower-priority layers to fit token limit. Maps to existing `RoleSystemPromptSpec` in orchestrate.rs. |

```toml
[[nodes]]
id = "build-prompt"
cell = "prompt-composer@^1"
[nodes.params]
role = "strategist"
max_tokens = 8000
priority_order = ["role", "task", "context", "knowledge", "constraints"]
```

### `vcg-composer`

VCG auction-based Signal combination (Vickrey-Clarke-Groves).

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Compose |
| Input | `Vec<Signal>` (with bids from context bidders) |
| Output | `Signal` (combined) |
| Capabilities | (none) |
| Description | Runs a VCG auction among context bidders (Neuro, Task, Research, Heuristic, Episode, Pheromone, Affect, System). Each bidder declares value for token budget. VCG allocates efficiently — pay your externality. Section effect tracking via beta-distribution posteriors adjusts bidder valuations over time. Built and exported but greedy path currently dominates at runtime. Novelty attenuation: `1/(1+ln(freq))` ([doc-01](01-SIGNAL.md)). |

```toml
[[nodes]]
id = "auction-compose"
cell = "vcg-composer@^1"
[nodes.params]
max_tokens = 16000
bidders = ["neuro", "task", "research"]
```

### `greedy-composer`

Top-K by score Signal combination.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Compose |
| Input | `Vec<Signal>` (scored) |
| Output | `Signal` (combined) |
| Capabilities | (none) |
| Description | Sorts input Signals by score, takes top K that fit within budget. Simple, fast, predictable. The default composition strategy. |

```toml
[[nodes]]
id = "top-k"
cell = "greedy-composer@^1"
[nodes.params]
max_signals = 10
max_tokens = 4000
sort_by = "relevance"
```

---

## 7. React Cells

Cells implementing the React protocol: watch Pulse streams, emit new Signals as interventions.

React Cells operate on **Pulses** (ephemeral), not Signals ([doc-01](01-SIGNAL.md)). The rationale: policies react to live events (heartbeats, gate verdicts, budget warnings, calibration updates), not stored artifacts. React output can include both Pulses (ephemeral reactions) and Signals (durable reactions that graduate).

```rust
pub struct ReactOutput {
    pub pulses: Vec<Pulse>,    // ephemeral reactions
    pub signals: Vec<Signal>,  // durable reactions (graduated)
}
```

### `safety-reactor`

Halt on danger signals.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | React |
| Input | `&[Pulse]` (subscribed topics) |
| Output | `ReactOutput` |
| Capabilities | (none) |
| Description | Monitors Pulse stream for safety violations: cost anomalies, permission escalation attempts, infinite loops, prompt injection indicators. Emits halt Pulses that the execution engine respects. Graduates critical safety events to durable Signals for audit. |

```toml
[[nodes]]
id = "safety"
cell = "safety-reactor@^1"
[nodes.params]
sensitivity = "high"
actions = ["halt", "alert"]
```

### `budget-reactor`

Alert and throttle on budget thresholds.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | React |
| Input | `&[Pulse]` (CostReport and BudgetAlert Pulses) |
| Output | `ReactOutput` |
| Capabilities | (none) |
| Description | Watches cost Pulses from BudgetLens. At 75% budget: emits warning Pulse. At 90%: emits throttle Pulse (engine switches to cheaper models via CascadeRouter). At 100%: emits halt Pulse. Thresholds configurable. Graduates budget breach events to durable Signals. |

```toml
[[nodes]]
id = "budget-watch"
cell = "budget-reactor@^1"
[nodes.params]
warn_pct = 75
throttle_pct = 90
halt_pct = 100
```

### `escalation-reactor`

Notify humans on escalation conditions.

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | React |
| Input | `&[Pulse]` (gate results, error events, confidence updates) |
| Output | `ReactOutput` |
| Capabilities | `Net` (for notification delivery) |
| Description | When conditions are met (repeated failures, confidence below threshold, structural changes proposed), emits notification Pulses routed to configured channels (Slack, email, dashboard). Graduates escalation decisions to durable Signals for audit trail. |

```toml
[[nodes]]
id = "escalate"
cell = "escalation-reactor@^1"
[nodes.params]
conditions = ["gate_fail_count > 3", "confidence < 0.3"]
channels = ["slack", "dashboard"]
```

### `calibration-policy`

Per-operator calibration from prediction/outcome streams. The structural implementation of predict-publish-correct ([doc-02](02-CELL.md), [doc-07](07-LEARNING.md)).

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | React |
| Input | `&[Pulse]` (subscribed to `prediction.**` and `outcome.**` topics) |
| Output | `ReactOutput` |
| Capabilities | (none) |
| Description | Subscribes to all `prediction.{operator}` and `outcome.{operator}` Pulse topics. Joins predictions with outcomes by `lineage_hint` (content hash). Computes calibration error and publishes updates on `calibration.{operator}.updated`. Maintains per-operator calibration state: Scorer (online least-squares per axis), Router (EFE posterior update), Composer (section effect beta update), Gate (threshold EMA), Policy (per-policy online calibration). Persists to `.roko/learn/calibration.json`. |

```toml
[[nodes]]
id = "calibration"
cell = "calibration-policy@^1"
[nodes.params]
topics = ["prediction.**", "outcome.**"]
persist_interval = "60s"
```

---

## 8. Observe Cells (Lenses)

Cells implementing the Observe protocol. Lenses are read-only observers that emit observation Signals onto the Bus. They never modify what they observe. See [doc-15](15-TELEMETRY.md) for the full Lens system, StateHub projections, and composition rules.

Every Lens Cell follows the standard 6-field format: Version, Protocols, Input, Output, Capabilities, and a TOML usage example.

### 8.1 `cost-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: CellExecution }` (observed Cell/Graph/Agent execution events via Bus) |
| Output | `Signal { kind: CostReport { total_usd: f64, rate_usd_per_min: f64, breakdown: BTreeMap<String, f64> } }` |
| Capabilities | (none) |
| Description | Aggregates USD cost across observed scope (Cell, Graph, or Agent). Emits periodic CostReport Signals with total, rate, and breakdown by model/provider. Configurable emission interval. Scope: Cell, Graph, Agent. |

```toml
[[lenses]]
name = "cost-monitor"
cell = "roko:cost-lens@^1.0"
scope = "graph"
[lenses.params]
interval = "60s"
```

### 8.2 `latency-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: CellExecution }` (observed Cell/Graph execution events via Bus) |
| Output | `Signal { kind: LatencyReport { p50_ms: f64, p95_ms: f64, p99_ms: f64, sample_count: u64 } }` |
| Capabilities | (none) |
| Description | Tracks execution duration across observed scope. Emits percentile distributions (p50, p95, p99) at configurable intervals. Scope: Cell, Graph. |

```toml
[[lenses]]
name = "latency-monitor"
cell = "roko:latency-lens@^1.0"
scope = "graph"
[lenses.params]
interval = "60s"
```

### 8.3 `quality-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: Verdict }` (observed Verify Cell outcomes via Bus) |
| Output | `Signal { kind: QualityReport { pass_rate: f64, mean_reward: f64, evidence_breakdown: BTreeMap<EvidenceKind, u32>, hard_failure_rate: f64, rung_breakdown: BTreeMap<u8, f64> } }` |
| Capabilities | (none) |
| Description | Observes Verify Cell Verdicts. Tracks `verify_pre` vetoes and `verify_post` outcomes. Emits rolling pass rate, mean continuous reward, evidence type breakdown, hard-criteria failure rate, per-gate rung breakdown. Scope: Graph. |

```toml
[[lenses]]
name = "quality-monitor"
cell = "roko:quality-lens@^1.0"
scope = "graph"
[lenses.params]
window = "1h"
```

### 8.4 `efficiency-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: AgentTurn }` (observed Agent execution turns via Bus) |
| Output | `Signal { kind: EfficiencyReport { tokens_per_task: f64, tasks_completed: u32, total_tokens: u64 } }` |
| Capabilities | (none) |
| Description | Tracks token usage relative to task completion. Lower ratio = more efficient agent. Feeds CascadeRouter's EFE learning loop. Scope: Agent. |

```toml
[[lenses]]
name = "efficiency-monitor"
cell = "roko:efficiency-lens@^1.0"
scope = "agent"
[lenses.params]
interval = "300s"
```

### 8.5 `error-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: CellError }` (observed error events via Bus) |
| Output | `Signal { kind: ErrorReport { by_type: BTreeMap<ErrorCategory, u32>, total: u32, trend: TrendDirection } }` |
| Capabilities | (none) |
| Description | Categorizes errors by type (timeout, capability, external, logic, cancelled). Emits error frequency and trend data. `ErrorCategory` enum: `Timeout`, `Capability`, `External`, `Logic`, `Cancelled`, `Unknown`. Scope: Cell, Graph, Agent. |

```toml
[[lenses]]
name = "error-monitor"
cell = "roko:error-lens@^1.0"
scope = "agent"
[lenses.params]
interval = "60s"
```

### 8.6 `drift-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: MemoryState }` (observed Memory Cell state via Bus) |
| Output | `Signal { kind: DriftReport { balance_distribution: Vec<f64>, cold_entry_count: u32, mean_heuristic_calibration: f64, stale_citation_count: u32 } }` |
| Capabilities | (none) |
| Description | Monitors a Memory Cell for staleness: entries losing balance via demurrage ([doc-01, section 6](01-SIGNAL.md)), citations gone dead, scores declining. Emits drift alerts with balance distribution, cold-entry count, heuristic calibration averages. Scope: Memory. |

```toml
[[lenses]]
name = "drift-monitor"
cell = "roko:drift-lens@^1.0"
scope = "memory"
[lenses.params]
cold_threshold = 0.2
interval = "3600s"
```

### 8.7 `budget-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: CostReport }` (from cost-lens or direct cost events via Bus) |
| Output | `Signal { kind: BudgetAlert { usage_pct: f64, level: AlertLevel, remaining_usd: f64 } }` |
| Capabilities | (none) |
| Description | Watches budget consumption rate. Emits alerts at configurable thresholds: `Warn` (default 75%), `Throttle` (default 90%), `Halt` (default 100%). Consumed by budget-reactor. `AlertLevel` enum: `Warn`, `Throttle`, `Halt`. Scope: Agent, Space. |

```toml
[[lenses]]
name = "budget-monitor"
cell = "roko:budget-lens@^1.0"
scope = "agent"
[lenses.params]
warn_pct = 75
throttle_pct = 90
halt_pct = 100
```

### 8.8 `trend-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal` (any Lens output -- trend-lens chains with any other Lens) |
| Output | `Signal { kind: TrendReport { slope: f64, ema: f64, derivative: f64, direction: TrendDirection } }` |
| Capabilities | (none) |
| Description | Meta-Lens: observes another Lens's output stream and computes statistical trends: slope (linear regression), EMA (exponential moving average), first derivative. Chains with any other Lens (e.g., trend-lens watching cost-lens computes cost trajectory). `TrendDirection` enum: `Rising`, `Falling`, `Stable`. Scope: any other Lens. |

```toml
[[lenses]]
name = "cost-trend"
cell = "roko:trend-lens@^1.0"
scope = "lens"
[lenses.params]
source_lens = "cost-monitor"
ema_alpha = 0.3
```

### 8.9 `anomaly-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal` (any Lens output -- anomaly-lens chains with any other Lens) |
| Output | `Signal { kind: AnomalyAlert { z_score: f64, iqr_outlier: bool, severity: AnomalySeverity, source_value: f64 } }` |
| Capabilities | (none) |
| Description | Meta-Lens: detects anomalies in another Lens's output using Z-score (default threshold: 3.0) and IQR methods. Configurable sensitivity. Feeds safety-reactor. `AnomalySeverity` enum: `Low`, `Medium`, `High`, `Critical`. Scope: any other Lens. |

```toml
[[lenses]]
name = "cost-anomaly"
cell = "roko:anomaly-lens@^1.0"
scope = "lens"
[lenses.params]
source_lens = "cost-monitor"
z_threshold = 3.0
sensitivity = "medium"
```

### 8.10 `usage-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: ArtifactEvent }` (install, run, fork events via Bus) |
| Output | `Signal { kind: UsageReport { installs: u64, active_runs: u64, forks: u64, error_rate: f64, avg_cost_usd: f64 } }` |
| Capabilities | (none) |
| Description | Tracks usage metrics for published artifacts: installs, active runs, forks, error rates, cost averages. Powers marketplace creator analytics and trending algorithms. Scope: Space, Marketplace. |

```toml
[[lenses]]
name = "artifact-usage"
cell = "roko:usage-lens@^1.0"
scope = "marketplace"
[lenses.params]
interval = "3600s"
```

### 8.11 `collective-intelligence-lens`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Observe |
| Input | `Signal { kind: AgentTurn }` + `Signal { kind: Citation }` + `Signal { kind: HdcFingerprint }` (multi-agent interaction events via Bus) |
| Output | `Signal { kind: CFactorReport { c_factor: f64, turn_taking_entropy: f64, peer_prediction_accuracy: f64, citation_reciprocity: f64, hdc_diversity: f64 } }` |
| Capabilities | (none) |
| Description | Computes the **c-factor** -- collective intelligence as a runtime observable (Woolley et al. 2010, *Science*). Derived from four components: turn-taking entropy, peer prediction accuracy, citation reciprocity, HDC diversity. The c-factor gates L4 structural adaptation ([doc-07](07-LEARNING.md)) -- only evolve configurations that increase genuine collective intelligence. It is a covariate, not an objective. Scope: Space (across all agents). |

```toml
[[lenses]]
name = "collective-intelligence"
cell = "roko:collective-intelligence-lens@^1.0"
scope = "space"
[lenses.params]
interval = "300s"
min_agents = 2
```

---

## 9. Connect Cells (Connectors)

Cells implementing the Connect protocol: `connect / query / execute / health / disconnect`. See [doc-11](11-CONNECTIVITY.md) for the full Connector model and exoskeleton protocols.

### `chain-rpc-connector`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Connect |
| Input | `Signal { kind: RpcRequest { method: String, params: Value } }` |
| Output | `Signal { kind: RpcResponse { result: Value, tx_hash: Option<String> } }` |
| Capabilities | `Chain { read: true, write: configurable }`, `Net` |
| Description | Connects to blockchain RPC endpoints. Supports read queries (balances, events, contract state) and write operations (transactions). Health check via `eth_blockNumber`. Publishes Feeds on Bus topics per chain. |

```toml
[[nodes]]
id = "chain"
cell = "chain-rpc-connector@^1"
[nodes.params]
rpc_url = "https://mainnet.base.org"
chain_id = 8453
```

### `mcp-connector`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Connect |
| Input | `Signal { kind: McpToolCall { tool_name: String, arguments: Value } }` |
| Output | `Signal { kind: McpToolResult { result: Value, is_error: bool } }` |
| Capabilities | (depends on MCP server capabilities) |
| Description | Wraps an MCP server as a Connector. Auto-discovered from `agent.mcp_config`. Exposes MCP tools as queryable operations. Every invocation carries CellInput/CellOutput ([doc-11, section 4.1](11-CONNECTIVITY.md)). |

```toml
[[nodes]]
id = "mcp"
cell = "mcp-connector@^1"
[nodes.params]
server = "github"
config_path = ".roko/mcp.json"
```

### `database-connector`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Connect |
| Input | `Signal { kind: DbQuery { sql: String, params: Vec<Value> } }` |
| Output | `Signal { kind: DbResult { rows: Vec<BTreeMap<String, Value>>, affected: u64 } }` |
| Capabilities | `Net` (remote), `FsRead` (SQLite) |
| Description | Connection-pooled database access. Read queries via `query()`, mutations via `execute()`. Health check via `SELECT 1`. Supports PostgreSQL, MySQL, SQLite. |

```toml
[[nodes]]
id = "db"
cell = "database-connector@^1"
[nodes.params]
driver = "sqlite"
path = ".roko/data.db"
```

### `webhook-connector`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Connect |
| Input | `Signal` (any Signal to deliver) |
| Output | `Signal { kind: WebhookResult { status_code: u16, response_body: Option<String> } }` |
| Capabilities | `Net { domains: configurable }` |
| Description | Delivers Signals to external HTTP endpoints. Supports retry with exponential backoff, HMAC signature, configurable headers. |

```toml
[[nodes]]
id = "notify-webhook"
cell = "webhook-connector@^1"
[nodes.params]
url = "https://hooks.example.com/roko"
hmac_secret_key = "webhook.hmac"
```

### `api-connector`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Connect |
| Input | `Signal { kind: ApiRequest { method: String, path: String, body: Option<Value>, headers: BTreeMap<String, String> } }` |
| Output | `Signal { kind: ApiResponse { status_code: u16, body: Value, headers: BTreeMap<String, String> } }` |
| Capabilities | `Net { domains: configurable }` |
| Description | General-purpose API client. Supports authentication strategies (Bearer, API key, OAuth2). Rate limiting and circuit breaker built in. |

```toml
[[nodes]]
id = "api"
cell = "api-connector@^1"
[nodes.params]
base_url = "https://api.example.com"
auth = "bearer"
secret_key = "api.example"
```

---

## 10. Trigger Cells

Cells implementing the Trigger protocol: `arm / disarm / poll` for events that fire Graphs. See [doc-13](13-TRIGGERS.md) for the full trigger specification.

### `cron-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | (none -- fires on schedule) |
| Output | `TriggerEvent { fired_at, payload: { expression }, source: Cron }` |
| Capabilities | (none) |
| Description | Schedule-based Graph firing. Standard 6-field cron syntax (second-resolution) plus `@hourly`, `@daily`, `@weekly` shortcuts. |

```toml
[[triggers]]
name = "daily-gc"
kind = "cron"
expression = "@daily"
graph = "plans/knowledge-gc.toml"
```

### `webhook-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | `HTTP Request` (inbound webhook payload) |
| Output | `TriggerEvent { fired_at, payload: { method, path, headers, body }, source: Webhook }` |
| Capabilities | `Net` (binds HTTP listener) |
| Description | Inbound HTTP webhook listener. Supports payload filtering, HMAC verification, path-based routing. Used for GitHub webhooks, Slack events. |

```toml
[[triggers]]
name = "github-pr"
kind = "webhook"
path = "/hooks/github"
hmac_secret = "github.webhook_secret"
graph = "plans/pr-review.toml"
[triggers.filter]
event = "pull_request"
action = ["opened", "synchronize"]
```

### `file-watch-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | (none -- watches filesystem) |
| Output | `TriggerEvent { fired_at, payload: { path, event_kind }, source: FileWatch }` |
| Capabilities | `FsRead` |
| Description | Filesystem change detection. Debounced with configurable delay. Uses `notify::RecommendedWatcher`. Supports glob patterns for filtering. |

```toml
[[triggers]]
name = "src-change"
kind = "file_watch"
paths = ["src/"]
glob = "**/*.rs"
debounce_ms = 500
graph = "plans/local-ci.toml"
```

### `bus-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | `Pulse` (matching Bus topic subscription) |
| Output | `TriggerEvent { fired_at, payload: { topic, pulse }, source: Bus }` |
| Capabilities | (none) |
| Description | Pulse Bus topic listener. Fires when a Pulse of a specific Kind appears on a Bus topic. The primary internal event mechanism for chaining Graphs. |

```toml
[[triggers]]
name = "on-gate-fail"
kind = "bus"
topic = "gate.verdict"
graph = "plans/gate-failure-replan.toml"
[triggers.filter]
body = { passed = false }
```

### `chain-event-trigger`

| Field | Value |
|---|---|
| Version | 0.1.0 |
| Protocols | Trigger |
| Input | (none -- watches chain events) |
| Output | `TriggerEvent { fired_at, payload: { chain_id, block_number, tx_hash, log_data }, source: ChainEvent }` |
| Capabilities | `Chain { read: true }`, `Net` |
| Description | Smart contract event listener (EVM log topics). Subscribes to on-chain events and fires Graphs when matching events are detected. Phase 2+. |

```toml
[[triggers]]
name = "agent-registered"
kind = "chain_event"
contract = "0x..."
event = "AgentRegistered(address,bytes32)"
graph = "plans/onboard-agent.toml"
```

### `manual-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | User action (CLI command, API call, dashboard button) |
| Output | `TriggerEvent { fired_at, payload: { user, args }, source: Manual }` |
| Capabilities | (none) |
| Description | User-initiated Graph firing. Every `roko run <graph>` creates an implicit manual-trigger. |

```toml
[[triggers]]
name = "deploy-staging"
kind = "manual"
description = "Deploy the current build to staging"
confirm = true
graph = "plans/deploy-staging.toml"
```

### `hdc-bus-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | `Pulse` (Pulse stream on Bus with HDC fingerprint) |
| Output | `TriggerEvent { fired_at, payload: { matched_pulse, similarity, reference_fingerprint }, source: Bus }` |
| Capabilities | (none) |
| Description | HDC-similarity pattern matching on the **Pulse stream** (ephemeral Bus). Fires when a Pulse with HDC fingerprint similar to a reference pattern appears above a configurable threshold. Enables content-addressable event matching on live streams. **Distinct from `signal-pattern-trigger`**: this Cell matches individual Pulses in real-time; `signal-pattern-trigger` polls aggregate conditions in Store. |

```toml
[[triggers]]
name = "similar-error"
kind = "hdc_bus"
reference_fingerprint = "base64-encoded-hdc-vector"
similarity_threshold = 0.85
topic = "error.**"
graph = "plans/investigate-similar-error.toml"
```

### `signal-pattern-trigger`

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | Trigger |
| Input | (none -- polls Store periodically) |
| Output | `TriggerEvent { fired_at, payload: { matched_signals, match_count }, source: SignalPattern }` |
| Capabilities | (none) |
| Description | Aggregate pattern matching by **polling Store** (durable Signals). Fires when a query against Store produces `min_matches` or more results within a time window. Unlike `hdc-bus-trigger` (which matches individual Pulses on Bus in real-time), this Cell detects aggregate conditions over persisted Signals. Example: "fire when 3+ high-severity Findings appear within 5 minutes." Defined in [doc-13, section 3.7](13-TRIGGERS.md). |

```toml
[[triggers]]
name = "failure-cluster"
kind = "signal_pattern"
graph = "plans/investigate-failures.toml"

[triggers.query]
kind = "Finding"
severity = "high"
min_matches = 3
window_seconds = 300
poll_interval_seconds = 30
```

---

## 11. Domain-Specific Cells

Beyond protocol Cells, Roko ships domain Cells that implement the base `Cell` trait and compose into common Graphs. Each category has at least one representative Cell with full typed I/O. All domain Cells declare `input_schema` and `output_schema` for Graph edge validation.

### 11.1 Authoring

**Representative Cell: `prd-synthesize`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: DocumentCluster { segments: Vec<ClassifiedSegment>, source_files: Vec<PathBuf> } }` |
| Output | `Signal { kind: Prd { title: String, slug: String, sections: Vec<PrdSection>, acceptance_criteria: Vec<String> } }` |
| Capabilities | `Llm` |
| Description | Generate a PRD from clustered document segments. Produces structured PRD with title, sections, and acceptance criteria. Auto-fills slug from title. |

```toml
[[nodes]]
id = "synthesize"
cell = "prd-synthesize@^1"
[nodes.params]
style = "structured"
max_sections = 10
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `fs-walk` | `Signal { kind: FsWalkRequest { root: PathBuf, glob: String } }` | `Signal { kind: FileList { paths: Vec<PathBuf>, total_bytes: u64 } }` | `FsRead` |
| `markdown-segment` | `Signal { kind: Text }` | `Signal { kind: SegmentList { segments: Vec<Segment { heading: String, content: String, level: u8 }> } }` | (none) |
| `markdown-classify` | `Signal { kind: Segment }` | `Signal { kind: ClassifiedSegment { segment: Segment, intent: SegmentIntent } }` | `Llm` |
| `doc-cluster` | `Vec<Signal { kind: ClassifiedSegment }>` | `Signal { kind: DocumentCluster { clusters: Vec<Vec<ClassifiedSegment>> } }` | `Llm` |
| `prd-audit` | `Signal { kind: Prd }` | `Signal { kind: AuditReport { contradictions: Vec<String>, vague_language: Vec<Span>, missing_criteria: Vec<String> } }` | `Llm` |
| `prd-plan` | `Signal { kind: Prd }` | `Signal { kind: TaskPlan { tasks: Vec<Task>, dag: Vec<Edge> } }` | `Llm` |
| `plan-validate` | `Signal { kind: TaskPlan }` | `Signal { kind: ValidationResult { valid: bool, cycles: Vec<Vec<String>>, orphans: Vec<String> } }` | (none) |
| `artifact-persist` | `Signal` (any) | `Signal { kind: PersistResult { path: PathBuf, content_hash: ContentHash } }` | `FsWrite` |

### 11.2 Research

**Representative Cell: `web-search`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: SearchQuery { query: String, max_results: u32, domains: Option<Vec<String>> } }` |
| Output | `Signal { kind: SearchResults { results: Vec<SearchResult { title: String, url: String, snippet: String, relevance: f64 }>, provider: String } }` |
| Capabilities | `Net`, `Llm` |
| Description | Web search via Perplexity or configured provider. Returns ranked results with snippets and relevance scores. Provider selected via config or CascadeRouter. |

```toml
[[nodes]]
id = "search"
cell = "web-search@^1"
[nodes.params]
provider = "perplexity"
max_results = 10
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `academic-search` | `Signal { kind: SearchQuery }` | `Signal { kind: PaperResults { papers: Vec<Paper { title, authors, abstract_, doi, year }> } }` | `Net` |
| `citation-check` | `Signal { kind: CitedClaim { claim: String, source_url: String } }` | `Signal { kind: CitationVerdict { verified: bool, confidence: f64, discrepancies: Vec<String> } }` | `Net`, `Llm` |
| `fact-check` | `Signal { kind: FactClaim { claim: String, corpus: Vec<SignalRef> } }` | `Signal { kind: FactVerdict { supported: bool, confidence: f64, evidence: Vec<String> } }` | `Llm` |
| `knowledge-ingest` | `Signal` (any) | `Signal { kind: IngestResult { stored_count: u32, deduplicated: u32 } }` | `FsWrite` |
| `knowledge-link` | `Signal { kind: HdcFingerprint }` | `Signal { kind: CrossDomainLinks { links: Vec<Link { source: SignalRef, target: SignalRef, similarity: f64 }> } }` | (none) |

### 11.3 Execution

**Representative Cell: `agent-dispatch`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: TaskAssignment { prompt: String, role: String, tools: Vec<String>, context: Vec<Signal> } }` |
| Output | `Signal { kind: AgentResult { output: String, tokens_used: u64, cost_usd: f64, model: String, turn_count: u32 } }` |
| Capabilities | `Llm`, `Shell` |
| Description | Dispatch task to an Agent (Claude CLI, Claude API, Codex, Cursor, OpenAI-compat, Ollama, Gemini). Backend selected via CascadeRouter or explicit override. Wraps the full agent tool loop. |

```toml
[[nodes]]
id = "implement"
cell = "agent-dispatch@^1"
[nodes.params]
role = "coding"
backend = "auto"
max_turns = 20
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `test-run` | `Signal { kind: TestRequest { command: String, working_dir: PathBuf } }` | `Signal { kind: TestResult { passed: u32, failed: u32, skipped: u32, output: String } }` | `Shell`, `FsRead` |
| `build` | `Signal { kind: BuildRequest { command: String, working_dir: PathBuf } }` | `Signal { kind: BuildResult { success: bool, artifacts: Vec<PathBuf>, output: String } }` | `Shell`, `FsRead` |
| `script-run` | `Signal { kind: ScriptRequest { script: String, args: Vec<String> } }` | `Signal { kind: ScriptResult { exit_code: i32, stdout: String, stderr: String } }` | `Shell` |
| `refactor-apply` | `Signal { kind: RefactorSpec { pattern: String, replacement: String, files: Vec<PathBuf> } }` | `Signal { kind: RefactorResult { files_changed: Vec<PathBuf>, hunks: u32 } }` | `FsRead`, `FsWrite`, `Llm` |

### 11.4 Deploy

**Representative Cell: `deploy-railway`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: DeployRequest { target: String, config_path: Option<PathBuf>, env_vars: BTreeMap<String, String> } }` |
| Output | `Signal { kind: DeployResult { url: String, deploy_id: String, status: DeployStatus, duration_secs: f64 } }` |
| Capabilities | `Net`, `Shell`, `Secrets` |
| Description | Deploy to Railway. Uses railway.toml or interactive configuration. Supports `--from-config` for pre-configured deployments. |

```toml
[[nodes]]
id = "deploy"
cell = "deploy-railway@^1"
[nodes.params]
config = "railway.toml"
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `deploy-fly` | `Signal { kind: DeployRequest }` | `Signal { kind: DeployResult }` | `Net`, `Shell`, `Secrets` |
| `deploy-vercel` | `Signal { kind: DeployRequest }` | `Signal { kind: DeployResult }` | `Net`, `Shell`, `Secrets` |
| `deploy-shell` | `Signal { kind: DeployRequest }` | `Signal { kind: DeployResult }` | `Shell`, `Secrets` |
| `smoke-test` | `Signal { kind: SmokeTestRequest { endpoints: Vec<String>, expected_status: u16, timeout_secs: u32 } }` | `Signal { kind: SmokeTestResult { passed: bool, results: Vec<EndpointResult { url, status, latency_ms }> } }` | `Net` |
| `rollback` | `Signal { kind: RollbackRequest { deploy_id: String, target: String } }` | `Signal { kind: RollbackResult { success: bool, rolled_back_to: String } }` | `Net`, `Shell`, `Secrets` |

### 11.5 Operations

**Representative Cell: `backup`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: BackupRequest { source_path: PathBuf, destination: String, compression: bool } }` |
| Output | `Signal { kind: BackupResult { snapshot_id: String, size_bytes: u64, files_included: u32, destination: String } }` |
| Capabilities | `FsRead`, `Net`, `Secrets` |
| Description | Snapshot `.roko/` to configured remote (S3, GCS, local path). Supports incremental backups via content-hash comparison. |

```toml
[[nodes]]
id = "backup"
cell = "backup@^1"
[nodes.params]
source = ".roko/"
destination = "s3://my-bucket/roko-backups/"
compression = true
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `restore` | `Signal { kind: RestoreRequest { snapshot_id: String, target_path: PathBuf } }` | `Signal { kind: RestoreResult { files_restored: u32, size_bytes: u64 } }` | `FsWrite`, `Net`, `Secrets` |
| `gc` | `Signal { kind: GcRequest { max_age_days: u32, dry_run: bool } }` | `Signal { kind: GcResult { removed_count: u32, freed_bytes: u64, dry_run: bool } }` | `FsWrite` |
| `cost-report` | `Signal { kind: CostReportRequest { scope: String, period: Duration } }` | `Signal { kind: CostSummary { total_usd: f64, by_model: BTreeMap<String, f64>, by_agent: BTreeMap<String, f64> } }` | (none) |
| `dependency-update` | `Signal { kind: DepUpdateRequest { working_dir: PathBuf } }` | `Signal { kind: DepUpdateResult { updated: Vec<DepChange { name, from, to }>, gates_passed: bool } }` | `Shell`, `FsWrite` |
| `dependency-audit` | `Signal { kind: DepAuditRequest { working_dir: PathBuf } }` | `Signal { kind: DepAuditResult { vulnerabilities: Vec<Cve { id, severity, package }>, abandoned: Vec<String> } }` | `Shell` |

### 11.6 Communication

**Representative Cell: `slack-notify`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: NotifyRequest { channel: String, message: String, blocks: Option<Value> } }` |
| Output | `Signal { kind: NotifyResult { delivered: bool, message_id: Option<String>, channel: String } }` |
| Capabilities | `Net`, `Secrets` |
| Description | Post message to Slack channel. Supports plain text and Block Kit structured messages. Uses Slack Bot Token from secrets store. |

```toml
[[nodes]]
id = "notify"
cell = "slack-notify@^1"
[nodes.params]
channel = "#deployments"
secret_key = "slack.bot_token"
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `github-comment` | `Signal { kind: GithubCommentRequest { repo: String, issue_or_pr: u64, body: String } }` | `Signal { kind: GithubCommentResult { comment_id: u64, url: String } }` | `Net`, `Secrets` |
| `email-send` | `Signal { kind: EmailRequest { to: String, subject: String, body: String } }` | `Signal { kind: EmailResult { delivered: bool, message_id: String } }` | `Net`, `Secrets` |
| `discord-notify` | `Signal { kind: DiscordRequest { channel_id: String, message: String } }` | `Signal { kind: DiscordResult { delivered: bool, message_id: String } }` | `Net`, `Secrets` |

### 11.7 Code Intelligence

**Representative Cell: `impact-analysis`**

| Field | Value |
|---|---|
| Version | 1.0.0 |
| Protocols | (base Cell) |
| Input | `Signal { kind: Diff }` |
| Output | `Signal { kind: ImpactReport { affected_symbols: Vec<Symbol { name, file, line }>, affected_tests: Vec<String>, risk_score: f64 } }` |
| Capabilities | `FsRead` |
| Description | Given a diff, report downstream impacts: affected symbols, affected tests, risk score. Uses the code-intel index (built by `index-build`) for symbol resolution. |

```toml
[[nodes]]
id = "impact"
cell = "impact-analysis@^1"
[nodes.params]
index_path = ".roko/index/"
include_tests = true
```

| Cell | Input | Output | Capabilities |
|---|---|---|---|
| `index-build` | `Signal { kind: IndexRequest { root: PathBuf, languages: Vec<String> } }` | `Signal { kind: IndexResult { files_indexed: u32, symbols_found: u32, index_path: PathBuf } }` | `FsRead`, `Shell` |
| `code-search` | `Signal { kind: CodeQuery { query: String, mode: SearchMode, max_results: u32 } }` | `Signal { kind: CodeResults { matches: Vec<CodeMatch { file, line, snippet, score }> } }` | `FsRead` |
| `type-check` | `Signal { kind: TypeCheckRequest { working_dir: PathBuf, language: String } }` | `Signal { kind: TypeCheckResult { errors: Vec<TypeError { file, line, message }>, passed: bool } }` | `Shell`, `FsRead` |
| `symbol-graph` | `Signal { kind: SymbolGraphRequest { root: PathBuf } }` | `Signal { kind: SymbolGraph { nodes: Vec<SymbolNode>, edges: Vec<SymbolEdge { from, to, relation }> } }` | `FsRead` |

---

## 12. MCP Integration

Roko Cells are accessible as MCP tools and Roko Agents can consume MCP tools. The integration is bidirectional.

### Roko Cells as MCP tools

Every Cell that declares `capabilities: [Mcp]` or is registered via `agent.mcp_config` is automatically exposed as an MCP tool. The Cell's typed I/O schema maps to MCP's `tool.call` request/response format:

- **CellInput** -> MCP `tool.call` request (JSON parameters)
- **CellOutput** -> MCP `tool.call` response (JSON result)

### External MCP tools as Cells

MCP servers configured in `agent.mcp_config` auto-register as `McpConnector` instances ([doc-11](11-CONNECTIVITY.md)). Each MCP tool becomes a queryable operation via the Connector's `query()` and `execute()` methods.

### Safety hooks for tool dispatch

Before any tool call executes, the Extension system's L4 Action hooks fire ([doc-12](12-EXTENSIONS.md)):

1. `pre_action()` — can Block or Modify the tool call
2. `on_tool_call()` — can Block or Substitute the tool call
3. (tool executes)
4. `post_action()` — observes result

This means safety Extensions intercept both built-in Cell invocations and external MCP tool calls uniformly. A safety Extension that blocks `rm -rf` works whether the call originates from a built-in Cell or an external MCP server.

### Tool schema

Every Cell declares its input/output schema via `TypeSchema`:

```rust
pub enum TypeSchema {
    /// Expects/produces Signals of a specific Kind.
    Signal { kind: Option<Kind> },
    /// Expects/produces a specific JSON schema.
    Json { schema: Value },
    /// Expects/produces raw bytes.
    Bytes,
    /// Any type (for generic Cells).
    Any,
}
```

The schema is used for:
- **Compile-time Graph validation**: edges checked for type compatibility
- **Runtime dispatch**: MCP tool discovery advertises the schema
- **Documentation**: `roko cell show <name>` displays the schema

---

## 13. Synergy Patterns

The catalog is designed for emergent composition. Cells combine via Graphs to create synergistic pipelines that no single Cell could provide:

| # | Pattern | Cells | Trigger | Effect |
|---|---|---|---|---|
| 1 | **Doc-to-Plan** | `fs-walk` + `markdown-classify` + `doc-cluster` + `prd-synthesize` + `prd-plan` | `file-watch-trigger` on docs/ | New docs auto-produce plans |
| 2 | **PR Review** | `webhook-trigger` (GitHub) + `agent-dispatch` (code-review) + `github-comment` | `webhook-trigger` | Every PR gets automated review |
| 3 | **Code-to-Docs** | `file-watch-trigger` on src/ + `impact-analysis` + `agent-dispatch` (doc-writer) | `file-watch-trigger` | Docs stay in sync with code |
| 4 | **Local CI** | `file-watch-trigger` on src/ + `compile-gate` + `test-gate` + `clippy-gate` | `file-watch-trigger` | Continuous local verification |
| 5 | **Ship Pipeline** | `build` + `deploy-railway` + `smoke-test` + `slack-notify` | `manual-trigger` | One-command ship |
| 6 | **Idea Pipeline** | `web-search` + `prd-synthesize` + `prd-plan` + `agent-dispatch` | `manual-trigger` | Idea to shipped code |
| 7 | **Knowledge GC** | `gc` + `knowledge-link` | `cron-trigger` weekly | Pruning + new connections |
| 8 | **Cost Alert** | `cost-lens` + `trend-lens` + `budget-reactor` + `escalation-reactor` | `bus-trigger` on CostReport | Auto-triage on cost spikes |
| 9 | **Learning Loop** | `cascade-router` + `efficiency-lens` + `trend-lens` + `calibration-policy` | Implicit (every run) | System improves model selection per Cell |
| 10 | **Collective Intelligence** | `collective-intelligence-lens` + `trend-lens` + `anomaly-lens` | `cron-trigger` or continuous | c-factor tracked and trends monitored |

These patterns are not hardcoded pipelines. They emerge from composing individual Cells via Graphs and Triggers. Users discover useful patterns and publish them as Graphs in the marketplace.

---

## 14. Implementation Tiers

| Tier | When | Cells |
|---|---|---|
| **Tier 0** (kernel) | First | All Verify Cells (gates), `prompt-composer`, `cascade-router`, `agent-dispatch`, `file-store`, `prd-synthesize`, `prd-plan`, `calibration-policy` |
| **Tier 1** (authoring) | First | `fs-walk`, `markdown-classify`, `doc-cluster`, `prd-audit`, `citation-check`, `artifact-persist`, `knowledge-ingest` |
| **Tier 2** (deploy + verify) | Soon | All Deploy Cells, `smoke-test`, `llm-judge-gate`, `consensus-gate`, `webhook-trigger`, `file-watch-trigger` |
| **Tier 3** (operations) | Soon | `backup`, `gc`, `cost-report`, `dependency-update`, all Communication Cells, `cron-trigger` |
| **Tier 4** (knowledge + observe) | Mid | All Observe Cells (Lenses) including `collective-intelligence-lens`, all React Cells, `knowledge-link`, `hdc-scorer`, `hdc-bus-trigger`, `signal-pattern-trigger` |
| **Tier 5** (chain + advanced) | Late | `chain-store`, `chain-rpc-connector`, `chain-event-trigger`, `vcg-composer` |

---

## 15. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| TL-1 | Every Tier 0 Cell ships with typed I/O, capabilities, and a TOML usage example | `roko cell list` returns the full Tier 0 set |
| TL-2 | Each Cell validates clean when composed in a Graph | `roko graph validate` passes for all builtin Graphs |
| TL-3 | Capability intersection enforced at runtime per Cell | Cell requesting ungranteed capability fails closed |
| TL-4 | Synergy patterns from section 13 work via Graph + Trigger composition | Multi-step integration test |
| TL-5 | Every Cell declares cost estimation | `roko cell show <name>` displays estimates |
| TL-6 | Every catalog entry's TOML lives in `<roko-install>/builtin/cells/` | Filesystem invariant check |
| TL-7 | All Verify Cells implement both `verify_pre` and `verify_post` | Compile check + unit test per gate |
| TL-8 | All Verify Cells emit typed Evidence alongside Findings | Unit test: Verdict contains Evidence with correct EvidenceKind |
| TL-9 | Verify Cells produce continuous `Verdict.reward` (not just binary) | Unit test: reward is f64 in (0.0..=1.0) |
| TL-10 | Verify Cells carry both `hard_criteria` and `soft_criteria` in Verdict | Compile check |
| TL-11 | CascadeRouter uses EFE (not LinUCB) for selection | Unit test: route output includes `efe_score` field |
| TL-12 | CascadeRouter responds to regime conditioning | Test: Calm -> higher exploration, Crisis -> exploit |
| TL-13 | All React Cells accept `&[Pulse]` input (not `&[Signal]`) | Compile check |
| TL-14 | CalibrationPolicy joins predictions with outcomes by lineage_hint | Integration test: publish prediction + outcome, verify calibration update |
| TL-15 | CalibrationPolicy publishes on `calibration.{operator}.updated` | Integration test: verify Pulse topic and payload |
| TL-16 | CollectiveIntelligenceLens computes c-factor from 4 components | Integration test with multi-agent Space |
| TL-17 | All Cells publish predictions for calibration tracking | Integration test: run Cell, verify prediction Pulse emitted |
| TL-18 | MCP integration: Cell invocable via MCP tool.call | Integration test: invoke Cell via MCP, verify CellInput/CellOutput |
| TL-19 | Safety hooks intercept MCP tool calls same as built-in Cells | Integration test: safety Extension blocks dangerous MCP tool call |
| TL-20 | Tool schema declared for every built-in Cell | `roko cell show <name>` displays TypeSchema |
| TL-21 | All Observe Cells (Lenses) use standard 6-field table format (Version, Protocols, Input, Output, Capabilities, Description) | Doc review: every Lens in section 8 has all 6 fields |
| TL-22 | All Observe Cells declare typed Input and Output schemas | Compile check: `input_schema()` and `output_schema()` return `Some` |
| TL-23 | `hdc-bus-trigger` matches individual Pulses by HDC similarity on Bus | Integration test: publish Pulse with HDC fingerprint, verify trigger fires above threshold |
| TL-24 | `signal-pattern-trigger` polls Store for aggregate Signal conditions | Integration test: add N Signals to Store, verify trigger fires at `min_matches` |
| TL-25 | `hdc-bus-trigger` and `signal-pattern-trigger` are distinct Cells with different semantics | Unit test: verify different `TriggerSource` variants (Bus vs SignalPattern) |
| TL-26 | All domain Cells (section 11) declare `input_schema` and `output_schema` | Compile check: every domain Cell returns `Some` for both schema methods |
| TL-27 | All Connect Cells use standard 6-field table format | Doc review: every Connector in section 9 has all 6 fields |
| TL-28 | All Trigger Cells use standard 6-field table format with TOML examples | Doc review: every trigger in section 10 has all 6 fields + TOML |
| TL-29 | Graph edge validation works for all domain Cell compositions | Integration test: compose domain Cells in synergy pattern Graphs, `roko graph validate` passes |

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 3.1 | 2026-04-26 | Standardized all Observe Cells (section 8) to full 6-field table format with typed Input/Output. Split `signal-pattern-trigger` into `hdc-bus-trigger` (real-time Pulse stream HDC matching) and `signal-pattern-trigger` (Store polling aggregate). Expanded all domain Cells (section 11) with typed I/O schemas and representative Cells with full 6-field tables + TOML examples. Standardized Connect Cells (section 9) and Trigger Cells (section 10) to full 6-field format with TOML examples. Fixed `block =` to `cell =` in all TOML examples. Updated catalog count (46 protocol Cells). Added acceptance criteria TL-21 through TL-29. |
| 3.0 | 2026-04-26 | Unified spec: full catalog by protocol, synergy patterns, implementation tiers. |
| 2.0 | 2026-04-20 | Initial tool catalog. |

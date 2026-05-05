# 26. Agent Lifecycle Type-State

> Agent creation, deletion, and provisioning as a type-state machine. States are compile-time enforced. Three creation flows. Three successor patterns. Deletion is an 8-step Pipeline. Knowledge backup/restore uses confidence decay per generational distance.

See [02-CELL.md](../../unified/02-CELL.md) for Pipeline pattern, [03-GRAPH.md](../../unified/03-GRAPH.md) for Graph definition, [05-AGENT.md](../../unified/05-AGENT.md) for Agent specialization.

---

## 1. Lifecycle as Type-State Machine

The Agent lifecycle is a type-state machine: each state is a distinct Rust type, and transitions are methods that consume the current state and produce the next. This means illegal transitions are **compile-time errors**, not runtime panics.

```rust
/// Agent lifecycle states as distinct types.
/// Transitions consume self and produce the next state.
/// Illegal transitions cannot be expressed in code.
///
/// Crate: `crates/roko-core/src/lifecycle.rs`

pub struct Initializing { manifest: AgentManifest }
pub struct Bootstrapping { config: ResolvedConfig, resources: AllocatedResources }
pub struct Ready { agent: ConfiguredAgent }
pub struct Running { agent: ConfiguredAgent, flows: RunningFlows }
pub struct Draining { agent: ConfiguredAgent, deadline: Instant }
pub struct Terminated { backup: Option<BackupHandle>, reason: TerminationReason }

// Type-state transitions (each consumes self)
impl Initializing {
    pub fn validate(self) -> Result<Bootstrapping, ManifestError> { /* ... */ }
}

impl Bootstrapping {
    pub fn provision(self) -> Result<Ready, ProvisionError> { /* ... */ }
}

impl Ready {
    pub fn start(self) -> Running { /* ... */ }
}

impl Running {
    pub fn drain(self, deadline: Duration) -> Draining { /* ... */ }
    pub fn pause(self) -> Ready { /* ... */ }  // reversible
}

impl Draining {
    pub fn terminate(self) -> Terminated { /* ... */ }
}
```

### 1.1 State Transition Graph

```
Initializing --validate()--> Bootstrapping --provision()--> Ready
                                                              |
                                                         start()
                                                              |
                                                              v
                            Ready <--pause()-- Running --drain()--> Draining
                                                              |                    |
                                                              |              terminate()
                                                              |                    |
                                                              v                    v
                                                         (budget pressure)    Terminated
                                                              |
                                                              v
                                                         Degraded(stage)
                                                              |
                                                         (recover/drain)
```

The Degraded state is not a separate type but a runtime flag on Running:

```rust
impl Running {
    pub fn degrade(&mut self, stage: DegradationStage) {
        self.degradation = Some(stage);
        // Progressively restrict capabilities
        match stage {
            DegradationStage::ModelDowngrade => self.router.force_tier(T1),
            DegradationStage::T0Emphasis => self.router.force_tier(T0),
            DegradationStage::ReducedFrequency => self.clock.extend_intervals(4.0),
            DegradationStage::MonitoringOnly => self.flows.disable_actions(),
            DegradationStage::BudgetPaused => self.flows.pause_all(),
        }
    }
}
```

---

## 2. Three Creation Flows

All creation flows converge on a single artifact: the `AgentManifest`.

### 2.1 Describe -> Review -> Confirm (Interactive)

The user provides natural-language intent. AI autofill generates a complete manifest. The user reviews and confirms.

```
User: "Monitor our staging cluster and alert on anomalies"
  |
  v
AI Autofill (Haiku-class, ~$0.0003)
  |
  v
AgentManifest { prompt, mode, domain, strategy_md, routing, ... }
  |
  v
User reviews, edits, confirms
  |
  v
Initializing { manifest }
```

### 2.2 Template Expansion (Instant)

Five curated templates cover common patterns. No LLM needed.

```
roko init --template rust-coding --param crate_path=crates/roko-core
  |
  v
Template expanded with parameters (deterministic, auditable)
  |
  v
AgentManifest (fully specified)
  |
  v
Initializing { manifest }
```

### 2.3 Config File (Programmatic)

For CI/CD and fleet management. The operator provides a complete `roko.toml`.

```
roko init --config ./production-agent.toml
  |
  v
Config parsed, merged with defaults (CLI > env > TOML > defaults)
  |
  v
AgentManifest (fully specified)
  |
  v
Initializing { manifest }
```

---

## 3. Provisioning Pipeline

Bootstrapping runs an 7-step Pipeline Graph:

```toml
[graph]
id = "provisioning_pipeline"
kind = "pipeline"

[[cells]]
id = "validate_manifest"
protocol = "Verify"
description = "Check manifest against domain feature set"

[[cells]]
id = "allocate_resources"
protocol = "Store"
description = "Reserve compute, memory, network (L0)"

[[cells]]
id = "init_neuro"
protocol = "Store"
description = "Initialize knowledge store with empty tiers"

[[cells]]
id = "configure_routing"
protocol = "Route"
description = "Set up CascadeRouter, tier router, model registry"

[[cells]]
id = "load_tools"
protocol = "Connect"
description = "Load tool profile, discover MCP servers"

[[cells]]
id = "register_mesh"
protocol = "Connect"
description = "Register with Agent Mesh (if enabled)"
condition = "config.mesh.enabled"

[[cells]]
id = "start_flows"
protocol = "Trigger"
description = "Start HeartbeatPolicy, gamma/theta/delta flows"
```

Each Cell in the Pipeline can reject (abort provisioning with error) or pass (proceed to next Cell).

---

## 4. Three Successor Patterns

When an Agent is terminated and its knowledge should be passed on:

### 4.1 Clean Successor

A fresh Agent with zero inherited knowledge. Used when the predecessor's knowledge is irrelevant or contaminated.

```
Terminate predecessor -> Create new Agent -> Start fresh
```

### 4.2 Same Strategy

A new Agent with the same `STRATEGY.md` and restored knowledge. The most common case.

```
Backup predecessor -> Terminate -> Create new Agent (same strategy) -> Restore backup
```

### 4.3 Lineage (Generational Transfer)

Knowledge is passed through a confidence decay pipeline. Each generational hop multiplies confidence by 0.85:

```
Agent A (gen 0): confidence 0.90
  |
  backup + restore to Agent B (gen 1): confidence 0.90 * 0.85 = 0.765
  |
  backup + restore to Agent C (gen 2): confidence 0.765 * 0.85 = 0.650
```

This implements the Weismann barrier (Heard & Martienssen 2014): inherited knowledge is treated with appropriate skepticism. The receiving Agent must independently validate to restore confidence.

---

## 5. Deletion as 8-Step Pipeline

Deletion is not instant destruction. It is an orderly shutdown Pipeline:

```toml
[graph]
id = "deletion_pipeline"
kind = "pipeline"
timeout_secs = 30

[[cells]]
id = "drain_work"
protocol = "React"
description = "Complete current turn, stop accepting new tasks"
timeout_secs = 10

[[cells]]
id = "serialize_knowledge"
protocol = "Store"
description = "Flush all in-memory Signals to Store, snapshot dream state"
timeout_secs = 5

[[cells]]
id = "backup_if_requested"
protocol = "Store"
description = "Create automatic backup if --backup flag"
condition = "config.backup_on_delete"
timeout_secs = 5

[[cells]]
id = "deregister_mesh"
protocol = "Connect"
description = "Send deregistration to Mesh relay, close WebSocket"
timeout_secs = 3

[[cells]]
id = "release_resources"
protocol = "Store"
description = "Close file handles, cancel HTTP requests, release memory"
timeout_secs = 2

[[cells]]
id = "archive_logs"
protocol = "Store"
description = "Flush episodes.jsonl, efficiency.jsonl to disk"
timeout_secs = 3

[[cells]]
id = "notarize"
protocol = "Verify"
description = "Write deletion record with timestamp and reason"
timeout_secs = 1

[[cells]]
id = "confirm"
protocol = "React"
description = "Emit deletion Pulse on Bus, exit process"
timeout_secs = 1
```

If any Cell exceeds its timeout, it is skipped and the Pipeline proceeds. This prevents a hung Mesh connection from blocking shutdown.

---

## 6. Knowledge Backup/Restore with Confidence Decay

### 6.1 Backup Format

A backup is a portable Store snapshot:

```
{agent_id}-{timestamp}.neuro/
  manifest.toml          # Agent metadata, schema version
  signals/               # All Signals (content-addressed by SHA-256)
  scores/                # 7-axis scores per Signal
  tiers/                 # Tier assignments
  provenance/            # Lineage chains
  decay_state/           # Current demurrage state
  playbook.md            # Compiled heuristics
  checksum.blake3        # Archive integrity
```

### 6.2 Confidence Decay on Restore

```rust
/// Apply generational confidence decay during restore.
///
/// Each generational hop multiplies confidence by decay_factor (default: 0.85).
/// This implements the Weismann barrier: inherited knowledge arrives at
/// reduced confidence and must be independently validated.
///
/// Anti-proletarianization: restored knowledge that does not diverge
/// by >= 0.15 from the source is flagged for review. The receiving Agent
/// must develop its own understanding, not merely copy.
pub fn restore_with_decay(
    backup: &Backup,
    target_store: &mut Store,
    generations: u32,
    config: &RestoreConfig,
) -> RestoreReport {
    let decay = config.decay_factor.powi(generations as i32);
    // 0.85^1 = 0.85, 0.85^2 = 0.72, 0.85^3 = 0.61, 0.85^4 = 0.52

    let mut report = RestoreReport::default();

    for signal in &backup.signals {
        let mut restored = signal.clone();

        // Apply confidence decay
        restored.score.confidence *= decay;

        // Quarantine: all restored signals start in quarantine
        restored.status = SignalStatus::Quarantined;

        // Provenance: record restoration lineage
        restored.provenance.push(ProvenanceEntry::Restored {
            from_agent: backup.manifest.agent_id.clone(),
            generations,
            decay_applied: decay,
            timestamp: now(),
        });

        // Filter: skip signals that fall below minimum threshold
        if restored.score.confidence < config.min_confidence_threshold {
            report.filtered += 1;
            continue;
        }

        target_store.put(restored);
        report.restored += 1;
    }

    report
}
```

### 6.3 Quarantine Pipeline

Restored Signals enter quarantine and must be validated before promotion:

```
Quarantined (confidence * 0.85^N)
  |
  v  (first retrieval + use in gamma tick)
Validating (awaiting gate outcome)
  |
  v  (gate passed -> confidence += 0.1)
Adopted (normal decay applies)
  |
  v  (gate failed -> confidence -= 0.2)
Rejected (marked as AntiKnowledge, fast decay)
```

### 6.4 Anti-Proletarianization Check

Restored knowledge must diverge from the source by at least 0.15 (measured via HDC cosine distance between the Agent's new experience and the restored signals). If after 100 ticks the Agent has not developed novel insights (HDC distance < 0.15 from restored corpus), a warning is emitted:

```rust
/// Check whether the Agent is merely copying inherited knowledge
/// or developing its own understanding.
fn proletarianization_check(
    inherited: &[Signal],
    new_signals: &[Signal],
) -> f64 {
    let inherited_bundle = hdc_bundle(inherited);
    let new_bundle = hdc_bundle(new_signals);
    cosine_distance(&inherited_bundle, &new_bundle)
    // Must be >= 0.15 to pass the anti-proletarianization threshold
}
```

---

## 7. Genomic Bottleneck Compression

For compressed backups, the genomic bottleneck (Shuvaev et al. 2024) selects at most 2048 Signals:

| Allocation | Selection Criterion | Percentage |
|---|---|---|
| Safety reserve | All Warning-type + all Persistent-tier | 25% (~512) |
| Diversity sample | Top Signals across all 6 types, diversity-weighted | 50% (~1024) |
| Quality fill | Highest-scored regardless of type | 25% (~512) |

The compression acts as a regularizer: regime-specific overfitting is stripped while generalizable knowledge is preserved.

---

## What This Enables

- **Compile-time safety**: Illegal lifecycle transitions cannot be expressed in code. No "start before provisioned" bugs.
- **Explicit knowledge lineage**: Every restored Signal carries provenance showing exactly where it came from and how much confidence was lost.
- **Graceful degradation**: Budget pressure dims the Agent progressively through well-defined stages, never kills it.
- **Anti-proletarianization**: Agents must develop their own competence, not merely inherit and execute (Stiegler 2010).
- **Clean shutdown guarantees**: The 30-second timeout budget ensures deletion always completes, even with hung subsystems.

## Feedback Loops

1. **Restored confidence -> validation in gamma ticks -> confidence update -> tier promotion** (Loop): Inherited knowledge proves itself through use.
2. **Degradation stage -> reduced cost -> budget recovery -> degradation lifted** (Loop): Budget pressure self-corrects via reduced spending.
3. **HDC divergence check -> proletarianization warning -> Agent explores more -> divergence increases** (Loop): The warning incentivizes the Agent to develop original insights.

## Open Questions

1. Should the decay factor (0.85) be configurable per knowledge type (Warnings might decay less)?
2. Should there be a maximum generational depth beyond which restore is refused?
3. How should partial failures in the deletion Pipeline be reported (which steps failed)?
4. Should the anti-proletarianization check be a hard gate (block further restore) or a soft warning?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define type-state lifecycle types | `crates/roko-core/src/lifecycle.rs` | Not started |
| Implement provisioning Pipeline Graph | `crates/roko-runtime/src/provisioning.rs` | Partial (init command exists) |
| Implement deletion Pipeline Graph | `crates/roko-runtime/src/deletion.rs` | Partial (force_shutdown in orchestrate.rs) |
| Implement confidence decay on restore | `crates/roko-neuro/src/restore.rs` | Not started |
| Implement quarantine pipeline | `crates/roko-neuro/src/quarantine.rs` | Not started |
| Implement genomic bottleneck compression | `crates/roko-neuro/src/backup.rs` | Not started |
| Wire lifecycle states into CLI commands | `crates/roko-cli/src/lib.rs` | Partial (agent start/stop exist) |
| Anti-proletarianization HDC check | `crates/roko-neuro/src/divergence.rs` | Not started |

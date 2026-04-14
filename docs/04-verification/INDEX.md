# 04 — Verification (L3 Harness)

> **Layer**: L3 Harness
> **Crate**: `roko-gate` (`crates/roko-gate/src/`)
> **Generated**: 2026-04-11
> **Source**: Prompt 04-verification

---

## What This Section Covers

Layer 3 (Harness) is Roko's verification layer. It establishes ground truth about
agent-produced artifacts by running gates — deterministic external tools (compilers,
test runners, linters, static analyzers) that produce `Verdict`s. Gate verdicts flow
back into the system as signals, feeding the conductor, the router, the learning loops,
and the agents themselves.

The key design principle: **Gate failure is a verdict, not an error.** The `Gate` trait
returns `Verdict` directly — not `Result<Verdict>`. This means every downstream consumer
receives a definitive answer without error-handling code.

---

## Sub-Documents

| # | File | Topic | Lines |
|---|---|---|---|
| 00 | [00-gate-trait.md](./00-gate-trait.md) | The Gate trait: signature, `-> Verdict` design, `name()`, position in universal loop | 210 |
| 01 | [01-gate-implementations.md](./01-gate-implementations.md) | 11 concrete gates: ShellGate, CompileGate, ClippyGate, TestGate, SymbolGate, DiffGate, and scaffolds | 247 |
| 02 | [02-6-rung-selector.md](./02-6-rung-selector.md) | 7-rung selector: Compile→Lint→Test→Symbol→GeneratedTest→PropertyTest→Integration, PlanComplexity, escalation | 230 |
| 03 | [03-gate-pipeline.md](./03-gate-pipeline.md) | GatePipeline: sequential composition, short-circuit, verdict aggregation, test count merging | 218 |
| 04 | [04-artifact-store.md](./04-artifact-store.md) | ArtifactStore: BLAKE3 content-addressed, append-only, deduplicating artifact storage | 200 |
| 05 | [05-ratcheting.md](./05-ratcheting.md) | GateRatchet: monotonic rung tracking, regression prevention, convergence thrashing protection | 225 |
| 06 | [06-adaptive-thresholds.md](./06-adaptive-thresholds.md) | AdaptiveThresholds: per-rung EMA pass rates, retry budget, skip advisory, persistence | 210 |
| 07 | [07-process-reward-models.md](./07-process-reward-models.md) | Process rewards: Promise + Progress scoring, per-turn intervention, multi-timescale feedback | 220 |
| 08 | [08-agent-feedback-from-gates.md](./08-agent-feedback-from-gates.md) | GateFeedback: line classification, noise filtering, severity buckets, token economy | 225 |
| 09 | [09-evaluation-lifecycle.md](./09-evaluation-lifecycle.md) | 14 feedback loops across 5 speed tiers, four-phase lifecycle, Karpathy property, Gauntlet | 215 |
| 10 | [10-autonomous-eval-generation.md](./10-autonomous-eval-generation.md) | Autonomous test generation: verification pipeline, separation of concerns, cheap-model convergence | 220 |
| 11 | [11-evoskills.md](./11-evoskills.md) | EvoSkills: three-tier learning hierarchy, adversarial surrogate verification, cross-model transfer | 220 |
| 12 | [12-forensic-ai-causal-replay.md](./12-forensic-ai-causal-replay.md) | Forensic AI: content-addressed causal chains, regulatory compliance, gap analysis | 225 |

**Total**: ~2,865 lines across 13 sub-documents plus this index.

---

## Architecture Summary

```
                    RungSelector
                   (complexity + caps + failures)
                         │
                         ▼
                    GatePipeline
                   (sequential, short-circuit)
                         │
              ┌──────────┼──────────┐
              ▼          ▼          ▼
         CompileGate  ClippyGate  TestGate  ...  IntegrationGate
          (Rung 0)    (Rung 1)   (Rung 2)       (Rung 6)
              │          │          │                │
              └──────────┼──────────┘                │
                         ▼                           │
                    Aggregated Verdict ◄─────────────┘
                         │
         ┌───────────────┼───────────────────┐
         ▼               ▼                   ▼
    GateRatchet    AdaptiveThresholds   GateFeedback
   (regression)   (retry budget, skip)  (agent retry)
         │               │                   │
         ▼               ▼                   ▼
    ArtifactStore   EfficencyEvents     ProcessRewards
   (BLAKE3 hash)   (per-turn data)   (Promise+Progress)
         │               │                   │
         └───────────────┼───────────────────┘
                         ▼
                  Evaluation Lifecycle
                (14 loops × 5 speed tiers)
                         │
                    ┌────┼────┐
                    ▼         ▼
              EvoSkills   ForensicReplay
            (skill lib)  (causal chains)
```

---

## Key Design Decisions

### 1. `Gate::verify()` returns `Verdict`, not `Result<Verdict>`

Gate failure is a verdict, not an error. Infrastructure failures (spawn errors,
timeouts, malformed input) are encoded as `Verdict::fail()`. This eliminates error
propagation in the pipeline, ratchet, and all downstream consumers.

**Source**: `crates/roko-core/src/traits.rs:102–108`

### 2. Sequential gate execution with short-circuit

Gates run cheapest-first. The first failure stops the pipeline. This is the primary
optimization that makes the 7-rung system efficient: a 3-second compile failure prevents
a 15-minute test run.

**Source**: `crates/roko-gate/src/gate_pipeline.rs`

### 3. Monotonic ratchet

Once a plan passes rung N, it cannot regress below N. This prevents convergence
thrashing where the agent oscillates between fixing different rungs.

**Source**: `crates/roko-gate/src/ratchet.rs`

### 4. EMA-based adaptive thresholds

Per-rung pass rates tracked via EMA (α=0.1) inform retry budgets and skip advisories.
Persistent to disk for cross-session continuity.

**Source**: `crates/roko-gate/src/adaptive_threshold.rs`

### 5. Separation of test generation from implementation

Different agents generate tests and implement code. This adversarial setup prevents the
implementation agent from generating easy-to-pass tests.

**Source**: Verification-first architecture (bardo-backup reference)

---

## Cross-References

| Topic | See Also |
|---|---|
| Gate trait in the Synapse Architecture | `docs/01-architecture/` (Synapse traits) |
| Gate feedback → agent prompts | `docs/03-scaffold/` (prompt assembly) |
| Gate verdicts → model routing | `docs/05-learning/` (CascadeRouter) |
| Gate verdicts → conductor | `docs/06-conductor/` (circuit breaker, watchers) |
| Orchestrator wiring | `crates/roko-cli/src/orchestrate.rs` |
| Agent dispatch | `crates/roko-agent/src/dispatcher/mod.rs` |
| Episode logging | `.roko/episodes.jsonl` |
| Efficiency events | `.roko/learn/efficiency.jsonl` |
| Gate thresholds | `.roko/learn/gate-thresholds.json` |

---

## Source Material

### Canonical Sources (refactoring-prd/)

| File | Sections Used |
|---|---|
| `01-synapse-architecture.md` | Gate trait signature, cybernetic feedback loops |
| `02-five-layers.md` | Layer 3 Harness definition, process reward models |
| `07-implementation-priorities.md` | Tier 2J prediction tracking |
| `08-translation-guide.md` | Naming map, reframe rules |
| `09-innovations.md` | Forensic AI, EvoSkills |

### Legacy Sources (bardo-backup/)

| File | What It Provided |
|---|---|
| `prd/16-testing/01-gauntlet.md` | Gauntlet benchmark suite |
| `prd/16-testing/07-fast-feedback-loops.md` | 5 fast evaluation loops |
| `prd/16-testing/08-slow-feedback-loops.md` | 3 slow evaluation loops |
| `prd/16-testing/09-evaluation-map.md` | 14-loop composition diagram |
| `tmp/mori-refactor/06-harness.md` | Full harness layer spec, academic foundations |
| `tmp/mori-agents/06-eval-and-scoring.md` | Why LLM-as-Judge fails |
| `tmp/mori-agents/20-verification-first-architecture.md` | 6-rung gate system |
| `tmp/death/16-autonomous-verification.md` | Autonomous test infrastructure |

### Implementation Plans

| File | What It Provided |
|---|---|
| `modelrouting/12-advanced-patterns.md` | Gate-to-scaffold feedback, section effectiveness, predictive foraging, process reward tracking |
| `modelrouting/13-architectural-gaps.md` | Generated test gates (GVU verification) |
| `11-sections/phase-7-8.md` | PRD-driven workflow gate verification |

### Active Code

| File | What It Provided |
|---|---|
| `roko-core/src/traits.rs` | Gate trait definition |
| `roko-gate/src/lib.rs` | Module structure |
| `roko-gate/src/gate_pipeline.rs` | GatePipeline implementation |
| `roko-gate/src/rung_selector.rs` | RungSelector, PlanComplexity, Rung enum |
| `roko-gate/src/ratchet.rs` | GateRatchet implementation |
| `roko-gate/src/artifact_store.rs` | ArtifactStore implementation |
| `roko-gate/src/adaptive_threshold.rs` | AdaptiveThresholds implementation |
| `roko-gate/src/feedback.rs` | GateFeedback implementation |
| `roko-gate/src/compile.rs` | CompileGate implementation |
| `roko-gate/src/test_gate.rs` | TestGate implementation |
| `roko-gate/src/shell.rs` | ShellGate implementation |
| `roko-gate/src/clippy_gate.rs` | ClippyGate implementation |
| `roko-gate/src/diff_gate.rs` | DiffGate implementation |
| `roko-gate/src/symbol_gate.rs` | SymbolGate implementation |

### Academic References

| Citation | Context |
|---|---|
| Song et al. (ICLR 2025) | Generation-Verification-Update framework, Variance Inequality |
| Lightman et al. (2023) | PRM800K, "Let's Verify Step by Step" |
| AgentPRM (arXiv:2502.10325) | Per-step rewards for agent tool use |
| SAGE (arXiv:2512.17102) | Self-acquired generalist expertise, skill libraries |
| Voyager (Wang et al. 2023) | Skill accumulation in LLM agents |
| Self-Refine (Madaan et al. 2023) | Iterative self-improvement with feedback |
| Reflexion (Shinn et al. 2023) | Verbal reinforcement for agents |
| Guo et al. (2017) | Expected calibration error |
| ACON (arXiv:2510.00615) | Context compaction |
| Agent Behavioral Contracts (arXiv:2602.22302) | Formal behavioral specifications |

---

## Naming Map Applied

| Old Term | New Term | Notes |
|---|---|---|
| Bardo | Roko | System name |
| Golem | Agent | Actor entity |
| Mori | Roko Orchestrator | Orchestration subsystem |
| Grimoire | Neuro | Knowledge/memory subsystem |
| Signal | Engram | Used "Signal" in code, "Engram" in docs |
| GNOS | KORAI / DAEJI | Meta-cognitive subsystem |
| Clade | Collective / Mesh | Multi-agent groups |
| Succession | Backup / Restore | No death framing |
| Mortality | Resource Management | No death framing |

---

## Generation Notes

- **Prompt**: 04-verification
- **Context pack**: 8 files read from `tmp/prd-migration/context-pack/`
- **Canonical sources**: 5 files from `refactoring-prd/`
- **Legacy sources**: 11 files from `bardo-backup/`
- **Implementation plans**: 3 files from `tmp/implementation-plans/`
- **Active code**: 14 files from `crates/roko-gate/src/`
- **No death/mortality framing** applied throughout
- **Naming map** applied throughout
- **Gate returns Verdict, not Result<Verdict>** emphasized in docs 00, 01, 03, INDEX

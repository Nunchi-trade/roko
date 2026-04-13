# 07 — Conductor Subsystem

> The Conductor is the agent's theory-of-mind about its own pipeline.
> It observes agent behavior, detects anomalies, and issues graduated
> interventions — not as a timeout manager, but as a cybernetic
> regulator that models the system it governs.

---

## Document Index

| # | File | Topic | Lines |
|---|------|-------|-------|
| 00 | [conductor-architecture.md](00-conductor-architecture.md) | Architecture overview, L3 placement, synapse position | ~250 |
| 01 | [watcher-ensemble.md](01-watcher-ensemble.md) | All 10 watchers with thresholds and detection logic | ~350 |
| 02 | [circuit-breaker.md](02-circuit-breaker.md) | Per-plan breaker, 3-state model, DashMap concurrency | ~280 |
| 03 | [graduated-interventions.md](03-graduated-interventions.md) | Severity→Decision mapping, no-nudge policy | ~300 |
| 04 | [diagnosis-engine.md](04-diagnosis-engine.md) | 34 patterns, 20 categories, intervention suggestions | ~320 |
| 05 | [stuck-detection.md](05-stuck-detection.md) | 6 heuristics, MetaCognitionHook, Theta frequency | ~300 |
| 06 | [health-monitors.md](06-health-monitors.md) | SystemSnapshot, 4 checks, VSM System 3* | ~250 |
| 07 | [ooda-cybernetic-loop.md](07-ooda-cybernetic-loop.md) | OODA mapping, cybernetic structure, feedback properties | ~280 |
| 08 | [good-regulator-self-model.md](08-good-regulator-self-model.md) | Conant-Ashby theorem, self-model components, adaptive vs static | ~300 |
| 09 | [cognitive-signals.md](09-cognitive-signals.md) | 8 typed interrupts, signal semantics, implementation path | ~260 |
| 10 | [adaptive-timeouts-state-machine.md](10-adaptive-timeouts-state-machine.md) | Phase timeouts, complexity bands, graceful shutdown | ~300 |
| 11 | [anomaly-detection-learning.md](11-anomaly-detection-learning.md) | EWMA, prompt loops, learning integration, feedback loops | ~360 |
| 12 | [yerkes-dodson-pressure.md](12-yerkes-dodson-pressure.md) | Inverted-U curve, pressure tuning, cooperation metrics | ~310 |
| 13 | [process-supervision-wiring.md](13-process-supervision-wiring.md) | ProcessSupervisor integration, PID tracking, orphan cleanup | ~310 |
| 14 | [production-failure-catalog.md](14-production-failure-catalog.md) | 21 production failures mapped to conductor responses | ~360 |
| 15 | [conductor-learning-federation.md](15-conductor-learning-federation.md) | Learned policies, federated control, self-healing | ~400 |

---

## Reading Order

### Quick Start (Understand the Conductor in 3 docs)

1. **00-conductor-architecture.md** — What the Conductor is, where
   it sits, what it does
2. **01-watcher-ensemble.md** — The 10 watchers that produce signals
3. **03-graduated-interventions.md** — How signals become decisions

### Full Understanding (Add theory and mechanisms)

4. **07-ooda-cybernetic-loop.md** — The cybernetic theory behind
   the design
5. **08-good-regulator-self-model.md** — Why the Conductor must
   model itself
6. **02-circuit-breaker.md** — Per-plan failure tracking
7. **04-diagnosis-engine.md** — Error classification and auto-fix
8. **05-stuck-detection.md** — Agent progress monitoring
9. **06-health-monitors.md** — System-level health checks

### Advanced Topics (Adaptive behavior and learning)

10. **09-cognitive-signals.md** — Future typed interrupt system
11. **10-adaptive-timeouts-state-machine.md** — Phase lifecycle
12. **11-anomaly-detection-learning.md** — Statistical anomaly
    detection and feedback loops
13. **12-yerkes-dodson-pressure.md** — Pressure dynamics and
    cooperation curves

### Operational (Production and infrastructure)

14. **13-process-supervision-wiring.md** — OS-level process management
15. **14-production-failure-catalog.md** — Every known failure and
    its conductor response

### Frontier (Adaptive and self-improving conductor)

16. **15-conductor-learning-federation.md** — Learned intervention
    policies, federated multi-level control, self-healing conductor

---

## Key Concepts

### The Conductor Is Not a Timeout Manager

The Conductor is a **cybernetic regulator** (Wiener, 1948) — it
implements a closed-loop control system where agent behavior is
observed, compared against expectations, and corrected through
graduated interventions. Timeouts are one mechanism among many.
The theoretical foundation is the Good Regulator Theorem (Conant &
Ashby, 1970): any effective regulator must contain a model of the
system it regulates.

### Decide, Don't Nudge

The Conductor issues **decisions** (Continue, Restart, Fail), not
suggestions. This design reflects Yerkes-Dodson pressure dynamics:
ambiguous nudges add unpredictable cognitive load to agents, while
clear decisions produce predictable outcomes. Three actions is
sufficient variety for effective regulation (Ashby's Law of
Requisite Variety, 1956).

### The Policy Trait

Every watcher implements the same trait:

```rust
pub trait Policy {
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal>;
    fn name(&self) -> &str;
}
```

The Conductor is a composite Policy that runs all 10 watchers,
collects their outputs, and applies an intervention policy to
determine the final decision. This uniform interface means watchers
are independent, composable, and testable in isolation.

### Graduated Severity

| Severity | Decision | Meaning |
|----------|----------|---------|
| Info | Continue | Monitor, no action |
| Warning | Restart | Reset agent with fresh context |
| Critical | Fail | Abort the plan |

Severity escalation is monotonic within a plan — once a watcher
fires at Warning, the system's suspicion of that plan is permanently
elevated.

---

## Cross-Cutting Themes

### Theme: Anticipate, Don't React

Multiple subsystems implement early detection:
- **Anomaly detector** checks prompt BEFORE the turn (doc 11)
- **Phase timeouts** fire at 80% to leave cleanup margin (doc 10)
- **Context pressure** warns before the window fills (doc 01)
- **Cost tracking** uses EWMA to detect spikes early (doc 11)

### Theme: Data Produces Better Interventions

Every intervention is a data point for the learning system:
- Conductor interventions → negative routing signals (doc 11)
- Efficiency events → cascade router training (doc 11)
- Gate outcomes → adaptive threshold tuning (doc 11)
- Cooperation metrics → pressure calibration (doc 12)
- Intervention outcomes → conductor bandit training (doc 15)

### Theme: Hierarchical Control

The conductor operates at multiple timescales simultaneously:
- **Gamma** (per-turn): 10 watchers + anomaly detector (docs 01, 11)
- **Theta** (per-task): MetaCognitionHook + stuck detection (docs 05, 07)
- **Delta** (per-batch): cascade router + threshold adaptation (docs 11, 15)
- Slower loops set parameters for faster loops (doc 07, nested OODA)
- Each level implements the Policy trait independently (doc 15)

### Theme: Multiple Levels of Protection

Protection operates at three levels simultaneously:
- **OS level**: ProcessSupervisor manages PIDs, kills, cleanup (doc 13)
- **Plan level**: Circuit breaker tracks plan failures (doc 02)
- **API level**: Provider health breaker tracks provider errors (doc 11)

### Theme: Production-Derived Design

Every threshold and mechanism traces to a real failure:
- MAX_GHOST_TURNS=3 → Issue #9 (ghost turns)
- MAX_COMPILE_FAILS=3 → Issue #14 (stale references)
- MAX_PLAN_FAILURES=2 → Issues #3, #16 (divergence, rebase)
- Context pressure at 80% → Issue #12 (large prompts)
- Full catalog in doc 14 (21 failures, 6 categories)

---

## Theoretical Foundations

| Theory | Author | Year | Applied In |
|--------|--------|------|-----------|
| Cybernetics | Wiener | 1948 | OODA loop structure (doc 07) |
| Law of Requisite Variety | Ashby | 1956 | 3-action decision space (doc 03) |
| Good Regulator Theorem | Conant & Ashby | 1970 | Self-model design (doc 08) |
| Viable System Model | Beer | 1972 | System 3 / System 3* mapping (docs 06, 07) |
| OODA Loop | Boyd | — | Observe-Orient-Decide-Act cycle (doc 07) |
| Yerkes-Dodson Law | Yerkes & Dodson | 1908 | Pressure dynamics (doc 12) |
| Stigmergy | Grassé | 1959 | Indirect coordination (doc 12) |
| Self-Improvement Convergence | Song et al. | ICLR 2025 | Verifier exceeds generator (doc 08) |
| Internal Model Principle | Francis & Wonham | 1976 | Forward prediction, self-model learning (doc 08) |
| Cognitive Load Theory | Sweller | 1988 | Intrinsic/extraneous/germane load mapping (doc 12) |
| Flow State | Csikszentmihalyi | 1975 | Challenge-skill balance, flow detection (doc 12) |
| Complex Event Processing | Luckham | 2002 | Watcher composition patterns (doc 01) |
| Isolation Forest | Liu et al. | 2008 | Streaming anomaly detection (doc 01) |
| Dempster-Shafer Theory | Dempster/Shafer | 1967/76 | Watcher fusion under uncertainty (doc 01) |
| Recovery-Oriented Computing | Patterson et al. | 2002 | Self-healing, micro-reboots (doc 15) |
| Active Inference | Friston | 2010 | Precision-weighted model updates (doc 08) |

---

## Source Code References

| File | What |
|------|------|
| `crates/roko-conductor/src/lib.rs` | Module exports |
| `crates/roko-conductor/src/conductor.rs` | Conductor struct, evaluate() |
| `crates/roko-conductor/src/circuit_breaker.rs` | PlanCircuitBreaker, DashMap |
| `crates/roko-conductor/src/interventions.rs` | ConductorDecision, severity, policies |
| `crates/roko-conductor/src/diagnosis.rs` | DiagnosisEngine, 34 patterns |
| `crates/roko-conductor/src/health.rs` | SystemSnapshot, HealthStatus |
| `crates/roko-conductor/src/state_machine.rs` | PhaseTimeout, ComplexityBand |
| `crates/roko-conductor/src/stuck_detection.rs` | StuckDetector, MetaCognitionHook |
| `crates/roko-conductor/src/watchers/` | All 10 watcher implementations |
| `crates/roko-learn/src/anomaly.rs` | AnomalyDetector, EWMA |
| `crates/roko-learn/src/efficiency.rs` | AgentEfficiencyEvent |
| `crates/roko-learn/src/cascade_router.rs` | Cascade router |
| `crates/roko-learn/src/provider_health.rs` | Provider health tracker |
| `crates/roko-gate/src/adaptive_threshold.rs` | Adaptive gate thresholds |
| `crates/bardo-runtime/` | ProcessSupervisor |
| `crates/roko-learn/src/conductor.rs` | ConductorBandit (learned intervention policy) |

---

## Generation Notes

- **Source material**: 19 roko-conductor source files, 7 refactoring
  PRD documents, 5 implementation plan files, 3 legacy reference docs
- **Naming**: Roko naming conventions applied throughout (Bardo→Roko,
  Golem→Agent, Mori→Roko Orchestrator, Grimoire→Neuro, Styx→Agent Mesh, Clade→Collective)
- **Citations**: All academic references preserved with full
  attribution
- **Framing**: Conductor as cybernetic regulator and theory-of-mind,
  not timeout manager

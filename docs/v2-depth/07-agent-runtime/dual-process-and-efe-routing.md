# Dual-Process Cognition and EFE-Driven Routing

> Depth for [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md). Maps dual-process theory and active inference onto the Cell/Graph execution model — how agents select inference tiers and route between cheap/expensive strategies using Expected Free Energy.

## The Three Inference Tiers

The unified model replaces the classical "System 1 / System 2" dichotomy with three tiers that map directly to Route protocol candidates:

| Tier | Name | What Runs | Latency | Cost | When |
|---|---|---|---|---|---|
| T0 | Reflex | Pattern-match Cells (no LLM) | <50ms | ~$0 | Known patterns, cache hits, heuristic matches |
| T1 | Fast | Small/cached model Cells | 1-5s | $0.001-0.01 | Moderate complexity, familiar domains |
| T2 | Deep | Full model Cells (Opus, o3) | 10-120s | $0.01-1.00 | Novel problems, high-stakes decisions |

### T0 Probes (16 Reflex Cells)

These fire before any LLM call. Each is a Cell implementing the Route protocol that returns `Selected` if confident, `Escalate` otherwise:

1. **ExactCacheProbe** — content-hash match in Store
2. **HDCSimilarityProbe** — nearest-neighbor in HDC space (radius < 0.15)
3. **HeuristicMatchProbe** — `when` predicates from active Heuristic Signals
4. **PlaybookProbe** — playbook rule pattern match
5. **SchemaValidationProbe** — input validates against known schema
6. **RegexClassifierProbe** — lightweight regex-based task classification
7. **CostGuardProbe** — reject if estimated cost exceeds remaining budget
8. **RateLimitProbe** — throttle if tier capacity exhausted
9. **SafetyVetoProbe** — block if safety contract prohibits
10. **DuplicateDetectorProbe** — content-hash dedup within session
11. **SomaticMarkerProbe** — strong negative valence from similar past episodes
12. **VitalityGuardProbe** — restrict tier based on behavioral phase
13. **RegimeProbe** — crisis regime forces T2, calm regime prefers T0
14. **ContextWindowProbe** — estimate token requirement, skip if > model max
15. **ToolRequirementProbe** — if task requires specific tool, route to tier that has it
16. **TemporalDeadlineProbe** — if deadline < T2 latency estimate, force T0/T1

### Expected Free Energy as the Routing Principle

EFE subsumes UCB/epsilon-greedy by decomposing the value of each candidate into three terms:

```
EFE(candidate) = -pragmatic_value - epistemic_value + cost_term
```

Where:
- **Pragmatic value** = expected reward (from Verify verdicts in similar contexts)
- **Epistemic value** = information gain from trying this candidate (high for under-explored routes)
- **Cost term** = estimated resource consumption (tokens × price + latency × urgency)

The Route Cell selects the candidate with the **lowest** EFE (most negative = best).

### Why EFE > LinUCB

| Property | LinUCB | EFE |
|---|---|---|
| Exploration | Bonus from confidence interval width | Explicit epistemic term (KL divergence) |
| Context | Linear features only | Full context via CorticalState |
| Regime awareness | None (manual override) | Regime → prior shifts (crisis amplifies pragmatic) |
| Cost-awareness | None | Explicit cost term in objective |
| Composability | Standalone | EFE is additive across cascaded decisions |

### Progressive Cascade Emergence

EFE naturally produces cascading behavior without hard-coded rules:

1. **Novel task**: High epistemic value for T2 (never tried) → T2 selected → learns
2. **Familiar task**: T0 reflex has strong pragmatic record → T0 selected → fast
3. **Moderate task**: T1 has some history, T2 expensive → T1 selected → balanced

The cascade "learns itself" through the predict-publish-correct loop on Bus:
- `prediction.route.{cell_id}` published before selection
- `outcome.route.{cell_id}` published after Verify verdict
- CalibrationReact joins and updates per-candidate EFE priors

## Affect-Modulated Routing

The SomaticState (PAD vector) modulates EFE computation:

```rust
fn modulated_efe(base_efe: f64, pad: &PadVector, regime: Regime) -> f64 {
    let risk_aversion = 1.0 + (1.0 - pad.dominance).max(0.0) * 0.5;
    let urgency_boost = pad.arousal.max(0.0) * 0.3;
    let exploration_damping = if pad.pleasure < -0.3 { 0.5 } else { 1.0 };

    let pragmatic = base_efe.pragmatic * risk_aversion;
    let epistemic = base_efe.epistemic * exploration_damping;
    let cost = base_efe.cost * (1.0 + urgency_boost);

    -pragmatic - epistemic + cost
}
```

Low dominance → more risk-averse (prefer known routes).
Low pleasure → dampen exploration (stick with what works).
High arousal → amplify cost sensitivity (urgency increases).

## Cognitive Architecture References

The three-tier model draws from:

- **ACT-R** (Anderson 2007): Declarative/procedural memory → Store/Heuristic duality
- **SOAR** (Laird 2012): Impasse-driven elaboration → T0 failure triggers T1 escalation
- **CLARION** (Sun 2002): Implicit/explicit processing → T0 reflex (implicit) vs T2 reasoning (explicit)
- **Global Workspace Theory** (Baars 1988): Broadcast on Bus = global workspace; CognitiveWorkspace VCG = competitive access

The key insight from active inference (Friston 2006): agents minimize prediction error across ALL levels simultaneously. Each tier doesn't just solve the task — it reduces the system's overall free energy (uncertainty). This is why EFE works as a unified routing metric.

## What This Enables

- **Zero-configuration routing**: Agents learn which tier to use per-context without manual rules
- **Affect-aware escalation**: Stress/frustration automatically shifts toward conservative strategies
- **Cost optimization**: EFE naturally prefers cheap tiers when they're good enough
- **Emergent specialization**: Over time, agents develop tier preferences per domain

## Feedback Loops

- **Predict-publish-correct on routing**: Every route decision publishes a prediction on Bus; outcomes update EFE priors
- **Somatic markers from routing outcomes**: Gate verdicts after each tier create somatic markers that modulate future routing
- **Tier utilization Lens**: Observe protocol tracks tier selection frequency → surfaces in TUI/HTTP

## Open Questions

- Should EFE priors be shared across agents (collective routing knowledge) or per-agent?
- How to handle tier degradation (model outage) — should EFE cost term go to infinity or should there be an explicit circuit breaker?
- Can we use HDC similarity between task descriptions to warm-start EFE priors for novel task types?

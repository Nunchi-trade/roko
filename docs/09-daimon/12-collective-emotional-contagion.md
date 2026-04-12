# Collective Emotional Contagion

> How emotional state propagates across the agent mesh with exponential decay, somatic field formation, and anti-cascade safeguards.

**Topic**: [Daimon](./INDEX.md)
**Prerequisites**: [01-pad-vector.md](./01-pad-vector.md), [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)
**Key sources**: `bardo-backup/prd/03-daimon/07-runtime-daimon.md` §5, `refactoring-prd/03-cognitive-subsystems.md` §2, `refactoring-prd/09-innovations.md` §III

---

## Abstract

When agents share a mesh (collective), their emotional states interact. An agent that discovers a critical issue (high arousal) influences mesh peers to raise their own vigilance. An agent experiencing sustained success (high pleasure) influences peers toward confidence. This emotional contagion is not metaphorical — it is a concrete data flow where PAD deltas propagate between agents with specific attenuation factors, caps, and decay rates.

The contagion mechanism is carefully constrained to prevent cascading panic or cascading overconfidence. Pleasure and arousal attenuate to 30% of the source intensity. Dominance does not propagate at all — an agent's sense of control is always locally determined. Arousal is capped at +0.3 per sync cycle. Propagation is unidirectional (no reciprocal feedback). Borrowed emotions decay with a 6-hour half-life unless reinforced by the agent's own experience.

---

## Contagion Rules

### Attenuation Factors

| Dimension | Attenuation | Rationale |
|---|---|---|
| **Pleasure** | 0.3 (30%) | A mesh peer's success is informative but not as significant as your own success. 30% preserves the signal while preventing emotional overreaction to others' outcomes. |
| **Arousal** | 0.3 (30%) | A peer's urgency should increase your vigilance but not overwhelm your own assessment. 30% creates awareness without panic. |
| **Dominance** | 0.0 (no propagation) | Control perception is strictly local. Knowing that a peer feels confident doesn't make *you* more capable. Dominance must be earned through your own outcomes. |

### Cap per Sync Cycle

| Parameter | Value | Rationale |
|---|---|---|
| Arousal cap per sync | +0.3 | Even if multiple peers send alarming data simultaneously, the receiving agent's arousal increase is bounded. Prevents cascading panic where N agents all alarm each other to maximum arousal. |

### Decay

| Parameter | Value | Rationale |
|---|---|---|
| Contagion decay half-life | 6 hours | Borrowed emotions dissipate quickly unless reinforced by the agent's own experience. After 12 hours without reinforcement, contagion effects are at 25% of original strength (two half-lives). |

### Propagation Direction

| Property | Value | Rationale |
|---|---|---|
| Direction | Unidirectional | Agent A's contagion to Agent B does NOT cause Agent B to emit contagion back to Agent A. This prevents positive feedback loops where two agents continuously amplify each other's emotional state. |

---

## Contagion Triggers

Emotional contagion is not continuous — it fires on specific events that one agent shares with the mesh:

| Trigger | Emotional Effect on Receiver | Source |
|---|---|---|
| **Peer warning push** | Arousal +0.1 (capped) | Agent shares a Warning-type Engram with the mesh |
| **Peer alert** (critical issue) | Arousal +0.1 (capped) | Agent detects a significant anomaly |
| **Peer sustained success** | Dominance +0.05 | Agent reports consistently high task success rate |
| **Peer sustained failure** | Pleasure -0.05 | Agent reports consistently low task success rate |
| **Peer dream insight** | Arousal +0.05 | Agent's dream cycle produces a validated insight |

### Warning Push Example

When Agent A discovers a critical issue and pushes a Warning to the mesh:

```rust
// Agent A: discovers critical issue
let warning = Engram::new(
    Kind::Warning,
    "roko-gate compile check failing on stable channel due to MSRV bump",
);
mesh.push_warning(warning);  // shared with mesh peers

// Agent B: receives the warning
// Contagion trigger fires:
agent_b.daimon.apply_contagion(ContagionEvent {
    source: agent_a.id,
    trigger: ContagionTrigger::WarningPush,
    source_pad: agent_a.daimon.query().pad,
});
```

### Contagion Application

```rust
impl DaimonState {
    pub fn apply_contagion(&mut self, event: ContagionEvent) {
        let source = event.source_pad;

        // Attenuate: P and A at 30%, D at 0%
        let p_delta = source.pleasure * 0.3;
        let a_delta = source.arousal * 0.3;
        // D never propagates

        // Cap arousal at +0.3 per sync cycle
        let a_capped = a_delta.min(0.3);

        let now = Utc::now();
        self.state.apply_delta(p_delta, a_capped, 0.0, 0.0, now);

        // Mark this delta as borrowed (subject to accelerated decay)
        self.borrowed_affect.push(BorrowedAffect {
            source: event.source,
            p_delta,
            a_delta: a_capped,
            applied_at: now,
        });
    }
}
```

---

## Anti-Cascade Design

### The Cascade Problem

Without safeguards, emotional contagion produces cascading amplification:

```
Agent A detects anomaly → arousal spike
  → Shares with mesh
    → Agent B receives contagion → arousal increases
      → Agent B shares its heightened state
        → Agent A receives contagion from B → arousal increases further
          → (positive feedback loop → all agents at maximum arousal)
```

This is analogous to financial panic propagation, where fear of a bank run causes a bank run. The contagion mechanism must prevent this while preserving the useful signal (legitimate anomalies should raise collective vigilance).

### Three Anti-Cascade Mechanisms

**Mechanism 1: Cap per sync cycle (+0.3 arousal max)**

Even if every peer in the mesh sends an alarm simultaneously, the receiving agent's arousal increases by at most 0.3 per sync cycle. With a typical sync interval of 30 seconds, the maximum arousal ramp rate is 0.3/30s = 0.6/min. Combined with the 4-hour decay half-life, this limits the steady-state arousal contribution from contagion to approximately 0.4 (the level where decay rate equals accumulation rate).

**Mechanism 2: Unidirectional propagation**

Contagion flows one way: source → receiver. The receiver does not re-emit the borrowed emotional state. This breaks the feedback loop at the first hop — Agent A's alarm reaches Agent B, but Agent B's response to that alarm does not reach Agent A.

If Agent B independently discovers the same anomaly, it will generate its own alarm through normal appraisal. This is desirable — it means the collective arousal reflects genuinely redundant detection, not echo amplification.

**Mechanism 3: Rapid decay (6h half-life)**

Borrowed emotions decay at a 6-hour half-life, which is faster than the standard 4-hour half-life applied to the agent's own emotional state. Wait — the borrowed decay is actually *slower* than self-generated state decay (6h vs. 4h). This is because borrowed emotions represent information from peers and should persist long enough to be acted on, but not so long that they dominate the agent's own experience.

After 6 hours without reinforcement:
```
t=0h:   borrowed arousal = 0.30
t=6h:   borrowed arousal = 0.15  (one half-life)
t=12h:  borrowed arousal = 0.075 (two half-lives)
t=18h:  borrowed arousal = 0.037 (three half-lives — negligible)
```

If the agent's own experience confirms the peer's alarm (the agent also detects the anomaly), the arousal is reinforced through the normal appraisal pipeline and no longer depends on the borrowed component.

---

## Somatic Field Formation

### From Individual Markers to Collective Landscape

When multiple agents share a mesh, their individual somatic landscapes can be aggregated into a **somatic field** — a collective emotional memory over the shared strategy space. Each agent contributes its somatic markers to the field, creating a higher-resolution map of which strategies work and which don't.

```rust
pub struct SomaticField {
    /// Merged k-d tree from all mesh members.
    landscape: SomaticLandscape,
    /// Contribution weights per agent.
    agent_weights: HashMap<AgentId, f64>,
}

impl SomaticField {
    /// Merge an agent's somatic landscape into the collective field.
    pub fn merge(&mut self, agent_id: AgentId, markers: &[SomaticMarker]) {
        let weight = self.agent_weights.get(&agent_id).copied().unwrap_or(1.0);

        for marker in markers {
            let weighted_marker = SomaticMarker {
                strategy_coords: marker.strategy_coords,
                valence: marker.valence * weight,
                intensity: marker.intensity * weight,
                episodes: marker.episodes.clone(),
            };
            self.landscape.tree.add(&marker.strategy_coords, weighted_marker);
        }
    }
}
```

### Weight Calibration

Agent contribution weights in the somatic field are calibrated by historical accuracy. An agent whose somatic markers correlate with actual outcomes gets higher weight; an agent whose markers are unreliable gets lower weight:

```
weight = (correct_predictions / total_predictions) × seniority_factor
```

This prevents a single unreliable agent from corrupting the collective emotional memory while allowing experienced, accurate agents to contribute proportionally more.

### Privacy Boundary

Somatic markers shared with the mesh contain only:
- Strategy coordinates (8D vector)
- Valence (positive/negative)
- Intensity (strength)

They do **not** contain:
- Episode content (what actually happened)
- Task details (what the agent was doing)
- PAD vector (the agent's full emotional state)

This preserves operational privacy while sharing the emotional signal. An agent can query the somatic field and learn "strategies in this region tend to go badly" without learning the specific failures that generated the markers.

---

## C-Factor Integration

The C-Factor (Collective Intelligence Factor, from the collective intelligence literature) measures how well the mesh performs as a group beyond the sum of individual capabilities. Emotional contagion contributes to the C-Factor through two channels:

1. **Collective vigilance**: When one agent raises an alarm, the entire mesh becomes more vigilant. This means the collective detects issues faster than any individual agent, even if only one agent directly encounters the anomaly.

2. **Strategy diversification**: The somatic field enables agents to avoid strategies that peers have found unsuccessful. This reduces redundant failure — agents explore different regions of strategy space rather than all attempting the same approach.

The C-Factor is not yet implemented, but emotional contagion is one of the mechanisms that would contribute to its measurement.

---

## Observability

### Metrics

The Daimon exports contagion-related metrics:

| Metric | Type | Description |
|---|---|---|
| `roko_daimon_contagion_received_total` | Counter | Number of contagion events received |
| `roko_daimon_contagion_arousal_cap_hits` | Counter | Times the arousal cap was reached |
| `roko_daimon_borrowed_affect_active` | Gauge | Current count of active borrowed affect entries |

### Alerts

| Alert | Condition | Severity | Response |
|---|---|---|---|
| Contagion saturation | Arousal cap hit 3+ times in 24h | Info | Review mesh peer alert frequency |
| Collective anxiety | >50% of mesh members in Struggling state | Warning | Check for systemic issue |
| Contagion isolation | No contagion received in 48h | Info | Check mesh connectivity |

---

## Current Status and Gaps

**Specified**: Full contagion rules in `07-runtime-daimon.md` §5. Attenuation factors, caps, decay, and anti-cascade mechanisms. Somatic field concept in `09-innovations.md` §III.

**Not implemented**: No contagion code exists in `roko-daimon` or `roko-golem`. The agent mesh (Korai) is not yet built. Somatic field aggregation is not implemented. C-Factor measurement is not implemented.

**Dependency**: Requires the agent mesh infrastructure (topic [04-knowledge](../04-knowledge/INDEX.md)) for inter-agent communication. Requires the somatic landscape (see [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md)) for somatic field formation.

---

## Academic Foundations

- Hatfield, E., Cacioppo, J.T., & Rapson, R.L. (1993). "Emotional contagion." *Current Directions in Psychological Science*, 2(3), 96–100.
- Bower, G.H. (1981). "Mood and Memory." *American Psychologist*, 36(2), 129–148.
- Woolley, A.W. et al. (2010). "Evidence for a Collective Intelligence Factor in the Performance of Human Groups." *Science*, 330(6004), 686–688.
- Grassé, P.P. (1959). "La reconstruction du nid et les coordinations interindividuelles chez Bellicositermes natalensis et Cubitermes sp." *Insectes Sociaux*, 6(1), 41–80.

---

## Cross-references

- See [01-pad-vector.md](./01-pad-vector.md) for PAD vector structure
- See [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) for individual somatic landscape
- See [10-integration-points.md](./10-integration-points.md) for Daimon integration points
- See topic [04-knowledge](../04-knowledge/INDEX.md) for mesh infrastructure and Engram sharing

# Collective Contagion

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How emotional contagion between agents emerges from Bus subscription at Group boundaries with attenuation, rather than a dedicated contagion protocol.

---

## 1. The Problem: Emotional Isolation vs. Emotional Avalanche

Agents in a Group face a coordination dilemma. If each agent's affect state is entirely private, the group loses a valuable signal: when one agent discovers that a strategy region is dangerous (high arousal, negative pleasure after a gate failure), peer agents working in the same region have no way to learn from that experience until the knowledge is formally consolidated and shared. They repeat the same mistake.

But unrestricted emotional propagation is worse. If agent A's stress directly overwrites agent B's confidence, a single failure cascades through the group, creating synchronized panic. Every agent simultaneously shifts to Conservative routing, throughput collapses, and the group enters a deadlock of caution.

The solution is attenuation at the Group boundary. Emotional signals propagate across Space boundaries through the Bus, but a Functor at the relay point dampens them. The intensity is reduced enough to inform without overwhelming. No new protocol is needed -- contagion is Bus subscription with a Functor applied at the Group's relay.

---

## 2. Contagion IS Bus Subscription Across Space Boundaries

Every agent owns a Space (Bus partition + Store partition). When agents join a Group (see [10-GROUPS.md](../../unified/10-GROUPS.md)), their Spaces are connected through the Group's relay room. The Bus already supports cross-Space message delivery -- the relay room `group:{id}` fans out Pulses to all members.

Emotional contagion adds one new Pulse type to this existing infrastructure:

```rust
/// PAD affect Pulse published to the Group's Bus relay room.
/// Emitted by an agent when its affect state changes significantly.
///
/// This is a standard Pulse -- it uses the same Bus envelope format
/// as all other group events (see 10-GROUPS.md SS8).
pub struct AffectPulse {
    /// Source agent's current PAD vector.
    pub pad: PadVector,

    /// Behavioral state classification.
    pub behavioral_state: BehavioralState,

    /// The trigger that caused the affect change.
    /// One of: gate_pass, gate_fail, task_complete, task_timeout,
    /// sustained_success, sustained_failure, dream_insight.
    pub trigger: ContagionTrigger,

    /// Source agent's historical accuracy in this Group's domain.
    /// Used for accuracy-weighted somatic field merging.
    pub source_accuracy: f64,
}
```

The Pulse is published on the agent's local Bus partition at topic `agent:{id}.affect.changed`. The Group's relay mechanism (already wired for pheromones, knowledge, and coordination -- see [10-GROUPS.md](../../unified/10-GROUPS.md) SS3) picks up Pulses that match the `affect.changed` pattern and relays them to `group:{id}:affect`.

No new subscription mechanism. No new relay protocol. The Bus already does this for pheromone deposits and coordination messages. Affect contagion rides the same rails.

---

## 3. Attenuation IS a Functor at the Group's Bus Relay

The attenuation Functor sits at the relay boundary between the source agent's Space and the Group's Bus partition. It transforms the `AffectPulse` before delivery to peer agents, dampening each PAD dimension by a fixed coefficient.

### Attenuation Coefficients

| PAD Dimension | Attenuation | Rationale |
|---|---|---|
| Pleasure | x 0.3 | Moderate propagation. Peer success/failure informs but does not dominate. |
| Arousal | x 0.3 | Moderate propagation. Urgency spreads at 30% strength -- enough to raise alertness, not enough to cause panic. |
| Dominance | x 0.0 | Zero propagation. Dominance is an agent's self-assessment of its own control over its situation. Hearing that a peer feels in control tells you nothing about your own situation. |

```rust
/// Functor applied at the Group's Bus relay point.
/// Transforms AffectPulses before delivering them to peer agents.
///
/// This is not a separate infrastructure component. It is an entry
/// in the Group's relay configuration: a filter function applied
/// to Pulses matching the "affect.changed" topic before fan-out.
pub struct AttenuationFunctor {
    /// Per-dimension attenuation coefficients.
    pleasure_coeff: f64,  // default 0.3
    arousal_coeff: f64,   // default 0.3
    dominance_coeff: f64, // default 0.0

    /// Maximum arousal delta per sync cycle.
    /// Prevents runaway arousal even at 30% attenuation.
    arousal_cap: f64,     // default 0.3
}

impl AttenuationFunctor {
    /// Transform an AffectPulse for relay to peer agents.
    pub fn attenuate(&self, pulse: &AffectPulse) -> AffectPulse {
        AffectPulse {
            pad: PadVector::new(
                pulse.pad.pleasure * self.pleasure_coeff,
                (pulse.pad.arousal * self.arousal_coeff).clamp(-self.arousal_cap, self.arousal_cap),
                pulse.pad.dominance * self.dominance_coeff,
            ),
            behavioral_state: pulse.behavioral_state,
            trigger: pulse.trigger.clone(),
            source_accuracy: pulse.source_accuracy,
        }
    }
}
```

### Where the Functor Runs

The attenuation Functor is registered as a relay filter on the Group's Bus partition. When a Pulse matching `agent:*.affect.changed` enters the Group relay room, the filter applies `attenuate()` before fan-out. Peer agents receive the dampened Pulse, never the raw one.

This is the same mechanism that the Group uses for pheromone decay rate modification (see [10-GROUPS.md](../../unified/10-GROUPS.md) SS3.1 -- the `pheromone_decay_rate` modifies signal behavior at the Group boundary). The pattern is: Group relay applies a Functor at the boundary. Different Functors for different Pulse types.

### Configuration

```toml
[[groups]]
name = "code-team"
coordination = "stigmergic"

# Affect contagion settings (optional, these are defaults).
[groups.contagion]
pleasure_attenuation = 0.3
arousal_attenuation = 0.3
dominance_attenuation = 0.0
arousal_cap = 0.3
decay_half_life_hours = 6.0
enabled = true
```

Setting `enabled = false` disables affect relay entirely. The Group still functions -- pheromones, knowledge, and coordination are unaffected. Contagion is an optional enrichment, not a structural dependency.

---

## 4. Trigger Types and Propagation Deltas

Not all affect changes propagate equally. The trigger type determines the PAD delta applied to the raw pulse before attenuation. These are the events that generate `AffectPulse` messages:

| Trigger | Raw PAD delta | Propagation semantics |
|---|---|---|
| `peer_warning` | A +0.1 | A peer's arousal bump: "something unexpected happened." Mild alertness increase. |
| `peer_alert` | A +0.1 | Equivalent to warning but from a different trigger source (e.g., gate failure). |
| `sustained_success` | D +0.05 | Peer has succeeded on 3+ consecutive tasks. Slight dominance boost to the group. |
| `sustained_failure` | P -0.05 | Peer has failed on 3+ consecutive tasks. Slight pleasure reduction across the group. |
| `dream_insight` | A +0.05 | Peer's dream consolidation produced a novel hypothesis. Mild curiosity signal. |

These are the pre-attenuation deltas. After the `AttenuationFunctor` applies, the actual impact on peer agents is:

| Trigger | Post-attenuation effect | Impact on peer routing |
|---|---|---|
| `peer_warning` | A +0.03 | Negligible. Barely perceptible arousal nudge. |
| `sustained_success` | D +0.0 | Zero. Dominance is not propagated. |
| `sustained_failure` | P -0.015 | Very small. Enough to shift a marginal routing decision if the peer is already near a threshold. |

The design is deliberately conservative. A single contagion event should never be the primary cause of a peer's behavioral shift. Contagion provides background modulation -- a statistical tendency that accumulates over many events, not a dramatic single-event override.

---

## 5. Anti-Cascade Mechanisms

Three properties of the Bus and Functor design naturally prevent emotional avalanches. No dedicated anti-cascade system is needed.

### 5.1 Unidirectional Relay (No Echo)

The Group's relay is unidirectional: A's affect Pulse is relayed to B, C, D. But B's affect change (even if caused by A's Pulse) is a new Pulse from B, attenuated again at the relay. There is no feedback path where A's attenuated signal returns to A amplified.

```
Agent A                    Group Relay              Agent B
   |                          |                        |
   |--- AffectPulse --------->|                        |
   |                          |--- attenuate(pulse) -->|
   |                          |                        |
   |                          |     B reacts, its own  |
   |                          |     affect changes     |
   |                          |                        |
   |                          |<-- B's AffectPulse ----|
   |<-- attenuate(B.pulse) ---|                        |
   |                          |                        |
```

If A publishes PAD = (P: -0.5, A: 0.8, D: -0.3), B receives (P: -0.15, A: 0.24, D: 0.0). Even if B's affect shifts to match and B re-publishes, A receives B's already-attenuated signal attenuated again: (P: -0.045, A: 0.072, D: 0.0). The signal decays geometrically with each relay hop. After two hops, it is negligible.

### 5.2 Arousal Cap (Functor Clamp)

The `AttenuationFunctor` clamps arousal to +/- 0.3 per sync cycle, regardless of the source agent's raw arousal. Even if an agent is in Crisis regime with arousal = 1.0, peers receive at most arousal = 0.3 (before their own attenuation applies).

This cap prevents a pathological scenario: an agent in a panic loop (arousal spiraling from repeated gate failures) broadcasting high-arousal Pulses that push peers past their own arousal thresholds, triggering their own panic loops.

### 5.3 Decay Half-Life (Bus TTL)

Contagion influence decays with a 6-hour half-life. The receiving agent applies an exponential decay to the accumulated contagion contribution:

```rust
/// Accumulated contagion influence on this agent from all peers.
/// Decays with configurable half-life (default 6 hours).
pub struct ContagionAccumulator {
    /// Accumulated PAD contribution from peers.
    accumulated: PadVector,
    /// Timestamp of last update.
    last_update: DateTime<Utc>,
    /// Decay half-life.
    half_life: Duration,
}

impl ContagionAccumulator {
    /// Apply decay and add a new attenuated pulse.
    pub fn absorb(&mut self, attenuated_pulse: &AffectPulse, now: DateTime<Utc>) {
        // Decay existing accumulation.
        let elapsed = now.signed_duration_since(self.last_update);
        let hours = elapsed.num_seconds() as f64 / 3600.0;
        let decay = (-hours * (2.0_f64.ln()) / self.half_life.num_hours() as f64).exp();

        self.accumulated = PadVector::new(
            self.accumulated.pleasure * decay + attenuated_pulse.pad.pleasure,
            self.accumulated.arousal * decay + attenuated_pulse.pad.arousal,
            self.accumulated.dominance * decay + attenuated_pulse.pad.dominance,
        ).clamped();

        self.last_update = now;
    }

    /// Current contagion contribution to this agent's affect state.
    pub fn current(&self, now: DateTime<Utc>) -> PadVector {
        let elapsed = now.signed_duration_since(self.last_update);
        let hours = elapsed.num_seconds() as f64 / 3600.0;
        let decay = (-hours * (2.0_f64.ln()) / self.half_life.num_hours() as f64).exp();

        PadVector::new(
            self.accumulated.pleasure * decay,
            self.accumulated.arousal * decay,
            self.accumulated.dominance * decay,
        )
    }
}
```

The 6-hour half-life means that a peer's emotional event has 50% influence after 6 hours, 25% after 12 hours, and less than 1% after 2 days. Contagion is a short-term signal, not a permanent shift.

### Anti-Cascade Summary

| Mechanism | How it prevents cascade | Kernel primitive |
|---|---|---|
| Unidirectional relay | No feedback loops (A -> B -> A decays geometrically) | Bus relay architecture (already in Group) |
| Arousal cap | Hard limit on per-event arousal injection | Functor clamp (configurable per Group) |
| 6h decay half-life | Influence fades over time | Standard exponential decay (same math as demurrage) |

No additional anti-cascade infrastructure. The properties fall out of the existing Bus relay design and the attenuation Functor.

---

## 6. The Somatic Field IS a Shared Store Partition

Within a Group, agents can contribute their somatic markers to a shared somatic field. This is a Store partition within the Group's Space -- the same mechanism the Group uses for shared knowledge (see [10-GROUPS.md](../../unified/10-GROUPS.md) SS5.1) and pheromones (SS5.2).

### Accuracy-Weighted Contributions

When an agent contributes markers to the shared field, each marker is weighted by the source agent's historical accuracy in the Group's domain. An agent with 90% gate pass rate contributes markers at full intensity. An agent with 50% gate pass rate contributes markers at half intensity.

```rust
/// A Group's shared somatic field.
/// Store partition within the Group Space, merged from member contributions.
pub struct SomaticField {
    /// Group-level k-d tree index over contributed markers.
    index: SomaticIndex,

    /// Per-agent accuracy weights for contribution scaling.
    accuracy_weights: HashMap<AgentId, f64>,

    /// Group's Store partition reference.
    store: Arc<GroupStorePartition>,
}

impl SomaticField {
    /// Accept a contributed marker from a member agent.
    /// The marker's intensity is scaled by the contributor's accuracy weight.
    pub async fn accept_contribution(
        &mut self,
        marker: SomaticMarker,
        contributor: &AgentId,
    ) -> Result<(), StoreError> {
        let weight = self.accuracy_weights
            .get(contributor)
            .copied()
            .unwrap_or(0.5); // default 50% weight for unknown agents

        let weighted_marker = SomaticMarker {
            intensity: marker.intensity * weight,
            // Strip private fields before storage (see privacy section).
            source_episodes: vec![], // episodes are private
            ..marker
        };

        self.store.put(weighted_marker.into_signal()).await?;
        self.index.insert(weighted_marker);
        Ok(())
    }

    /// Query the shared somatic field.
    /// Same k-NN interface as the agent's private somatic Store.
    pub async fn query(
        &self,
        coords: &[f64; 8],
        k: usize,
    ) -> Vec<(f64, SomaticMarker)> {
        self.index.query_nearest(coords, k)
            .iter()
            .map(|(dist, marker)| (*dist, (*marker).clone()))
            .collect()
    }

    /// Update accuracy weight for an agent based on recent gate outcomes.
    pub fn update_accuracy(&mut self, agent_id: &AgentId, accuracy: f64) {
        self.accuracy_weights.insert(agent_id.clone(), accuracy.clamp(0.0, 1.0));
    }
}
```

### Field Queries in Context Assembly

When a Group member assembles its context for a tick, the `GroupContextBidder` (see [10-GROUPS.md](../../unified/10-GROUPS.md) SS5.3) can include the shared somatic field in the VCG auction alongside pheromones and knowledge. The somatic field competes for context space like any other context source.

The query flow:

```
Task arrives at agent in Group
    |
    v
1. Agent queries private somatic landscape (per SS6-7 of somatic-landscape.md)
    |
    v
2. Agent queries Group somatic field (same coords, same k-NN)
    |
    v
3. Results are merged: private markers at full weight, field markers at
   contributor-accuracy weight (already applied during contribution)
    |
    v
4. ContrarianRetrievalFunctor applies to merged results
    |
    v
5. Blended SomaticSignal informs Daimon ASSESS
```

---

## 7. Privacy IS Capability Intersection at Space Boundaries

The attenuation Functor does not just dampen PAD values -- it also strips private information. An agent's `AffectPulse` on its local Bus contains the full PAD vector, behavioral state, and trigger. The relay Functor removes fields that should not cross the Space boundary.

### What Crosses the Boundary

| Field | Local (agent Bus) | Group relay (after Functor) | Rationale |
|---|---|---|---|
| PAD vector | Full precision | Attenuated (P x0.3, A x0.3, D x0.0) | Dampened for anti-cascade |
| Behavioral state | Full | Transmitted | Useful for peer awareness (e.g., "agent is Struggling") |
| Trigger type | Full | Transmitted | Useful for interpreting the affect change |
| Source episodes | Full list | **Stripped** | Episode content is private to the agent |
| Task assignment | Full | **Stripped** | What the agent is working on may be sensitive |
| Strategy coordinates | Full | **Stripped** | Exact position in strategy space reveals tactical details |
| Source accuracy | Full | Transmitted | Needed for accuracy-weighted somatic field merging |

For somatic field contributions (not just Pulses), the privacy Functor applies a similar strip:

```rust
/// Privacy Functor applied to somatic markers before Group contribution.
/// Strips private fields, retaining only spatial + affect data.
pub struct PrivacyFunctor;

impl PrivacyFunctor {
    /// Strip private fields from a marker before contributing to the Group's
    /// somatic field. Coordinates and valence/intensity are kept (they are
    /// the functional content). Episodes, task IDs, and full PAD history
    /// are removed.
    pub fn strip_for_sharing(marker: &SomaticMarker) -> SomaticMarker {
        SomaticMarker {
            signal_id: SignalId::new(), // new ID for the shared copy
            strategy_coords: marker.strategy_coords,
            valence: marker.valence,
            intensity: marker.intensity,
            source_episodes: vec![],  // private
            balance: marker.balance,
            last_touched_at: marker.last_touched_at,
        }
    }
}
```

This is the same capability-intersection pattern used throughout the Space architecture: when data crosses a Space boundary, a Functor at the boundary determines what passes through based on the capability grants of the target Space. Here, the Group's relay configuration defines the privacy policy. The agent does not decide what to share -- the Group boundary decides what can pass.

---

## 8. Contagion as React Cell (Receiving Side)

On the receiving side, the attenuated `AffectPulse` is consumed by a React Cell in the peer agent's pipeline. This Cell subscribes to the `group:{id}:affect` room on the Bus and integrates the contagion signal into the agent's own affect state.

```rust
/// React Cell that absorbs attenuated peer affect Pulses.
/// Subscribes to group:{id}:affect room on the Bus.
pub struct ContagionReceiverCell {
    accumulator: ContagionAccumulator,
    daimon: Arc<DaimonState>,
}

impl Cell for ContagionReceiverCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "contagion.receiver" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let pulse = AffectPulse::from_signals(&input)?;
        let now = Utc::now();

        // Absorb the attenuated pulse into the accumulator (decays old influence).
        self.accumulator.absorb(&pulse, now);

        // The accumulated contagion contribution is applied as a soft bias
        // to the agent's affect state, not a hard override.
        let contagion_pad = self.accumulator.current(now);

        // The contagion contribution is mixed at 10% weight into the
        // emotion layer of the ALMA model. This means peer affect
        // influences the fast (emotion) layer but not the slower
        // mood or temperament layers directly.
        self.daimon.apply_contagion_delta(
            contagion_pad.pleasure * 0.1,
            contagion_pad.arousal * 0.1,
            contagion_pad.dominance * 0.1, // already 0.0 from attenuation
        );

        Ok(vec![]) // React Cells do not produce output Signals.
    }
}
```

### Effective Impact Calculation

An event with raw PAD delta of (P: -0.5, A: 0.8, D: -0.3):

1. Attenuation Functor: (P: -0.15, A: 0.24, D: 0.0)
2. Decay: If absorbed immediately, full strength. After 6h, halved.
3. Contagion receiver mix: 10% weight into emotion layer.
4. Effective delta on peer: (P: -0.015, A: 0.024, D: 0.0)

A single event shifts the peer's emotion layer by less than 0.025 on any axis. This is below the threshold for a behavioral state change (which requires PAD magnitude change > 0.15 to create a somatic marker). Contagion works through accumulation, not individual events.

---

## 9. Contagion Susceptibility

Not all agents are equally susceptible to contagion. The receiver's own affect state modulates how much contagion influence is absorbed:

```rust
/// Compute susceptibility based on the receiver's current state.
/// Agents in unstable states are more susceptible to contagion;
/// agents in stable states are more resistant.
///
/// Uses the canonical six behavioral states from
/// [19-behavioral-states-and-routing.md](19-behavioral-states-and-routing.md).
pub fn contagion_susceptibility(receiver_state: &AffectState) -> f64 {
    let stability = match receiver_state.behavioral_state {
        BehavioralState::Focused => 0.7,     // stable, low susceptibility
        BehavioralState::Engaged => 0.8,     // stable
        BehavioralState::Coasting => 0.9,    // comfortable, slightly resistant
        BehavioralState::Exploring => 1.0,   // baseline, open to signals
        BehavioralState::Struggling => 1.2,  // unstable, high susceptibility
        BehavioralState::Resting => 0.5,     // low arousal, dampened absorption
    };

    stability.clamp(0.3, 1.5)
}
```

A Focused agent absorbs only 70% of the already-attenuated contagion. A Struggling agent absorbs 120%. This creates a natural stabilization effect: agents that are doing well are less affected by peer distress; agents that are already struggling are more receptive to warning signals.

---

## 10. Crate Mapping (Implementation Reality)

| Spec concept | Crate | Current status |
|---|---|---|
| `ContagionEvent`, `ContagionTrigger` | `roko-daimon/src/phase2_stubs.rs` | Implemented: types and trigger enum exist |
| `contagion()`, `contagion_susceptibility()` | `roko-daimon/src/phase2_stubs.rs` | Implemented: susceptibility calculation exists as a function |
| `SomaticField` | `roko-daimon/src/phase2_stubs.rs` | Stub: struct exists but not wired to Group Store partition |
| `AttenuationFunctor` | Not yet implemented | Bus relay filter pattern not yet generalized |
| `ContagionAccumulator` | Not yet implemented | Decay logic exists in `AffectState.decay()` but not as a separate accumulator for peer influence |
| `ContagionReceiverCell` | Not yet implemented | Contagion is computed but not consumed as a React Cell in the pipeline |
| Group Bus relay for affect | `roko-serve` / `roko-runtime` | Partial: Group relay rooms exist but affect-specific relay filtering is not wired |
| Privacy stripping | Not yet implemented | No Functor at Space boundary for affect Pulses |

The data types and susceptibility math exist in `roko-daimon`. The architectural wiring -- Bus relay filters, React Cells, Functor composition at Group boundaries -- is the work remaining.

---

## What This Enables

1. **Emergent group caution** -- When one agent in a Group discovers a dangerous strategy region (repeated gate failures), the attenuated affect signal raises mild arousal in peers. Peers do not panic, but their EFE routing shifts slightly toward higher tiers for tasks in that region. The group collectively becomes more careful without any explicit coordination protocol.

2. **Accuracy-meritocratic influence** -- The shared somatic field weights contributions by historical accuracy. An agent with 95% gate pass rate has nearly double the influence of an agent with 50%. Good agents shape the group's emotional landscape more than poor ones. This emerges from the accuracy weight, not from an explicit reputation system.

3. **Natural dampening** -- The three anti-cascade mechanisms (unidirectional relay, arousal cap, 6h decay) mean the system cannot enter a synchronized panic state. Even in the worst case (all agents failing simultaneously), the geometric attenuation prevents the group's aggregate arousal from exceeding the cap.

4. **Privacy-preserving coordination** -- Agents share valence and intensity but not episodes, tasks, or strategy coordinates. A peer knows "agent A is stressed" but not "agent A failed on the auth module refactor." This allows emotional coordination without leaking task-specific information.

---

## Feedback Loops

1. **Contagion -> routing -> gate outcome -> marker -> contagion**: Peer affect influences routing (via the somatic signal -> EFE routing bias path). Better routing leads to better gate outcomes. Better outcomes improve the peer's affect, which propagates back (attenuated) as a positive contagion signal. This is a stabilizing loop: good outcomes dampen bad affect, bad outcomes dampen good affect, both through the attenuation Functor.

2. **Accuracy weight -> somatic field influence -> peer routing -> peer accuracy -> accuracy weight**: An agent's influence on the shared somatic field is proportional to its accuracy. If the field's guidance improves peer routing, peer accuracy increases, which increases their own contribution weight. This is a slowly-compounding positive loop bounded by the fact that accuracy cannot exceed 1.0.

3. **Susceptibility -> contagion absorption -> state change -> susceptibility**: A Struggling agent absorbs more contagion. If the contagion is positive (peer success), the agent shifts toward Engaged, reducing its susceptibility. If the contagion is negative (peer failure), the agent shifts toward Cautious, which has only slightly elevated susceptibility (1.1 vs. 1.2). The susceptibility gradient is designed to bias toward stabilization -- positive contagion lifts the vulnerable more than negative contagion depresses them.

4. **Group size -> contagion frequency -> accumulator saturation**: Larger groups produce more `AffectPulse` messages. The `ContagionAccumulator`'s 6h decay prevents unbounded growth, but the steady-state accumulated PAD is proportional to the message rate. Groups with 50 agents will have a noisier contagion signal than groups with 5. The attenuation coefficients may need to be adjusted per-group based on size (an open question).

---

## Open Questions

1. **Attenuation coefficient tuning**: The coefficients (P: 0.3, A: 0.3, D: 0.0) are derived from the original source material but have not been empirically validated in this system. Should the coefficients be learnable (a React Cell that adjusts them based on group outcome metrics)? The risk: adaptive attenuation could converge to zero (ignore all peers) or to 1.0 (full propagation, cascade risk).

2. **Cross-group contagion**: If an agent belongs to multiple Groups, should contagion from Group A influence the agent's behavior in Group B? Currently, contagion is accumulated per-agent (single `ContagionAccumulator`), so yes, it bleeds across groups. An alternative: per-group accumulators, so the agent's affect in Group A is modulated only by Group A peers. This adds complexity but respects group boundaries.

3. **Contagion in leader-follower groups**: In a leader-follower Group (see [10-GROUPS.md](../../unified/10-GROUPS.md) SS3.4), should the leader's affect be attenuated differently than a follower's? A leader's stress signal might be more informative (the leader has broader context). One option: asymmetric attenuation (leader -> follower at 0.4, follower -> leader at 0.2).

4. **Contagion and Dreams**: Should accumulated contagion influence the agent's dream consolidation priorities? If the group is collectively stressed about a particular strategy region, the individual agent's NREM replay could prioritize episodes from that region. This would create a "shared dream" effect -- the group's emotional state shapes each member's consolidation, even though dreams are private.

5. **Opt-out semantics**: Should individual agents be able to opt out of contagion within a Group? The TOML config controls group-level contagion, but some agents (e.g., an auditor role that should be emotionally independent) might need per-agent override. This could be a capability grant: `contagion.receive = false` in the agent's Space grants.

6. **Contagion observability**: The current design does not surface contagion influence in the TUI or dashboard. Should there be a visualization showing the flow of affect between agents in a Group? This would help operators understand why an agent's behavior shifted ("agent B became conservative because agent A had three gate failures"). The data is available in the Bus Pulses; the question is whether to build the surface.

---

## Citations

1. Hatfield, E., Cacioppo, J. T., & Rapson, R. L. (1993). "Emotional contagion." *Current Directions in Psychological Science*, 2(3), 96-100. -- Foundational work on emotional contagion between humans.
2. Barsade, S. G. (2002). "The ripple effect: Emotional contagion and its influence on group behavior." *Administrative Science Quarterly*, 47(4), 644-675. -- Group-level contagion dynamics.
3. Gebhard, P. (2005). "ALMA -- A Layered Model of Affect." *Proc. AAMAS*. -- Three-layer temporal affect model used in `AlmaLayers`.
4. See [05-AGENT.md](../../unified/05-AGENT.md) SS4 for `CorticalState.affect` (the `AtomicPAD` that contagion modulates).
5. See [10-GROUPS.md](../../unified/10-GROUPS.md) SS3 for Group coordination modes and Bus relay architecture.
6. See [20-somatic-landscape.md](20-somatic-landscape.md) for the somatic Store that the shared somatic field extends.
7. See [cross-cut-functors.md](cross-cut-functors.md) SS4.1 for `DaimonBiasAssess` where the contagion-modulated PAD enters the cognitive pipeline.

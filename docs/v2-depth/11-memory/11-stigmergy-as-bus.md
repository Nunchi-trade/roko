# Stigmergy as Bus

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How indirect coordination emerges from the Bus fabric and Pulse lifecycle rather than requiring dedicated pheromone infrastructure.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal/Pulse duality, demurrage, Kind), [02-CELL](../../unified/02-CELL.md) (Cell, React, Observe, Route protocols), [03-GRAPH](../../unified/03-GRAPH.md) (Graph topologies, Loop), [10-GROUPS](../../unified/10-GROUPS.md) (Group, CoordinationMode, Bus partitions), [store-and-bus-duality](../02-block/store-and-bus-duality.md) (graduation/projection bridges), [c-factor-as-lens](../10-learning-loops/c-factor-as-lens.md) (collective intelligence Lens)

---

## 1. The Redundancy Problem

06-MEMORY defines pheromones as Pulses with `PheromoneKind`, location hash, and intensity. 10-GROUPS defines a `GroupPheromone` struct stored as Signals in a group's Store partition with demurrage-weighted decay. The existing stigmergy depth doc (03-graph/stigmergy-and-cross-domain) describes three coordination channels: git commits, Signal log, and neuro Store. These are three separate mechanisms solving the same problem: agents leaving traces for other agents.

The problem is not that these mechanisms are wrong. The problem is that they are special-purpose where the kernel already provides a general-purpose solution. The Bus is a pub/sub fabric with ring-buffer eviction. Store is a durable fabric with demurrage decay. The graduation bridge converts ephemeral Pulses into durable Signals. These three primitives -- Bus, Store, graduation -- already implement every behavior that the pheromone literature requires:

| Pheromone behavior | Kernel mechanism |
|---|---|
| Deposit | `Bus::publish(pulse)` |
| Evaporation | Ring-buffer eviction (Bus) or demurrage decay (Store) |
| Reinforcement | Repeated publication at same topic + location hash |
| Reading the field | `Bus::subscribe(topic_filter)` or `Store::query_similar(fingerprint)` |
| Scoped visibility | Bus topic partitions (`pheromone.local.*`, `group:{id}:pheromone.*`, `workspace:*`) |
| Persistence graduation | Standard graduation bridge: React Cell promotes high-reinforcement Pulses to Signals |

The redesign eliminates dedicated pheromone infrastructure by recognizing that stigmergy IS what the dual-fabric architecture does. Every Signal written to Store already projects a notification Pulse on Bus. Every coordination Pulse on Bus already evaporates through ring eviction. Pheromones are not a category of data -- they are a usage pattern over existing primitives.

---

## 2. Pheromones as Pulses with Evaporation Kind

A pheromone is a Pulse published on a scoped Bus topic, carrying a `Kind::Pheromone` discriminant with structured metadata. No new struct. No separate registry. The Pulse carries enough information for subscribers to interpret it as a stigmergic trace.

```rust
/// Pheromone Pulse payload. Carried in the standard Pulse::body field.
/// The Pulse's topic determines scope; the payload determines semantics.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PheromonePulse {
    /// What kind of coordination signal this is.
    pub pheromone_kind: PheromoneKind,

    /// HDC fingerprint of the location/context where this pheromone
    /// was deposited. Agents with similar working contexts will match
    /// on this vector without needing shared naming conventions.
    pub location: HdcVector,

    /// Intensity at deposit time. Starts at 1.0.
    /// Decays implicitly through ring-buffer position: older Pulses
    /// are further from the ring head, and are evicted first.
    /// Reinforcement is modeled by multiple deposits at the same
    /// location -- the count of matching Pulses IS the field strength.
    pub intensity: f64,

    /// Structured metadata for the specific pheromone kind.
    /// Threat: { source, severity, evidence_hash }
    /// Opportunity: { topic, estimated_value, source }
    /// Wisdom: { insight_ref, confidence }
    /// Curiosity: { question, novelty_score }
    pub metadata: Value,
}

pub enum PheromoneKind {
    Wisdom,        // "I learned something useful here"
    Opportunity,   // "There is value to capture here"
    Threat,        // "Danger -- avoid or prepare"
    Curiosity,     // "Something unexplained -- investigate"
    Progress,      // "I completed work here" (trail marker)
}
```

### Topic conventions

Pheromone Pulses use the standard Bus topic hierarchy. The topic encodes scope and kind:

```text
pheromone.local.{agent_id}.{kind}         Local (only this agent sees it)
pheromone.group.{group_id}.{kind}         Group scope
pheromone.workspace.{kind}                Workspace-wide
pheromone.global.{kind}                   Mesh-wide (crosses workspace boundaries)
```

An agent subscribes to the scopes it cares about:

```rust
// Subscribe to all pheromones in my group and workspace
let filter = TopicFilter::Or(
    Box::new(TopicFilter::Glob("pheromone.group.a1b2c3d4.*".into())),
    Box::new(TopicFilter::Glob("pheromone.workspace.*".into())),
);
let rx = bus.subscribe(filter).await?;
```

### Why evaporation IS ring-buffer eviction

06-MEMORY specifies pheromone decay with a 1-hour default half-life. This suggests a timer-based mechanism. But the Bus ring buffer already provides a superior model:

1. **Capacity-based eviction is load-adaptive.** A busy system evicts old Pulses faster (higher throughput pushes the ring). A quiet system retains them longer. This is exactly what biological pheromone evaporation does -- it correlates with environmental activity, not wall-clock time.

2. **No timer infrastructure needed.** Demurrage requires periodic balance recomputation. Ring eviction is free -- it happens at publish time.

3. **Reinforcement is count-based.** A pheromone that five agents independently deposit at the same location hash exists as five separate Pulses in the ring. A subscriber querying `replay_since` sees all five. The field strength at a location is `count(matching_pulses_in_ring)`, not a decaying scalar.

The one case where ring eviction diverges from the pheromone model is when the system needs pheromones to persist beyond the ring window. This is exactly the graduation case: a React Cell watches for high-reinforcement pheromone Pulses and graduates them to Signals in Store with standard demurrage. Ephemeral coordination stays on Bus. Persistent landmarks graduate to Store.

```rust
/// Pheromone evaporation through ring-buffer mechanics.
///
/// The "field" at a location is computed by counting matching Pulses
/// currently in the ring. As Pulses are evicted, field strength
/// drops naturally. No timer, no decay function, no GC.
pub fn field_strength_at(
    ring: &[Pulse],
    location: &HdcVector,
    radius: f32,
) -> PheromoneField {
    let mut field = PheromoneField::default();

    for pulse in ring.iter().rev() {
        // Only consider pheromone Pulses
        let body: PheromonePulse = match serde_json::from_value(pulse.body.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        // HDC similarity check: is this Pulse near our query location?
        let distance = hdc_hamming_distance(&body.location, location);
        if distance > radius { continue; }

        // Recency weight: Pulses closer to ring head have more influence.
        // This is the evaporation analog -- position in ring IS decay.
        let recency = 1.0; // uniform weight; or: ring_head_seq - pulse.seq

        match body.pheromone_kind {
            PheromoneKind::Wisdom     => field.wisdom += body.intensity * recency,
            PheromoneKind::Opportunity => field.opportunity += body.intensity * recency,
            PheromoneKind::Threat     => field.threat += body.intensity * recency,
            PheromoneKind::Curiosity  => field.curiosity += body.intensity * recency,
            PheromoneKind::Progress   => field.progress += body.intensity * recency,
        }
    }

    field
}

#[derive(Default)]
pub struct PheromoneField {
    pub wisdom: f64,
    pub opportunity: f64,
    pub threat: f64,
    pub curiosity: f64,
    pub progress: f64,
}
```

### Persistent pheromones: graduation, not a third mechanism

Some pheromone traces need to outlive the ring buffer. A Threat pheromone about a known bad pattern should persist for days, not minutes. The answer is not a PheromoneRegistry -- it is the standard graduation bridge.

```rust
/// Graduation policy for pheromone Pulses.
/// Watches pheromone Bus topics and graduates high-value pheromones
/// to durable Signals in Store.
pub struct PheromoneGraduationPolicy {
    /// Graduate Threat pheromones above this intensity.
    threat_graduation_threshold: f64,    // default: 0.8

    /// Graduate when N+ agents deposit at the same location within
    /// the ring window (reinforcement-based graduation).
    reinforcement_threshold: usize,      // default: 3

    /// Graduate Wisdom pheromones that reference a gate-verified Signal.
    graduate_verified_wisdom: bool,      // default: true
}

impl PheromoneGraduationPolicy {
    fn should_graduate(
        &self,
        pulse: &PheromonePulse,
        ring_context: &RingContext,
    ) -> bool {
        // High-severity threats always graduate
        if pulse.pheromone_kind == PheromoneKind::Threat
            && pulse.intensity >= self.threat_graduation_threshold
        {
            return true;
        }

        // Reinforced pheromones graduate (multiple agents agree)
        let deposit_count = ring_context.count_similar_deposits(
            &pulse.location,
            0.15, // HDC radius
        );
        if deposit_count >= self.reinforcement_threshold {
            return true;
        }

        // Verified wisdom graduates
        if self.graduate_verified_wisdom
            && pulse.pheromone_kind == PheromoneKind::Wisdom
            && pulse.metadata.get("gate_verified").and_then(|v| v.as_bool()) == Some(true)
        {
            return true;
        }

        false
    }
}
```

Once graduated, the pheromone is a Signal subject to standard demurrage. It decays based on use, not on a pheromone-specific timer. The Group's `pheromone_decay_rate` config (from 10-GROUPS) maps directly to a demurrage weight modifier on the graduated Signal -- no custom exponential needed.

---

## 3. Scopes as Bus Topic Partitions

06-MEMORY mentions pheromone scope (local, group, workspace, global). 10-GROUPS defines `group:{id}:pheromones` as a Bus sub-room. These are the same concept: Bus topic namespaces provide nested scope naturally.

### The scope hierarchy

```text
Space hierarchy:           Bus topic partition:
------                     ------
Agent (local)              pheromone.local.{agent_id}.*
  |
Group (shared)             pheromone.group.{group_id}.*
  |                          (= group:{id}:pheromones in 10-GROUPS convention)
Workspace (all agents)     pheromone.workspace.*
  |
Global (mesh-wide)         pheromone.global.*
```

Each scope is a Bus topic partition. An agent subscribes to the scopes defined by its Space memberships. A Group member subscribes to `pheromone.group.{group_id}.*` on join. A workspace agent subscribes to `pheromone.workspace.*` at startup.

This is not a separate mechanism from Group Bus partitions. It IS the Group Bus partition. The `group:{id}:pheromones` sub-room defined in 10-GROUPS is the same topic space as `pheromone.group.{group_id}.*`. The topic naming convention differs; the underlying Bus dispatch is identical.

### Nested scope visibility

An agent in a Group sees: its local scope + the Group scope + the workspace scope. This is a union of topic subscriptions:

```rust
/// Build the pheromone subscription for an agent based on its Space memberships.
pub fn pheromone_subscription(agent_id: &AgentId, memberships: &[GroupId]) -> TopicFilter {
    let mut filters = vec![
        // Always see own local pheromones
        TopicFilter::Glob(format!("pheromone.local.{}.*", agent_id)),
        // Always see workspace-wide pheromones
        TopicFilter::Glob("pheromone.workspace.*".into()),
    ];

    // See group pheromones for each group membership
    for group_id in memberships {
        filters.push(TopicFilter::Glob(
            format!("pheromone.group.{}.*", group_id),
        ));
    }

    // Combine with Or
    filters.into_iter().reduce(|a, b| TopicFilter::Or(Box::new(a), Box::new(b)))
        .unwrap_or(TopicFilter::All)
}
```

### Hop decay for mesh-wide pheromones

Global pheromones cross workspace boundaries via the relay transport (see 11-CONNECTIVITY). Hop decay (0.85 per hop, max 3 hops from the source material) is modeled by the relay transport attenuating the `intensity` field on each forward:

```rust
/// Relay-level hop attenuation for mesh-wide pheromone Pulses.
/// Applied by the MultiBus relay bridge when forwarding to a remote Bus.
pub fn attenuate_on_hop(pulse: &mut Pulse, hop_decay: f64) {
    if let Ok(mut body) = serde_json::from_value::<PheromonePulse>(pulse.body.clone()) {
        body.intensity *= hop_decay;
        pulse.body = serde_json::to_value(&body).unwrap_or(pulse.body.clone());
    }
}

const HOP_DECAY: f64 = 0.85;
const MAX_HOPS: u8 = 3;
```

This is a Bus-level concern, not a pheromone-specific mechanism. The MultiBus (from store-and-bus-duality) already aggregates backends; adding hop attenuation at the relay boundary is a natural extension of its `publish` method.

---

## 4. Coordination Modes as Graph Topologies

10-GROUPS defines four coordination modes: Stigmergic, Pipeline, Broadcast, Leader-follower. These are not runtime configuration flags -- they are Graph topologies that describe how Cells are wired within a Group's Space.

### The key insight

A coordination mode is a Graph template. The Group's CoordinationMode enum selects which Graph template is instantiated for the Group's coordination Bus partition. The agents in the Group are Cells (specifically, Agent Cells from 05-AGENT). The coordination mode determines how they are wired.

```text
Stigmergic:
  No direct edges between Agent Cells.
  All agents subscribe to the Group's pheromone Bus topics.
  React Cells on the shared Bus respond to deposits.
  Coordination emerges from environment modification.

  [Agent A] --publish--> [Bus: pheromone.group.X.*] <--subscribe-- [Agent B]
                               |
                          [React Cell: pheromone aggregator]

Pipeline:
  Linear edges between Agent Cells.
  Each stage's output is the next stage's input.
  This is a Flow (03-GRAPH) where nodes are Agent dispatch Cells.

  [Agent A] --> [Agent B] --> [Agent C]

Broadcast:
  Fan-out edges from the Bus to all agents.
  Every message reaches every member.
  This is a flat Graph where all agents subscribe to the same topic.

  [Agent A] --publish--> [Bus: group.X] --deliver--> [Agent B]
                                        --deliver--> [Agent C]
                                        --deliver--> [Agent D]

Leader-follower:
  Hierarchical edges.
  Leader Cell receives all events, dispatches to follower Cells.
  Follower outputs feed back to leader.

  [Leader] --assign--> [Follower A] --report--> [Leader]
           --assign--> [Follower B] --report-->
           --assign--> [Follower C] --report-->
```

### Stigmergic mode: React Cells on shared Bus

In stigmergic mode, agents do not communicate directly. They deposit pheromone Pulses on the Group Bus. Other agents read the field and decide what to do. The coordination logic lives in React Cells that subscribe to pheromone topics:

```rust
/// React Cell that watches the pheromone field on a Group Bus
/// and adjusts agent behavior via somatic marker Pulses.
///
/// This is the operational core of stigmergic coordination:
/// environment modification (deposits) -> observation (React) ->
/// behavioral adjustment (somatic Pulse) -> further modification.
pub struct StigmergicReactCell {
    group_id: GroupId,
    /// Threat threshold: above this, emit a caution somatic marker.
    threat_threshold: f64,
    /// Opportunity threshold: above this, emit an exploration somatic marker.
    opportunity_threshold: f64,
}

#[async_trait]
impl ReactProtocol for StigmergicReactCell {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let field = aggregate_pheromone_field(pulses);

        let mut output_pulses = Vec::new();

        // High threat field: emit caution somatic marker
        if field.threat > self.threat_threshold {
            output_pulses.push(Pulse::new(
                Topic::from(format!("somatic.group.{}.caution", self.group_id)),
                Kind::Somatic,
                json!({
                    "trigger": "pheromone_threat",
                    "intensity": field.threat,
                    "action": "increase_verify_depth",
                }),
            ));
        }

        // High opportunity field: emit exploration somatic marker
        if field.opportunity > self.opportunity_threshold {
            output_pulses.push(Pulse::new(
                Topic::from(format!("somatic.group.{}.explore", self.group_id)),
                Kind::Somatic,
                json!({
                    "trigger": "pheromone_opportunity",
                    "intensity": field.opportunity,
                    "action": "lower_exploration_threshold",
                }),
            ));
        }

        Ok(ReactOutput { pulses: output_pulses, signals: vec![] })
    }

    fn subscription(&self) -> TopicFilter {
        TopicFilter::Glob(format!("pheromone.group.{}.*", self.group_id))
    }
}
```

### Coordination mode as Route Cell selection

Here is the 10x insight: **the coordination mode itself should be selectable via a Route Cell.** Rather than fixing the mode at Group creation, a Route Cell watches task characteristics and selects the appropriate coordination topology for each task.

```rust
/// Route Cell that selects coordination mode based on task characteristics.
///
/// This is a Loop: the Route Cell observes outcomes of previous coordination
/// attempts and adjusts mode selection via predict-publish-correct.
pub struct CoordinationModeRouter {
    /// Learned preferences: task features -> mode effectiveness
    mode_scores: RwLock<ModePreferences>,
}

pub struct ModePreferences {
    /// Per-mode EMA of outcome quality
    stigmergic_ema: f64,
    pipeline_ema: f64,
    broadcast_ema: f64,
    leader_follower_ema: f64,
}

impl Cell for CoordinationModeRouter {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let task = extract_task_context(&input)?;
        let preferences = self.mode_scores.read();

        // Select mode based on task features + learned preferences
        let mode = if task.is_loosely_coupled && task.agent_count > 3 {
            // Many agents, weak coupling: stigmergic works best
            CoordinationMode::Stigmergic
        } else if task.has_sequential_dependencies {
            // Clear dependency chain: pipeline
            CoordinationMode::Pipeline
        } else if task.requires_rapid_response {
            // Real-time urgency: broadcast
            CoordinationMode::Broadcast
        } else if task.has_clear_decomposition {
            // Decomposable work: leader-follower with assignment
            CoordinationMode::LeaderFollower
        } else {
            // Tie-break by learned preferences
            preferences.best_mode()
        };

        // Publish prediction (predict-publish-correct)
        ctx.bus.publish(Pulse::new(
            Topic::from(format!("prediction.coordination_mode.{}", ctx.run_id())),
            Kind::Prediction,
            json!({ "selected_mode": mode, "task_id": task.id }),
        )).await?;

        Ok(vec![Signal::new(Kind::Route, json!({ "mode": mode }))])
    }
}
```

This makes coordination mode a learned decision within the Loop pattern, not a static Group configuration. The Group's `coordination` field in `roko.toml` becomes the default -- a starting point that the Route Cell can override per-task.

---

## 5. The Dual-Write Insight: Store Projection as Free Stigmergy

Every `Store::put(signal)` already triggers a projection Pulse on Bus (from store-and-bus-duality S5):

```text
Agent writes Signal to Store
  |
  v
Store::put(signal) -> SignalRef
  |
  v
project_store_write(signal, ref, bus) -> Pulse on "store.signal.written"
```

This means every Signal written to Store automatically emits an observable trace on Bus. The projection Pulse carries the `SignalRef`, `Kind`, `tags`, and `score_effective`. Any React Cell subscribing to `store.signal.written` sees every Store mutation in real time.

**This is free stigmergy.** Agents writing Signals to Store are already modifying the shared environment. Other agents observing the Bus already see those modifications. Coordination emerges without any agent intending to coordinate.

The explicit pheromone mechanism from 06-MEMORY S11 is then a way to deposit intentional coordination signals -- but the majority of stigmergic traces are the unintentional byproducts of agents doing their normal work:

| Trace | How it appears on Bus | Stigmergic meaning |
|---|---|---|
| Task completion Signal | `store.signal.written` with `Kind::Task` | Progress pheromone: "work was done here" |
| Verify verdict Signal | `store.signal.written` with `Kind::Verdict` | Threat/opportunity: "quality was assessed here" |
| Episode Signal | `store.signal.written` with `Kind::Episode` | Wisdom: "experience was gained here" |
| Heuristic Signal | `store.signal.written` with `Kind::Heuristic` | Wisdom: "a pattern was learned here" |
| Knowledge Signal | `store.signal.written` with `Kind::Knowledge` | Wisdom: "insight was captured here" |
| Conductor alert | `conductor.alert.*` on Bus | Threat: "danger detected" |
| Agent heartbeat | `agent:{id}.heartbeat` on Bus | Progress: "I am alive and working" |

Explicit pheromone deposits are needed only when an agent wants to leave a trace that has no Store-level artifact: "I investigated this area and found nothing useful" (curiosity), "this approach looks promising but I did not pursue it" (opportunity), "there is a trap here that I narrowly avoided" (threat).

```rust
/// PheromoneAwareness: a React Cell that synthesizes the pheromone
/// field from BOTH explicit pheromone deposits AND store projection
/// Pulses. This is the unified field view.
pub struct PheromoneAwareness {
    location_context: HdcVector,  // HDC fingerprint of agent's current working context
    radius: f32,                  // similarity radius for field aggregation
}

#[async_trait]
impl ReactProtocol for PheromoneAwareness {
    async fn react(
        &self,
        pulses: &[Pulse],
        ctx: &ReactContext,
    ) -> Result<ReactOutput> {
        let mut field = PheromoneField::default();

        for pulse in pulses {
            match pulse.topic.as_str() {
                // Explicit pheromone deposits
                t if t.starts_with("pheromone.") => {
                    if let Ok(body) = serde_json::from_value::<PheromonePulse>(pulse.body.clone()) {
                        if hdc_hamming_distance(&body.location, &self.location_context) <= self.radius {
                            field.add(body.pheromone_kind, body.intensity);
                        }
                    }
                }

                // Store projections: implicit stigmergic traces
                "store.signal.written" => {
                    if let Ok(notif) = serde_json::from_value::<StoreWriteNotification>(pulse.body.clone()) {
                        // Fetch Signal fingerprint for proximity check
                        if let Some(signal) = ctx.store.get(&notif.signal_ref.id).await? {
                            let dist = hdc_hamming_distance(&signal.hdc_fingerprint, &self.location_context);
                            if dist <= self.radius {
                                // Map Signal Kind to pheromone semantics
                                match signal.kind {
                                    Kind::Verdict if signal.score.effective() > 0.7 => {
                                        field.add(PheromoneKind::Progress, signal.score.effective());
                                    }
                                    Kind::Verdict => {
                                        field.add(PheromoneKind::Threat, 1.0 - signal.score.effective());
                                    }
                                    Kind::Heuristic | Kind::Knowledge => {
                                        field.add(PheromoneKind::Wisdom, signal.score.effective());
                                    }
                                    Kind::Episode => {
                                        field.add(PheromoneKind::Progress, 0.5);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                // Conductor alerts
                t if t.starts_with("conductor.alert.") => {
                    field.add(PheromoneKind::Threat, 0.9);
                }

                _ => {}
            }
        }

        // Emit the synthesized field as a local observation
        Ok(ReactOutput {
            pulses: vec![Pulse::new(
                Topic::from("telemetry.pheromone.field"),
                Kind::Telemetry,
                serde_json::to_value(&field)?,
            )],
            signals: vec![],
        })
    }

    fn subscription(&self) -> TopicFilter {
        TopicFilter::Or(
            Box::new(TopicFilter::Glob("pheromone.*".into())),
            Box::new(TopicFilter::Or(
                Box::new(TopicFilter::Exact("store.signal.written".into())),
                Box::new(TopicFilter::Glob("conductor.alert.*".into())),
            )),
        )
    }
}
```

---

## 6. Git Artifacts as Connect Cell Pulses

Git events (commits, branches, PR status, CI results) are stigmergic traces in the developer environment. In the unified vocabulary, a Connector Cell bridges git to the Bus.

```rust
/// Connector Cell that bridges git events to the Bus as pheromone Pulses.
///
/// Git events are the primary stigmergic medium in code-centric workflows.
/// This Connector translates them into the unified pheromone topic space.
pub struct GitConnectorCell {
    repo_path: PathBuf,
    /// Poll interval for git log changes (fallback if fsnotify unavailable).
    poll_interval: Duration,
}

impl Cell for GitConnectorCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Connect, ProtocolId::Trigger] }

    async fn execute(
        &self,
        _input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Triggered by file watcher or poll timer
        let new_commits = self.detect_new_commits().await?;

        for commit in &new_commits {
            // Each commit IS a Progress pheromone at the location of its changed files
            let location = HdcVector::encode_paths(&commit.changed_files);
            let pulse = Pulse::new(
                Topic::from("pheromone.workspace.progress"),
                Kind::Pheromone,
                json!(PheromonePulse {
                    pheromone_kind: PheromoneKind::Progress,
                    location: location.clone(),
                    intensity: 1.0,
                    metadata: json!({
                        "source": "git",
                        "commit_hash": commit.hash,
                        "author": commit.author,
                        "message": commit.message,
                        "changed_files": commit.changed_files,
                    }),
                }),
            );
            ctx.bus.publish(pulse).await?;

            // If commit message mentions failure/fix, also emit Threat/Wisdom
            if commit.message.contains("fix") || commit.message.contains("bug") {
                let threat_pulse = Pulse::new(
                    Topic::from("pheromone.workspace.threat"),
                    Kind::Pheromone,
                    json!(PheromonePulse {
                        pheromone_kind: PheromoneKind::Threat,
                        location,
                        intensity: 0.6,
                        metadata: json!({
                            "source": "git",
                            "commit_hash": commit.hash,
                            "reason": "commit indicates prior bug in this area",
                        }),
                    }),
                );
                ctx.bus.publish(threat_pulse).await?;
            }
        }

        Ok(vec![])
    }
}
```

CI status, PR comments, and branch creation are handled by the same pattern: each external event produces a Pulse on the appropriate pheromone topic. The Connect protocol handles lifecycle (connect, health_check, disconnect). The Trigger protocol handles event detection (listen, filter, debounce, fire).

---

## 7. Morphogenetic Specialization as Route Cell

Morphogenetic specialization (agents differentiating based on pheromone gradients) is a Route Cell that watches pheromone density on Bus topics and adjusts agent routing. When high demand in a domain creates a strong pheromone field, the Route Cell biases task assignment toward agents with matching capabilities.

```rust
/// Route Cell that implements morphogenetic specialization.
///
/// Watches pheromone field density across domains and adjusts
/// task-to-agent routing. High pheromone density in a domain
/// attracts agents to specialize there -- the Route Cell assigns
/// more tasks in that domain to agents with latent capability,
/// which builds their familiarity score over time.
pub struct MorphogeneticRouter {
    /// Per-domain pheromone field densities (rolling average)
    domain_densities: RwLock<HashMap<String, f64>>,

    /// Agent capability vectors (from CrateFamiliarityTracker + role)
    agent_capabilities: RwLock<HashMap<AgentId, HdcVector>>,

    /// Learning rate for domain density EMA
    density_alpha: f64,
}

impl Cell for MorphogeneticRouter {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let task = extract_task(&input)?;
        let task_domain_vector = HdcVector::encode_body(&task.payload);

        // Read current pheromone field from Bus replay
        let recent_pheromones = ctx.bus.replay_since(
            ctx.bus.current_seq().await?.saturating_sub(1000),
            &TopicFilter::Glob("pheromone.workspace.*".into()),
        ).await?;

        // Update domain densities
        let mut densities = self.domain_densities.write();
        for pulse in &recent_pheromones {
            if let Ok(body) = serde_json::from_value::<PheromonePulse>(pulse.body.clone()) {
                let domain_key = domain_key_from_location(&body.location);
                let entry = densities.entry(domain_key).or_insert(0.0);
                *entry = *entry * (1.0 - self.density_alpha) + body.intensity * self.density_alpha;
            }
        }
        drop(densities);

        // Select agent: prefer agents whose capability vector is close to
        // the task domain vector, weighted by pheromone density
        let capabilities = self.agent_capabilities.read();
        let mut best_agent = None;
        let mut best_score = f64::NEG_INFINITY;

        for (agent_id, cap_vector) in capabilities.iter() {
            let similarity = hdc_cosine_similarity(cap_vector, &task_domain_vector);
            let density_bonus = self.domain_densities.read()
                .get(&domain_key_from_location(&task_domain_vector))
                .copied()
                .unwrap_or(0.0);

            // Score: capability match + pheromone attraction
            let score = similarity * 0.7 + density_bonus * 0.3;

            if score > best_score {
                best_score = score;
                best_agent = Some(agent_id.clone());
            }
        }

        let selected = best_agent.ok_or(CellError::NoRoute("no capable agent found".into()))?;

        // Publish prediction for predict-publish-correct learning
        ctx.bus.publish(Pulse::new(
            Topic::from(format!("prediction.morphogenetic_route.{}", task.id)),
            Kind::Prediction,
            json!({ "agent": selected, "score": best_score }),
        )).await?;

        Ok(vec![Signal::new(Kind::Route, json!({ "agent": selected }))])
    }
}
```

Morphogenetic specialization emerges over time: as agents successfully complete tasks in a domain, their familiarity scores rise, the Route Cell assigns them more work in that domain, their familiarity rises further. This is a positive feedback loop bounded by the predict-publish-correct mechanism -- if over-specialization degrades outcome quality (because the agent loses flexibility), the correction signal reverses the trend.

---

## 8. C-Factor as Stigmergy Lens

The c-factor Lens (from c-factor-as-lens) already measures collective intelligence from Bus and Store statistics. Its five sub-lenses map directly to stigmergic dynamics:

| C-factor sub-lens | Stigmergic interpretation |
|---|---|
| Turn-taking entropy | How evenly agents deposit pheromones (vs. one agent dominating the field) |
| Peer prediction accuracy | How well agents predict each other's deposits (implicit model of the group) |
| Citation reciprocity | How often an agent's deposits reinforce another agent's prior deposits |
| Delivery rate | Infrastructure health of the pheromone transport (Bus reliability) |
| HDC diversity | Whether agents explore different regions of the pheromone space |

The c-factor Lens already subscribes to Bus traffic. To incorporate stigmergic dynamics explicitly, add a sixth sub-lens: **pheromone field coherence**.

```rust
/// Lens Cell: pheromone field coherence.
///
/// Measures whether the pheromone field is producing meaningful
/// coordination (agents respond to deposits) vs. noise (agents
/// ignore the field).
///
/// Metric: correlation between pheromone deposits and subsequent
/// agent actions within the deposit's HDC neighborhood.
///
/// Subscribes to: pheromone.* and agent:{id}.turn.completed
/// Publishes to:  telemetry.cohort.pheromone_coherence
pub struct PheromoneCoherenceLens;

impl PheromoneCoherenceLens {
    pub fn compute(deposits: &[DepositRecord], actions: &[ActionRecord]) -> f64 {
        if deposits.is_empty() { return 0.0; }

        let mut responded = 0u64;
        let mut total = 0u64;

        for deposit in deposits {
            total += 1;
            // Did any agent take action in the deposit's HDC neighborhood
            // within the response window?
            let nearby_actions = actions.iter().filter(|a| {
                a.timestamp > deposit.timestamp
                    && a.timestamp < deposit.timestamp + Duration::from_secs(300)
                    && hdc_hamming_distance(&a.location, &deposit.location) < 0.2
            });
            if nearby_actions.count() > 0 {
                responded += 1;
            }
        }

        responded as f64 / total as f64
    }
}
```

Low pheromone coherence means the stigmergic mechanism is not working -- agents are depositing but nobody is responding. This feeds into the coordination mode selection Loop: when coherence drops below a threshold, the CoordinationModeRouter may switch from stigmergic to a more directed mode (leader-follower or broadcast).

---

## 9. The Coordination Selection Loop

Tying it together: coordination mode selection is itself a Loop (03-GRAPH feedback pattern). The Loop reads c-factor and pheromone coherence, selects a coordination mode, observes outcomes, and adjusts.

```text
Coordination Selection Loop (L2 timescale: per-task-batch)
===========================================================

1. PheromoneCoherenceLens publishes field coherence.
2. CollectiveIntelligenceLens publishes c-factor.
3. CoordinationModeRouter reads both, selects mode for next batch.
4. Agents execute under the selected mode.
5. Outcome quality is measured (Verify pass rate, task completion time).
6. CohortWeightsLearner updates mode preferences.
7. Loop back to step 1.

Invariants:
- Mode selection is a prediction (published on Bus).
- Outcome is reality (published on Bus).
- CalibrationPolicy joins prediction and outcome.
- The CoordinationModeRouter's mode_scores update via gradient step.
```

```rust
/// The coordination selection Loop as a Graph.
///
/// Nodes:
///   coherence_lens    -> Observe protocol (pheromone coherence)
///   cfactor_lens      -> Observe protocol (c-factor)
///   mode_router       -> Route protocol (select coordination mode)
///   execution_graph   -> Agent execution under selected mode
///   outcome_scorer    -> Score protocol (measure task outcomes)
///   calibration       -> React protocol (join predictions with outcomes)
///
/// Feedback edge: outcome_scorer -> mode_router (closes the Loop)
pub fn coordination_loop_graph() -> Graph {
    Graph {
        name: "coordination-selection-loop".into(),
        nodes: vec![
            Node::cell("coherence_lens", CellRef::named("pheromone-coherence-lens")),
            Node::cell("cfactor_lens", CellRef::named("collective-intelligence-lens")),
            Node::cell("mode_router", CellRef::named("coordination-mode-router")),
            Node::cell("execution", CellRef::named("group-execution-graph")),
            Node::cell("outcome_scorer", CellRef::named("task-outcome-scorer")),
            Node::cell("calibration", CellRef::named("mode-calibration-react")),
        ],
        edges: vec![
            // Observations feed the router
            Edge::new("coherence_lens", "mode_router"),
            Edge::new("cfactor_lens", "mode_router"),
            // Router feeds execution
            Edge::new("mode_router", "execution"),
            // Execution produces outcomes
            Edge::new("execution", "outcome_scorer"),
            // Outcomes feed calibration
            Edge::new("outcome_scorer", "calibration"),
            // FEEDBACK EDGE: calibration adjusts the router (closes the Loop)
            Edge::new("calibration", "mode_router"),
        ],
        entry: vec!["coherence_lens".into(), "cfactor_lens".into()],
        exits: vec!["calibration".into()],
        policy: GraphPolicy::default(),
        ..Default::default()
    }
}
```

---

## 10. Reconciling the Two Evaporation Models

There is a genuine gap between Bus ring-buffer eviction and Store demurrage as models for pheromone evaporation.

**Ring-buffer eviction** (Bus): capacity-based, load-adaptive, no timer. A Pulse exists until pushed out by newer Pulses. Evaporation rate correlates with system activity.

**Demurrage** (Store): time-based, configurable rate, use-refreshable. A Signal decays at a fixed rate unless actively touched.

These are complementary, not contradictory:

| Pheromone lifetime | Mechanism | When |
|---|---|---|
| Seconds to minutes | Bus ring eviction only | Ephemeral coordination (heartbeats, micro-coordination) |
| Minutes to hours | Bus ring + graduation threshold | The graduation React Cell watches for reinforced Pulses and promotes them |
| Hours to days | Graduated Signal in Store + demurrage | Important pheromones (Threat, validated Wisdom) live as Signals |
| Days to permanent | Signal in Store + tier progression | Landmark pheromones that become Heuristics or Knowledge |

The gap is at the boundary: what happens to a pheromone that lives too long for the ring but is not yet worthy of graduation? Two options:

1. **Larger ring.** Size the pheromone-scoped Bus partition's ring buffer to cover the expected pheromone lifetime. For a 1-hour half-life at 10 pheromone Pulses/second, a ring of 36,000 entries covers the window.

2. **Intermediate graduation.** A graduation policy with a low threshold graduates borderline pheromones to Transient-tier Signals with aggressive demurrage (high `r`, high `beta`). They persist in Store for minutes-to-hours and then decay. This uses the standard graduation bridge without creating a third evaporation mechanism.

Option 2 is cleaner because it reuses existing primitives. The pheromone graduation policy just needs two thresholds: one for Transient graduation (low bar, aggressive demurrage) and one for Working-tier graduation (high bar, standard demurrage).

```rust
/// Two-tier pheromone graduation with differentiated demurrage.
pub struct TwoTierPheromoneGraduation {
    /// Low bar: graduate to Transient tier with fast decay.
    /// For pheromones that should persist 10-60 minutes.
    transient_threshold: usize,        // deposit count. default: 2
    transient_demurrage_rate: f64,     // high r, fast decay. default: 0.05

    /// High bar: graduate to Working tier with standard decay.
    /// For pheromones that should persist hours to days.
    working_threshold: usize,          // deposit count. default: 5
    working_demurrage_rate: f64,       // standard r. default: 0.01
}
```

---

## What This Enables

1. **Zero-cost stigmergy.** The dual-fabric architecture (Bus + Store + graduation bridge) already implements pheromone semantics. No dedicated PheromoneRegistry, no custom decay timers, no separate storage layer. Agents that write Signals to Store are already leaving stigmergic traces.

2. **Scope as topic namespace.** Pheromone scope (local, group, workspace, global) maps directly to Bus topic partitions. Group Bus sub-rooms from 10-GROUPS and pheromone scopes from 06-MEMORY are the same mechanism.

3. **Learnable coordination.** Coordination mode selection becomes a Route Cell in a Loop, not a static Group configuration. The system discovers which coordination topology works best for which task type.

4. **Git as native pheromone source.** A Connector Cell bridges git events to the Bus. Commits, branches, and CI status are pheromone deposits without requiring agents to explicitly coordinate.

5. **Unified field view.** The PheromoneAwareness React Cell synthesizes a single field from both explicit deposits and implicit Store projection Pulses. An agent perceives all environmental traces through one interface.

6. **Graceful persistence graduation.** Ephemeral pheromones live on Bus, important pheromones graduate to Store Signals, landmark pheromones progress through tiers and become Heuristics. The lifecycle is continuous, not categorical.

---

## Feedback Loops

1. **Stigmergic reinforcement loop.** Agent deposits pheromone -> other agents read field -> agents act on field -> action produces Store Signals -> projection Pulses strengthen the field -> more agents respond. Bounded by ring-buffer eviction (ephemeral traces) and demurrage (graduated Signals). This is the positive feedback that makes stigmergy work; evaporation is the negative feedback that prevents runaway accumulation.

2. **Morphogenetic specialization loop.** Pheromone density in domain X rises -> MorphogeneticRouter assigns more tasks in X to capable agents -> agents build familiarity in X -> agents succeed more -> more pheromone deposits in X. Bounded by predict-publish-correct: if specialization degrades outcome quality (overfitting to a domain), the correction signal reverses routing.

3. **Coordination mode selection loop.** PheromoneCoherenceLens measures field responsiveness -> CollectiveIntelligenceLens measures c-factor -> CoordinationModeRouter selects mode -> agents execute -> outcome quality feeds back to router. Bounded by the Goodhart guard: mode changes are retained only when both c-factor AND outcome quality improve.

4. **Graduation pressure loop.** Pheromone reinforcement (multiple deposits at same location) -> graduation threshold met -> pheromone becomes Signal in Store -> Signal demurrage begins -> if Signal is not used (retrieved, cited), it decays back below the prune threshold -> field returns to purely ephemeral. Use sustains graduated pheromones; neglect prunes them.

---

## Open Questions

1. **Ring sizing for pheromone partitions.** Should pheromone-scoped Bus partitions have a different ring capacity than the general Bus? The field_strength_at computation scans the ring linearly -- at 100K entries this takes ~1ms, which is acceptable. At 1M entries it takes ~10ms, which may not be. Is per-topic ring capacity worth the complexity?

2. **HDC location hash vs. topic hierarchy.** The redesign uses both HDC location vectors (for similarity-based field queries) and topic hierarchy (for scope-based subscriptions). Are both needed? Could the topic hierarchy encode enough location information to eliminate the HDC field scan? Likely not -- topic hierarchy is discrete (exact match or glob) while HDC similarity is continuous (Hamming distance). Both are needed, but the interaction between topic-based filtering and HDC-based aggregation deserves formalization.

3. **Cross-workspace pheromone pollution.** Global pheromones (`pheromone.global.*`) cross workspace boundaries. Without governance, one noisy workspace can pollute the global field. The hop decay mechanism (0.85 per hop) attenuates intensity, but does not address volume. Should there be a rate limit on global pheromone publication? A reputation-weighted filter? This intersects with the security model (16-SECURITY).

4. **Pheromone coherence and the cold-start problem.** The PheromoneCoherenceLens measures whether agents respond to deposits. In a new workspace with few agents, coherence will be near zero because there are not enough agents to respond. Should the CoordinationModeRouter treat low coherence differently in cold-start vs. steady-state? A simple heuristic: if total pheromone deposit count is below a minimum, skip coherence-based mode selection and use the TOML default.

5. **Reinforcement counting across Bus eviction.** The graduation threshold counts matching Pulses in the ring. If the ring evicts older deposits before the threshold is reached, the pheromone never graduates even if many agents deposited over time. Should the graduation policy maintain a separate counter outside the ring? This would add state, but the alternative is graduated pheromones that never reach threshold under high Bus throughput. The two-tier graduation (section 10) partially addresses this, but the fundamental tension between stateless ring semantics and stateful reinforcement counting remains.

6. **Affect integration.** The Daimon affect engine (05-AGENT) produces somatic markers that modulate agent behavior. Pheromone deposits should influence affect (a strong Threat field should increase arousal). The StigmergicReactCell in section 4 emits somatic Pulses in response to pheromone fields, but the reverse direction (affect influencing pheromone deposit intensity) is not addressed. Should a highly aroused agent deposit higher-intensity pheromones? This creates a secondary feedback loop (affect -> pheromone intensity -> other agents' affect) that could amplify or dampen collective mood.

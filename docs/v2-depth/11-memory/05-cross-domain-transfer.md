# Cross-Domain Transfer and Federation

> Depth for [06-MEMORY.md](../../unified/06-MEMORY.md). How cross-domain knowledge transfer and multi-level federation emerge from nested Spaces, Store boundaries, and Pipeline ingestion rather than bespoke federation machinery.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, HDC fingerprints, demurrage, taint), [02-CELL](../../unified/02-CELL.md) (Store protocol, Score protocol, Verify protocol, Compose protocol, Connect protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph, Pipeline pattern, Loop pattern), [05-AGENT](../../unified/05-AGENT.md) (Space, CognitiveWorkspace, 9-step pipeline), [06-MEMORY](../../unified/06-MEMORY.md) (Memory specialization, tiers, AntiKnowledge, HDC resonator networks), [10-GROUPS](../../unified/10-GROUPS.md) (Group as Space, Bus partition, Store partition), [11-CONNECTIVITY](../../unified/11-CONNECTIVITY.md) (Connect protocol, relay wire)

**Source docs**: `docs/06-neuro/08-cross-domain-hdc-transfer.md`, `docs/06-neuro/14-library-of-babel.md`, `docs/13-coordination/06-agent-mesh-sync.md`

---

## 1. The Core Insight: Transfer Is Just Retrieval Across a Boundary

The v1 source material treats cross-domain transfer and multi-level federation as separate subsystems -- resonance detection, transfer risk scoring, ingestion pipelines, Library of Babel levels, mesh sync protocols. But in the unified vocabulary, every one of these reduces to operations on primitives we already have.

Cross-domain resonance detection is `query_similar` on a Store where the entries happen to come from different domains. Transfer risk scoring is a Score Cell followed by a Verify Cell. The three-level federation (local, mesh, chain) is three nested Spaces, each with its own Store partition and Bus partition. Confidence discounting is a Functor applied at Space boundaries. The four-stage ingestion pipeline is a Pipeline Graph.

The old design needed a `ResonanceDetector`, a `ConfirmationTracker`, a `TransferRisk` scorer, a `LibraryOfBabel` layer, and a `MeshSync` protocol. The new design needs none of these as first-class types. They fall out of composing Cells with standard patterns.

This is the payoff of the unified vocabulary: machinery that previously required dedicated subsystems is expressed as wiring between Cells in Graphs.

---

## 2. Cross-Domain Resonance as Store Query

### 2.1 The Mechanism

Cross-domain resonance detection is the observation that `query_similar` on the Store protocol naturally returns results from any domain whose HDC vectors share structural overlap with the query vector. There is no separate "resonance detection loop." The same `query_similar` that retrieves knowledge during SENSE also detects cross-domain analogies -- the only difference is whether the caller filters results to the same domain or deliberately includes other domains.

```rust
/// Cross-domain resonance is just query_similar with a domain filter inverted.
///
/// During SENSE (step 2 of the cognitive loop), the agent queries its Memory Store
/// for knowledge relevant to the current task. The results may include entries from
/// other domains if their HDC similarity exceeds the threshold. That IS resonance.
pub struct ResonanceQueryCell;

impl Cell for ResonanceQueryCell {
    fn name(&self) -> &str { "resonance-query" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let task_context = extract_task_context(&input)?;
        let query_vector = task_context.to_hdc_vector();
        let source_domain = task_context.domain();

        // Standard query_similar -- the Store protocol method that already exists.
        // The only difference from normal retrieval: we EXCLUDE the source domain
        // so that results are guaranteed cross-domain.
        let candidates = ctx.store().query_similar(
            &query_vector,
            QueryOptions {
                exclude_domains: vec![source_domain.clone()],
                min_similarity: CROSS_DOMAIN_THRESHOLD, // 0.526
                top_k: MAX_RESONANCES_PER_ENTRY,        // 5
                min_balance: 0.1, // skip near-dead Signals
                ..Default::default()
            },
        ).await?;

        // Each candidate IS a resonance. Package as output Signals.
        let resonances: Vec<Signal> = candidates
            .into_iter()
            .map(|hit| {
                Signal::new(Kind::Insight)
                    .with_tag("cross-domain-resonance")
                    .with_tag(&format!("source:{}", source_domain))
                    .with_tag(&format!("target:{}", hit.signal.domain()))
                    .with_metadata("similarity", hit.similarity)
                    .with_metadata("abstract_pattern",
                        extract_abstract_pattern(&query_vector, &hit.signal))
                    .with_lineage(vec![hit.signal.hash()])
            })
            .collect();

        Ok(resonances)
    }
}

const CROSS_DOMAIN_THRESHOLD: f32 = 0.526;
const MAX_RESONANCES_PER_ENTRY: usize = 5;
```

### 2.2 Why 0.526

The threshold 0.526 comes from the false positive mathematics (see `docs/06-neuro/09-false-positive-math.md`). For 10,240-bit binary vectors, the expected Hamming similarity between two independent random vectors is 0.500 with standard deviation ~0.005. At 0.526, the z-score is ~5.2, giving a per-comparison p-value of approximately 1e-7. With Bonferroni correction for 100K comparisons, the family-wise error rate stays below 1%.

The number is not tuned. It is a statistical guarantee. Any pair of independently drawn 10,240-bit vectors that shows similarity above 0.526 is, with 99%+ confidence, not a coincidence.

### 2.3 Analogical Reasoning as a Compose Cell

The v1 source describes analogical reasoning ("A is to B as C is to ?") as a special API. In the unified vocabulary, this is a Compose Cell -- it takes input Signals (the A, B, C terms) and composes them using HDC algebra to produce output Signals (the answer D and its confidence).

```rust
/// Analogical reasoning: "A is to B as C is to ?"
///
/// HDC encoding: relationship = bind(A, B), answer = bind(relationship, C).
/// The answer vector is similar to D iff A:B :: C:D holds structurally.
///
/// This is a Compose Cell because it assembles a new Signal from existing Signals
/// without external side effects.
pub struct AnalogyComposeCell {
    codebook: Arc<ItemMemory>,
}

impl Cell for AnalogyComposeCell {
    fn name(&self) -> &str { "analogy-compose" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Compose] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Input: three Signals carrying HDC vectors for A, B, C
        let (hv_a, hv_b, hv_c) = extract_triple(&input)?;

        // Capture the A->B relationship via XOR binding (self-inverse)
        let relationship = hv_a.bind(&hv_b);
        // Apply to C
        let answer = relationship.bind(&hv_c);

        // Query the codebook for the nearest known concept
        let matches = self.codebook.top_k(&answer, 5);

        let results: Vec<Signal> = matches
            .into_iter()
            .filter(|(_, sim)| *sim > CROSS_DOMAIN_THRESHOLD)
            .map(|(concept, similarity)| {
                Signal::new(Kind::Insight)
                    .with_tag("analogy-result")
                    .with_body(serde_json::json!({
                        "concept": concept,
                        "similarity": similarity,
                        "significant": similarity > CROSS_DOMAIN_THRESHOLD,
                    }))
            })
            .collect();

        Ok(results)
    }
}
```

The point: analogical reasoning is not a special API on `NeuroStore`. It is a Compose Cell in a Graph. It can be wired into any pipeline -- agent cognitive loops, dream consolidation, research workflows. Its inputs and outputs are Signals, so they compose with every other Cell.

---

## 3. Transfer Risk as Score + Verify

### 3.1 The Risk Score Cell

Transfer risk scoring is a Score Cell. It takes a candidate cross-domain resonance Signal and produces a risk score between 0.0 and 1.0.

```rust
/// Score protocol Cell: assess the risk of applying a cross-domain transfer.
///
/// Risk = weighted combination of:
///   - domain_distance (0.4 weight)
///   - 1 - historical_success (0.3 weight)
///   - similarity_risk (0.2 weight) -- paradoxically, very high similarity is risky
///   - 1 - causal_alignment (0.1 weight)
pub struct TransferRiskScoreCell {
    domain_profiles: Arc<RwLock<BTreeMap<String, DomainProfile>>>,
}

impl Cell for TransferRiskScoreCell {
    fn name(&self) -> &str { "transfer-risk-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let resonance = &input[0];
        let source_domain = resonance.tag_value("source").unwrap_or("unknown");
        let target_domain = resonance.tag_value("target").unwrap_or("unknown");
        let similarity: f32 = resonance.metadata_f32("similarity").unwrap_or(0.5);

        let profiles = self.domain_profiles.read().await;
        let distance = match (profiles.get(source_domain), profiles.get(target_domain)) {
            (Some(a), Some(b)) => compute_domain_distance(a, b),
            _ => DomainDistance::unknown(), // conservative default: 0.6
        };

        // Similarity paradox: very high cross-domain similarity (>0.58) suggests
        // surface structure match rather than deep analogy
        let similarity_risk = if similarity > 0.58 { 0.3 } else { 0.0 };

        let risk_score = (distance.combined * 0.4
            + (1.0 - distance.historical_success()) * 0.3
            + similarity_risk * 0.2
            + (1.0 - distance.causal_alignment()) * 0.1)
            .clamp(0.0, 1.0);

        // Annotate the Signal with the risk score
        let mut scored = resonance.clone();
        scored.set_score(Score {
            confidence: 1.0 - risk_score as f64,
            relevance: similarity as f64,
            novelty: if source_domain != target_domain { 1.0 } else { 0.0 },
            ..Default::default()
        });
        scored.set_metadata("transfer_risk", risk_score);
        scored.set_metadata("domain_distance", distance.combined);

        Ok(vec![scored])
    }
}
```

### 3.2 The Risk Verify Cell

After scoring, a Verify Cell makes the accept/reject decision. This is a standard Verify protocol Cell -- it returns `GateVerdict::Pass` or `GateVerdict::Fail` based on the scored risk.

```rust
/// Verify protocol Cell: gate cross-domain transfers by risk level.
///
/// Accept < 0.3, Caution 0.3-0.6, NeedsReview 0.6-0.8, Reject > 0.8.
pub struct TransferRiskVerifyCell;

impl Cell for TransferRiskVerifyCell {
    fn name(&self) -> &str { "transfer-risk-verify" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let signal = &input[0];
        let risk: f64 = signal.metadata_f64("transfer_risk").unwrap_or(1.0);

        let (verdict, action) = match risk {
            r if r < 0.3 => (GateVerdict::Pass, TransferAction::Accept),
            r if r < 0.6 => (GateVerdict::Pass, TransferAction::AcceptWithCaution {
                confidence_discount: 0.3,
            }),
            r if r < 0.8 => (GateVerdict::Fail, TransferAction::NeedsReview),
            _ => (GateVerdict::Fail, TransferAction::Reject),
        };

        let mut result = signal.clone();
        result.set_metadata("transfer_verdict", verdict);
        result.set_metadata("transfer_action", action);

        // For AcceptWithCaution, apply a heavy confidence discount
        if let TransferAction::AcceptWithCaution { confidence_discount } = action {
            let original_confidence = result.score().confidence;
            result.set_score(Score {
                confidence: original_confidence * confidence_discount,
                ..result.score()
            });
        }

        Ok(vec![result])
    }
}

enum TransferAction {
    Accept,
    AcceptWithCaution { confidence_discount: f64 },
    NeedsReview,
    Reject,
}
```

### 3.3 Wired as a Pipeline

The two Cells compose into a Pipeline Graph:

```toml
[graph]
name = "transfer-risk-pipeline"
kind = "Pipeline"

[[nodes]]
id = "score-risk"
cell = "roko:transfer-risk-score"
protocol = "Score"

[[nodes]]
id = "verify-risk"
cell = "roko:transfer-risk-verify"
protocol = "Verify"

[[edges]]
from = "score-risk"
to = "verify-risk"
```

This Pipeline is invoked wherever cross-domain Signals cross a boundary. It is not a global singleton -- each Space boundary can instantiate its own copy with different thresholds.

---

## 4. Three-Level Federation as Nested Spaces

### 4.1 The Core Decomposition

The Library of Babel's three levels are three nested Spaces. Each Space owns a Bus partition and a Store partition. The nesting defines trust boundaries.

```
Korai Space (global, public)
    Bus partition: korai:*
    Store partition: on-chain IInsightStore
    Trust: 0.60x confidence discount

    Mesh Space (collective, private)
        Bus partition: mesh:{collective-id}:*
        Store partition: shared JSONL + HDC index
        Trust: 0.80x confidence discount

        Agent Space (per-agent, private)
            Bus partition: agent:{agent-id}:*
            Store partition: local .roko/neuro/knowledge.jsonl
            Trust: 1.00x (self-distillation)
```

Each Space boundary is a point where the confidence Functor is applied (section 5). Knowledge flows upward (publishing) and downward (importing) through these boundaries, with different trust discounts in each direction.

### 4.2 Space Definitions

```rust
/// The three federation Spaces. Each is a standard Space specialization
/// with its own Bus partition and Store partition.
pub struct FederationSpaces {
    /// Per-agent local knowledge. Highest trust.
    pub agent: Space,
    /// Collective mesh. Shared among agents in the same Group.
    pub mesh: Option<Space>,
    /// Global public chain. Lowest trust.
    pub korai: Option<Space>,
}

impl FederationSpaces {
    pub fn new(agent_id: AgentId, group: Option<&Group>) -> Self {
        let agent = Space::new(SpaceConfig {
            id: SpaceId::from(agent_id),
            bus_partition: format!("agent:{}:", agent_id),
            store_path: PathBuf::from(".roko/neuro/knowledge.jsonl"),
            trust_level: 1.0,
            capabilities: Capabilities::all_local(),
        });

        let mesh = group.map(|g| Space::new(SpaceConfig {
            id: SpaceId::from(g.id),
            bus_partition: format!("mesh:{}:", g.id),
            store_path: g.store_path().join("knowledge.jsonl"),
            trust_level: 0.80,
            capabilities: Capabilities::mesh(),
        }));

        let korai = Space::new(SpaceConfig {
            id: SpaceId::korai(),
            bus_partition: "korai:".into(),
            store_path: PathBuf::from("chain"), // backed by ChainConnectorCell
            trust_level: 0.60,
            capabilities: Capabilities::chain_read(),
        });

        Self { agent, mesh, korai: Some(korai) }
    }

    /// Query across all available Spaces, applying confidence discounts
    /// at each boundary. Results are merged and ranked.
    pub async fn federated_query(
        &self,
        query: &HdcVector,
        opts: &QueryOptions,
    ) -> Result<Vec<ScoredHit>> {
        let mut results = Vec::new();

        // Local: full trust
        let local = self.agent.store().query_similar(query, opts).await?;
        results.extend(local);

        // Mesh: 0.80x discount
        if let Some(ref mesh) = self.mesh {
            let mesh_results = mesh.store().query_similar(query, opts).await?;
            results.extend(
                mesh_results.into_iter().map(|mut hit| {
                    hit.signal.discount_confidence(mesh.trust_level());
                    hit
                })
            );
        }

        // Korai: 0.60x discount, runs in parallel with local+mesh
        if let Some(ref korai) = self.korai {
            let chain_results = korai.store().query_similar(query, opts).await?;
            results.extend(
                chain_results.into_iter().map(|mut hit| {
                    hit.signal.discount_confidence(korai.trust_level());
                    hit
                })
            );
        }

        // Merge, deduplicate by content hash, rank by discounted confidence
        results.sort_by(|a, b| b.discounted_score().partial_cmp(&a.discounted_score())
            .unwrap_or(std::cmp::Ordering::Equal));
        results.dedup_by(|a, b| a.signal.hash() == b.signal.hash());

        Ok(results)
    }
}
```

### 4.3 Why Nesting Matters

The nesting is not decorative. It determines:

1. **Bus visibility**: A Pulse on `agent:alpha:knowledge.ingested` is visible only within the agent Space. To propagate to the mesh, it must be explicitly published to `mesh:{collective}:knowledge.shared`. This is the standard Bus partition isolation from [10-GROUPS.md](../../unified/10-GROUPS.md).

2. **Store isolation**: An agent's local Store is not readable by other agents or the mesh. Knowledge enters the mesh Store only through the publishing pipeline (section 7). This prevents accidental leakage of proprietary knowledge.

3. **Capability intersection**: Each Space carries capability grants. The agent Space grants `FsRead + FsWrite` to its own store. The mesh Space grants `Net` for peer communication. The Korai Space grants `Chain { read: true }`. A Cell running in one Space cannot exercise capabilities from a parent Space without explicit delegation.

4. **Taint re-evaluation**: When a Signal moves from one Space to another, its taint is re-evaluated at the boundary: `import_taint = join(original_taint, space_trust_level)` (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) S10). This means a `Confidential`-tainted Signal in the agent Space cannot leak to the mesh Space unless explicitly declassified.

---

## 5. Confidence Discounting as Functor

### 5.1 The Boundary Functor

Confidence discounting at Space boundaries is a Functor -- a cross-cut that transforms Signals pre/post without changing the Graph topology (see [cross-cut-functors.md](../07-agent-runtime/cross-cut-functors.md)). The Functor wraps the Store query at each boundary with a confidence multiplier.

```rust
/// Endofunctor F_boundary: Signal -> Signal
///
/// Applied at each Space boundary during federated retrieval.
/// Transforms the confidence score by the boundary's trust level.
///
/// Composition: F_mesh(F_korai(signal)) applies both discounts.
/// For a Signal that traversed korai -> mesh -> agent:
///   confidence = original * 0.60 * 0.80 = 0.48x
pub struct BoundaryConfidenceFunctor {
    /// Trust level of the Space being crossed into.
    trust_level: f64,
    /// Source channel identifier for provenance.
    source_channel: InflowChannel,
}

impl CrossCutFunctor for BoundaryConfidenceFunctor {
    fn name(&self) -> &str { "boundary-confidence" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // No pre-enrichment needed; discounting happens on output
        Ok(input)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        Ok(output.into_iter().map(|mut signal| {
            // Apply confidence discount
            let original = signal.score().confidence;
            signal.set_score(Score {
                confidence: original * self.trust_level,
                ..signal.score()
            });

            // Record provenance: which boundary was crossed
            signal.add_tag(&format!("inflow:{}", self.source_channel));
            signal.set_metadata("trust_discount", self.trust_level);

            signal
        }).collect())
    }
}

/// The five inflow channels from the source material,
/// each with its associated trust level.
#[derive(Debug, Clone, Copy)]
pub enum InflowChannel {
    SelfDistillation,   // 1.00x -- agent's own episodes
    CollectiveMesh,     // 0.80x -- same Group
    UserRestore,        // 0.85x -- human-directed import
    KoraiMarketplace,   // 0.60x -- public chain
    CrossCollective,    // 0.50x -- inter-Group exchange
}

impl InflowChannel {
    pub fn trust_level(&self) -> f64 {
        match self {
            Self::SelfDistillation => 1.00,
            Self::CollectiveMesh   => 0.80,
            Self::UserRestore      => 0.85,
            Self::KoraiMarketplace => 0.60,
            Self::CrossCollective  => 0.50,
        }
    }
}
```

### 5.2 Inheritance Discounting

When knowledge traverses multiple boundaries (agent A -> agent B -> agent C), confidence discounts compound geometrically: `confidence * 0.85^N`. This falls out naturally from functor composition. Each boundary application multiplies by the trust level. After N boundaries:

```
F_1(F_2(...F_N(signal)))

confidence_final = confidence_original * trust_1 * trust_2 * ... * trust_N
```

For the common case of N transfers through the mesh channel:

```
confidence = original * 0.85^N
```

| N transfers | Confidence remaining | Interpretation |
|---|---|---|
| 0 | 1.000 | Direct experience |
| 1 | 0.850 | One-hop peer knowledge |
| 2 | 0.722 | Two-hop (friend-of-friend) |
| 3 | 0.614 | Starting to lose fidelity |
| 5 | 0.444 | Less than half -- telephone game |
| 10 | 0.197 | Effectively noise floor |

The geometric decay is not a designed mechanism. It is a consequence of applying the same Functor at every boundary. The system does not track N explicitly -- it observes the confidence on the Signal, which already encodes the full chain of discounts.

### 5.3 The Functor Commutes

A critical property: the confidence Functor commutes with the transfer-risk Pipeline. It does not matter whether you discount confidence first and then score risk, or score risk first and then discount. The final accept/reject decision is the same because both operate on independent dimensions (confidence is a Signal-level score; risk is a domain-pair-level assessment).

This means the federated_query (section 4.2) can apply discounts eagerly during retrieval, and the transfer-risk Pipeline (section 3.3) can run afterward on the already-discounted results. No ordering constraint.

---

## 6. Four-Stage Ingestion as Pipeline Graph

### 6.1 The Pipeline

The four-stage ingestion pipeline from the source material (QUARANTINE -> CONSENSUS -> SKILL SANDBOX -> ADOPT) is a Pipeline Graph. Each stage is a Cell. Signals flow through the pipeline; any stage can reject the Signal.

```toml
[graph]
name = "knowledge-ingestion-pipeline"
kind = "Pipeline"

# Stage 1: Quarantine
[[nodes]]
id = "quarantine"
cell = "roko:quarantine-store"
protocol = "Store"
label = "Isolate + AntiKnowledge check + confidence discount"

# Stage 2: Consensus verification
[[nodes]]
id = "consensus"
cell = "roko:consensus-verify"
protocol = "Verify"
label = "Multi-agent confirmation for cross-domain entries"

# Stage 3: Skill sandbox
[[nodes]]
id = "sandbox"
cell = "roko:sandbox-verify"
protocol = "Verify"
label = "Test heuristics/strategies in sandboxed environment"

# Stage 4: Adopt into main Store
[[nodes]]
id = "adopt"
cell = "roko:adopt-store"
protocol = "Store"
label = "Admit to NeuroStore at Transient tier"

[[edges]]
from = "quarantine"
to = "consensus"

[[edges]]
from = "consensus"
to = "sandbox"

[[edges]]
from = "sandbox"
to = "adopt"
```

### 6.2 Stage Cells

**Quarantine (Store)**: Isolates the incoming Signal in a temporary Store partition. Runs AntiKnowledge similarity check -- if the incoming Signal's HDC vector is within 0.7 similarity of a known AntiKnowledge Signal, its initial balance is halved. If within 0.9, the Signal is rejected outright. Applies the BoundaryConfidenceFunctor for the inflow channel.

```rust
/// Stage 1: Quarantine. Temporary Store + AntiKnowledge guard.
pub struct QuarantineStoreCell {
    quarantine_store: Box<dyn Store>,
    main_store: Arc<dyn Store>,
}

impl Cell for QuarantineStoreCell {
    fn name(&self) -> &str { "quarantine-store" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut results = Vec::new();

        for signal in input {
            let hdc = signal.hdc_fingerprint()
                .ok_or(CellError::MissingField("hdc_fingerprint"))?;

            // Check against known AntiKnowledge in the main store
            let anti_hits = self.main_store.query_similar(
                hdc,
                &QueryOptions {
                    kinds: vec![Kind::AntiKnowledge],
                    min_similarity: 0.5,
                    top_k: 1,
                    ..Default::default()
                },
            ).await?;

            if let Some(hit) = anti_hits.first() {
                if hit.similarity > 0.9 {
                    // Reject: too similar to known-bad knowledge
                    ctx.bus().publish(Pulse::new(
                        "knowledge.ingestion.rejected",
                        json!({ "reason": "anti_knowledge_match",
                                "similarity": hit.similarity,
                                "anti_ref": hit.signal.hash() }),
                    )).await;
                    continue; // Signal is dropped
                }
                if hit.similarity > 0.7 {
                    // Discount: halve initial balance
                    let mut discounted = signal.clone();
                    discounted.set_balance(discounted.balance() * 0.5);
                    self.quarantine_store.put(discounted.clone()).await?;
                    results.push(discounted);
                    continue;
                }
            }

            // No AntiKnowledge concern: store in quarantine as-is
            self.quarantine_store.put(signal.clone()).await?;
            results.push(signal);
        }

        Ok(results)
    }
}
```

**Consensus (Verify)**: For cross-domain Signals (those tagged `cross-domain-resonance`), requires confirmation from 2+ independent agents. This is a Verify Cell that publishes confirmation requests on the Bus, waits for responses, and returns `Pass` only when quorum is reached.

```rust
/// Stage 2: Multi-agent consensus for cross-domain entries.
///
/// Publishes a confirmation request on the Bus, waits for 2+ independent
/// confirmations from agents with access to both domains.
/// Non-cross-domain entries pass through immediately.
pub struct ConsensusVerifyCell {
    required_confirmations: usize, // default: 2
    deadline: Duration,            // default: 5 minutes
}

impl Cell for ConsensusVerifyCell {
    fn name(&self) -> &str { "consensus-verify" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut passed = Vec::new();

        for signal in input {
            if !signal.has_tag("cross-domain-resonance") {
                // Non-cross-domain: pass through without consensus
                passed.push(signal);
                continue;
            }

            // Publish confirmation request on Bus
            let request_id = Uuid::new_v4();
            ctx.bus().publish(Pulse::new(
                "knowledge.confirmation.requested",
                json!({
                    "request_id": request_id,
                    "signal_hash": signal.hash(),
                    "source_domain": signal.tag_value("source"),
                    "target_domain": signal.tag_value("target"),
                    "similarity": signal.metadata_f32("similarity"),
                    "deadline_ms": self.deadline.as_millis(),
                }),
            )).await;

            // Wait for confirmations (or timeout)
            let confirmations = ctx.bus().collect(
                &format!("knowledge.confirmation.response.{}", request_id),
                self.deadline,
                self.required_confirmations,
            ).await;

            let confirmed_count = confirmations.iter()
                .filter(|p| p.payload_bool("confirmed").unwrap_or(false))
                .count();

            if confirmed_count >= self.required_confirmations {
                // Quorum reached
                let mut confirmed = signal.clone();
                confirmed.add_tag("consensus-confirmed");
                confirmed.set_metadata("confirmations", confirmed_count);
                passed.push(confirmed);
            } else {
                // Failed consensus: publish rejection event
                ctx.bus().publish(Pulse::new(
                    "knowledge.ingestion.consensus_failed",
                    json!({
                        "signal_hash": signal.hash(),
                        "confirmations": confirmed_count,
                        "required": self.required_confirmations,
                    }),
                )).await;
            }
        }

        Ok(passed)
    }
}
```

**Sandbox (Verify)**: For Heuristic and StrategyFragment Signals, runs them in a sandboxed environment. The sandbox Cell creates a temporary agent Space with restricted capabilities, executes the heuristic's `when/then` against synthetic inputs, and checks whether the prediction holds. Non-heuristic Signals pass through.

**Adopt (Store)**: Admits the Signal to the main NeuroStore at Transient tier with initial balance 1.0 (or the discounted balance from Quarantine). Publishes a `knowledge.ingested` Pulse on the Bus.

### 6.3 Immune Memory

The ingestion pipeline maintains an immune memory -- a Store of previously rejected Signals, stored as HDC vectors in a compact LSH index. When a new candidate arrives in Quarantine, it is checked against the immune memory in addition to AntiKnowledge. This prevents persistent re-injection: once a Signal is rejected, structurally similar Signals face elevated scrutiny.

```rust
/// Immune memory: compact record of previously rejected knowledge.
/// Stored as an LSH Bloom filter of rejected HDC vectors.
/// Checked during Quarantine to flag repeat injection attempts.
pub struct ImmuneMemory {
    /// Bloom filter of rejected vector hashes.
    /// False positive rate ~1% at 100K entries with 1MB memory.
    bloom: BloomFilter,
    /// Full vectors for similarity comparison on bloom-positive hits.
    rejected_vectors: Vec<(ContentHash, HdcVector, DateTime<Utc>)>,
    /// Immune memory decays: entries older than max_age are pruned.
    max_age: Duration, // default: 30 days
}

impl ImmuneMemory {
    /// Check if a candidate vector matches a previously rejected entry.
    pub fn check(&self, candidate: &HdcVector) -> ImmuneResponse {
        let hash = candidate.to_content_hash();
        if !self.bloom.might_contain(&hash) {
            return ImmuneResponse::Clear;
        }

        // Bloom positive: check full vectors for similarity
        for (ref_hash, ref_vector, rejected_at) in &self.rejected_vectors {
            let sim = candidate.similarity(ref_vector);
            if sim > 0.7 {
                return ImmuneResponse::PreviouslyRejected {
                    original_hash: *ref_hash,
                    similarity: sim,
                    rejected_at: *rejected_at,
                };
            }
        }

        ImmuneResponse::Clear // Bloom false positive
    }
}
```

---

## 7. Publishing as Bus Subscription Between Spaces

### 7.1 The Mechanism

Knowledge flows between Spaces via Bus subscriptions. This is the key insight that eliminates bespoke mesh sync protocols: publishing is a React Cell that subscribes to the local Bus and republishes qualifying Signals to the parent Space's Bus.

```rust
/// React protocol Cell: publishes qualifying knowledge to a parent Space.
///
/// Subscribes to the local Bus topic "knowledge.tier_promoted".
/// When a Signal reaches Consolidated tier, evaluates it against
/// the publishing policy. If it qualifies, republishes to the
/// parent Space's Store and Bus.
pub struct KnowledgePublisherCell {
    policy: PublishingPolicy,
    target_space: SpaceId,
}

impl Cell for KnowledgePublisherCell {
    fn name(&self) -> &str { "knowledge-publisher" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut published = Vec::new();

        for signal in input {
            // Apply publishing policy
            if !self.policy.should_publish(&signal) {
                continue;
            }

            // Strip private metadata before publishing
            let mut public_signal = signal.clone();
            public_signal.strip_tags(&["proprietary", "internal", "alpha"]);
            public_signal.strip_metadata(&["local_path", "agent_config"]);

            // Publish to target Space's Store
            ctx.space(&self.target_space)
                .store()
                .put(public_signal.clone())
                .await?;

            // Announce on target Space's Bus
            ctx.space(&self.target_space)
                .bus()
                .publish(Pulse::new(
                    "knowledge.shared",
                    json!({
                        "signal_hash": public_signal.hash(),
                        "kind": public_signal.kind(),
                        "domain": public_signal.domain(),
                        "confidence": public_signal.score().confidence,
                        "source_agent": ctx.agent_id(),
                    }),
                ))
                .await;

            published.push(public_signal);
        }

        Ok(published)
    }
}

/// Publishing policy from roko.toml.
pub struct PublishingPolicy {
    pub auto_publish: bool,
    pub publish_to: PublishTarget,        // Mesh, Korai, or Both
    pub allowed_kinds: Vec<Kind>,         // Insight, Heuristic, Warning, AntiKnowledge
    pub excluded_tags: Vec<String>,       // proprietary, internal, alpha
    pub min_confidence: f64,              // 0.7
    pub min_tier: Tier,                   // Consolidated
}

impl PublishingPolicy {
    pub fn should_publish(&self, signal: &Signal) -> bool {
        self.auto_publish
            && self.allowed_kinds.contains(&signal.kind())
            && signal.score().confidence >= self.min_confidence
            && signal.tier() >= self.min_tier
            && !signal.tags().iter().any(|t| self.excluded_tags.contains(t))
    }
}
```

### 7.2 Automatic Transfer via Bus

The design from the task prompt asks: "What if cross-domain transfer was automatic via Bus subscriptions between Spaces?"

It is. Here is the complete flow:

1. Agent A ingests a new knowledge Signal into its local Store.
2. The Signal's HDC fingerprint is computed on ingestion.
3. `query_similar` during SENSE returns cross-domain matches (section 2). These are cross-domain resonances.
4. Resonances that pass the transfer-risk Pipeline (section 3) are admitted.
5. When a Signal reaches Consolidated tier (via demurrage reinforcement + gate passes), the KnowledgePublisherCell (section 7.1) fires on the `knowledge.tier_promoted` Bus event.
6. The Signal is published to the mesh Space's Store.
7. Agent B, subscribing to `mesh:{collective}:knowledge.shared`, receives the Pulse.
8. Agent B's ingestion Pipeline (section 6) quarantines, validates, and potentially adopts the Signal.
9. If Agent B uses the Signal successfully (gate pass), its balance is reinforced, and it may eventually be published further -- to Korai.

No explicit "sync" protocol. No "push" or "pull" API. Knowledge flows through Bus subscriptions and Store operations that already exist. The three-level federation is an emergent property of Bus partition scoping and publishing policy configuration.

### 7.3 Cross-Domain as Automatic Side Effect

Here is the subtlety: cross-domain transfer does not require a separate mechanism from federation. When Agent B subscribes to mesh knowledge, it receives knowledge from all domains represented in the collective. During Agent B's SENSE step, `query_similar` on its local Store (which now includes mesh-imported Signals) naturally returns cross-domain matches.

The cross-domain resonance detection from section 2 fires on every `query_similar`. It does not care whether the cross-domain Signal was locally distilled or imported from the mesh. The mechanism is the same.

This means cross-domain transfer scales with the collective's diversity. A collective with agents in Rust, TypeScript, and DeFi domains naturally discovers structural analogies across all three -- not because a "resonance detector" was configured, but because `query_similar` with HDC similarity is the only retrieval mechanism in the system.

---

## 8. Domain Distance as a Score Cell in a Maintenance Loop

### 8.1 Three-Component Domain Distance

Domain distance quantifies how different two knowledge domains are. It drives the transfer risk assessment (section 3). The three components:

| Component | Measurement | Weight | Range |
|---|---|---|---|
| Vocabulary divergence | Jaccard distance of domain codebook concepts | 0.3 | 0.0 (identical) to 1.0 (disjoint) |
| Structural divergence | H-divergence of aggregate HDC vectors (Ben-David 2010) | 0.5 | 0.0 to 1.0 |
| Outcome correlation | Pearson correlation of gate pass rates on shared knowledge types | 0.2 | -1.0 to 1.0, mapped to [0, 1] distance |

Combined distance is the weighted geometric mean:

```
distance = vocab^0.3 * structural^0.5 * outcome_distance^0.2
```

### 8.2 As a Loop

Domain distance is not static. It changes as the collective gains experience. A pair of domains that starts with high distance (0.62 between DeFi and Research) may converge as agents discover transferable patterns. A pair that starts close (0.24 between Rust and TypeScript) may diverge if the agents specialize.

This is a Loop: observe domain pairs, compute distance, update the DomainProfile store, use updated distances in the next transfer risk assessment.

```toml
[graph]
name = "domain-distance-maintenance"
kind = "Loop"
min_interval = "1h"

[[nodes]]
id = "observe-outcomes"
cell = "roko:domain-outcome-observer"
protocol = "Observe"
label = "Collect gate outcomes for cross-domain transferred knowledge"

[[nodes]]
id = "score-distance"
cell = "roko:domain-distance-score"
protocol = "Score"
label = "Recompute 3-component domain distance for each pair"

[[nodes]]
id = "persist-profiles"
cell = "roko:domain-profile-store"
protocol = "Store"
label = "Update DomainProfile entries in the knowledge store"

[[edges]]
from = "observe-outcomes"
to = "score-distance"

[[edges]]
from = "score-distance"
to = "persist-profiles"

# Feedback: updated profiles feed into the next observation cycle
[[edges]]
from = "persist-profiles"
to = "observe-outcomes"
condition = "always"
```

The Loop runs at delta timescale (hourly by default). Each iteration updates the DomainProfile for every observed domain pair, which flows into the next cycle of transfer risk assessments.

---

## 9. The Abstract Role Hierarchy

### 9.1 Why Roles Enable Transfer

Cross-domain resonance works because HDC encoding uses role-filler bindings. The roles are the bridge. Two entries from different domains share structure when they bind the same abstract role with different domain-specific fillers.

The role hierarchy has two levels:

**Abstract roles** (6, shared across all domains):

| Role | Encodes | Example binding |
|---|---|---|
| `role:risk_factor` | What creates risk | `bind(role:risk_factor, hv_high_complexity)` |
| `role:response` | How to respond | `bind(role:response, hv_more_review)` |
| `role:pattern` | Observable signal | `bind(role:pattern, hv_increasing_churn)` |
| `role:severity` | How serious | `bind(role:severity, hv_medium)` |
| `role:temporal` | Time dimension | `bind(role:temporal, hv_accelerating)` |
| `role:confidence` | Certainty level | `bind(role:confidence, hv_high)` |

**Domain-specific roles** (unlimited, per-domain):

| Domain | Roles |
|---|---|
| Coding | `role:crate`, `role:function`, `role:module` |
| Chain | `role:protocol`, `role:asset`, `role:pool` |
| Research | `role:source`, `role:citation`, `role:method` |

Cross-domain similarity arises from the abstract roles. Domain-specific roles provide within-domain precision but are quasi-orthogonal across domains (their seed bytes differ, so `role:crate` and `role:protocol` produce independent vectors).

### 9.2 Encoding Example

```rust
/// Encode a knowledge entry with both abstract and domain-specific roles.
pub fn encode_knowledge(entry: &KnowledgeEntry, roles: &RoleRegistry) -> HdcVector {
    let mut components = Vec::new();

    // Abstract roles: enable cross-domain transfer
    if let Some(risk) = &entry.risk_factor {
        components.push(roles.abstract_role("risk_factor").bind(
            &HdcVector::from_seed(risk.as_bytes())
        ));
    }
    if let Some(response) = &entry.response {
        components.push(roles.abstract_role("response").bind(
            &HdcVector::from_seed(response.as_bytes())
        ));
    }
    if let Some(pattern) = &entry.pattern {
        components.push(roles.abstract_role("pattern").bind(
            &HdcVector::from_seed(pattern.as_bytes())
        ));
    }
    // ... severity, temporal, confidence

    // Domain-specific roles: within-domain precision
    for (role_name, filler) in &entry.domain_roles {
        components.push(roles.domain_role(role_name).bind(
            &HdcVector::from_seed(filler.as_bytes())
        ));
    }

    // Bundle all components into a single vector
    HdcVector::bundle(&components.iter().collect::<Vec<_>>())
}
```

### 9.3 The Confirmation Quorum

The source material recommends requiring 2+ independent agent confirmations before accepting a cross-domain resonance. With a per-agent false positive rate of ~1% (from the 0.526 threshold), requiring 2 of 3 agents to confirm reduces the joint false positive rate to approximately:

```
P(2+ of 3 confirm | no true resonance) = C(3,2) * 0.01^2 * 0.99 + C(3,3) * 0.01^3
                                        = 3 * 0.0001 * 0.99 + 0.000001
                                        ~ 2.97e-4
```

This is the ConsensusVerifyCell from section 6.2. It is not a separate confirmation protocol -- it is a stage in the ingestion Pipeline.

---

## 10. The Complete Wiring

Here is how all the pieces connect in a single agent's knowledge flow:

```
                         SENSE (cognitive loop step 2)
                              |
                    +---------+---------+
                    |                   |
              Local Store         Federated Query
            (query_similar)    (mesh + korai Spaces)
                    |                   |
                    +----> merge <------+
                              |
                    confidence Functor applied
                    at each Space boundary
                              |
                    cross-domain hits tagged
                    "cross-domain-resonance"
                              |
                    transfer-risk Pipeline
                    (Score + Verify)
                              |
                   +----------+----------+
                   |                     |
              Accept/Caution          Reject
                   |
            enter CognitiveWorkspace
            VCG auction with other bidders
                   |
              injected into system prompt
                   |
              agent executes task
                   |
              Verify evaluation
                   |
         +--------+---------+
         |                  |
   Verify pass        Verify fail
         |                  |
   reinforce balance    no reinforcement
   on context pack      (demurrage continues)
         |
   if tier promoted:
   KnowledgePublisherCell
   publishes to mesh/korai
         |
   other agents receive
   via Bus subscription
         |
   ingestion Pipeline
   (Quarantine -> Consensus -> Sandbox -> Adopt)
```

No bespoke federation machinery. No dedicated resonance detector. No mesh sync protocol. The system is composed of:

- Store protocol Cells (Memory, Quarantine, Adopt)
- Score protocol Cells (TransferRiskScore, DomainDistance)
- Verify protocol Cells (TransferRiskVerify, ConsensusVerify, SandboxVerify)
- React protocol Cells (KnowledgePublisher)
- Compose protocol Cells (AnalogyCompose)
- Functor pattern (BoundaryConfidenceFunctor)
- Pipeline pattern (ingestion pipeline, transfer-risk pipeline)
- Loop pattern (domain distance maintenance)
- Space pattern (agent, mesh, korai -- nested, with Bus + Store partitions)

---

## What This Enables

1. **Zero-configuration cross-domain transfer**. An agent that joins a diverse collective immediately benefits from structural analogies across all domains represented in the mesh. No setup. No "enable cross-domain mode." The mechanism is the retrieval itself.

2. **Automatic quality pressure via demurrage**. Transferred knowledge that is never used decays. Knowledge that is used and passes gates gets reinforced. The system converges on high-quality cross-domain transfers without explicit curation.

3. **Composable ingestion policies**. The four-stage Pipeline can be extended by adding new Verify stages (e.g., an LLM judge for semantic validation) or shortened by removing stages (e.g., skip consensus for intra-Group transfers). It is a Graph, not a hardcoded protocol.

4. **Emergent collective intelligence**. As more agents join a collective, the diversity of domains increases, which increases the surface area for cross-domain resonance, which increases the collective's ability to discover non-obvious structural analogies. This is Metcalfe's law applied to knowledge -- the value of the collective scales superlinearly with membership.

5. **Privacy by default**. Knowledge only leaves an agent's Space through the KnowledgePublisherCell, which applies a publishing policy. Proprietary knowledge, alpha-generating insights, and sensitive data stay local unless explicitly configured otherwise. The Space boundary is an information barrier.

6. **Graceful degradation of federation levels**. If the chain is unavailable, the agent still has local + mesh. If the mesh is unavailable, the agent still has local. The FederationSpaces struct handles `Option<Space>` for mesh and korai. No federation is required for the system to work -- it just means the agent sees a narrower knowledge base.

---

## Feedback Loops

### F1: Transfer Success Loop (per domain pair)

When a cross-domain transfer is accepted and the recipient agent uses the knowledge successfully (gate pass), the DomainDistance maintenance Loop (section 8.2) observes this outcome and updates the historical success rate for that domain pair. Higher success rate -> lower transfer risk score -> more transfers accepted in the future. Conversely, failed transfers increase domain distance and make future transfers harder.

**Convergence**: The Loop converges when domain pair distances stabilize (success rates stop changing). This happens when the system has discovered which transfers are genuinely useful and which are noise.

**Timescale**: Delta (hourly updates). The EMA window is 50 observations per domain pair by default.

### F2: Resonance-to-Heuristic Loop

A cross-domain resonance that repeatedly proves useful (cited in 5+ successful gate passes across different contexts) is a candidate for heuristic distillation during dream consolidation (D2 stage, see [06-MEMORY.md](../../unified/06-MEMORY.md) section 9). The distilled heuristic encodes the abstract structural pattern ("when risk increases, verify more") as a first-class testable claim with a falsifier.

Once crystallized as a heuristic, the pattern no longer depends on cross-domain similarity detection to be applied. It is retrieved directly by precondition matching. The resonance was the discovery mechanism; the heuristic is the durable outcome.

### F3: AntiKnowledge Immune Loop

When a cross-domain transfer is rejected (gate failure) and the rejection is confirmed by the ConsensusVerifyCell, the rejected Signal may be converted to AntiKnowledge. This AntiKnowledge entry then guards the Quarantine stage against future structurally similar Signals. The immune memory (section 6.3) provides a fast-path rejection for repeat offenders.

**Stability mechanism**: AntiKnowledge itself decays via demurrage (~30-day effective lifetime). Old mistakes eventually stop blocking new discoveries. If the environment changes and a previously bad transfer becomes useful, the AntiKnowledge decays and the transfer is re-evaluated.

### F4: Publishing Pressure Loop

Agents that publish high-quality knowledge to the mesh see their published Signals reinforced (via citations by mesh peers). Agents that publish noise see their Signals decay without reinforcement. Over time, the mesh Store naturally contains the collective's best knowledge -- high-quality publishers dominate, low-quality publishers' contributions fade.

On Korai, this is explicit: publishing costs KORAI tokens, and successful publications earn KORAI from citations. The economic pressure loop mirrors the demurrage-based quality pressure in the local Store.

### F5: Diversity-Driven Resonance Amplification

As a collective grows, it accumulates agents from more domains. More domains means more cross-domain pairs for resonance detection. More resonance means more transferred knowledge. More knowledge means each agent is more capable. More capable agents produce higher-quality work, attracting more participants.

This is a positive feedback loop bounded by:
- The transfer risk Pipeline rejecting low-quality transfers
- Demurrage decaying unused transferred knowledge
- The 0.526 similarity threshold filtering spurious resonances
- The confirmation quorum (2+ agents) blocking false positives

---

## Open Questions

### Q1: Should the resonance threshold adapt per domain pair?

The current threshold (0.526) is a global constant derived from HDC false positive mathematics. But domain pairs with high structural divergence (DeFi <-> Research, distance 0.62) may need a higher threshold to maintain the same false positive rate, while close pairs (Rust <-> TypeScript, distance 0.24) could use a lower threshold to discover more subtle analogies.

The mechanism would be a Loop that adjusts the threshold based on historical false positive rate per domain pair. The risk: if the threshold adapts too aggressively, the system may lock itself out of discovering genuinely novel cross-domain patterns.

### Q2: How should analogical reasoning interact with the VCG auction?

Currently, analogy results (section 2.3) are Insight Signals that enter the CognitiveWorkspace VCG auction alongside other knowledge bidders. But analogies have a unique property: they are compositionally derived from multiple source Signals. Should the analogy's bid be proportional to the calibration scores of its source terms? Should it be boosted by the diversity of domains involved?

The current design treats analogies as ordinary Insights. This may undervalue genuinely novel cross-domain connections.

### Q3: What is the right granularity for domain distance?

The source material defines domains as strings ("rust", "defi", "research"). But real knowledge does not partition cleanly into domains. "Smart contract testing in Rust" straddles Rust, DeFi, and testing. The current design computes domain distance between the Signal's primary domain and all other domains. A richer approach might use multi-label domain assignments with fractional weights.

### Q4: Can the ingestion Pipeline be self-modifying?

The current four-stage Pipeline is static. But the system could learn which stages are valuable for which inflow channels. If consensus verification never rejects anything from the local collective, it wastes latency. If sandbox verification always passes for Insight Signals (they have no executable component), the stage is unnecessary.

A meta-Loop that observes stage rejection rates and dynamically removes unnecessary stages would improve ingestion throughput. The risk is removing a safety stage just before it becomes critical.

### Q5: How does the confirmation quorum scale with collective size?

With 3 agents, requiring 2 confirmations is a reasonable quorum. With 100 agents, requiring 2 out of 100 is too permissive -- it approaches the single-agent false positive rate. Should the quorum scale logarithmically with collective size? Should it be weighted by the confirming agents' domain expertise?

### Q6: Missing Loop -- role vocabulary expansion

The 6 abstract roles (risk_factor, response, pattern, severity, temporal, confidence) are manually chosen. There is no Loop that discovers new abstract roles from data. If a structural pattern exists across domains that is not captured by any existing role, the system cannot detect it.

A dream-cycle process that analyzes high-confidence cross-domain transfers and identifies recurring structural components not explained by existing roles could propose new abstract roles. This is meta-learning: the system learns what to look for, not just what it finds.

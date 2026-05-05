# 26 — Cross-Cut Functors

> Cognitive cross-cuts (Memory, Daimon, Dreams) are endofunctors F: Signal -> Signal that transform the cognitive loop from the side. They do not occupy positions in the 7-step sequence — they modify it. Safety is a fourth endofunctor that operates at the capability level, outside VCG arbitration. Every cross-cut is a Cell specialization processing Signals through Bus and Store.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal, Pulse, demurrage, HDC fingerprint), [02-CELL](02-CELL.md) (Cell, protocols, predict-publish-correct), [03-GRAPH](03-GRAPH.md) (Graph composition), [05-AGENT](05-AGENT.md) (Agent lifecycle, cognitive loop), [06-MEMORY](06-MEMORY.md) (Knowledge Store, tiers, distillation), [16-SECURITY](16-SECURITY.md) (CaMeL IFC, capability grants)

---

## 1. Cross-Cuts Are Not Loop Steps

The 7-step cognitive loop (SENSE, ASSESS, COMPOSE, ACT, VERIFY, PERSIST/BROADCAST, REACT) is a sequential pipeline. Memory (neuro), Daimon (affect), and Dreams (offline consolidation) do not occupy positions in this sequence. They operate orthogonally — each one modifies the loop's behavior from the side, touching multiple steps simultaneously.

The precise structure: each cross-cut is an **endofunctor F: Signal -> Signal** that transforms Signals passing through the loop. When you apply Memory enrichment to SENSE, you are not adding a step before SENSE. You are replacing SENSE with F_memory(SENSE) — a version of SENSE that includes knowledge retrieval.

This distinction matters because:
1. **Cross-cuts compose independently.** You can enable Memory without Daimon, or Daimon without Dreams.
2. **Cross-cuts do not change the loop's topology.** The Graph TOML stays the same 7 nodes. Extension Cells inject at hook points within those nodes.
3. **Cross-cuts can be tested independently.** Test Memory injection by running SENSE with and without F_memory.

---

## 2. The Functorial Structure

### 2.1 Category of Signals

Define a category **Sig** where:
- Objects are typed Signal bundles (e.g., `Vec<Signal>` with a particular schema)
- Morphisms are Cells (Signal -> Signal transformations)
- Composition is Graph sequencing (Cell A's output feeds Cell B's input)
- Identity is the pass-through Cell (output = input)

### 2.2 Cross-Cuts as Endofunctors

An endofunctor F: **Sig** -> **Sig** maps:
- Each Signal to an enriched Signal: F(s) has additional metadata or content.
- Each Cell to an enriched Cell: F(cell) wraps the original Cell with pre/post hooks.

```rust
/// A cross-cut endofunctor. Wraps a Cell with pre/post enrichment.
///
/// F(cell).execute(input) =
///   pre_enrich(input)
///     -> cell.execute(enriched_input)
///       -> post_enrich(output)
trait CrossCutFunctor: Send + Sync + 'static {
    /// Identity: which cross-cut this is.
    fn name(&self) -> &str;

    /// Pre-enrichment: transform input Signals before the Cell runs.
    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError>;

    /// Post-enrichment: transform output Signals after the Cell runs.
    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError>;
}
```

### 2.3 The Three Functors

| Cross-Cut | Functor | F(Signal) | Injection Points |
|---|---|---|---|
| **Memory** | F_memory | Signal enriched with knowledge entries, HDC similarity scores, tier metadata | SENSE (knowledge retrieval), COMPOSE (context enrichment via VCG bids), VERIFY/REACT (consolidation feedback: reinforcement/weakening) |
| **Daimon** | F_daimon | Signal annotated with PAD bias, somatic markers, behavioral state | ASSESS (score bias via PAD + somatic markers, tier selection), ACT (action gating via prospect value, risk tolerance) |
| **Dreams** | F_dreams | Signal augmented with consolidated patterns, hypotheses, depotentiated affect | Delta speed (runs as its own loop); NREM replay + REM imagination + integration results feed into Memory and Daimon |

---

## 3. Memory as Endofunctor (F_memory)

### 3.1 F_memory on SENSE

Memory enriches SENSE by injecting durable knowledge into the perception phase. The endofunctor wraps the SENSE Cell:

```rust
struct MemoryEnrichSense {
    memory: Arc<MemoryCell>,
    max_entries: usize,
    similarity_threshold: f32,
}

impl CrossCutFunctor for MemoryEnrichSense {
    fn name(&self) -> &str { "memory.sense" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Query Memory for knowledge relevant to the current task
        let task_context = TaskContext::from_signals(&input)?;
        let hdc_query = task_context.to_hdc_vector();

        let knowledge = self.memory.query_by_similarity(
            &hdc_query,
            self.max_entries,
            self.similarity_threshold,
        ).await?;

        // Inject knowledge entries into the input Signal bundle
        let mut enriched = input;
        for entry in knowledge {
            enriched.push(entry.to_signal_with_metadata(SignalMetadata {
                source: Source::Memory,
                tier: entry.tier,
                similarity: entry.similarity_score,
                demurrage_balance: entry.balance,
            }));
        }

        Ok(enriched)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // No post-enrichment for SENSE
        Ok(output)
    }
}
```

### 3.2 F_memory on COMPOSE

Memory enriches COMPOSE by providing knowledge entries to the VCG auction. This is where Memory competes for token budget.

```rust
struct MemoryEnrichCompose {
    memory: Arc<MemoryCell>,
}

impl CrossCutFunctor for MemoryEnrichCompose {
    fn name(&self) -> &str { "memory.compose" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Memory contributes via NeuroBidder and HeuristicBidder
        // in the VCG auction. Pre-enrichment loads the relevant entries.
        let task = TaskContext::from_signals(&input)?;

        let knowledge_bids = self.memory.generate_bids(&task).await?;

        let mut enriched = input;
        for bid in knowledge_bids {
            enriched.push(bid.to_signal());
        }

        Ok(enriched)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        Ok(output)
    }
}
```

### 3.3 F_memory on REACT

After verification, Memory consumes the outcome to reinforce or weaken knowledge entries. Gate pass reinforces; gate fail weakens. This is the feedback loop that makes knowledge self-trimming via demurrage.

```rust
struct MemoryReact {
    memory: Arc<MemoryCell>,
}

impl CrossCutFunctor for MemoryReact {
    fn name(&self) -> &str { "memory.react" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        Ok(input)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let verdict = Verdict::from_signals(&output)?;

        // Gate pass: reinforce knowledge that was in context
        if verdict.passed() {
            let context_refs = ctx.get::<Vec<SignalRef>>("compose.included_refs")?;
            for r in context_refs {
                self.memory.reinforce(&r, ReinforcementKind::GatePass).await?;
            }
        }

        // Gate fail: weaken knowledge that was in context
        if verdict.failed() {
            let context_refs = ctx.get::<Vec<SignalRef>>("compose.included_refs")?;
            for r in context_refs {
                self.memory.weaken(&r, WeakeningKind::GateFail).await?;
            }
        }

        Ok(output)
    }
}
```

---

## 4. Daimon as Endofunctor (F_daimon)

### 4.1 F_daimon on ASSESS

The Daimon biases the ASSESS step by modulating Score weights and tier selection based on the PAD vector (Pleasure-Arousal-Dominance, Mehrabian 1996).

```rust
struct DaimonBiasAssess {
    daimon: Arc<DaimonState>,
}

impl CrossCutFunctor for DaimonBiasAssess {
    fn name(&self) -> &str { "daimon.assess" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let pad = self.daimon.current_pad();
        let behavioral_state = self.daimon.behavioral_state();

        // Inject PAD annotations into scored Signals
        let mut enriched = input;
        enriched.push(Signal::metadata("daimon.pad", pad.to_value()));
        enriched.push(Signal::metadata("daimon.state", behavioral_state.to_value()));

        // Somatic marker retrieval: recall how similar decisions felt
        let somatic_markers = self.daimon.retrieve_somatic_markers(
            &ctx.cortical().current_context_hash(),
            5,  // retrieve 5 nearest markers
        );

        // 15% mandatory contrarian retrieval
        let contrarian = self.daimon.retrieve_contrarian_markers(
            &pad,
            1,  // at least 1 contrarian marker
        );

        for marker in somatic_markers.iter().chain(contrarian.iter()) {
            enriched.push(marker.to_signal());
        }

        Ok(enriched)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // After ASSESS: check if Daimon wants to override tier selection
        let assessment = Assessment::from_signals(&output)?;
        let pad = self.daimon.current_pad();

        // Safety override: if PAD indicates high anxiety + low dominance,
        // escalate to higher tier regardless of EFE
        if pad.arousal > 0.5 && pad.dominance < -0.3 {
            let escalated = assessment.with_tier(
                assessment.tier.max(CognitiveTier::T2Reflective)
            );
            return Ok(escalated.into_signals());
        }

        Ok(output)
    }
}
```

### 4.2 F_daimon on ACT

The Daimon gates risky actions and applies prospect-theoretic value computation (Kahneman-Tversky). In the Cautious or Anxious behavioral state, high-risk actions are suppressed or deferred.

```rust
struct DaimonGateAct {
    daimon: Arc<DaimonState>,
}

impl CrossCutFunctor for DaimonGateAct {
    fn name(&self) -> &str { "daimon.act" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let action_plan = ActionPlan::from_signals(&input)?;
        let behavioral_state = self.daimon.behavioral_state();

        match behavioral_state {
            BehavioralState::Cautious | BehavioralState::Anxious => {
                // Check action risk level
                if action_plan.risk_level() > RiskLevel::Medium {
                    // Inject deferral signal: delay high-risk action
                    let mut enriched = input;
                    enriched.push(Signal::metadata(
                        "daimon.gate",
                        serde_json::json!({
                            "action": "defer",
                            "reason": "behavioral state does not support high-risk action",
                            "state": behavioral_state.as_str(),
                        }),
                    ));
                    return Ok(enriched);
                }
            }
            _ => {}
        }

        Ok(input)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // After ACT: update Daimon with outcome via prospect theory
        let result = ActionResult::from_signals(&output)?;

        // Prospect theory: asymmetric valuation (lambda = 2.25)
        let reference = ctx.get::<f64>("expected_reward").unwrap_or(0.5);
        let actual = result.reward().unwrap_or(0.5);
        let prospect_value = prospect_value(actual, reference);

        self.daimon.update_from_outcome(prospect_value);

        Ok(output)
    }
}

/// Kahneman-Tversky prospect value with lambda = 2.25.
/// Losses hurt 2.25x more than equivalent gains.
/// Diminishing sensitivity exponent = 0.88 (Tversky & Kahneman 1992).
fn prospect_value(outcome: f64, reference: f64) -> f64 {
    let delta = outcome - reference;
    if delta >= 0.0 {
        delta.powf(0.88)             // diminishing sensitivity to gains
    } else {
        -2.25 * (-delta).powf(0.88)  // loss aversion
    }
}
```

---

## 5. Dreams as Endofunctor (F_dreams)

Dreams differs from Memory and Daimon: it does not inject per-tick. Instead, it runs as its own delta-speed loop and publishes results that Memory and Daimon consume. The functorial structure is:

```
F_dreams: Signal -> Signal

F_dreams(episode) = consolidated_knowledge | hypothesis | depotentiated_affect
```

Dreams is an endofunctor that operates on a different timescale. Its output feeds into F_memory (consolidated knowledge entries) and F_daimon (depotentiated affect state).

### 5.1 The Three-Phase Dream Cycle as a Sub-Graph

```toml
[graph]
name = "dream-cycle"
version = "1.0.0"

[[graph.nodes]]
id = "nrem_replay"
cell = "roko.dreams.nrem_replay"
execution_class = "activity"

[[graph.nodes]]
id = "rem_imagination"
cell = "roko.dreams.rem_imagination"
execution_class = "activity"

[[graph.nodes]]
id = "integration"
cell = "roko.dreams.integration_staging"
execution_class = "activity"

[[graph.edges]]
from = "nrem_replay"
to = "rem_imagination"

[[graph.edges]]
from = "rem_imagination"
to = "integration"
```

### 5.2 NREM Replay Cell

Replays recent episodes ordered by prediction error magnitude (Mattar & Daw 2018: replay what is most useful for future decisions).

```rust
struct NremReplayCell {
    memory: Arc<MemoryCell>,
    episode_store: Arc<dyn Store>,
}

impl Cell for NremReplayCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Retrieve recent episodes, ordered by prediction error magnitude
        let episodes = self.episode_store
            .query(Query::recent_episodes(100))
            .await?;

        // Mattar & Daw (2018): replay what is most useful for future decisions
        let mut prioritized: Vec<(Signal, f64)> = episodes.iter()
            .map(|e| {
                let pe = e.get::<f64>("prediction_error").unwrap_or(0.0);
                (e.clone(), pe)
            })
            .collect();
        prioritized.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Extract patterns from high-PE episodes
        let mut extracted = Vec::new();
        for (episode, pe) in prioritized.iter().take(20) {
            let patterns = extract_patterns(episode, *pe);
            extracted.extend(patterns);
        }

        Ok(extracted)
    }
}
```

### 5.3 REM Imagination Cell

Generates hypotheses via HDC recombination (cross-domain structural analogies), counterfactual generation (Pearl 2009), and emotional depotentiation (Walker & van der Helm 2009).

```rust
struct RemImaginationCell {
    memory: Arc<MemoryCell>,
}

impl Cell for RemImaginationCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let patterns = Vec::<Signal>::from_signals(&input)?;

        let mut hypotheses = Vec::new();

        // HDC recombination: combine knowledge from different domains
        let entries = self.memory.random_sample(50).await?;
        for pair in entries.windows(2) {
            let bundled = hdc_bundle(&pair[0].hdc_vector, &pair[1].hdc_vector);
            let similarity = self.memory.nearest_to(&bundled, 3).await?;

            if similarity.iter().any(|s| s.distance < 0.3) {
                // Structural analogy found across domains
                hypotheses.push(Signal::hypothesis(
                    "cross_domain_analogy",
                    pair,
                    &similarity,
                ));
            }
        }

        // Counterfactual generation (Pearl 2009)
        for pattern in &patterns {
            let counterfactual = generate_counterfactual(pattern, ctx).await?;
            if let Some(cf) = counterfactual {
                hypotheses.push(cf);
            }
        }

        // Emotional depotentiation (Walker & van der Helm 2009)
        // Reduce affective charge of negative experiences
        for pattern in &patterns {
            if let Some(pad) = pattern.get::<PADState>("affect") {
                if pad.pleasure < -0.3 {
                    hypotheses.push(Signal::depotentiated(
                        pattern,
                        pad.with_pleasure(pad.pleasure * 0.5),
                    ));
                }
            }
        }

        Ok(hypotheses)
    }
}
```

### 5.4 Integration Staging Cell

The integration cell writes consolidated knowledge to Store and publishes depotentiated affect to Bus for Daimon consumption. This is where Dreams outputs become inputs to the other two cross-cuts.

---

## 6. Natural Transformations Between Cross-Cuts

The cross-cuts interact with each other through **natural transformations** — structure-preserving maps between functors. There are 6 natural transformations forming a fully connected triangle.

```
eta_MN : Memory -> Daimon     (knowledge outcomes update PAD)
eta_NM : Daimon -> Memory     (PAD assessment stored as knowledge)
eta_MD : Memory -> Dreams     (episodes provided for replay)
eta_DM : Dreams -> Memory     (consolidated knowledge stored)
eta_ND : Daimon -> Dreams     (PAD triggers consolidation)
eta_DN : Dreams -> Daimon     (depotentiation updates PAD)
```

### 6.1 The Commuting Triangle

For the system to stay consistent, the composition of transformations must commute:

```
Daimon --eta_NM--> Memory --eta_MD--> Dreams
  |                                     ^
  +-------------eta_ND-----------------+
```

The path Daimon -> Memory -> Dreams (knowledge outcomes are stored, then replayed in dreams) must produce the same result as Daimon -> Dreams (PAD directly triggers consolidation). This is enforced by the arbitration protocol: when both paths produce conflicting consolidation priorities, the arbitrator resolves based on the priority hierarchy.

### 6.2 Gate Failure Cascade — Full 7-Step Example

When a gate fails, the natural transformations fire in sequence, demonstrating how all three cross-cuts interact:

```
1. VERIFY emits: gate_failure Verdict Signal
       |
       v
2. F_memory(REACT): Memory weakens knowledge entries that were in context
       |                          (eta_MN: knowledge outcome -> PAD update)
       v
3. F_daimon(ASSESS next tick): PAD is now shifted (pleasure down, arousal up)
       |                          Daimon lowers escalation threshold
       v
4. F_daimon -> Dreams (eta_ND): If Daimon is Struggling, may trigger delta
       |
       v
5. Dreams NREM: Replays the failed episode with high priority
       |         (eta_MD: Memory provided the episode)
       v
6. Dreams -> Memory (eta_DM): New heuristic stored: "this approach fails for X"
       |
       v
7. Dreams -> Daimon (eta_DN): Depotentiation reduces negative affect from failure
```

This cascade is emergent from the functor composition rules, not hardcoded. Each step follows from the natural transformation definitions.

---

## 7. VCG Arbitration When Cross-Cuts Compete

When two or more cross-cuts produce conflicting recommendations for the same decision, the system resolves the conflict through a two-layer protocol.

### 7.1 Layer 1: Priority Hierarchy

Fixed priority ordering, applied first:

| Priority | Cross-cut | Rationale |
|---|---|---|
| 1 (highest) | Daimon | Safety constraints and behavioral gating override other concerns |
| 2 | Memory | Validated knowledge overrides speculative hypotheses |
| 3 (lowest) | Dreams | Dream-generated hypotheses are speculative |

```rust
fn resolve_by_priority(
    daimon: Option<Recommendation>,
    memory: Option<Recommendation>,
    dreams: Option<Recommendation>,
) -> Option<Recommendation> {
    // Daimon safety override: always wins if safety_critical
    if let Some(d) = &daimon {
        if d.safety_critical {
            return Some(d.clone());
        }
    }

    // Memory at Consolidated tier or higher overrides Dreams
    if let Some(m) = &memory {
        if m.knowledge_tier >= KnowledgeTier::Consolidated {
            if let Some(d) = &dreams {
                if d.conflicts_with(m) {
                    return Some(m.clone());
                }
            }
        }
    }

    // No clear priority resolution -> fall through to VCG
    None
}
```

### 7.2 Layer 2: VCG Auction (Tiebreaker)

When priority does not cleanly resolve the conflict, a VCG (Vickrey-Clarke-Groves) attention auction breaks the tie. Each cross-cut bids its confidence. The winner pays the second-highest bid (truthful reporting by mechanism design).

```rust
/// VCG auction for cross-cut arbitration.
///
/// Each cross-cut bids its confidence in its recommendation.
/// The winner pays the second-highest bid (truthful reporting).
struct VcgAuction;

impl VcgAuction {
    fn resolve(bids: &[(CrossCutId, f32, Recommendation)]) -> ArbitrationResult {
        if bids.is_empty() {
            return ArbitrationResult::NoConflict;
        }

        // Sort by bid value (confidence), descending
        let mut sorted = bids.to_vec();
        sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

        let winner = &sorted[0];
        let second_price = if sorted.len() > 1 { sorted[1].1 } else { 0.0 };

        ArbitrationResult::Resolved {
            winner: winner.0.clone(),
            recommendation: winner.2.clone(),
            attention_cost: second_price,
            runner_up: sorted.get(1).map(|b| b.0.clone()),
        }
    }
}
```

The VCG mechanism ensures truthful bidding: a cross-cut gains nothing by inflating its confidence because the price it "pays" (in attention cost) is determined by the second-highest bid.

### 7.3 When VCG Is Invoked

VCG tiebreaking activates **only** when:
1. Two cross-cuts are at the **same priority level** (both Memory and Dreams bidding on a COMPOSE slot).
2. Both have confidence **above 0.5** (low-confidence bids are ignored).
3. The conflict affects a **Route or Compose** decision (not safety decisions — those always go to Daimon).

### 7.4 Arbitration as a Cell

The arbitrator is implemented as a Cell specialization at layer L3 (Cognition) that intercepts the pipeline at ASSESS and COMPOSE:

```rust
struct CrossCutArbitrator {
    memory: Arc<MemoryCell>,
    daimon: Arc<DaimonState>,
    dreams: Arc<DreamState>,
}

impl Cell for CrossCutArbitrator {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Collect recommendations from each cross-cut
        let memory_rec = self.memory.recommend(&input)?;
        let daimon_rec = self.daimon.recommend(&input)?;
        let dreams_rec = self.dreams.recommend(&input)?;

        // Layer 1: priority hierarchy
        if let Some(resolved) = resolve_by_priority(
            daimon_rec.clone(),
            memory_rec.clone(),
            dreams_rec.clone(),
        ) {
            return Ok(resolved.into_signals());
        }

        // Layer 2: VCG auction
        let mut bids = Vec::new();
        if let Some(m) = memory_rec {
            bids.push((CrossCutId::Memory, m.confidence, m));
        }
        if let Some(d) = daimon_rec {
            bids.push((CrossCutId::Daimon, d.confidence, d));
        }
        if let Some(r) = dreams_rec {
            bids.push((CrossCutId::Dreams, r.confidence, r));
        }

        let result = VcgAuction::resolve(&bids);

        match result {
            ArbitrationResult::Resolved { recommendation, attention_cost, .. } => {
                // Log the arbitration for learning
                ctx.bus().publish(Pulse::arbitration_resolved(
                    &bids,
                    &recommendation,
                    attention_cost,
                )).await?;
                Ok(recommendation.into_signals())
            }
            ArbitrationResult::NoConflict => Ok(input),
        }
    }
}
```

---

## 8. Safety as Fourth Endofunctor (F_safety)

The three named cross-cuts (Memory, Daimon, Dreams) are the architectural ones. Safety is a fourth endofunctor that operates at a fundamentally different level.

### 8.1 F_safety: Signal -> Signal

Safety is an endofunctor that operates at the **capability level**, not the behavioral level:

- **Filters SENSE output**: Remove Signals that reference forbidden capabilities
- **Gates ASSESS decisions**: Reject route selections that violate safety contracts
- **Constrains COMPOSE**: Redact Signals with safety labels from prompt context
- **Blocks ACT**: Prevent tool calls that exceed capability grants
- **Augments VERIFY**: Add safety-specific verification criteria

```rust
struct SafetyFunctor {
    contracts: Vec<AgentContract>,
    capability_set: CapabilitySet,
}

impl CrossCutFunctor for SafetyFunctor {
    fn name(&self) -> &str { "safety" }

    async fn pre_enrich(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // CaMeL IFC: tag all Signals with capability provenance
        let tagged = input.into_iter()
            .map(|s| s.with_capability_tag(ctx.current_capability_scope()))
            .collect::<Vec<_>>();

        // Filter: remove Signals that require capabilities not in grant set
        let filtered = tagged.into_iter()
            .filter(|s| self.capability_set.permits(s.required_capabilities()))
            .collect();

        Ok(filtered)
    }

    async fn post_enrich(
        &self,
        output: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Verify output against safety contracts
        for contract in &self.contracts {
            for signal in &output {
                if !contract.permits(signal) {
                    tracing::warn!(
                        contract = %contract.name,
                        signal_hash = %signal.content_hash(),
                        "safety contract violation, filtering output"
                    );
                }
            }
        }

        let safe_output = output.into_iter()
            .filter(|s| self.contracts.iter().all(|c| c.permits(s)))
            .collect();

        Ok(safe_output)
    }
}
```

### 8.2 Safety vs Daimon: Different Levels

Safety and Daimon both claim high priority. The resolution: **Safety operates at the capability level** (what is permitted), **Daimon operates at the behavioral level** (what is advisable). Safety is a hard constraint; Daimon is a soft bias.

```
Safety:  "This tool call is not in the capability grant set. Blocked."
Daimon:  "This action is risky given current PAD state. Deferred."
```

**Safety does not participate in VCG arbitration.** It is a pre-filter that runs before the arbitrator, not a bidder within it. F_safety composes with the other functors as an outer wrapper:

```
F_total = F_safety . F_arbitrated(F_memory, F_daimon, F_dreams)
```

This means Safety never loses a "vote." It cannot be outbid. It is structurally prior to the cross-cut competition.

---

## 9. Feedback Loops

Five feedback loops ensure cross-cuts improve over time:

| Loop | What It Observes | What It Adjusts |
|---|---|---|
| **Memory reinforcement** | Gate pass/fail with knowledge entries in context | Demurrage balance of knowledge entries (reinforced on pass, weakened on fail). Entries that consistently lead to gate passes accumulate balance; unhelpful entries decay below cold threshold and are archived. |
| **Daimon adaptation** | Prospect-theory-weighted outcomes (lambda=2.25, alpha=0.88) | PAD vector (pleasure, arousal, dominance). Losses shift pleasure down + arousal up more than equivalent gains shift them up + down. 15% contrarian retrieval prevents echo chambers. |
| **Dream prioritization** | Prediction error magnitudes from Memory episodes | NREM replay ordering (highest PE replayed first, Mattar & Daw 2018). REM hypothesis generation rate (more hypotheses when PE variance is high). |
| **Arbitration calibration** | VCG auction outcomes correlated with downstream gate results | Bidder confidence calibration. If a cross-cut consistently wins auctions but its recommendations lead to gate failures, its confidence estimates are too high — the system applies a discount factor. |
| **Safety contract evolution** | Safety violations logged over time; false-positive rate | Contract refinement: tighten contracts with high violation rates, relax contracts with high false-positive rates. Manual review required for relaxation. |

---

## 10. Composition Order and Overhead

### 10.1 Functor Application Order

F_memory and F_daimon both enrich ASSESS. The default application order:

```
ASSESS_enriched = F_daimon(F_memory(ASSESS_raw))
```

F_daimon runs after F_memory, so Daimon biases scores that already include knowledge context. This order is intentional: Daimon's somatic markers operate on the fully-contextualized assessment.

### 10.2 Short-Circuit Optimization

Each cross-cut functor adds pre/post enrichment to relevant Cells. With 3 cross-cuts and 7 loop steps, the maximum is 42 enrichment calls per tick. In practice, short-circuiting reduces this:

- **F_memory** short-circuits when knowledge store is empty or query returns zero results
- **F_daimon** short-circuits when PAD vector is in the Neutral region (|P|, |A|, |D| all < 0.1)
- **F_dreams** short-circuits always (it does not inject per-tick; it runs on its own schedule)
- **F_safety** never short-circuits (safety is always active)

Typical overhead per tick: 2-4 active enrichment calls (not 42).

---

## 11. Acceptance Criteria

| Criterion | Verification |
|---|---|
| `CrossCutFunctor` trait defined with pre_enrich/post_enrich | Unit test: implement a test functor, verify pre/post composition |
| F_memory enriches SENSE with knowledge entries from HDC query | Integration test: populate store, run SENSE, verify knowledge in output |
| F_memory enriches COMPOSE via VCG bids | Integration test: verify NeuroBidder/HeuristicBidder participate in auction |
| F_memory REACT reinforces on gate pass, weakens on gate fail | Unit test: gate pass -> balance increases; gate fail -> balance decreases |
| F_daimon biases ASSESS with PAD vector and somatic markers | Unit test: set PAD to anxious, verify tier escalation |
| F_daimon gates ACT in Cautious/Anxious state | Unit test: Cautious + high-risk action -> defer Signal emitted |
| F_daimon applies prospect value (lambda=2.25) on ACT outcome | Unit test: loss of 0.5 -> prospect_value = -2.25 * 0.5^0.88 |
| F_dreams NREM replays episodes ordered by prediction error | Unit test: episodes with PE [0.1, 0.9, 0.5] -> replayed in [0.9, 0.5, 0.1] order |
| F_dreams REM generates cross-domain hypotheses via HDC bundling | Integration test: two domain entries -> bundled vector -> analogy found |
| F_dreams emotional depotentiation reduces negative PAD by 50% | Unit test: pleasure = -0.6 -> depotentiated to -0.3 |
| 6 natural transformations wired (eta_MN, eta_NM, eta_MD, eta_DM, eta_ND, eta_DN) | Integration test: gate failure cascade fires all 7 steps |
| Commuting triangle: Daimon->Memory->Dreams = Daimon->Dreams | Property test: both paths produce compatible consolidation results |
| Priority hierarchy: Daimon > Memory > Dreams | Unit test: conflicting recommendations resolved by priority |
| VCG invoked only when same level + both confidence > 0.5 | Unit test: different levels -> priority resolves; same level + one < 0.5 -> no VCG |
| VCG second-price mechanism correct | Unit test: bids [0.8, 0.6, 0.3] -> winner pays 0.6 |
| F_safety blocks capability violations before arbitration | Unit test: Signal requiring unauthorized capability -> filtered before VCG runs |
| F_safety does not participate in VCG | Structural test: SafetyFunctor not passed to VcgAuction |
| F_total = F_safety . F_arbitrated(F_memory, F_daimon, F_dreams) | Integration test: full pipeline with all 4 functors |
| Short-circuit: empty knowledge store -> F_memory is identity | Unit test: empty store, verify no enrichment overhead |
| Short-circuit: neutral PAD -> F_daimon is identity | Unit test: PAD = (0, 0, 0), verify no enrichment |
| Feedback: arbitration calibration discounts consistently-wrong bidder | Integration test: cross-cut wins 5 auctions, all lead to gate fail -> confidence discount applied |

---

## Version History

| Version | Date | Changes |
|---|---|---|
| 3.0 | 2026-04-26 | Unified spec: full functorial treatment with CrossCutFunctor trait, 6 natural transformations, commuting triangle, VCG arbitration protocol, Safety as 4th functor, 5 feedback loops, short-circuit optimization, acceptance criteria. |
| 2.0 | 2026-04-22 | Depth doc: cross-cut-functors.md with Rust code and category theory framing. |
| 1.0 | 2026-04-18 | Initial agent runtime cross-cut design. |

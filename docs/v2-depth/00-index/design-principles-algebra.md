# Design Principles as Algebraic Laws

> Depth for [00-INDEX.md](../../unified/00-INDEX.md) design principles and naming decisions. This doc derives each principle as an algebraic law over the three fundamentals (Signal, Cell, Graph), distinguishes structural enforcement from behavioral convention, identifies the missing principle, and adds cybernetic feedback.

---

## 1. The Three Fundamentals as an Algebra

The unified spec defines three fundamentals: Signal, Cell, Graph. See [01-SIGNAL.md](../../unified/01-SIGNAL.md), [02-CELL.md](../../unified/02-CELL.md), [03-GRAPH.md](../../unified/03-GRAPH.md).

These form an algebra with three sorts and defined operations:

```rust
// Sort 1: Signal -- the data element
// Operations: content_address(), score(), decay(), fingerprint(), graduate()
//
// Sort 2: Cell -- the computation element
// Operations: process(Signal) -> Signal, declare_protocols(), predict(), correct()
//
// Sort 3: Graph -- the composition element
// Operations: wire(Cell, Cell, TypeSchema), fire(Signal), snapshot(), resume()

// Key algebraic properties:
//
// Signal identity:    content_address(s) == content_address(t) iff s ≡ t
//                     (content addressing is faithful)
//
// Cell composition:   Cell_a ; Cell_b is a Cell
//                     (sequential composition is closed)
//
// Graph nesting:      Graph containing Graph is a Graph
//                     (Graphs are recursively composable)
//
// Signal flow:        Signal enters Cell, Signal exits Cell
//                     (Cells are endomorphisms on the Signal sort)
```

Every design principle can be stated as a law over this algebra. The principles are not guidelines -- they are invariants that the type system either enforces structurally or that convention must maintain behaviorally.

---

## 2. Deriving Each Principle

### Principle 1: Two mediums, two fabrics

**Algebraic statement**: The Signal sort has two subspecies with distinct lifetimes and storage semantics.

```
Signal = Signal_durable | Signal_ephemeral
Signal_durable  ∈ Store    (content-addressed, lineage-bearing, persisted)
Signal_ephemeral ∈ Bus     (sequence-numbered, ring-buffered, transient)

Graduation: Signal_ephemeral → Signal_durable  (the only injection into the audit DAG)
Projection: Signal_durable → Signal_ephemeral  (lossy broadcast, the reverse is not faithful)
```

**Enforcement**: **Structural**. The Rust type system separates `Signal` (durable) from `Pulse` (ephemeral). You cannot accidentally persist a Pulse or accidentally broadcast a Signal -- graduation and projection are explicit operations with different types.

```rust
// Structural enforcement via distinct types
struct Signal { /* content_hash, lineage, score, decay, ... */ }
struct Pulse  { /* sequence, topic, source, body, ttl, ... */ }

// Graduation is an explicit, auditable operation
impl Pulse {
    fn graduate(self, store: &dyn Store) -> Signal {
        // Assigns content_hash, lineage, score, decay
        // The ONLY path from transport to audit DAG
        store.put(Signal::from_pulse(self))
    }
}

// You cannot call store.put(pulse) -- type mismatch
// You cannot call bus.publish(signal) -- type mismatch
// The compiler enforces the two-medium invariant
```

**What this law prevents**: Mixing durable audit records with ephemeral coordination traffic. Without this law, the Store fills with transient noise, the Bus carries heavy lineage-bearing records, and neither fabric can be optimized for its actual access pattern.

### Principle 2: Every operator is a learner

**Algebraic statement**: Every Cell publishes a prediction Pulse before acting and subscribes to its own correction topic.

```
For all Cell c:
  c.process(input) =
    let prediction = c.predict(input)
    bus.publish(Pulse { topic: "prediction.{c.id}", body: prediction })
    let output = c.execute(input)
    // Somewhere else, a CalibrationPolicy joins prediction with outcome:
    // bus.subscribe("calibration.{c.id}.updated") feeds back corrections
    output
```

**Enforcement**: **Behavioral**. The type system cannot force a Cell to publish predictions. This is a protocol convention enforced by the `predict_publish_correct` contract in the Cell specification. A Cell that does not predict still compiles but does not learn.

```rust
// The predict-publish-correct contract (behavioral, not structural)
trait PredictPublishCorrect {
    fn predict(&self, input: &Signal) -> Pulse;
    fn correct(&mut self, error: &Pulse);
}

// A CalibrationPolicy Cell joins predictions with outcomes:
struct CalibrationPolicy;
impl React for CalibrationPolicy {
    fn react(&self, pulses: &[Pulse]) -> ReactOutput {
        // Join prediction.{cell_id} with outcome.{cell_id} by lineage_hint
        // Compute error = prediction - outcome
        // Publish calibration.{cell_id}.updated with error and adjustment
        // The originating Cell subscribes and calls self.correct(error)
    }
}
```

**What this law prevents**: Cells that are black boxes. Without predict-publish-correct, you cannot tell whether a Cell is improving, degrading, or static. Learning becomes a separate bolt-on system instead of emerging from the same pub/sub fabric that carries heartbeats and gate verdicts.

### Principle 3: Demurrage is default

**Algebraic statement**: Every durable Signal has a balance field that decreases over time unless reinforced by use.

```
For all Signal s in Store:
  s.balance(t) = s.balance(t_0) * exp(-demurrage_rate * (t - t_0)) + sum(reinforcements)

  where reinforcements occur when:
    - s is retrieved (cited in a Compose)
    - s is verified (passed a Verify gate)
    - s is surprising (high novelty per HDC fingerprint)
    - s is quoted (an Agent references s.content_hash)

  when s.balance < cold_threshold:
    s migrates to cold storage (slower, cheaper, still resolvable)
```

**Enforcement**: **Structural** (partially). The `balance` field exists on every Signal. The decay operation runs in Store.prune(). Reinforcement is structural for retrieval (Store.get() calls reinforce()) but behavioral for other sources.

```rust
struct Signal {
    // ... other fields ...
    balance: f64,           // starts at 1.0, decays unless reinforced
    demurrage_rate: f64,    // per-second holding cost
    last_reinforced: Timestamp,
}

impl Store for ConcreteStore {
    fn get(&self, hash: &ContentHash) -> Option<Signal> {
        let mut signal = self.inner_get(hash)?;
        // Structural: retrieval always reinforces
        signal.balance += RETRIEVAL_BONUS;
        signal.last_reinforced = now();
        self.inner_put(signal.clone());
        Some(signal)
    }

    fn prune(&self) {
        // Structural: decay runs on every prune cycle
        for signal in self.iter_mut() {
            let elapsed = now() - signal.last_reinforced;
            signal.balance *= (-signal.demurrage_rate * elapsed.as_secs_f64()).exp();
            if signal.balance < COLD_THRESHOLD {
                self.migrate_to_cold(signal);
            }
        }
    }
}
```

**What this law prevents**: Unbounded Store growth. Without demurrage, knowledge accumulates without pressure, search results dilute with stale entries, and the system cannot distinguish current insight from historical artifact. Demurrage is the algebraic dual of reinforcement learning: instead of rewarding good Signals, it taxes idle ones.

### Principle 4: Verify is load-bearing

**Algebraic statement**: Every Cell output traverses a Verify Cell before Store.put().

```
For all Cell c, Signal s where s = c.process(input):
  Store.put(s) is valid ONLY IF exists Verify Cell v such that:
    v.check(s) == Verdict::Pass(evidence)

  The Verdict is itself a Signal persisted in Store with lineage pointing to s.
  Skipping verification leaves a visible gap in the lineage DAG.
```

**Enforcement**: **Structural** (in the Graph wiring) + **Behavioral** (in single-Cell usage).

```rust
// In a Graph, the Verify Cell is a mandatory node between Act and Store
// The Graph schema validator rejects Graphs without a Verify node
// on any path from an Act Cell to a Store Cell.
fn validate_graph(graph: &Graph) -> Result<(), GraphError> {
    for path in graph.all_paths_from_act_to_store() {
        if !path.contains_cell_of_protocol(Protocol::Verify) {
            return Err(GraphError::MissingVerifyOnPath(path));
        }
    }
    Ok(())
}

// Verify serves four roles simultaneously:
trait Verify {
    // 1. Binary gate: pass or fail
    fn check(&self, signal: &Signal) -> Verdict;
    // 2. Continuous reward: domain-specific learning signal
    //    Verdict.reward: f64
    // 3. Pre-action veto: check BEFORE execution
    fn verify_pre(&self, signal: &Signal) -> Verdict;
    // 4. Economic attestation: reputation flows from verified work
}
```

**What this law prevents**: Hallucination amplification. Without mandatory verification, Agent B takes Agent A's unverified output as trusted input, errors compound through the lineage DAG, and the system cannot distinguish verified knowledge from confabulation.

### Principle 5: Budget-awareness

**Algebraic statement**: Every Compose Cell operates under a Budget constraint. No Cell may produce unbounded output.

```
For all Compose Cell c, Budget b:
  c.compose(signals, b) produces output where:
    output.token_count <= b.max_tokens
    output.signal_count <= b.max_signals
    output.wall_time <= b.max_wall_ms
    output.cost <= b.max_cost
```

**Enforcement**: **Structural**. The Compose protocol signature includes Budget as a required parameter.

```rust
trait Compose {
    fn compose(
        &self,
        inputs: &[Signal],
        budget: &Budget,     // REQUIRED -- cannot be omitted
        scorer: &dyn Score,
        ctx: &Context,
    ) -> Signal;
}

struct Budget {
    max_tokens: u32,
    max_signals: u32,
    max_bytes: u64,
    max_wall_ms: u64,
    max_cost_usd: f64,
}
// Budget has no Default impl -- you must specify constraints explicitly
```

**What this law prevents**: Context window overflow, unbounded inference cost, and runaway token generation. Budget-awareness is structural because the type system refuses to compile a Compose call without a Budget argument.

### Principle 6: Content-addressing

**Algebraic statement**: Signal identity is its content hash. Two Signals with identical content have identical hashes.

```
For all Signal s, t:
  content_address(s) == content_address(t) iff
    s.kind == t.kind AND s.body == t.body AND
    s.author == t.author AND s.tags == t.tags AND
    s.lineage == t.lineage
```

**Enforcement**: **Structural**. ContentHash is computed deterministically from identity fields. There is no `set_hash()` method.

```rust
impl Signal {
    pub fn content_hash(&self) -> ContentHash {
        // Deterministic: hash is a pure function of content
        // No external state, no randomness, no timestamps in hash input
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.kind.as_bytes());
        hasher.update(&self.body.to_bytes());
        hasher.update(self.author.as_bytes());
        for tag in &self.tags {
            hasher.update(tag.as_bytes());
        }
        for parent in &self.lineage {
            hasher.update(parent.as_bytes());
        }
        ContentHash(hasher.finalize())
    }
    // No pub fn set_content_hash() -- the hash is derived, not assigned
}
```

**What this law prevents**: Duplication, tampering, and lineage forgery. Content addressing gives deduplication, integrity verification, and lineage verification simultaneously.

### Principle 7: Observable by default

**Algebraic statement**: Every Cell invocation produces an observable trace. The Observe protocol (Lens) can read any layer without mutation.

```
For all Cell c, invocation i:
  exists Pulse p on topic "trace.{c.id}.{i.sequence}" such that:
    p contains: input_hash, output_hash, latency, cost, verdict_hash
```

**Enforcement**: **Behavioral**. The Cell runtime publishes trace Pulses, but the type system does not prevent a Cell implementation from suppressing them.

---

## 3. Structural vs Behavioral Enforcement Summary

| Principle | Algebraic Law | Enforcement |
|---|---|---|
| Two mediums | Signal = Durable \| Ephemeral, distinct types | **Structural** (type system) |
| Every operator learns | Cell publishes prediction, subscribes to correction | **Behavioral** (protocol convention) |
| Demurrage default | Signal.balance decays unless reinforced | **Structural** (field exists) + **Behavioral** (reinforcement sources) |
| Verify load-bearing | Every Act-to-Store path traverses Verify | **Structural** (Graph validation) |
| Budget-aware | Compose requires Budget parameter | **Structural** (type signature) |
| Content-addressed | Signal identity = content hash | **Structural** (derived field, no setter) |
| Observable | Every Cell invocation emits trace | **Behavioral** (runtime convention) |

The pattern: principles that constrain *data shape* (P1, P3, P5, P6) are structurally enforceable. Principles that constrain *behavior* (P2, P7) are conventionally enforced. Principle P4 is enforced structurally at the Graph level but behaviorally at the single-Cell level.

---

## 4. The Missing Principle: Heterogeneous Verification

The seven principles above protect against most failure modes. But one critical failure mode is not covered: **an LLM judging its own output**.

The MAST taxonomy (Cemri et al. 2025) shows 9.1% of multi-agent failures involve incorrect verification -- agents rationalizing their own output as correct. The current principles require verification (P4) but do not require that the verifier be *different* from the generator.

### Principle 8: Variance Inequality

**Algebraic statement**: For every Verify Cell v that checks output from Cell c, the spectral variance of v must be lower than the spectral variance of c.

```
For all Cell c (generator), Verify Cell v (verifier):
  variance(v) < variance(c)

  Practically:
  - If c uses LLM model M, v MUST NOT use model M
  - If c is stochastic, v should be deterministic when possible
  - If c operates on natural language, v should operate on
    formal artifacts (compilation, test execution, type checking)
```

**Enforcement**: **Behavioral** (protocol convention) + **Partially structural** (Graph validation can check that Verify Cells have different `connect_backend` declarations than their upstream Act Cells).

```rust
// Graph validation for heterogeneous verification
fn validate_variance_inequality(graph: &Graph) -> Result<(), GraphError> {
    for (act_cell, verify_cell) in graph.act_verify_pairs() {
        if act_cell.connect_backend() == verify_cell.connect_backend() {
            // Same backend = same spectral characteristics = violation
            return Err(GraphError::VarianceInequality {
                generator: act_cell.id(),
                verifier: verify_cell.id(),
                shared_backend: act_cell.connect_backend(),
            });
        }
    }
    Ok(())
}
```

**What this law prevents**: Self-verification loops where an LLM rationalizes its own hallucinations. The compile gate (deterministic, external subprocess) is a better verifier of code than the LLM that generated it, precisely because its spectral characteristics are different.

---

## 5. Principle Interactions

The principles are not independent. They form a lattice of mutual reinforcement:

```
Content-addressing (P6) enables:
  → Observable (P7): content hashes make traces tamper-evident
  → Verify (P4): lineage DAG provides causal replay for forensics

Two mediums (P1) enables:
  → Every operator learns (P2): predict-publish-correct needs Bus for Pulses
  → Demurrage (P3): only durable Signals decay; Pulses are transient by design

Budget-aware (P5) constrains:
  → Compose: directly (Budget in signature)
  → Route: indirectly (cheaper model preferred under budget pressure)
  → Verify: indirectly (gate pipeline has its own budget for subprocess execution)

Variance Inequality (P8) strengthens:
  → Verify (P4): verification is not just mandatory, it is heterogeneous
  → Every operator learns (P2): calibration is meaningful only if the
    verifier's signal is cleaner than the generator's noise
```

### 5.1 The irreducible set

Can any principle be derived from the others? Testing each:

- Remove P1 (two mediums): P2 (learning) breaks because predict-publish-correct needs ephemeral Pulses. **Not derivable.**
- Remove P2 (learning): the system still works but does not improve. **Not derivable** (learning is a goal, not a consequence).
- Remove P3 (demurrage): Store grows unboundedly; P5 (budget-aware) constrains Compose but not Store. **Not derivable.**
- Remove P4 (verify): P8 (variance inequality) has nothing to constrain. **Not derivable.**
- Remove P5 (budget): Compose produces unbounded output; no other principle limits token count. **Not derivable.**
- Remove P6 (content-address): P7 (observable) loses tamper evidence, P4 (verify) loses lineage verification. But P7 and P4 could still function without content addressing, just weaker. **Partially derivable** -- content addressing strengthens other principles but is not strictly required for them.
- Remove P7 (observable): the system becomes opaque but still functions. **Not derivable** (observability is a deployment requirement, not a functional one).
- Remove P8 (variance inequality): P4 (verify) still exists but allows self-verification. **Not derivable.**

The result: P6 (content-addressing) is the only principle that could theoretically be weakened without breaking the others, but the practical cost (loss of deduplication, integrity, and forensic replay) makes it load-bearing in practice.

---

## 6. Naming as Algebraic Convention

The naming decisions in [00-INDEX.md](../../unified/00-INDEX.md) are not arbitrary preferences. Each name reflects the algebraic role of the concept:

| Name | Algebraic Role | Why This Name |
|---|---|---|
| **Signal** (not Engram) | Element of the durable sort | "Signal" is immediately meaningful; Rust struct stays `Engram`, bridged by `type Signal = Engram` |
| **Pulse** (not Event) | Element of the ephemeral sort | Names the distinct lifetime; "Event" is overloaded across every framework |
| **Cell** (not Module) | Morphism in the Signal algebra | Smallest composable unit; "Module" implies larger granularity |
| **Graph** (not Workflow) | Composition of morphisms | Mathematical precision; "Workflow" implies BPM linearity |
| **Store** (not Substrate) | Durable fabric | Protocol name matches verb (Store.put); trait name stays `Substrate` in code |
| **Bus** (not EventBus) | Ephemeral fabric | Kernel-level alongside Store; "EventBus" implies implementation detail |
| **Demurrage** (not Decay) | Active economic mechanism | "Decay" implies passive time loss; "demurrage" implies use restores value |
| **Lens** (not Monitor) | Read-only observation | Stacking Lenses gives different views; "Monitor" implies mutation capability |
| **Loop** (not Feedback) | Graph with feedback edge | Direct and unambiguous; Graph terminology, not cybernetics jargon |

---

## 7. What This Enables

1. **Machine-checkable principles** -- structural principles are enforced by the Rust type system; behavioral principles can be checked by Graph validation and CI lints
2. **Principle completeness argument** -- the eight principles form an irreducible set; removing any one breaks a capability that no other principle provides
3. **New-feature evaluation** -- before adding a feature, check which principles it satisfies and which it violates; if it violates a structural principle, it cannot compile; if it violates a behavioral principle, it needs explicit justification

## 8. Feedback Loops

| Loop | What It Checks | Cadence |
|---|---|---|
| Structural enforcement CI | Graph validation: all Act-to-Store paths have Verify, variance inequality holds | Every commit |
| Behavioral compliance Lens | Fraction of Cells publishing predictions on `prediction.*` topics | Theta (plan-level) |
| Demurrage calibration Loop | Are useful Signals being cold-archived prematurely? Are stale Signals being retained too long? | Delta (hourly) |
| Principle violation tracking | Count of principle violations detected in code review or runtime | Theta (weekly) |

## 9. Open Questions

1. **Can behavioral principles become structural?** If Rust gains effect systems (algebraic effects), the predict-publish-correct contract could become a type-level requirement. Every Cell would declare its effects, and a Cell without a `Predict` effect would be a compile error. This is theoretical but architecturally significant.

2. **Is content-addressing too strong?** P6 requires that identity fields include `lineage`. This means the same body text with different parents produces different content hashes. Is this the right semantics for all Signal kinds? For Heuristics (where the lineage tracks provenance), yes. For raw data imports (where lineage is artificial), possibly too strict.

3. **Demurrage rate discovery**: What is the right demurrage rate for different Signal kinds? The algebra says "every Signal decays," but the rate is a parameter. Too fast and useful knowledge vanishes; too slow and the Store bloats. The Theta-cadence calibration Loop (section 8) addresses this, but the initial rates need empirical tuning.

4. **Variance Inequality measurement**: How do you measure spectral variance of a verifier vs a generator in practice? For deterministic verifiers (compiler, test suite), the variance is zero -- clearly satisfying the inequality. For LLM-based judges, the variance depends on the model, temperature, and prompt. A practical proxy: the verifier's self-consistency on repeated inputs should exceed the generator's.

5. **Missing behavioral enforcement**: The "observable by default" principle (P7) has no enforcement mechanism beyond convention. A Cell can suppress trace Pulses and no one notices until debugging a production incident. Should there be a `TraceLens` that alerts when a Cell goes silent?

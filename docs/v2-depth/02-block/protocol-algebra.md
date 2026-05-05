# Protocol Algebra

> Depth for [02-CELL.md](../../unified/02-CELL.md). Derives the categorical structure of the 9 protocols: Cells as objects, typed Signal/Pulse flows as morphisms, composition rules as functorial constraints, and the free monad over the protocol vocabulary.

---

## 1. The Category of Cells

The 9 protocols are not an ad hoc list. They form a category **Cell** where:

- **Objects** are Cells (each carrying typed I/O schemas, capability sets, and protocol conformance declarations).
- **Morphisms** are typed Signal/Pulse flows between Cells. A morphism `f: A -> B` exists when `A.output_schema` is compatible with `B.input_schema` under `TypeSchema::is_compatible`.
- **Composition** is sequential piping: if `f: A -> B` and `g: B -> C`, then `g . f: A -> C` is the pipeline that feeds A's output into B, then B's output into C.
- **Identity** is the trivial passthrough Cell that emits its input unchanged.

This is a concrete category, not a metaphor. The typing discipline is enforced at Graph construction time (see [03-GRAPH.md](../../unified/03-GRAPH.md)), and the composition rules below determine which edges are legal.

### 1.1 Morphism Well-Formedness

A morphism (edge) from Cell `A` to Cell `B` is well-formed when three conditions hold simultaneously:

```rust
/// An edge A -> B is valid iff:
fn edge_valid(a: &dyn Cell, b: &dyn Cell, space: &SpacePolicy) -> bool {
    // 1. Type compatibility: A's output fits B's input
    let type_ok = match (a.output_schema(), b.input_schema()) {
        (_, None) => true,                          // B accepts anything
        (None, Some(_)) => false,                   // A emits anything, B is specific
        (Some(out), Some(inp)) => out.is_compatible(inp),
    };

    // 2. Capability intersection is non-empty across all three layers
    let cap_ok = {
        let a_eff = a.capabilities().effective();
        let b_eff = b.capabilities().effective();
        // The composed pipeline needs the union of both capability sets
        // to be satisfiable within the Space
        let union = a_eff.union(&b_eff);
        space.permits_all(&union)
    };

    // 3. Protocol compatibility: the output protocol of A can feed the input
    //    protocol of B (see the protocol morphism table in S1.3)
    let proto_ok = protocol_composable(a.protocols(), b.protocols());

    type_ok && cap_ok && proto_ok
}
```

This is **Cell composition = Graph edge validation at the type level**, enforced before any Flow begins execution.

### 1.2 TypeSchema as a Preorder

`TypeSchema` forms a preorder (reflexive, transitive) under compatibility:

```text
Any >= OfKind(k) >= JsonSchema(s) >= Record({...})
                                   >= ArrayOf(s')

OneOf([a, b]) is compatible with target T
  iff EVERY variant is compatible with T.
  (conservative: we cannot know at construction time which variant fires)

AllOf([a, b]) is compatible with target T
  iff ANY component is compatible with T.
  (any satisfied component suffices)
```

This preorder is the subtyping lattice. `Any` is top (accepts everything), specific schemas are lower. Compatibility is checked by walking the lattice -- see [02-CELL.md](../../unified/02-CELL.md) S3.1 for the `is_compatible` implementation.

The lattice enables **coercion morphisms**: when `A` outputs `OfKind(CodeDiff)` and `B` accepts `Any`, there is an implicit coercion. When `A` outputs `Any` and `B` requires `OfKind(CodeDiff)`, the edge is rejected at Graph load time. This is fail-closed by design.

---

## 2. Protocol Morphism Table

Not all protocol-to-protocol compositions are meaningful. The table below defines which Cell-protocol pairings can legally form edges:

```text
Source protocol -> Target protocol    Legal?    Morphism type
--------------------------------------------------------------------
Store  -> Score                       Yes       Query -> Rate
Store  -> Verify                      Yes       Retrieve -> Check
Store  -> Route                       Yes       Retrieve -> Select
Store  -> Compose                     Yes       Retrieve -> Assemble
Store  -> React                       No        (Store is pull, React is push)
Store  -> Observe                     Yes       Retrieve -> Read
Store  -> Connect                     No        (Store is internal)
Store  -> Trigger                     No        (Store is passive)

Score  -> Route                       Yes       Rate -> Select (scored ranking)
Score  -> Verify                      Yes       Rate -> Check (quality predicts pass)
Score  -> Compose                     Yes       Rate -> Budget allocation
Score  -> React                       No        (Score is synchronous, React is stream)
Score  -> Score                       Yes       Cascade scoring

Verify -> React                       Yes       Verdict -> React (reward signal)
Verify -> Route                       Yes       Verdict -> Select (learn from outcomes)
Verify -> Score                       Yes       Verdict -> Recalibrate (correction)
Verify -> Compose                     No        (Verdict is terminal, not material)
Verify -> Store                       Yes       Verdict -> Persist (audit)

Route  -> Compose                     Yes       Selection -> Assemble context
Route  -> Verify                      Yes       Selection -> Check (pre-verify)
Route  -> Connect                     Yes       Selection -> Dispatch (tool/model)

Compose -> Verify                     Yes       Assembled -> Check (the golden path)
Compose -> Connect                    Yes       Assembled -> Send (prompt -> LLM)
Compose -> Store                      Yes       Assembled -> Persist

React  -> Store                       Yes       Graduate (Pulse -> Signal)
React  -> React                       Yes       Chain reactions
React  -> Compose                     No        (React is ephemeral-first)

Observe -> Score                      Yes       Observation -> Rate
Observe -> React                      Yes       Observation -> React (telemetry)
Observe -> Route                      Yes       Observation -> Select (regime detection)

Connect -> Verify                     Yes       External result -> Check
Connect -> Store                      Yes       External result -> Persist
Connect -> React                      Yes       External event -> React

Trigger -> React                      Yes       Fire event -> React
Trigger -> Compose                    Yes       Fire event -> Assemble context
Trigger -> Route                      Yes       Fire event -> Select handler
```

### 2.1 Encoding as an Adjacency Matrix

```rust
/// Protocol adjacency matrix. True = legal composition.
/// Indexed by [source_protocol][target_protocol].
const PROTOCOL_ADJACENCY: [[bool; 9]; 9] = {
    //          Store Score Verfy Route Comps React Obsrv Conct Trigr
    /* Store */ [true, true, true, true, true, false,true, false,false],
    /* Score */ [false,true, true, true, true, false,false,false,false],
    /* Verfy */ [true, true, false,true, false,true, false,false,false],
    /* Route */ [false,false,true, false,true, false,false,true, false],
    /* Comps */ [true, false,true, false,false,false,false,true, false],
    /* React */ [true, false,false,false,false,true, false,false,false],
    /* Obsrv */ [false,true, false,true, false,true, false,false,false],
    /* Conct */ [true, false,true, false,false,true, false,false,false],
    /* Trigr */ [false,false,false,true, true, true, false,false,false],
};

fn protocol_composable(src: &[ProtocolId], tgt: &[ProtocolId]) -> bool {
    // At least one protocol pair must be composable
    src.iter().any(|s| {
        tgt.iter().any(|t| {
            PROTOCOL_ADJACENCY[*s as usize][*t as usize]
        })
    })
}
```

This adjacency matrix is checked at Graph load time. An edge that violates it is a static error, not a runtime surprise.

---

## 3. Natural Transformations Between Protocols

Several protocol-to-protocol relationships are not arbitrary -- they are natural transformations in the categorical sense. A natural transformation `eta: F => G` means there is a systematic, structure-preserving way to go from one protocol's output to another protocol's input, independent of the specific Cell instances involved.

### 3.1 Score => Verify (Quality Prediction to Pass/Fail Verdict)

The most important natural transformation. Every Score Cell's output (a 5-dimensional `Score` vector) can be systematically lifted into a `Verdict`:

```rust
/// Natural transformation: Score => Verify
///
/// Lifts a quality prediction (continuous Score) to a pass/fail
/// verdict (binary Verdict). This is the bridge between "how good
/// is this?" and "should this proceed?"
fn score_to_verdict(score: &Score, thresholds: &VerifyThresholds) -> Verdict {
    // Hard criteria: each score dimension has a minimum threshold.
    // Falling below ANY threshold is a hard fail.
    let hard_criteria = vec![
        CriterionResult {
            criterion: Criterion::RelevantToTask,
            passed: score.relevance >= thresholds.min_relevance,
            score: score.relevance,
            evidence_refs: vec![],
        },
        CriterionResult {
            criterion: Criterion::ClippyClean,
            passed: score.quality >= thresholds.min_quality,
            score: score.quality,
            evidence_refs: vec![],
        },
    ];

    let hard_pass = hard_criteria.iter().all(|c| c.passed);

    // Soft criteria: remaining dimensions form the Pareto surface.
    // No weighted sum -- each is an independent optimization axis.
    let soft_criteria = vec![
        CriterionResult {
            criterion: Criterion::ConsistentWithContext,
            passed: true, // soft criteria don't "fail" -- they rank
            score: score.novelty,
            evidence_refs: vec![],
        },
        CriterionResult {
            criterion: Criterion::Custom {
                name: "utility".into(),
                description: "task-specific utility estimate".into(),
            },
            passed: true,
            score: score.utility,
            evidence_refs: vec![],
        },
    ];

    // Reward is the geometric mean of score dimensions
    // (geometric mean penalizes any single zero dimension more than
    // arithmetic mean -- a single catastrophic dimension tanks reward)
    let reward = (score.relevance * score.quality * score.confidence
        * score.novelty * score.utility)
        .powf(1.0 / 5.0);

    Verdict {
        reward,
        hard_pass,
        hard_criteria,
        soft_criteria,
        evidence: vec![],
        duration: Duration::ZERO,
        explanation: None,
    }
}
```

**Why this is natural**: The transformation commutes with Cell composition. If you score-then-verify or verify-directly, the pass/fail boundary is the same (given the same thresholds). Naturality means Score Cells can be interposed before Verify Cells without changing the verification semantics -- they only add early rejection (performance) and continuous reward (learning).

### 3.2 Verify => React (Verdict to Reactive Response)

A Verdict systematically transforms into a React input:

```rust
/// Natural transformation: Verify => React
///
/// Every Verdict becomes a Pulse on the Bus, enabling reactive
/// downstream behavior without direct coupling to the Verify Cell.
fn verdict_to_pulse(verdict: &Verdict, block_ref: &CellRef) -> Pulse {
    Pulse {
        topic: Topic::from(format!(
            "verify.verdict.{}",
            if verdict.hard_pass { "passed" } else { "failed" }
        )),
        kind: Kind::from("verdict"),
        body: Body::json(verdict),
        source: PulseSource {
            component: format!("verify:{}", block_ref.name),
            agent_id: None,
        },
        // lineage_hint connects the Verdict Pulse to the Signal
        // that was verified, enabling causal reconstruction
        lineage_hint: Some(block_ref.id.clone()),
        ..Default::default()
    }
}
```

This transformation is what makes the predict-publish-correct Loop work (see [02-CELL.md](../../unified/02-CELL.md) S3.10): Verify results flow through the Bus as Pulses, React Cells consume them, and calibration updates propagate without any Cell needing to know about any other Cell.

### 3.3 Store <=> React (Graduation and Projection)

These are dual natural transformations forming an adjunction (see S4 below):

- **Graduation** (`React => Store`): A React Cell's `ReactOutput.signals` are persisted via Store. This is the Pulse-to-Signal bridge.
- **Projection** (`Store => React`): A Store write emits a notification Pulse on `store.signal.written`, which React Cells can subscribe to. This is the Signal-to-Pulse bridge.

```rust
/// Graduation: ReactOutput -> StoreProtocol::put
async fn graduate(output: &ReactOutput, store: &dyn StoreProtocol) -> Result<Vec<SignalRef>> {
    let mut refs = Vec::with_capacity(output.signals.len());
    for signal in &output.signals {
        let r = store.put(signal.clone()).await?;
        refs.push(r);
    }
    Ok(refs)
}

/// Projection: Store write -> Bus Pulse
async fn project(signal: &Signal, ref_: &SignalRef, bus: &dyn Bus) -> Result<()> {
    let pulse = Pulse {
        topic: Topic::from("store.signal.written"),
        kind: signal.kind.clone(),
        body: Body::json(&SignalWriteEvent {
            ref_: ref_.clone(),
            kind: signal.kind.clone(),
            tags: signal.tags.clone(),
        }),
        ..Default::default()
    };
    bus.publish(pulse).await.map(|_| ())
}
```

---

## 4. The Store-Bus Adjunction

Graduation and Projection are not merely useful bridges. They form an **adjunction** `F -| G` where:

- `F: Bus -> Store` is graduation (ephemeral Pulse becomes durable Signal)
- `G: Store -> Bus` is projection (durable Signal becomes notification Pulse)

The adjunction means: for every React Cell `R` that produces Pulses, and every Store Cell `S` that consumes Signals, there is a natural bijection:

```text
Hom_Bus(Pulses, G(Signals))  ~=  Hom_Store(F(Pulses), Signals)
```

In plain terms: subscribing to store-write notifications on the Bus is equivalent to querying the Store for recently graduated Signals. The two perspectives yield the same information, connected by the unit and counit of the adjunction.

**Unit** (eta): Pulse -> project(graduate(Pulse)). You graduate a Pulse to a Signal, then the Store projects it back as a notification Pulse. The result is a "cleaned" Pulse: stripped of ephemerality, enriched with a SignalRef and content hash.

**Counit** (epsilon): Signal -> graduate(project(Signal)). You project a Signal as a Pulse, then some downstream React Cell graduates it. If the Signal was already durable, the result is idempotent (same content hash).

### 4.1 Why This Matters for Consistency

The adjunction guarantees that the system never loses information at the Store/Bus boundary:

- Every graduated Signal is observable on the Bus (via projection).
- Every observed store notification can be re-queried from the Store (via graduation).
- The round-trip is idempotent on content-addressed Signals.

When a graduated Signal is written to Store but the originating Pulse has already been evicted from the Bus ring buffer, the adjunction still holds: the Signal is in Store, and the *next* projection Pulse is still published. Late subscribers miss the original Pulse but can query the Store directly. The projection Pulse is a notification, not the data itself.

---

## 5. Capability Intersection as a Pullback

The three-layer capability model in [02-CELL.md](../../unified/02-CELL.md) S3.2 is a pullback in the category of capability sets.

Given three capability sets:

- `D` = declared (Cell author)
- `G` = granted (Agent operator)
- `P` = permitted (Space policy)

The effective capability set is the pullback:

```text
            Effective
           /        \
          D          G
           \        /
            P ------
```

More precisely, with `Cap` as the category of capability sets ordered by inclusion:

```rust
/// The pullback of three capability sets.
///
/// In Cap (the category of sets ordered by inclusion):
///   Effective = D ∩ G ∩ P
///
/// This is the limit of the diagram D -> U <- G <- P -> U
/// where U is the universe of all capabilities.
///
/// Properties:
/// 1. Fail-closed: missing from ANY layer means denied
/// 2. Monotone: adding caps to one layer never removes effective caps
/// 3. Composable: for a pipeline A -> B, effective caps = eff(A) ∩ eff(B)
pub fn effective_capabilities(
    declared: &CapabilitySet,
    granted: &CapabilitySet,
    permitted: &CapabilitySet,
) -> CapabilitySet {
    declared
        .intersection(granted)
        .intersection(permitted)
}
```

### 5.1 Composition of Capability Pullbacks

When two Cells are composed into a pipeline, the effective capability set is the intersection of their individual effective sets:

```rust
/// For a pipeline A -> B, the composed capability set is:
///   eff(A -> B) = eff(A) ∩ eff(B)
///
/// This means composition can only NARROW capabilities.
/// A pipeline is never more powerful than its least-privileged Cell.
/// This is the principle of least privilege, enforced structurally.
fn pipeline_capabilities(cells: &[&dyn Cell]) -> CapabilitySet {
    cells.iter()
        .map(|c| c.capabilities().effective())
        .reduce(|acc, cap| acc.intersection(&cap))
        .unwrap_or_default()
}
```

The pullback structure prevents **capability escalation through composition**: you cannot build a pipeline that circumvents a Space policy by chaining Cells that individually satisfy it but collectively exceed it. The intersection collapses to the most restrictive participant.

---

## 6. The Free Monad Over Protocol

The 9 protocols define an algebra. The **free monad** over this algebra is the type of all possible Cell programs before any interpretation (execution). This matters because it separates *description* from *execution* -- a Graph is a value in the free monad, and a Flow is its interpretation.

### 6.1 The Protocol Functor

```rust
/// The protocol functor F: the set of one-step Cell operations.
///
/// Each variant represents "do one protocol operation, then continue
/// with the result." The continuation `k` is what makes this a functor.
enum ProtocolF<A> {
    /// Store: put or query, continue with the result.
    Put(Signal, Box<dyn FnOnce(SignalRef) -> A>),
    Query(StoreQuery, Box<dyn FnOnce(Vec<Signal>) -> A>),

    /// Score: rate a Signal, continue with the score.
    Score(Signal, ScoreContext, Box<dyn FnOnce(Score) -> A>),

    /// Verify: check a Signal, continue with the verdict.
    Verify(Vec<Signal>, VerifyContext, Box<dyn FnOnce(Verdict) -> A>),

    /// Route: select from candidates, continue with the selection.
    Route(Vec<RouteCandidate>, RouteContext, Box<dyn FnOnce(RouteResult) -> A>),

    /// Compose: assemble under budget, continue with the composed Signal.
    Compose(Vec<ComposeBid>, ComposeBudget, Box<dyn FnOnce(ComposeResult) -> A>),

    /// React: process Pulses, continue with the reaction output.
    React(Vec<Pulse>, Box<dyn FnOnce(ReactOutput) -> A>),

    /// Observe: read state, continue with observations.
    Observe(ObserveContext, Box<dyn FnOnce(Vec<Signal>) -> A>),

    /// Connect: query external system, continue with the result.
    Connect(ConnectionHandle, Value, Box<dyn FnOnce(Value) -> A>),

    /// Trigger: wait for event, continue with the trigger data.
    Trigger(TriggerBinding, Box<dyn FnOnce(Vec<TriggerEvent>) -> A>),
}
```

### 6.2 The Free Monad

```rust
/// Free monad over ProtocolF.
///
/// A CellProgram<A> is a description of a Cell computation that
/// will eventually produce a value of type A. It is NOT yet executed.
/// Execution happens when an interpreter (the Flow engine) runs it.
enum CellProgram<A> {
    /// Pure value -- computation is done.
    Pure(A),

    /// One protocol step followed by a continuation.
    Step(ProtocolF<CellProgram<A>>),
}

impl<A> CellProgram<A> {
    /// Monadic bind: sequence two Cell programs.
    fn and_then<B>(self, f: impl FnOnce(A) -> CellProgram<B>) -> CellProgram<B> {
        match self {
            CellProgram::Pure(a) => f(a),
            CellProgram::Step(step) => {
                // Recursively push `f` into the continuation
                CellProgram::Step(step.map_continuation(|prog| prog.and_then(f)))
            }
        }
    }

    /// The standard Cell pipeline: query -> score -> route -> compose -> verify
    fn standard_pipeline(query: StoreQuery, budget: ComposeBudget) -> CellProgram<Verdict> {
        CellProgram::query(query)
            .and_then(|signals| CellProgram::score_all(signals))
            .and_then(|scored| CellProgram::route(scored))
            .and_then(|selected| CellProgram::compose(selected, budget))
            .and_then(|composed| CellProgram::verify(composed))
    }
}
```

### 6.3 Why Free Monads Matter Here

The free monad gives three concrete capabilities:

1. **Static analysis of Graphs**: Before executing a Graph, the runtime can inspect the `CellProgram` structure to determine resource needs, verify capability satisfaction, and estimate cost. This is how the execution engine (see [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md)) does budget pre-checks.

2. **Interpretation swapping**: The same `CellProgram` can be interpreted by a real executor (against live Cells), a mock executor (for testing), or a cost estimator (for budget planning). The description is decoupled from the execution strategy.

3. **Optimization**: A compiler pass over the free monad can fuse adjacent Store operations, batch Score calls, or eliminate redundant Verify checks. This is the same trick that database query planners use: optimize the description, then execute the optimized plan.

---

## 7. Composition Rules: When Can You Pipe A into B?

Building on the category, adjunction, and capability pullback, the complete composition check is:

```rust
/// Complete edge validation for Graph construction.
///
/// This is the single function that determines whether Cell A
/// can feed Cell B in a Graph. It is called at Graph load time.
/// Every Graph that passes this check is guaranteed to be
/// type-safe, capability-safe, and protocol-compatible.
fn validate_edge(
    source: &RegisteredCell,
    target: &RegisteredCell,
    space: &SpacePolicy,
) -> Result<EdgeValidation, EdgeError> {
    // 1. Type compatibility (S1.2 preorder)
    let type_compat = match (
        source.block.output_schema(),
        target.block.input_schema(),
    ) {
        (_, None) => TypeCompat::AnyAccepted,
        (None, Some(expected)) => {
            return Err(EdgeError::TypeMismatch {
                source: source.block.name().into(),
                target: target.block.name().into(),
                expected: expected.clone(),
                got: TypeSchema::Any,
            });
        }
        (Some(out), Some(inp)) => {
            if out.is_compatible(inp) {
                TypeCompat::Compatible
            } else {
                return Err(EdgeError::TypeMismatch {
                    source: source.block.name().into(),
                    target: target.block.name().into(),
                    expected: inp.clone(),
                    got: out.clone(),
                });
            }
        }
    };

    // 2. Protocol adjacency (S2.1 matrix)
    let proto_compat = protocol_composable(
        source.block.protocols(),
        target.block.protocols(),
    );
    if !proto_compat {
        return Err(EdgeError::ProtocolIncompatible {
            source_protocols: source.block.protocols().to_vec(),
            target_protocols: target.block.protocols().to_vec(),
        });
    }

    // 3. Capability pullback (S5 intersection)
    let source_eff = source.block.capabilities().effective();
    let target_eff = target.block.capabilities().effective();
    let pipeline_caps = source_eff.intersection(&target_eff);

    // Check that the Space permits the composed pipeline
    if !space.permits_all(&pipeline_caps) {
        return Err(EdgeError::CapabilityDenied {
            required: pipeline_caps,
            permitted: space.permitted_capabilities().clone(),
        });
    }

    // 4. Cost estimation (optional but informative)
    let estimated_cost = match (
        source.block.estimated_cost(),
        target.block.estimated_cost(),
    ) {
        (Some(a), Some(b)) => Some(Cost(a.0 + b.0)),
        _ => None,
    };

    Ok(EdgeValidation {
        type_compat,
        proto_compat: true,
        effective_capabilities: pipeline_caps,
        estimated_cost,
    })
}
```

### 7.1 The Categorical Summary

```text
Category Cell:
  Objects   = { C | C : Cell, registered with protocol conformance }
  Morphisms = { f : A -> B | validate_edge(A, B, space) = Ok(_) }
  Identity  = PassthroughCell (output_schema = input_schema, all protocols)
  Compose   = g . f defined when validate_edge(A, B) and validate_edge(B, C) both hold

Functors:
  Store : Cell -> Set     (forgetful: Cell -> its stored Signals)
  Bus   : Cell -> Stream  (forgetful: Cell -> its published Pulses)
  Adjunction: Bus -| Store via graduation/projection (S4)

Natural transformations:
  eta_sv : Score => Verify        (score_to_verdict)
  eta_vr : Verify => React        (verdict_to_pulse)
  eta_rs : React => Store         (graduation)
  eta_sr : Store => React         (projection)

Free monad:
  CellProgram<A> = Free(ProtocolF, A)
  Interpretation: CellProgram<A> -> Flow<A> via the execution engine
```

---

## 8. Multi-Protocol Cells and the Product Category

A Cell that implements multiple protocols (see [02-CELL.md](../../unified/02-CELL.md) S3.9) lives in the product category. For example, a `CodeReviewCell` implementing both Score and Verify lives in `Score x Verify`:

```rust
/// A Cell in the product category Score x Verify.
///
/// The runtime dispatches based on the protocol being invoked.
/// The Cell sees both protocol contexts but the caller only
/// interacts with one protocol at a time.
///
/// Categorically: there are two projection functors
///   pi_score : Score x Verify -> Score
///   pi_verify : Score x Verify -> Verify
/// and the Cell is a section of both.
impl Cell for CodeReviewCell {
    fn protocols(&self) -> &[ProtocolId] {
        &[ProtocolId::Score, ProtocolId::Verify]
    }

    fn input_schema(&self) -> Option<&TypeSchema> {
        // Must accept the UNION of inputs for both protocols.
        // In practice: OneOf(Score's input, Verify's input)
        Some(&self.combined_input_schema)
    }

    fn output_schema(&self) -> Option<&TypeSchema> {
        // Output depends on which protocol is invoked.
        // The Graph edge validator checks against the specific
        // protocol being used on that edge.
        Some(&self.combined_output_schema)
    }
}
```

The product structure means that multi-protocol Cells are strictly more flexible than single-protocol Cells, but they must satisfy the input/output contracts of ALL declared protocols. This is enforced at registration time via `ProtocolConformance` verification.

---

## 9. Protocol Algebra Laws

The composition of protocols obeys algebraic laws that the Graph validator can exploit for optimization:

### 9.1 Idempotence of Store

```text
put(put(signal)) = put(signal)
```

Content-addressed storage is idempotent. Two consecutive Store writes of the same Signal produce the same SignalRef. The Graph optimizer can eliminate redundant Store operations.

### 9.2 Score Monotonicity Under Composition

```text
score(compose([a, b])) >= min(score(a), score(b))
```

Composing higher-quality inputs cannot produce a lower-quality output (assuming a well-behaved Compose Cell). This enables early rejection: if any input scores below threshold, skip composition entirely.

### 9.3 Verify Conjunctivity

```text
verify(a AND b) = verify(a) AND verify(b)
```

Hard criteria are conjunctive by construction (see [02-CELL.md](../../unified/02-CELL.md) S2.3). This enables parallel verification: split the criteria, verify independently, AND the results.

### 9.4 React Commutativity (Unordered Pulses)

```text
react([p1, p2]) = react([p2, p1])   (for commutative React Cells)
```

React Cells that aggregate (e.g., counting, summing) are order-independent. The runtime can reorder Pulses for batching without changing the outcome. Non-commutative React Cells (e.g., sequential state machines) must declare this, and the runtime preserves ordering for them.

---

## What This Enables

1. **Static Graph validation**: Every Graph is type-checked, capability-checked, and protocol-checked before execution. Malformed pipelines are rejected at load time, not at runtime.

2. **Compositional reasoning**: The free monad and algebraic laws let the execution engine optimize Graphs without executing them. Fuse adjacent Stores, batch Scores, parallelize Verifies.

3. **Principled extension**: New protocols slot into the algebra by defining their adjacency row, natural transformations, and algebraic laws. The framework is open to extension without modifying existing Cells.

4. **Capability safety by construction**: The pullback structure makes it impossible to compose a pipeline that exceeds its Space's permissions. No runtime check needed -- the property holds by construction.

5. **Interpretation flexibility**: The same Graph description can be executed, simulated, cost-estimated, or tested by swapping the free monad interpreter.

---

## Feedback Loops

The protocol algebra is not static -- it improves through use:

- **Score => Verify calibration**: The Score-to-Verdict natural transformation uses thresholds (`VerifyThresholds`). These thresholds are updated by the adaptive gate threshold system (see [10-LEARNING-LOOPS.md](../../unified/10-LEARNING-LOOPS.md)) via EMA on historical verdicts. As the system learns which score ranges predict passes, the transformation becomes tighter.

- **Protocol adjacency expansion**: New edges can be added to the adjacency matrix when new composition patterns are validated. The matrix is not hardcoded but loaded from configuration, allowing learned composition rules.

- **Free monad optimization**: The execution engine tracks which fusion and batching optimizations actually improve performance (wall-clock time, cost, verdict pass rate). Optimizations that degrade outcomes are rolled back via the predict-publish-correct Loop.

---

## Open Questions

1. **Higher-order Cells**: Can a Cell take another Cell as input (a morphism as input rather than a Signal)? This would enable meta-programming patterns (a Cell that optimizes another Cell's parameters). The current algebra does not support this because `Cell::execute` takes `Vec<Signal>`, not `Vec<dyn Cell>`. Would require extending the free monad with a `Lambda` variant.

2. **Effect tracking**: The free monad describes computation structure but not effects (Bus publishes, Store writes). A free monad with algebraic effects (a la Eff or Polysemy) would let the runtime reason about side effects statically. Is this worth the complexity?

3. **Protocol versioning**: When a protocol gains new methods (e.g., Verify gains `verify_stream`), how does the adjacency matrix handle Cells that implement only the old version? The current `ProtocolConformance.version` field enables this, but the composition rules for mixed-version pipelines are not yet specified.

4. **Distributive law**: Is there a distributive law between the Store and Bus monads? If so, it would give a principled rule for interleaving durable writes and ephemeral publishes in a single Cell program. Currently this interleaving is ad hoc.

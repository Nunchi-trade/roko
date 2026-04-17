# The Six Synapse Traits

> **Abstract:** This document keeps the Synapse trait model as a load-bearing part of Roko's
> architecture, but no longer treats the historical "one noun, six verbs" mnemonic as the
> complete story. Roko's kernel is better read as two mediums — durable Engram and ephemeral
> Pulse — moving through two fabrics — Substrate and Bus — with six operators providing the
> durable-storage, assessment, verification, routing, composition, and reaction logic around
> them. The operators remain the right decomposition; the surrounding kernel story has grown.

> **See also:** `tmp/refinements/01-critique-one-noun.md` for the diagnosis,
> `tmp/refinements/02-engram-vs-pulse.md` for the Engram/Pulse split,
> `tmp/refinements/03-bus-as-first-class.md` for the Bus promotion,
> `tmp/refinements/04-operators-generalized.md` for signature generalization, and
> [01-naming-and-glossary.md](./01-naming-and-glossary.md) for the canonical terminology map.

> **Implementation:** Shipping v1 interfaces exist today; this document updates the architectural
> framing so the current trait set is explained in terms that remain compatible with the
> two-medium / two-fabric kernel.

---

## 1. The Composition Model

Roko still benefits from a small, composable operator vocabulary. What changed is the scope of
the story those operators sit inside.

The original mnemonic was useful because it emphasized parsimony: a small set of reusable
interfaces can express a wide range of agent behaviors. That remains true. What no longer holds
is the implication that one durable record type explains the whole runtime. The runtime now has:

- **Two mediums**: durable **Engram** and ephemeral **Pulse**
- **Two fabrics**: storage-oriented **Substrate** and transport-oriented **Bus**
- **Six operators**: the six synapse roles that assess, verify, route, compose, persist, and
  react around those mediums

This document therefore keeps the six-operator lens while treating it as part of a larger kernel
picture. The operators are still the right decomposition. The full architecture just includes
more than a single durable noun.

### 1.1 Historical Mnemonic, Revised

The historical "one noun, six verbs" phrase is best read as shorthand for the durable half of v1:
an Engram-centric storage and composition model. REF01 shows why that shorthand became too small:
live transport moved onto a Bus, runtime policies started reacting to in-flight traffic, and some
trait signatures became awkward because they were asked to pretend every input was already a
stored artifact.

### 1.2 What Still Holds

Several claims from the earlier framing remain correct and should not be thrown away:

1. The operator set is still compact and composable.
2. Strict downward layering is still the correct dependency rule.
3. Gamma, Theta, and Delta are still the right three-speed model.
4. Neuro, Daimon, and Dreams are still cross-cuts rather than loop steps.

REF01 is therefore a reframing, not a repudiation. It narrows the claim from "this is the whole
architecture" to "this is the operator model inside the broader kernel."

---

## 2. Operator Overview

The six synapse traits are still the stable operator vocabulary for Roko's kernel:

| Operator | Core job | Primary layer | Relationship to the two-medium / two-fabric model |
|---|---|---|---|
| **Substrate** | Persist and query durable state | L0 Runtime | Durable fabric for Engrams |
| **Scorer** | Assess salience, value, novelty, quality | L1-L2 | Operates over retrieved or observed data; generalized further in REF04 |
| **Gate** | Verify against external reality | L3 Harness | Verifies claims, actions, and composed artifacts |
| **Router** | Choose among candidates or next actions | L1 Framework | Makes selections informed by scores, costs, and context |
| **Composer** | Assemble bounded artifacts | L2 Scaffold | Builds prompt Engrams and other composed outputs |
| **Policy** | React to streams and outcomes | L3-L4 | Most obviously pulled toward Pulse-heavy runtime behavior |

The Bus is not a seventh operator in this document's framing. It is the second fabric the six
operators now work alongside. REF03 gives it the full first-class treatment.

---

## 3. Substrate — Durable Storage

Substrate remains the durable storage fabric. Its job is unchanged: persist Engrams, retrieve
them by query, and expose a stable memory surface to the rest of the system.

That durability boundary matters more clearly after REF01:

- Engrams belong in Substrate because their identity, lineage, provenance, and decay matter.
- Pulses do not belong in Substrate by default because they are transport traffic, not durable
  records.
- Graduation from Pulse to Engram is a deliberate step, not an architectural accident.

This is why Substrate still deserves its own deep-dive in
[07-substrate-trait.md](./07-substrate-trait.md). The critique is not that storage was wrong;
it is that storage was being asked to stand in for transport.

---

## 4. Scorer — Assessment

Scorers rate what the runtime should care about. In v1 documentation that usually meant scoring
Engrams already present in storage. In the fuller kernel story, assessment happens against both
durable and live inputs:

- retrieved Engrams during context selection
- candidate actions before execution
- live runtime signals that may later graduate into durable records

REF04 carries the signature-generalization work. REF01's point is simpler: the architecture
should stop pretending that every assessable thing is already a stored Engram.

---

## 5. Gate — Verification

Gates connect Roko to external reality. They compile code, run tests, simulate transactions,
validate schemas, check balances, and emit verdicts about whether a claim survives contact with
the world.

That part of the story does not change. What changes is the placement of verification in the
runtime:

- some Gates verify composed Engrams after an action completes
- some stream-oriented checks want to observe live runtime traffic before or during action
- the Gate pipeline should therefore be described as sitting beside both Substrate and Bus, not
  as a purely post-storage concern

REF05 retells the loop around this distinction. REF01 only establishes the diagnosis that the
Engram-only explanation was hiding real runtime behavior.

---

## 6. Router — Selection

Routers choose among alternatives: which model to call, which backend to use, which tool to run,
which plan branch to pursue, or which candidate artifact to advance.

The core idea remains stable. The nuance introduced by REF01 is that not every routed choice is a
choice among stored Engrams. Some routing decisions are about live control flow or in-flight
traffic, and only later become Engrams for audit and learning.

That distinction matters because it explains why routing feels natural in the system while some
of the older trait signatures felt stretched. The choice is first-class; the durable record of
the choice is often second.

---

## 7. Composer — Bounded Assembly

Composer remains the assembly operator: it turns multiple ingredients into a bounded output under
token, byte, time, or structural budgets.

Prompt construction is the clearest example:

1. retrieve relevant Engrams from Substrate
2. rank or filter them with Scorers and Routers
3. assemble a prompt Engram under a budget

That story still holds. What changes is the boundary around composition. When the runtime reacts
to live traffic, the input set may include observations that are not yet durable. REF04 handles
the generalized operator surface; REF01 simply makes room for that fact in the architecture
description.

---

## 8. Policy — Reaction

Policy is where the old framing showed the most strain.

Policies do not merely inspect a retrospective archive of durable records. They watch ongoing
activity and decide whether to emit interventions, summaries, alerts, pauses, promotions, or
other follow-on work. In practice that means:

- circuit breakers react to live failures
- health watchers react to changing runtime conditions
- approval flows react to in-flight decisions
- telemetry and observability often want to emit from the stream itself

REF01 calls this out explicitly because "stream of Engrams" was doing too much conceptual work.
The durable output of a Policy can absolutely be an Engram. But the thing the Policy is watching
is often better described as Pulse traffic on a Bus.

This is also why REF04 matters: operator signatures should generalize where the architecture has
already generalized in reality.

---

## 9. Trait × Layer Map

The six operators are distributed across the existing five-layer taxonomy:

```text
Layer 4: Orchestration  -> Policy (plan reactions, scheduling responses)
Layer 3: Harness        -> Gate (verification), Policy (watchers, breakers)
Layer 2: Scaffold       -> Scorer (context relevance), Composer (bounded assembly)
Layer 1: Framework      -> Router (selection), Scorer (dispatch relevance)
Layer 0: Runtime        -> Substrate (durable persistence), Bus (transport fabric)
```

The important REF01 correction is at L0. Runtime has two fabrics, not one. Substrate remains the
durable storage fabric. Bus is the transport fabric that dissolves the temptation to wire
cross-layer runtime concerns through direct crate dependencies.

That shift explains why the `roko-conductor -> roko-learn` dependency violation matters: it is
evidence that transport behavior wanted a kernel fabric of its own.

---

## 10. Composability Example

A complete runtime pass now reads more clearly when the two mediums and two fabrics are named:

1. **Sense durable state** via `Substrate.query`
2. **Sense live state** via `Bus.subscribe` and external I/O
3. **Assess and route** with Scorers and Routers
4. **Compose** a bounded output when durable assembly is required
5. **Act and verify** through tools, models, or chain execution plus Gates
6. **Persist** durable results as Engrams
7. **Broadcast/react** on the Bus and through Policies

The six operators still compose. The improvement is that the architecture no longer hides live
transport inside storage-centric language.

---

## 11. Sufficiency Analysis: Why Keep Six?

REF01 does not argue for exploding the operator count. It argues that the architecture should
name the real mediums and fabrics around the existing operators.

That is the key distinction:

- **Keep the six operators** because they are still the right decomposition of work
- **Add the missing kernel vocabulary** because the runtime has both durable and ephemeral data
- **Generalize signatures carefully** because some operators naturally work over more than stored
  artifacts

### 11.1 The Awkward Cases Are Diagnostic

The most important awkward cases are not random edge conditions. They are evidence:

| Boundary case | Why it feels awkward in the old framing | What REF01 says about it |
|---|---|---|
| Telemetry emission from `Policy::decide(&[], ctx)` | The operator wants to react without pretending an Engram stream already exists | The runtime has live traffic that should be described directly |
| Circuit breakers and watcher loops | Policies want to watch runtime changes as they happen | Policy is partly a Pulse-stream consumer, not only an Engram consumer |
| Direct cross-layer runtime dependencies | Subsystems bypass the architecture story to communicate | Bus needs first-class architectural recognition |

### 11.2 Why Not Add More Operators Here?

Because the problem is not primarily operator count. The problem is kernel framing.

The current six-role decomposition still separates storage, assessment, verification, selection,
assembly, and reaction cleanly. What REF04 changes is the signature surface for some operators,
not the fact that these six roles are the stable conceptual primitives.

### 11.3 What Changes Next

REF02, REF03, and REF04 carry the concrete follow-on work:

1. define Pulse as Engram's ephemeral sibling
2. promote Bus as the transport fabric
3. generalize operator signatures where the runtime already handles both mediums

This document therefore marks the end of the old full-story claim, not the end of the operator
model itself.

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Sumers et al. 2023 (arXiv:2309.02427) | CoALA-style cognitive decomposition supports a compact operator vocabulary. |
| Franklin et al. 2016, LIDA | Concurrent cognitive roles map well onto composable operators plus runtime transport. |
| Chen et al. 2023 (arXiv:2305.05176) | FrugalGPT-style cascades justify Router and Composer separation. |
| Friston 2010 | Verification and reaction sit naturally inside an active-inference loop. |
| Milewski 2014 | Small composable interfaces remain easier to reason about than sprawling inheritance hierarchies. |

---

## Current Status and Gaps

- **Implemented in shipping code**: the six synapse traits remain the operator vocabulary in
  `roko-core`.
- **Implemented in practice**: runtime transport already exists and multiple subsystems behave as
  live stream consumers.
- **Gap in documentation now closed by REF01**: this doc no longer presents the old mnemonic as
  the complete architecture story.
- **Gap still open for follow-on refinements**: Pulse, Bus, and generalized operator signatures
  need their dedicated specification updates in REF02-REF04.

---

## Cross-References

- [01-naming-and-glossary.md](./01-naming-and-glossary.md) — canonical terminology and retired-name guidance
- [07-substrate-trait.md](./07-substrate-trait.md) — durable storage fabric in depth
- [08-scorer-gate-router-composer-policy.md](./08-scorer-gate-router-composer-policy.md) — detailed operator specifications
- [09-universal-cognitive-loop.md](./09-universal-cognitive-loop.md) — loop retelling updated further in REF05
- [12-five-layer-taxonomy.md](./12-five-layer-taxonomy.md) — layer assignments and dependency rules
- [23-architectural-analysis-improvements.md](./23-architectural-analysis-improvements.md) — audit evidence and rewrite rationale

# Synergy & Integration Map

> **Abstract:** This chapter is the cross-reference map for the architecture stack. The prior architecture docs describe the primitives one by one; this doc shows how they reinforce each other. The claim is simple: Roko's moat is not any single feature, but the interaction density of the whole system. A competitor can copy one node or even a pair; reproducing the weave means reproducing the same architectural choices in the same order. See also [tmp/refinements/31-synergy-integration-map.md](../../tmp/refinements/31-synergy-integration-map.md) and [Naming and Glossary](./01-naming-and-glossary.md).

> **Implementation**: Reference

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Prerequisites**: [Naming and Glossary](./01-naming-and-glossary.md), [Engram Data Type](./02-engram-data-type.md), [Pulse Medium](./02b-pulse-ephemeral-event.md), [Bus Transport Fabric](./07b-bus-transport-fabric.md), [C-Factor: Collective Intelligence](./14-c-factor-collective-intelligence.md), [Heuristics, Worldviews, and Falsifiers](../05-learning/19-heuristics-worldviews-and-falsifiers.md)

---

## Abstract

The architecture is now best understood as a synergy graph. Some primitives are load-bearing in their own right, but the real leverage comes from the way they combine. This chapter names the ten primitives that sit at the center of the weave, shows how they exchange value in a compact matrix, and then walks the ten concrete synergies that make the system compounding rather than merely modular.

If the earlier chapters answer "what is the primitive?", this chapter answers "what does it unlock, and what does it depend on?" That framing is deliberate. The system is not a pile of adjacent subsystems. It is a matrix of mutually reinforcing constraints and affordances, and the moat lives in those couplings.

See also [tmp/refinements/31-synergy-integration-map.md](../../tmp/refinements/31-synergy-integration-map.md) for the full proposal source and [Naming and Glossary](./01-naming-and-glossary.md) for the canonical vocabulary used throughout this chapter.

---

## 1. The Ten Load-Bearing Primitives

These are the nodes of the synergy graph. Each one is already established in the architecture docs; this chapter treats them as the minimal set whose interactions explain the larger system.

| # | Primitive | Home doc | Role in the weave |
|---|---|---|---|
| P1 | Engram | [02](./02-engram-data-type.md) | Durable record, lineage anchor, and substrate resident |
| P2 | Pulse | [02b](./02b-pulse-ephemeral-event.md) | Ephemeral wire medium, live coordination unit |
| P3 | Bus | [07b](./07b-bus-transport-fabric.md) | Transport fabric, topic routing, replay surface |
| P4 | Substrate | [07](./07-substrate-trait.md) | Storage fabric, durable persistence, query surface |
| P5 | HDC fingerprint | [27](./27-temporal-knowledge-topology.md) | Similarity, clustering, and semantic indexing primitive |
| P6 | Demurrage | [04](./04-decay-variants.md) | Attention economy, holding cost, self-trimming pressure |
| P7 | Heuristics + falsifiers | [05-learning/19](../05-learning/19-heuristics-worldviews-and-falsifiers.md) | Learned rules of thumb with explicit calibration hooks |
| P8 | c-factor | [14](./14-c-factor-collective-intelligence.md) | Collective intelligence signal, diversity pressure, policy target |
| P9 | Replication ledger | [21-references/25](../21-references/25-research-to-runtime.md) | Claims, evidence, and falsification history |
| P10 | Plugin SPI + domain profiles | [18-tools/14](../18-tools/14-plugin-sdk.md), [02-agents/16](../02-agents/16-domain-profiles.md) | Ecosystem growth, extension surface, domain-specific packaging |

The key point is not that these primitives exist independently. It is that each one becomes more useful when it is allowed to constrain and enrich the others.

---

## 2. The Synergy Matrix

The matrix below is intentionally compact. Each cell says what the row primitive gives to the column primitive. Empty cells are not bugs; they are places where the architecture stays clean by refusing to force a coupling.

| gives \ to | P1 Engram | P2 Pulse | P3 Bus | P4 Substrate | P5 HDC | P6 Demurrage | P7 Heuristics | P8 c-factor | P9 Ledger | P10 Plugins |
|---|---|---|---|---|---|---|---|---|---|---|
| **P1 Engram** | - | graduation source | publish target | store target | encode target | balance owner | lineage anchor | cohort artifact | paper body | plugin config target |
| **P2 Pulse** | graduation destination | - | payload | sub-event | live evidence | reinforcement signal | calibration trial | cohort event | ledger observation | plugin event |
| **P3 Bus** | `substrate.*` wakeups | delivery | - | notify | routing input | freshness pressure | falsifier watch | cohort floor | watchdog stream | lifecycle events |
| **P4 Substrate** | home | - | bridge | - | fingerprint store | balance home | heuristic store | metric source | ledger store | plugin state |
| **P5 HDC** | fingerprint field | - | - | index key | - | novelty score | similarity cluster | diversity signal | paper search | encoder slot |
| **P6 Demurrage** | weight | - | - | tier logic | - | - | freshness decay | minority support | anti-drift | plugin aging |
| **P7 Heuristics** | record variant | - | prediction / outcome | store | cluster | reinforcement signal | - | peer model | claim body | heuristic plugin |
| **P8 c-factor** | cohort record | - | metrics topic | metric store | diversity source | - | peer prediction | - | replication support | c-factor plugin |
| **P9 Ledger** | paper Engram | - | watchdog topic | ledger store | paper fingerprint | claim decay | lifted claim | ledger observation | - | claim plugin |
| **P10 Plugins** | plugin Engram | plugin events | plugin topics | plugin reads | plugin encoder | plugin budget | new heuristic source | new metric | new claim | - |

Read the matrix as a design test. If a feature does not connect to at least two nodes, it is probably too thin to matter. If it connects to too many nodes without a clear purpose, it is probably too broad to land cleanly.

---

## 3. Ten Named Synergies

This section turns the matrix into concrete mechanisms. Each synergy is a real composition, not a slogan.

### 3.1 Demurrage × HDC -> self-trimming semantic memory

Substrate stores each Engram with a fingerprint. Demurrage charges holding cost over time. HDC makes novelty measurable by comparing a record to its nearest neighbors. The result is memory that gradually favors uniquely useful records rather than raw accumulation.

This is the core "unique-and-used" pressure. Without HDC, demurrage becomes a blunt tax. Without demurrage, HDC becomes an expensive search primitive with no pruning force. Together they make the memory layer economically selective.

### 3.2 Heuristics × Pulse × Bus -> continuous calibration

Heuristics carry explicit falsifiers. Pulses on the Bus provide live evidence of whether the heuristic helped, failed, or needs tightening. That turns calibration into a streaming process instead of a periodic audit.

The practical effect is continuous learning from lived outcomes. A heuristic is not trusted because it exists; it is trusted because it has survived repeated contact with relevant Pulses.

### 3.3 c-factor × Bus × HDC -> diversity-aware routing

Bus statistics show how work is being distributed. HDC shows whether the system's representations are converging too tightly. c-factor consumes both signals and can push policy toward role diversity, model diversity, or pair rotation when the system becomes too homogeneous.

This is not just observability. It is regulation. The system is watching for monoculture and actively correcting toward broader cognitive variety.

### 3.4 Replication ledger × Heuristics × paper Engram -> living research

Papers live as Engrams. Claims extracted from them become heuristics. The replication ledger records which claims have held up under test and which have been falsified.

This turns research into runtime material. A claim is not a static citation; it is a living object whose status can change as evidence arrives.

### 3.5 Plugin SPI × Substrate × Bus -> ecosystem growth path

Plugins declare what they can read from Substrate and what topics they subscribe to on the Bus. The SPI constrains the extension boundary so new tools, gates, roles, and domain profiles can land without rewriting the core.

The synergy matters because it makes growth structurally safe. The ecosystem expands along declared seams instead of through ad hoc hooks.

### 3.6 c-factor × Heuristics -> peer-model learning

The system can model other agents the same way it models the world: as a set of predictions to calibrate. Peer-model accuracy becomes part of the collective intelligence signal.

That makes social perception measurable. Agents that understand one another's likely responses can coordinate with less friction and less wasted synchronization.

### 3.7 Dreams × Substrate × Pulse -> retroactive insight

Dreams read durable records from Substrate, reinterpret them under updated priors, and publish new Pulses that refresh downstream caches and composers. Old episodes therefore remain actionable, but only after the system has grown enough to reinterpret them.

The consequence is important: the system does not merely remember. It re-learns from memory.

### 3.8 Demurrage × Heuristic × calibration -> graceful relearning

Confidence is not frozen. If a heuristic is not challenged, its confidence should soften enough that fresh contradictory evidence can move it without a manual reset.

This is the anti-stagnation mechanism. A long-stable rule can become stale; demurrage on confidence prevents it from dominating forever.

### 3.9 HDC × Consensus × Bus -> substantive agreement detection

Agents can emit agreement Pulses with fingerprints of the ideas they endorse. Aggregators compare those fingerprints to proposal fingerprints rather than treating surface wording as the ground truth.

That lets the system tell the difference between genuine agreement and merely similar phrasing. It is a semantic check, not a token-counting trick.

### 3.10 TypedContext × domain profiles × Gate -> auditable domain safety

Domain profiles package the behavior of a specific operating domain. TypedContext carries the structured situation. Gates evaluate typed predicates instead of free-text guesses, and Custody records who acted, why, and with what evidence.

The synergy is auditability. Domain-sensitive actions remain inspectable after the fact without forcing each team to invent its own ad hoc logging stack.

---

## 4. The Seven-Step Loop Across the Matrix

The synergy matrix is not separate from the universal loop. It explains why the same seven-step
cycle compounds instead of resetting each turn.

1. `SENSE` draws from `Substrate` queries, `Bus` subscriptions, and external I/O, then anchors
   those reads in `Engram`, `Pulse`, and `TypedContext`.
2. `ASSESS` uses `HDC fingerprint`, `demurrage`, `heuristics`, and `c-factor` signals to decide
   what deserves attention and which policy lever should move next.
3. `COMPOSE` pulls the right durable records into scope, injects domain-profile structure, and
   uses the same matrix to decide which evidence belongs in the prompt.
4. `ACT` emits live `Pulse` traffic, produces tool and agent outcomes, and creates the next
   candidates for durable `Engram` storage.
5. `VERIFY` turns gates, falsifiers, and replication checks into evidence about whether the
   current behavior should be reinforced, revised, or quarantined.
6. `PERSIST` writes durable artifacts back to `Substrate`, assigns economic weight through
   demurrage, records `HDC fingerprint` values, and stores claim or custody evidence where
   lineage matters.
7. `BROADCAST` and `REACT` publish the new live state on the `Bus`, update observers such as
   `c-factor` and plugin consumers, and trigger follow-on policy, calibration, or consolidation.

That is the compounding claim in operational form: every step touches multiple primitives, so
improving one cell in the matrix tends to improve several loop steps at once.

## 5. What the Matrix Is, and Is Not

The matrix is a design map, not a priority queue. It tells you which primitives reinforce one another and where the architecture has leverage, but it does not tell you what to build first.

It is also not a completeness claim. More synergies exist. The matrix names the most load-bearing ones because they explain the system's shape, not because they exhaust it.

Finally, it is not a vendor pitch. The matrix is an internal coherence tool. Its value is that it makes the architecture inspectable by composition rather than by feature list.

---

## 6. How To Design New Refinements

When proposing a new refinement, walk the matrix before writing the spec.

1. Identify which primitives the feature touches.
2. Decide whether it provides to the primitive or consumes from it.
3. Check whether the interaction is already covered by a named synergy.
4. If not, ask whether the missing coupling is a gap in the feature or a genuine opportunity for the architecture.

The practical test is simple: a strong refinement usually strengthens at least two existing edges, or creates one edge that clearly unlocks several others.

Use the matrix to avoid dead-end features. If a proposal has no durable connection to Engram, no live connection to Pulse or Bus, and no policy or calibration consequence, it probably belongs in a narrower subsystem note instead of a chapter-level refinement.

---

## 7. The Moat Restated

The moat is the matrix itself. P1 through P4 are important, but they are table stakes. P5 through P7 are individually useful, but they have prior art. P8 through P10 become strategically important because they integrate the earlier primitives into a coherent runtime.

The competitive claim is therefore architectural: a competitor can copy any node, and often a pair, but not the full interaction lattice without committing to the same dependency order and the same cross-cut discipline. That is what makes the system hard to clone.

This is also why the chapter belongs in the architecture tree rather than in a generic research note. The matrix is an implementation constraint, not just a narrative frame.

---

## 8. Non-Synergies Worth Naming

Some pairs remain intentionally loose.

P5 HDC and P9 Replication ledger are not the same thing. Papers may be fingerprinted, but the ledger's rigor comes from evidence and falsification, not from similarity search.

P10 Plugins and P8 c-factor are also distinct. Plugins can contribute telemetry that informs c-factor, but c-factor is not a plugin-selection rule.

P2 Pulse and P9 Replication ledger are different levels of granularity. Pulses are live stream material; the ledger consumes stabilized claims, not raw chatter.

These non-synergies matter because they keep the architecture honest. Not every pair should couple just because the system is compositional.

---

## 9. Emergent Properties

Three properties emerge from the composition that do not exist in the same way in any isolated primitive.

The first is self-improvement without a separate training pipeline. The runtime predicts, calibrates, and updates through its own Bus-mediated feedback.

The second is inspectability at every level. Pulse lineage, Engram lineage, heuristic provenance, and ledger status together make decisions traceable instead of opaque.

The third is substrate neutrality. Because the key behaviors are driven by HDC, demurrage, heuristics, and policy, the system can swap storage or transport implementations without changing its core cognitive behavior.

These are not decorative benefits. They are the direct result of the matrix.

---

## 10. Cross-References

This chapter connects outward to the rest of the architecture tree and to the refinement source that defined it.

- See also [tmp/refinements/31-synergy-integration-map.md](../../tmp/refinements/31-synergy-integration-map.md) for the canonical proposal.
- See [Naming and Glossary](./01-naming-and-glossary.md) for the vocabulary that keeps the matrix stable.
- See [Engram Data Type](./02-engram-data-type.md) and [Pulse Medium](./02b-pulse-ephemeral-event.md) for the two mediums.
- See [Bus Transport Fabric](./07b-bus-transport-fabric.md) and [Substrate Trait (Deep Dive)](./07-substrate-trait.md) for the two fabrics.
- See [C-Factor: Collective Intelligence](./14-c-factor-collective-intelligence.md) for the diversity and policy side of the matrix.
- See [Heuristics, Worldviews, and Falsifiers](../05-learning/19-heuristics-worldviews-and-falsifiers.md) and [Research to Runtime](../21-references/25-research-to-runtime.md) for the calibration and replication side of the matrix.
- See [Compositional Kinds](./19-compositional-kinds.md) for the kind-level substrate that supports cross-domain records.
- See [Cross-Section Integration Map](./24-cross-section-integration-map.md) for the broader section-level dependency matrix.
- See [Design Principles and Frontier Summary](./17-design-principles-and-frontier-summary.md) for the moat framing this chapter makes concrete.

The chapter should be read as an integrator: it does not replace the primitive docs, it explains why they belong in the same architecture.

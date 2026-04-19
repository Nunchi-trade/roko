# Innovations — Master Index

> Cross-pollination innovations for Roko: each connects two or more orthogonal subsystems to produce capabilities no single subsystem provides alone. These are structural compositions — new `impl` blocks that wire existing traits together.

**Source**: `docs/00-architecture/30-cross-pollination-innovations.md`  
**Total innovations**: 8  
**Last reviewed**: 2026-04-19

---

## Master table

| Slug | One-line description | Source domain | Status | Affected subsystems |
|---|---|---|---|---|
| [hdc-active-inference](./hdc-active-inference.md) | Encode the generative world model as a 10,240-bit HDC vector so prediction error becomes Hamming distance | Neuroscience, information theory, hyperdimensional computing | Speculative | Neuro, Heartbeat/gating, Daimon, Dreams |
| [affect-causal-discovery](./affect-causal-discovery.md) | Treat the PAD affect vector as a node in a Structural Causal Model to distinguish causal from correlational affect-outcome relationships | Causal inference, affective computing | Speculative | Daimon, Learning, Dreams |
| [dream-verification](./dream-verification.md) | Check REM imagination scenarios against formal invariants; turn safety violations into AntiKnowledge | Formal methods, safe RL, AI planning | Speculative | Dreams, Gate, Neuro, Safety |
| [knowledge-morphogenesis](./knowledge-morphogenesis.md) | Replace task-return activation in Gierer-Meinhardt dynamics with knowledge concentration gradients from Neuro | Biology, multi-agent systems, reaction-diffusion | Speculative | Neuro, Coordination, Pheromones |
| [stigmergic-bandits](./stigmergic-bandits.md) | Treat each pheromone trail as a bandit arm; new agents inherit collective exploration history instead of cold-starting | Swarm intelligence, reinforcement learning, bandits | Speculative | Learning, Coordination, CascadeRouter |
| [witness-world-model](./witness-world-model.md) | Extract a causal world model from Witness DAG Prediction→Resolution chains for active inference action selection | Causal inference, active inference, epistemology | Speculative | Witness DAG, Heartbeat, Learning, Dreams |
| [code-somatic-markers](./code-somatic-markers.md) | Auto-generate somatic markers from code intelligence metrics so the agent pre-analytically "feels" risk before touching high-complexity code | Neuroscience, software engineering | Speculative | Daimon, Code intelligence, Heartbeat, Dreams |
| [dream-token-economy](./dream-token-economy.md) | Make dreams a market good: dream budget grows when hypotheses promote to Consolidated knowledge, shrinks otherwise | Economics, mechanism design, RL | Speculative | Dreams, Learning, Neuro |

---

## Status breakdown

| Status | Count |
|---|---|
| Speculative | 8 |
| Evaluated | 0 |
| Queued | 0 |
| Absorbed | 0 |
| Rejected | 0 |

## Domain breakdown

| Domain | Innovations |
|---|---|
| Neuroscience | hdc-active-inference, code-somatic-markers |
| Causal inference | affect-causal-discovery, witness-world-model |
| Information theory / HDC | hdc-active-inference |
| Biology / morphogenesis | knowledge-morphogenesis |
| Swarm intelligence | stigmergic-bandits, knowledge-morphogenesis |
| Formal methods | dream-verification |
| Economics / mechanism design | dream-token-economy |
| Affective computing | affect-causal-discovery, code-somatic-markers |
| Reinforcement learning | stigmergic-bandits, dream-token-economy |
| Software engineering | code-somatic-markers |

## Implementation priority (from source)

| Priority | Innovation |
|---|---|
| P0 (highest) | hdc-active-inference, code-somatic-markers |
| P1 | stigmergic-bandits, dream-token-economy |
| P2 | knowledge-morphogenesis, witness-world-model |
| P3 | affect-causal-discovery, dream-verification |

## Supporting files

- [_cross-innovation-interactions.md](./_cross-innovation-interactions.md) — the four paired feedback loops that connect all eight innovations
- [_by-domain/](./\_by-domain/) — per-domain index files
- [_by-subsystem/](./\_by-subsystem/) — per-subsystem index files

---

## Guiding principle

These innovations are **not incremental improvements**. They are structural compositions: the Synapse Architecture's trait-based design means each innovation is a new `impl` block that wires existing traits together, not a new subsystem to build from scratch. Every innovation listed here has a concrete algorithm, Rust sketch, and integration plan specified in the source.

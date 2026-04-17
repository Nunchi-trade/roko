# Crate Map

> **Abstract:** This document describes Roko's current workspace shape and the target crate boundaries proposed by
> `tmp/refinements/20-modularity-composability.md`. The present tree still contains the organically grown crate map
> that the audit documents in `docs/00-architecture/23-architectural-analysis-improvements.md` call out. The target
> shape is stricter: three new kernel crates (`roko-bus`, `roko-hdc`, `roko-spi`), two splits (`roko-std` →
> `roko-defaults` + `roko-tools`, `roko-compose` → `roko-compose-core` + `roko-templates`), and a dep graph that
> keeps implementations replaceable without cross-layer leakage.

> **Implementation**: Shipping for the current workspace; target boundaries are proposed, not all yet shipping.

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [12-five-layer-taxonomy](./12-five-layer-taxonomy.md), [01-naming-and-glossary](./01-naming-and-glossary.md)
**Key sources**:
- [tmp/refinements/20-modularity-composability.md](../../tmp/refinements/20-modularity-composability.md) — canonical modularity proposal
- [23-architectural-analysis-improvements](./23-architectural-analysis-improvements.md) — audit basis for the dep graph changes
- [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) — layer placement and kernel rules
- [01-naming-and-glossary](./01-naming-and-glossary.md) — terminology and current naming

---

## Abstract

Roko's workspace already follows the broad five-layer architecture, but the current crate map still mixes kernel
contracts, runtime infrastructure, and implementation details in ways that make refactors more expensive than they
should be. REF20 reframes the workspace as a cleaner module graph with explicit boundaries:

- current crates stay where they are until migrated;
- target crates capture the kernel surfaces that are currently implicit;
- implementations depend on traits and fabrics, not on adjacent concrete subsystems;
- the `dep graph` should be machine-checkable, not just documented.

The target picture is intentionally conservative about scope. It does not claim every split is already complete.
Instead, it defines the boundaries that future work should converge on so the core system can swap substrates, buses,
templates, and plugin surfaces independently.

This file is the workspace map for that target shape. It should be read alongside
[12-five-layer-taxonomy](./12-five-layer-taxonomy.md) for layer placement, and
[23-architectural-analysis-improvements](./23-architectural-analysis-improvements.md) for the audit findings that
justify the split.

---

## 1. Layer-by-Layer Crate Map

### 1.1 Current Workspace Shape

The current workspace has the following load-bearing crates relevant to the modularity proposal:

| Crate | Current role | Notes |
|---|---|---|
| `roko-core` | Kernel contracts | Holds the durable Engram model and the core traits today. |
| `roko-runtime` | Runtime infrastructure | Houses process supervision, clocks, and bus-like runtime plumbing. |
| `roko-primitives` | HDC support | Contains the vector/similarity machinery that the target plan isolates more narrowly. |
| `roko-std` | Default implementations + built-in tools | Currently mixes defaults with tool inventory. |
| `roko-compose` | Prompt assembly + templates | Currently mixes composition engine and template data. |
| `roko-agent` | LLM/provider integration | Depends on many framework pieces to bridge the application surface. |
| `roko-gate` | Verification harness | Sits in the harness layer. |
| `roko-fs` | File substrate | Concrete substrate implementation. |
| `roko-orchestrator` | Planning and scheduling | Application-level coordination. |
| `roko-conductor` | Reactive watchers | Current audit hotspot for cross-layer leakage. |
| `roko-learn`, `roko-neuro`, `roko-daimon`, `roko-dreams` | Cross-cuts | Reflective and cognitive subsystems. |
| `roko-spi`, `roko-extension-abi`, `roko-wasm-host` | Extension surface | SPI and runtime boundaries for plugins. |
| `roko-cli`, `roko-serve`, `roko-index`, `roko-lang-*`, `roko-plugin`, `roko-chain`, `mirage-rs` | Application and domain crates | Top-level consumers and domain-specific crates. |

This current shape is workable, but the audit shows where the boundary discipline is still fuzzy. The target map below
is the cleaner version of the same workspace story.

### 1.2 Target Kernel and Fabric Crates

REF20 makes four crate surfaces first-class at the kernel boundary:

| Target crate | Role | Status |
|---|---|---|
| `roko-core` | Kernel contracts, shared types, operator traits | Existing kernel crate. |
| `roko-bus` | Transport fabric for Pulses, topics, publish/subscribe semantics | New target kernel crate. |
| `roko-hdc` | Hyperdimensional vector operations, similarity, encoding, fingerprints | New target kernel crate. |
| `roko-spi` | Plugin and extension SPI contracts | Existing scaffolded contract crate, promoted to the kernel boundary in the target graph. |

The target rule is simple: these crates define the shared contracts. Everything else consumes them through traits or
data boundaries, not by reaching into implementation crates.

### 1.3 Target Framework Crates

The framework layer stays broad, but its crate boundaries get cleaner:

| Target crate | Role | Target change |
|---|---|---|
| `roko-defaults` | Default implementations | Split from `roko-std`; no builtin tool catalog. |
| `roko-tools` | Builtin tools and tool inventory | Split from `roko-std`; tool additions do not perturb defaults. |
| `roko-agent` | Model/provider integration | Remains a framework crate, but should stop depending on adjacent implementation details. |
| `roko-plugin` | Plugin discovery and loading | Consumes `roko-spi` rather than defining its own contract vocabulary. |
| `roko-extension-abi`, `roko-wasm-host` | Native and WASM extension boundaries | Stay as host boundaries for higher-power extensions. |

### 1.4 Target Scaffold and Harness Crates

| Target crate | Role | Target change |
|---|---|---|
| `roko-compose-core` | Prompt assembly engine | Split from `roko-compose`; keeps the compositional machinery. |
| `roko-templates` | Role and prompt templates | Split from `roko-compose`; templates become separately versioned data. |
| `roko-gate` | Verification harness | Remains a harness crate. |
| `roko-fs` | File substrate | Remains a concrete substrate implementation below the kernel contracts. |

The split here matters because template data changes much faster than the engine. Separating them makes the prompt
surface easier to replace and easier to distribute independently.

### 1.5 Target Orchestration and Cross-Cuts

| Target crate | Role | Target change |
|---|---|---|
| `roko-orchestrator` | Planning and scheduling | Continues to orchestrate from above the kernel. |
| `roko-conductor` | Reactive control and watchers | Should consume shared buses and topics instead of reaching into learning internals. |
| `roko-learn` | Episodes, playbooks, experiments | Should publish and subscribe via the Bus rather than exposing internal state. |
| `roko-neuro` | Durable knowledge store | Cross-cut that reads Substrate and feeds composition. |
| `roko-daimon` | Affect and motivation | Cross-cut that biases scoring and action selection. |
| `roko-dreams` | Offline consolidation | Cross-cut that consolidates back into durable memory. |

### 1.6 Target Applications and Domain Crates

Top-level entry points and domain crates remain application-facing consumers of the workspace:

| Crate | Role |
|---|---|
| `roko-cli` | User-facing application assembly and command surface. |
| `roko-serve` | Remote API surface. |
| `roko-index` | Code intelligence and symbol analysis. |
| `roko-lang-rust`, `roko-lang-typescript`, `roko-lang-go` | Language-specific indexing support. |
| `roko-chain` | Chain integration. |
| `mirage-rs` | In-process EVM simulation for chain testing. |

These crates are consumers, not boundary-setters. They should depend on the kernel contracts and the layer directly
below them, not on sibling implementation details.

---

## 2. Crate Dissolution and Splits

### 2.1 What Has Already Happened

The current workspace already reflects some of the long-running decomposition work from earlier architecture changes.
Those historical moves are useful context, but they are not the focus of this refinement. The important point is that
Roko is already capable of dissolving umbrella crates and splitting responsibilities cleanly when the dependency graph
supports it.

### 2.2 What REF20 Adds

REF20 proposes the next round of decomposition:

- `roko-bus` extracts transport semantics from the runtime surface.
- `roko-hdc` extracts HDC math and encoding from the broader primitives bucket.
- `roko-spi` becomes the shared extension contract surface.
- `roko-std` splits into `roko-defaults` and `roko-tools`.
- `roko-compose` splits into `roko-compose-core` and `roko-templates`.

The design goal is not to proliferate crates for its own sake. It is to make the boundary between “contract,”
“implementation,” and “data” explicit so that changes in one area do not force unrelated recompilation or code
movement elsewhere.

### 2.3 Target Boundary Note

These are target boundaries, not an assertion that all of them are already shipping. The current workspace still has
the pre-split crates, and the migration path should preserve compatibility while the new crates land.

For the motivating audit trail, see
[23-architectural-analysis-improvements](./23-architectural-analysis-improvements.md) and the canonical proposal in
[tmp/refinements/20-modularity-composability.md](../../tmp/refinements/20-modularity-composability.md).

---

## 3. Dependency Rules

### 3.1 Downward-Only Invariant

The target dep graph keeps the same broad five-layer shape, but it makes the kernel surfaces explicit:

```
L4 (Orchestration) → may depend on L3, L2, L1, L0, Kernel
L3 (Harness)       → may depend on L2, L1, L0, Kernel
L2 (Scaffold)      → may depend on L1, L0, Kernel
L1 (Framework)     → may depend on L0, Kernel
L0 (Runtime)       → may depend on Kernel only
Kernel             → depends on nothing
```

Here, “Kernel” means `roko-core`, `roko-bus`, `roko-hdc`, and `roko-spi`. Those crates define the shared contract
surface. Every other crate must consume them through traits, data, or host interfaces.

### 3.2 Target Dep Graph

The dep graph below is the intended direction of travel, not a snapshot of every current Cargo.toml:

```text
                    roko-cli / roko-serve / roko-index / domain crates
                                      ▲
                                      │
                     roko-orchestrator / roko-conductor / roko-learn
                                      ▲
                                      │
                      roko-gate / roko-fs / roko-dreams / roko-neuro
                                      ▲
                                      │
        roko-agent / roko-compose-core / roko-templates / roko-defaults / roko-tools
                                      ▲
                                      │
            roko-core ─── roko-bus ─── roko-hdc ─── roko-spi
                                      ▲
                                      │
                           runtime and host implementations
```

Concrete rules for that graph:

1. `roko-core`, `roko-bus`, `roko-hdc`, and `roko-spi` are the only kernel-tier crates.
2. Runtime and host crates implement the kernel contracts; they do not become new contract surfaces themselves.
3. Framework crates consume the kernel and should not import each other unless there is a documented reason.
4. `roko-compose-core` sits above the kernel and below application assembly; templates are data, not engine code.
5. `roko-defaults` and `roko-tools` should not depend on each other.
6. `roko-conductor` should react through the Bus and topics rather than importing `roko-learn` internals.
7. Cross-cuts (`roko-neuro`, `roko-daimon`, `roko-dreams`) may read kernel contracts and inject behavior, but they do
   not define new kernel types.

### 3.3 Stability Tiers

REF20 also implies a public-API stability model for the target workspace:

| Tier | Stability | Examples |
|---|---|---|
| Core | Semver-major-only breaks | `Engram`, `Substrate`, `Bus`, `Topic`, kernel traits |
| Extended | Minor-version breaks with notice | `Pulse`, `TopicFilter`, gate/routing helpers, default compositions |
| Experimental | Anything goes behind feature flags or scaffolds | future chain, dreams, and host-specific boundaries |

The important constraint is that plugin authors and downstream applications should be able to depend on the Core tier
without tracking every implementation split in the workspace.

### 3.4 What a Clean Graph Buys Us

The cleaner graph makes replacement cheaper:

- swap `roko-fs` without editing unrelated framework crates;
- swap or extend bus transport without changing agent logic;
- add templates without touching the composition engine;
- add builtin tools without perturbing defaults;
- keep plugin contracts stable while host implementations evolve.

Those are not abstract benefits. They are the specific maintenance costs the current audit says are too high.

---

## 4. Migration Plan

### Phase 1: Kernel extraction

- Extract `roko-bus` from runtime transport plumbing.
- Narrow `roko-hdc` to the HDC vector and similarity surface.
- Keep `roko-spi` as the shared plugin contract surface.

### Phase 2: Framework split

- Split `roko-std` into `roko-defaults` and `roko-tools`.
- Ensure defaults can build without inheriting the entire builtin tool catalog.
- Ensure tools can evolve without forcing defaults to change.

### Phase 3: Composition split

- Split `roko-compose` into `roko-compose-core` and `roko-templates`.
- Keep the engine stable while template data becomes separately versioned.
- Let role packs evolve independently of prompt assembly code.

### Phase 4: Enforcement

- Add CI checks that fail when the dep graph drifts outside the target boundaries.
- Deprecate direct imports where a trait, topic, or shared contract should be used instead.
- Keep the migration mechanical: one crate boundary at a time, not a workspace-wide rewrite.

The sequencing matters. Kernel first, then framework splits, then composition splits, then enforcement. That keeps the
workspace compiling while the boundaries tighten.

---

## 5. Stability Tiers for Public APIs

The workspace needs a simple rule for what downstream code may rely on:

| Tier | Stability intent | Boundary examples |
|---|---|---|
| Core | Long-lived, semver-stable contracts | `roko-core`, `roko-bus`, `roko-hdc`, `roko-spi` |
| Extended | Compatible, but allowed to evolve with notice | `roko-defaults`, `roko-tools`, `roko-compose-core`, `roko-templates` |
| Experimental | Internal or feature-gated surfaces | migration shims, scaffolds, future host-specific integrations |

This tiering is the practical answer to “what can a plugin depend on?” and “what can an application assume?” It also
keeps the refactor honest: a split is only useful if the resulting surface is stable enough to justify depending on it.

---

## 6. CI Dep Graph Enforcement

The target dep graph is only useful if CI can keep it honest.

A workspace-level check should verify, at minimum:

- kernel crates do not depend on implementation crates;
- `roko-std`-derived splits remain separated;
- `roko-compose`-derived splits remain separated;
- `roko-conductor` does not reintroduce a direct dependency on `roko-learn` for reactive state;
- forbidden edges fail fast before merge.

The check can be implemented however the repo prefers, but the rule is simple: the declared dep graph must match the
actual Cargo metadata. This is the difference between a tidy diagram and a maintainable architecture.

For the audit context behind that enforcement, see
[23-architectural-analysis-improvements](./23-architectural-analysis-improvements.md).

---

## 7. Current Status and Gaps

The current workspace is not yet the target workspace.

- `roko-bus`, `roko-hdc`, and the split crates are target boundaries proposed by REF20, not all fully shipped.
- `roko-std` and `roko-compose` still exist in the current tree as the combined crates.
- The audit still matters because it identifies the exact dependency edges the target graph is meant to eliminate.
- The migration should preserve current behavior while changing the dependency surface underneath it.

That is the main reason this doc stays a crate map rather than a pure design note: it needs to describe both the current
workspace and the boundary we want to converge on.

---

## 8. Legacy Crate Names

This section is retained only as legacy context for older references.

| Old name | Current name | Notes |
|---|---|---|
| `bardo-primitives` | `roko-primitives` | legacy name for the HDC support bucket |
| `bardo-runtime` | `roko-runtime` | legacy runtime naming |
| `golem-core` | `roko-core` | legacy kernel naming |
| `roko-golem` | retired | umbrella crate dissolved in the current architecture story |

Legacy names should only appear in historical or retired contexts. New prose should use the current `roko-*` names.

---

## Cross-References

- See [12-five-layer-taxonomy](./12-five-layer-taxonomy.md) for the layer assignments that this crate map plugs into.
- See [01-naming-and-glossary](./01-naming-and-glossary.md) for the canonical vocabulary, including the current
  kernel terms.
- See [23-architectural-analysis-improvements](./23-architectural-analysis-improvements.md) for the audit that
  motivates the target dep graph.
- See [tmp/refinements/20-modularity-composability.md](../../tmp/refinements/20-modularity-composability.md) for the
  full proposal that this doc propagates.

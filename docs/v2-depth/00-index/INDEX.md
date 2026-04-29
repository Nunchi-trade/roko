# 00-index — Depth Index

Depth for [00-INDEX.md](../../unified/00-INDEX.md)

---

## Source docs (13)

### Vision and thesis

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/00-architecture/00-vision-and-thesis.md` | Done | architectural-thesis.md |
| `docs/EXECUTIVE-SUMMARY.md` | Absorbed | 05-vision-and-positioning.md |
| `docs/VISION-RUN-ANYWHERE.md` | Absorbed | 05-vision-and-positioning.md |

### Naming and glossary

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/00-architecture/01-naming-and-glossary.md` | Done | design-principles-algebra.md |

### Taxonomy and crate structure

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/00-architecture/12-five-layer-taxonomy.md` | Done | architectural-thesis.md |
| `docs/00-architecture/15-crate-map.md` | Done | architectural-thesis.md |

### Design principles and analysis

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/00-architecture/17-design-principles-and-frontier-summary.md` | Done | design-principles-algebra.md |
| `docs/00-architecture/23-architectural-analysis-improvements.md` | Done | architectural-thesis.md, integration-topology.md |
| `docs/00-architecture/30-cross-pollination-innovations.md` | Done | integration-topology.md |
| `docs/00-architecture/31-implementation-readiness-audit.md` | Done | implementation-readiness.md |

### Integration maps

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/00-architecture/24-cross-section-integration-map.md` | Done | integration-topology.md |
| `docs/00-architecture/34-synergy-integration-map.md` | Done | integration-topology.md |
| `docs/00-architecture/35-consolidated-roadmap.md` | Done | implementation-readiness.md |

### External-facing

| Source doc | Status | Consumed by |
|---|---|---|
| `docs/COMPARISON.md` | Absorbed | 05-vision-and-positioning.md |
| `docs/USE-CASES.md` | Absorbed | 05-vision-and-positioning.md |

---

## Depth docs

| Doc | Redesigns | Key insight |
|---|---|---|
| [architectural-thesis.md](./architectural-thesis.md) | vision-and-thesis + five-layer-taxonomy + crate-map | Five layers derived from protocol dependency lattice, not arbitrary boundaries. L5 self-evolution when spec is a runtime artifact. |
| [design-principles-algebra.md](./design-principles-algebra.md) | design-principles + naming-glossary | Eight principles as algebraic laws on Signal/Cell/Graph. Structural (type-enforced) vs behavioral (convention-enforced). Missing principle: Variance Inequality. |
| [integration-topology.md](./integration-topology.md) | cross-section-integration-map + synergy-integration-map + cross-pollination | System as typed directed graph. 3 wired SCCs, 5 disconnected nodes, 3 bottleneck edges. Bus migration transforms hub-and-spoke to pub/sub. |
| [implementation-readiness.md](./implementation-readiness.md) | implementation-readiness-audit + consolidated-roadmap | Build sequence from topological sort of Cell dependency graph. 5 phases (A-E), 4 risk cliffs, parallelization analysis. |
| [test-strategy-for-self-improving-systems.md](./test-strategy-for-self-improving-systems.md) | comprehensive-test-strategy | Five test layers for systems that evolve: unit/protocol, property-based, integration, adversarial, observability contracts. Invariant testing > specific output testing. |
| [05-vision-and-positioning.md](./05-vision-and-positioning.md) | EXECUTIVE-SUMMARY + VISION-RUN-ANYWHERE + COMPARISON + USE-CASES | Protocol vs framework positioning. Four defensible differentiators (demurrage, HDC identity, cryptographic provenance, self-development). Use cases as Graph templates. Five-shape deployment model. Competitive comparison by capability axis. Network effect thesis via Merkle-CRDT. |

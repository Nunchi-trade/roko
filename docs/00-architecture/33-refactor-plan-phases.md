# Refactoring Plan Phases

> **Abstract:** This chapter turns the phased refactoring plan into the canonical architecture narrative for the two-medium, two-fabric transition. It is the "how, in what order, with what risk" companion to the foundational architecture docs. The primary source is [tmp/refinements/06-refactoring-plan.md](../../tmp/refinements/06-refactoring-plan.md); terminology follows [01-naming-and-glossary.md](01-naming-and-glossary.md).


> **Implementation**: Planned

---

## 1. Overview

Roko's refactoring path is deliberately staged so the documentation can land before the kernel changes, and the kernel changes can land before subsystem migration. The work is split into four phases:

1. **Phase A - Docs Alignment**: normalize the architecture docs around Engram, Pulse, Substrate, Bus, Topic, TopicFilter, Datum, and the seven-step loop.
2. **Phase B - Kernel Addition**: add `Pulse`, `Topic`, `TopicFilter`, `Bus`, and the supporting types without changing existing behavior.
3. **Phase C - Subsystem Migration**: move callers from ad hoc transport shapes to Bus-backed Pulses, subsystem by subsystem.
4. **Phase D - Chain & Mesh Buses**: add the additional Bus backends required for Phase 2+ deployment shapes.

Each phase is independently mergeable and reversible. Phase A is doc-only. Phase B is additive. Phase C is the first phase that can change runtime call sites, but the migration is compiler-assisted and can be staged one subsystem at a time.

For the broader sequencing picture, see also
[tmp/refinements/35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md).

---

## 2. Phase A - Docs Alignment

**Duration:** 1 week  
**Risk:** None to runtime  
**Merge shape:** Doc-only, reversible by revert

Phase A updates the architecture narrative before the kernel changes exist. The goal is to remove terminology drift and make the later code work easy to justify and verify.

### 2.1 Scope

- Rewrite foundational architecture docs to the two-medium / two-fabric framing.
- Remove stale equivalence disclaimers from the canonical narrative.
- Ensure the glossary defines the load-bearing names once, with cross-references from every chapter that introduces them.
- Keep the loop, layer, and cross-cut terminology synchronized with the newer vocabulary.

### 2.2 Expected deliverables

- Architecture chapters describe Engram as the durable medium and Pulse as its ephemeral sibling.
- Bus is treated as a kernel fabric, not a later proposal.
- Topic and TopicFilter are named as first-class routing concepts.
- The universal loop reads as seven steps with co-equal PERSIST and BROADCAST.
- The five-layer taxonomy places Bus at Layer 0 with Substrate.

### 2.3 Merge boundary

This phase is safe to land before any code work begins. If the kernel slips, the docs still describe the intended shape clearly, but the chapter should remain explicit that the code is not yet there.

---

## 3. Phase B - Kernel Addition

**Duration:** 2 weeks  
**Risk:** Low to moderate, because it is additive  
**Merge shape:** New types, new modules, compatibility shims

Phase B adds the transport vocabulary to the kernel without forcing the workspace to migrate immediately.

### 3.1 Core additions

- Add `Pulse` as the ephemeral medium.
- Add `Topic` and `TopicFilter` as routing primitives.
- Add `Bus` as a kernel trait alongside the existing storage and operator traits.
- Extend `Datum<'_>` so operator signatures can accept either Engram or Pulse.
- Add the graduation path so a Pulse can become an Engram when lineage matters.

### 3.2 Implementation shape

- `roko-core` exports the new kernel types.
- `roko-std` provides the initial in-process Bus implementations.
- Existing callers continue to compile through compatibility shims while the new path is introduced.

### 3.3 Merge boundary

Phase B should not change subsystem behavior. It exists to make the later migration mechanical instead of speculative.

---

## 4. Phase C - Subsystem Migration

**Duration:** 3-4 weeks  
**Risk:** Moderate, because call sites change  
**Merge shape:** One subsystem at a time, compiler-assisted

Phase C replaces subsystem-specific transport enums and broadcast assumptions with Pulse topics and Bus subscriptions. The point is not only to change names; it is to remove the hidden transport model that is scattered across the workspace.

### 4.1 Migration order

1. **Runtime-facing callers** move first so the shared transport surface becomes Bus-shaped.
2. **Orchestration** follows, because its publication stream is already plan-driven and easy to topic-encode.
3. **Agent-side streams** move next, including agent-to-agent and sidecar publication paths.
4. **Conductor** migrates to Bus subscriptions and sheds its dependency shape where the plan allows.
5. **Learning** becomes topic-driven for feedback loops and reactive policy outputs.
6. **TUI and HTTP surfaces** switch from polling or local broadcast assumptions to Bus subscriptions.

### 4.2 What changes

- Typed ad hoc transport enums are replaced with topic names plus typed payloads.
- Publishing becomes `Bus.publish(Pulse)`.
- Consumption becomes subscription-driven rather than queue-specific.
- Deprecated shims remain long enough for compiler-assisted migration, then disappear.

### 4.3 Merge boundary

Each subsystem can land independently. If one migration is delayed, the others still move, because the Bus abstraction is shared rather than global-state driven.

---

## 5. Phase D - Chain & Mesh Buses

**Duration:** Phase 2+  
**Risk:** High, because it expands the transport matrix  
**Merge shape:** New backends and composition layers

Phase D is the transport expansion that comes after the core Bus model is stable.

### 5.1 Scope

- Add a chain-backed Bus for on-chain replay.
- Add a mesh-backed Bus for multi-process and distributed deployment shapes.
- Add a multi-backend Bus composition layer where needed.

### 5.2 Why it is later

The additional backends only make sense once the in-process Bus semantics are settled. Landing them earlier would force backend-specific compromises into the core model.

---

## 6. Total Effort

| Phase | Scope | Engineers | Duration |
|---|---|---:|---|
| A | Docs alignment | 1 | 1 week |
| B | Kernel addition | 1 | 2 weeks |
| C | Subsystem migration | 1-2 | 3-4 weeks |
| D | Chain and mesh buses | 1-2 | Phase 2+ |
| **Total (A-C)** | | **1-2** | **~6-7 weeks** |

The estimate assumes serial ownership of the migration path with a small amount of parallelism inside Phase C.

---

## 7. Rollback Plan

- **Phase A**: revert the documentation change set. No runtime effect.
- **Phase B**: revert the additive kernel work. Existing behavior remains intact.
- **Phase C**: revert the affected subsystem migration only. Each subsystem is intended to be a self-contained unit of change.
- **Phase D**: revert the backend-specific crate or module additions if the distributed transport assumptions do not hold.

There is no point of no return in Phases A-C. The plan is explicitly staged so the workspace can stop after any phase and still remain coherent.

---

## 8. Risks

1. **Bus buffer sizing** may be too small for high-chatter topics or too large for memory-sensitive deployments.
2. **Graduation policy regressions** may let a Pulse remain ephemeral when lineage should have been preserved.
3. **Backend schema drift** may appear once more than one Bus backend exists, especially if replay semantics differ.
4. **Doc/runtime drift** can happen if Phase A lands far ahead of Phase B and the docs read more complete than the code.
5. **Migration churn** may surface in Phase C if several subsystems try to change their transport shapes at once.

The main mitigation is the phase boundary itself: keep additive work additive, and keep the runtime changes isolated to one subsystem at a time.

---

## 9. Checkpoint Criteria

### Phase A done when

- The architecture chapter set consistently uses the two-medium / two-fabric framing.
- The glossary has entries for Engram, Pulse, Bus, Topic, and TopicFilter.
- The loop chapter and layer chapter both reflect the seven-step / L0 Bus framing.
- No canonical chapter still relies on the old equivalence disclaimer as its primary explanation.

### Phase B done when

- `Pulse`, `Bus`, `Topic`, `TopicFilter`, `Datum`, and the graduation path are exported from `roko-core`.
- The initial Bus implementations exist and are testable in isolation.
- Existing callers still compile, even if they have not migrated yet.

### Phase C done when

- Subsystem-specific transport enums are gone from the migrated call paths.
- Publishing uses Pulse topics rather than local broadcast assumptions.
- The TUI, conductor, learning, and orchestration paths all consume the shared transport model.

### Phase D done when

- Chain and mesh backends have parity with the core Bus surface.
- Replay behavior is defined and tested for each backend.
- Multi-backend composition works without forcing backend-specific details into callers.

---

## 10. Metrics That Should Move

| Metric | Baseline | Target after Phase C |
|---|---|---|
| Cross-crate subsystem transport imports | non-zero | 0 |
| Polling loops in the TUI | present | 0 |
| Transport-specific coupling in conductor and learning | present | reduced to shared Bus abstractions |
| Direct subsystem broadcast assumptions | present | eliminated in migrated paths |
| Workspace-level transport surface clarity | mixed | topic-driven and documented |

The qualitative target is simple: the architecture should look less like several incompatible
transport surfaces and more like one transport model with named topics.

---

## 11. Follow-On Refinements

This chapter sets up the refinements that depend on the new transport model:

- [10-self-learning-cybernetic-loops.md](../../tmp/refinements/10-self-learning-cybernetic-loops.md) can use Bus-driven prediction and outcome Pulses.
- [11-hyperdimensional-substrate.md](../../tmp/refinements/11-hyperdimensional-substrate.md) can combine HDC retrieval with the new storage and transport split.
- [12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md) inherits a cleaner storage/transport boundary.
- [13-collective-intelligence-c-factor.md](../../tmp/refinements/13-collective-intelligence-c-factor.md) gets a richer Bus substrate for coordination metrics.
- [17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md) benefits from the clearer runtime transport surface.
- [26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) can build on the Bus abstractions for projection and filtering.
- [35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md) remains the higher-level sequencing view.

---

## 12. Summary

This plan is intentionally conservative: it lands the docs first, then the kernel vocabulary, then the subsystem rewrites, and only after that the distributed transport backends. That order keeps the workspace mergeable at every step and makes the final model easier to verify.

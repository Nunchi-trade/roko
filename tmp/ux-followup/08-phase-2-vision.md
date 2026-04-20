# Phase 2 Vision — Golem / Chain / Dreams / Full-TUI / HTTP-Server Roadmap

> **Status (post-PR-13)**: no change. All 6 items remain P2 / strategic; not
> blocked or unblocked by PR #13. Confirmed aligned with CLAUDE.md "Self-hosting
> workflow" priorities (Phase 1 must close before Phase 2 starts). Refreshed
> 2026-04-16.

## Summary

Strategic, multi-week items. Not required for PR #13 or the next few batches,
but listed so the catalogue captures the shape of the longer roadmap. None of
these should be scheduled before Phase 1 (closing the self-hosting loop in
CLAUDE.md items 7–9) is complete.

## Items

### 49. Roko-golem chain-witness subsystem

**Evidence**: CLAUDE.md Key-crates: `roko-golem | Chain witness, daimon, dreams, grimoire | Phase 2+`. Existing `crates/roko-golem/` has skeleton but no integration path.

**Current state**: Module compiles, has exports; never called outside its own tests.

**Gap**: Design phase — decide chain-witness semantics (what's being witnessed, where is the chain of evidence persisted, who consumes it). Documents in `bardo-backup/tmp/agent-chain/` have historical context.

**Fix scope**: 2–3 weeks for a real implementation. Blocked on design.

**Priority**: P2.

---

### 50. Roko-chain primitives

**Evidence**: `crates/roko-chain/` exists; CLAUDE.md marks Phase 2+. `bardo-backup/tmp/agent-chain/` has design notes.

**Current state**: Empty or skeletal.

**Gap**: Implement once golem is specced.

**Fix scope**: 2 weeks post-design.

**Priority**: P2.

---

### 51. Roko-dreams full cycle (hypnagogia → imagination → consolidation)

**Evidence**: `crates/roko-dreams/src/` has `imagination.rs`, `hypnagogia.rs`, `cycle.rs`. Phase-2 per CLAUDE.md.

**Current state**: Compiles, has tests, no production invocation.

**Gap**: Full offline consolidation loop — drain recent episodes, cluster via HDC (from roko-primitives), compress into playbooks. Designed; not wired.

**Fix scope**: 2 weeks (consolidation heuristics + HDC bridge + playbook sink).

**Priority**: P2.

---

### 52. TUI "full Mori" beyond parity — live-edit of `roko.toml`, inline PRD editor, etc.

**Evidence**: T1–T19 parity batches bring us to feature-match with Mori's TUI. Mori has additional editor modes (TOML live-edit, PRD in-terminal edit) not covered by any T-batch.

**Current state**: Parity work in flight; "beyond parity" not started.

**Gap**: New widgets. A textarea-modal + commit-on-Enter flow.

**Fix scope**: 2 weeks per editor. Non-trivial ratatui work.

**Priority**: P2.

---

### 53. HTTP server Phase-2 — auth, multi-tenant, public cloud

**Evidence**: `roko-serve` has ~85 routes, zero auth middleware beyond the per-agent token stub in `/agents/{id}/token`. Multi-tenancy is not modelled.

**Current state**: Single-operator, localhost-only.

**Gap**: Add an `AuthLayer` on the axum router, JWT-based, tenant-aware. Document a public-cloud deployment mode.

**Fix scope**: 3 weeks. Security review + ops story.

**Priority**: P2.

---

### 54. `roko-plugin` — third-party extensibility surface

**Evidence**: `crates/roko-plugin/` exists; CLAUDE.md doesn't even list it in Key-crates. No external consumers documented.

**Current state**: Unknown — may be vestigial or in-progress.

**Gap**: Decide fate: delete, document as public ABI, or fold into another crate.

**Fix scope**: 2 hours audit + direction.

**Priority**: P2.

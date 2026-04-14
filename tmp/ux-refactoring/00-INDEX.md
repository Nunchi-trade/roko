# UX Refactoring Task List

**Generated**: 2026-04-14
**Total tasks**: ~100 across 6 sections
**Scope**: Everything remaining after integrate-prds batches 1-8

## Files

| File | Section | Tasks | Description |
|------|---------|-------|-------------|
| [A-dashboard-backend.md](A-dashboard-backend.md) | A | 10 | sdb-spec items: agent owner, skills, messaging, predictions, etc. |
| [B-demo-features.md](B-demo-features.md) | B | 18 | Demo scenario work: LLM providers, yield routing, TUI demo mode |
| [C-architecture-migration.md](C-architecture-migration.md) | C | 8 | Per-agent server crate, mirage-rs extraction, aggregator, auth |
| [D-architectural-gaps.md](D-architectural-gaps.md) | D | 44 | From integrate-prds/08: orchestration, agent, neuro, dreams, learning, coordination, heartbeat |
| [E-feedback-loops.md](E-feedback-loops.md) | E | 8 | Cybernetic feedback loops from integrate-prds/09 Tier 1M |
| [F-tui-interfaces.md](F-tui-interfaces.md) | F | 12 | Interactive TUI, CLI additions, daemon mode, deployment |

## Status Legend

- **NOT DONE** — No code exists
- **PARTIAL** — Some code exists but incomplete
- **SCAFFOLD** — Struct/trait exists but no real implementation
- **DONE** — Fully implemented (listed for completeness)

## Priority Legend

- **P0** — Demo-critical / blocks other work
- **P1** — Important for self-hosting loop
- **P2** — Valuable but not blocking
- **P3** — Future / research-grade

## Dependency Graph (cross-section)

```
A.01 (owner field) ──────────────────────┐
A.05 (agent messaging) ─────────────┐    │
A.04 (task artifacts) ──────────┐   │    │
                                │   │    │
                                ▼   ▼    ▼
                             A.10  A.08  C.01 (agent-server)
                                         │
                                    ┌────┴────┐
                                    ▼         ▼
                                 C.02      C.03
                              (mirage)  (aggregator)
                                    │         │
                                    └────┬────┘
                                         ▼
                                      C.04 (auth)

B.* (demo) ── independent track, parallel with A/C
D.* (gaps) ── mostly independent, some depend on E.*
E.* (feedback) ── can start after core wiring exists
F.* (TUI) ── F.01 is highest self-hosting priority
```

## Source Documents

| Source | Path |
|--------|------|
| sdb-spec | `tmp/sdb-spec/` (11 files) |
| Demo plan | `tmp/demo/DEMO-IMPLEMENTATION-PLAN.md` |
| Demo tasks | `tmp/demo/tasks/` (18 task files) |
| UX architecture | `tmp/ux/` (7 files) |
| Architectural gaps | `tmp/integrate-prds/08-DEEP-ARCHITECTURAL-GAPS.md` |
| Feedback loops | `tmp/integrate-prds/09-REFACTORING-PRD-ADDITIONS.md` |
| Build sequence | `tmp/integrate-prds/06-BUILD-SEQUENCE.md` |

## Overnight Execution Manifest

The raw section files are too large for unattended implementation. The overnight
runner uses smaller batches with explicit write scopes, dependencies, and verify
gates. The canonical batch definitions live in:

- [`BATCHES.md`](BATCHES.md)
- [`SOURCE-INDEX.md`](SOURCE-INDEX.md)
- `context-pack/`
- `prompts/`

### Recommended order

| Batch | Scope | Tasks | Primary write scope | Depends on |
|------|-------|-------|---------------------|------------|
| `A1` | Dashboard backend foundations | `A.01-A.05` | `apps/mirage-rs`, `crates/roko-serve` | none |
| `A2` | Dashboard backend completion | `A.06-A.10` | `apps/mirage-rs`, `crates/roko-serve`, `crates/roko-cli` | `A1` |
| `B1` | Demo foundations | `B.01-B.06` | `crates/roko-demo`, `contracts`, `demo/` | none |
| `B2` | Demo integration and polish | `B.07-B.18` | `crates/roko-demo`, `contracts`, `demo/` | `B1` |
| `C1` | Agent-server architecture core | `C.01-C.05` | `crates/roko-agent-server`, `crates/roko-serve`, `apps/mirage-rs` | none |
| `C2` | Migration cleanup and muxing | `C.06-C.08` | `apps/mirage-rs`, `crates/roko-serve`, docs | `C1` |
| `D1` | Core runtime gaps | `D.02-D.17` | `crates/roko-core`, `roko-chain`, `roko-neuro`, `roko-daimon`, `roko-agent`, `roko-cli` | none |
| `E1` | Feedback-loop wiring | `E.01-E.08` | `crates/roko-learn`, `roko-conductor`, `roko-compose`, `roko-orchestrator`, `roko-cli` | `D1` |
| `D2` | Orchestrator and dreams middle layer | `D.18-D.33` | `crates/roko-orchestrator`, `roko-agent`, `roko-runtime`, `roko-dreams` | `D1`, `E1` |
| `D3` | Long-horizon learning and deployment | `D.34-D.54` | `crates/roko-neuro`, `roko-learn`, `roko-daimon`, `roko-dreams`, `roko-compose`, `roko-cli` | `D2` |
| `F1` | TUI/API core | `F.01-F.06` | `crates/roko-cli/src/tui`, `crates/roko-serve` | none |
| `F2` | Interface extras | `F.07-F.12` | `crates/roko-cli`, `crates/roko-mcp-*`, `crates/roko-learn` | `F1` |

## Concurrency Notes

- The active `tmp/tui` runner already consumes one parallel coding lane.
- This UX harness is intentionally single-worktree and one-agent-at-a-time by
  default so it can run safely beside that job.
- If you later split work across multiple runners, keep them on disjoint tracks:
  `B*` can usually run beside `A*` or `C*`, but `D*`, `E*`, and `F*` all touch
  `roko-cli` and should not overlap.

## How To Use

See [`README.md`](README.md) for the runner entrypoint and failure/retry model.

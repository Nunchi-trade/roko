# Roko Post-PR-13 Gap Catalog — Master Index

> **Catalogue refreshed 2026-04-16 post-PR #13 merge** (`5ff264c9`). The 16 files
> listed below extend the original 12-file catalogue from earlier today with
> four new files (12–15) that capture TUI event-parity, session/state
> management, observability, and safety/learning gaps surfaced by the fresh
> audit sweep.
>
> **Re-audit 2026-04-20**: 48 more items verified as DONE against the actual
> codebase. The self-hosting P0 blockers (items 89 + 90 = CLAUDE.md items 10-11)
> are now fully closed. Total: 72 DONE / 40 open (was 24 DONE / 88 open).

## Purpose

This directory catalogues ~40 remaining actionable items plus ~72
evidence-trail entries for work completed through PR #13 and subsequent
commits on the `agent-refinements` branch. It is the single source of truth for:

1. What has been closed (the 72 `[DONE]` items).
2. What remains open (~40 items, broken down by severity below).
3. Which batch skeletons (`11-execution-plan.md`) queue the next runner pass.

Each file is focused, scannable, and self-contained. The severity tags let a
runner or human pick the next batch without reading prose.

## Severity vocabulary

| Tag | Meaning |
|-----|---------|
| **P0** | Blocks self-hosting closure (CLAUDE.md items 10–11) or leaves an advertised feature silently broken |
| **P1** | Hygiene, missing plumbing, easy wins (1–3 days) |
| **P2** | Strategic / larger-effort / roadmap (> 1 week) |
| **[DONE]** | Retained as evidence of PR #13 completion; no remaining action |

## File index

| # | File | Items | DONE | Open | Max severity | One-liner |
|---|------|-------|------|------|--------------|-----------|
| 00 | [00-INDEX.md](00-INDEX.md) | — | — | — | — | This file |
| 01 | [01-verified-p0-bugs.md](01-verified-p0-bugs.md) | 4 | 4 | 0 | [DONE] | Pre-merge hotfixes — all closed |
| 02 | [02-high-impact-quick-wins.md](02-high-impact-quick-wins.md) | 10 | 10 | 0 | [DONE] | All quick wins closed (2026-04-20 re-audit) |
| 03 | [03-non-batch-followups.md](03-non-batch-followups.md) | 6 | 5 | 1 | P1 | Deferred from T9–T19; only snapshot tests remain |
| 04 | [04-t9-t19-residuals.md](04-t9-t19-residuals.md) | 10 | 7 | 3 | P1 | Per-batch cleanup; runner hardening pending |
| 05 | [05-partially-wired-subsystems.md](05-partially-wired-subsystems.md) | 12 | 8 | 4 | P1 | 8 subsystems wired; Phase 2+ + MCP audit remain |
| 06 | [06-advanced-agent-backends.md](06-advanced-agent-backends.md) | 6 | 0 | 6 | P1 | Codex/Cursor/cascade-router parity (not audited) |
| 07 | [07-spec-code-drift.md](07-spec-code-drift.md) | 11 | 8 | 3 | P1 | All P0 drift items closed; docs sweeps remain |
| 08 | [08-phase-2-vision.md](08-phase-2-vision.md) | 6 | 0 | 6 | P2 | Chain / dreams / full-TUI / HTTP-server roadmap (not audited) |
| 09 | [09-hygiene-and-test-coverage.md](09-hygiene-and-test-coverage.md) | 11 | 8 | 3 | P1 | Major hygiene items closed; clippy docs + flaky tests remain |
| 10 | [10-stale-docs.md](10-stale-docs.md) | 8 | 3 | 5 | P1 | Terminology renames, old paths, banners |
| 11 | [11-execution-plan.md](11-execution-plan.md) | — | — | — | — | Phases A–G + T20–T32 batch prompt skeletons |
| 12 | [12-tui-event-parity.md](12-tui-event-parity.md) | 11 | 4 | 7 | P1 | notify watcher + hub wired; incremental tail-read remains |
| 13 | [13-session-state-mgmt.md](13-session-state-mgmt.md) | 4 | 3 | 1 | P1 | Schema versioning + zombie cleanup done; migration framework pending |
| 14 | [14-observability-gaps.md](14-observability-gaps.md) | 6 | 5 | 1 | P1 | All closed except per-gate timeline widget |
| 15 | [15-safety-and-learning-closure.md](15-safety-and-learning-closure.md) | 7 | 7 | 0 | [DONE] | Self-hosting loop fully closed (2026-04-20 re-audit) |

**Totals**: 112 catalogue entries = 72 DONE (evidence trail) + 40 open.

## Severity matrix (open items only, re-audited 2026-04-20)

| Category | P0 | P1 | P2 |
|----------|----|----|----|
| 02 high-impact-quick-wins | 0 | 0 | 0 |
| 03 non-batch-followups | 0 | 1 | 0 |
| 04 t9-t19-residuals | 0 | 3 | 0 |
| 05 partially-wired | 0 | 3 | 1 |
| 06 advanced-agent-backends | 0 | 5 | 1 |
| 07 spec-code-drift | 0 | 3 | 0 |
| 08 phase-2-vision | 0 | 0 | 6 |
| 09 hygiene | 0 | 3 | 0 |
| 10 stale-docs | 0 | 5 | 0 |
| 12 tui-event-parity | 0 | 6 | 1 |
| 13 session-state-mgmt | 0 | 1 | 0 |
| 14 observability-gaps | 0 | 1 | 0 |
| 15 safety-and-learning-closure | 0 | 0 | 0 |
| **Open totals** | **0** | **31** | **9** |

## Recommended read order (updated 2026-04-20)

1. **Self-hosting loop**: `15-safety-and-learning-closure.md` — all 7 items
   now DONE, including the two P0 blockers (CLAUDE.md items 10-11). File is
   now an evidence trail.
2. **Quick wins**: `02-high-impact-quick-wins.md` — all 10 items now DONE.
   Evidence trail only.
3. **TUI event-parity**: `12-tui-event-parity.md` — 4 items closed (polling
   replaced by notify watcher + in-process hub). 7 items remain: incremental
   tail-reading for episodes/signals/events/task-outputs and the learning-data
   watcher.
4. **Per-batch residuals**: `04-t9-t19-residuals.md` for 3 runner-hardening
   items still open.
5. **Stale docs**: `10-stale-docs.md` — 5 doc-only items (banners, terminology).
6. **Hygiene**: `09-hygiene-and-test-coverage.md` — 3 items remain (clippy docs,
   flaky tests, cascade router e2e).
7. **Partially-wired**: `05-partially-wired-subsystems.md` — 4 items remain
   (Phase 2+ crates, MCP audit, gate-rung wiring).
8. **Phase 2 vision**: `08-phase-2-vision.md` (read once; parked).
9. **Agent backends**: `06-advanced-agent-backends.md` (not re-audited this pass).

## Post-PR-13 delta — items newly marked `[DONE]`

PR #13 + its follow-up commits closed the following catalogue entries. Each is
preserved in-file as an evidence-trail entry with commit hash + file:line:

- `01` items **01, 02, 03, 04** — all pre-merge P0 hotfixes.
- `02` items **05** (auto-plan-on-promote) and **07** (ScrollAccel wired).
- `03` items **15** (T14 modal consolidation), **16** (T17 scroll/nav), **17**
  (T19 integration tests).
- `04` items **21, 22, 23, 24, 25, 26** — all T12–T19 batch closeouts.
- `07` items **41, 42, 43, 44, 48** — PR body + CLAUDE.md spec/code drift
  reconciliations.
- `09` item **57** — messaging integration tests landed via `c9029e20`.
- `10` items **61, 62, 63** — CLAUDE.md TUI-status and crate-naming fixes.

Twenty-four `[DONE]` entries from PR #13; none are actionable work.

## 2026-04-20 re-audit — 48 additional items marked `[DONE]`

A full codebase audit on the `agent-refinements` branch verified 48 items
that were implemented since the 2026-04-16 catalogue freeze:

- `02` items **06, 08, 09, 10, 11, 12, 13, 14** (all 8 remaining quick wins).
- `03` items **18** (CancellationToken), **20** (experiment winners panel).
- `04` item **28a** (CI dry-run for runner).
- `05` items **29** (enrichment wired), **30** (HDC per-episode), **31** (diagnosis
  panel), **35** (metrics schema), **35b** (playbook query), **35c** (verdict readers),
  **35d** (agent contracts enforced), **35e** (role-based tool ACL).
- `07` items **48a** (adaptive threshold load), **48b** (roko.toml role keys),
  **48c** (CLAUDE.md items 10-11 closed).
- `09` items **55** (unwrap cleanup), **59** (CI coverage), **60** (e2e smoke test),
  **60a** (OpenAPI), **60b** (Episode backend field), **60d** (snapshot schema version),
  **60e** (ProcessSupervisor zombie cleanup).
- `12` items **68** (standalone hub), **69** (notify watcher), **75** (git notify),
  **77** (bounded channel).
- `13` items **79** (schema version), **80** (SIGTERM escalation + Drop impl),
  **82** (resume reconciliation).
- `14` items **83** (verdict trend reader), **84** (diagnosis endpoint + panel),
  **85** (efficiency trend aggregation), **86** (canonical metric schema),
  **88** (experiment winners rendered).
- `15` items **89** (gate feedback replan), **90** (PRD-publish auto-trigger),
  **91** (agent contracts enforced), **92** (role tool whitelist), **93** (HDC
  fingerprint), **94** (playbook query), **95** (enrichment pipeline).

All six P0 blockers are now closed. Seventy-two `[DONE]` entries in total.

## How to consume this catalog

- **Reviewer**: open `00-INDEX.md` → `01` (verify closed) → `15` (self-hosting
  P0s) → `12` (TUI P0s).
- **Batch runner (Codex/Claude)**: `11-execution-plan.md` contains the
  T20–T32 skeletons with read-lists and write-scopes.
- **Strategic planning**: `08-phase-2-vision.md` is the long-range roadmap.

## Source evidence (refreshed)

- PR #13 merge commit: `5ff264c9` (`Merge pull request #13 from
  Nunchi-trade/roko-integrate-prds-followup`).
- T9–T19 afternoon merge: `e792e649` (`Merge TUI parity batches T9-T19 from
  afternoon Codex runner`).
- T19 integration tests: `c9029e20`.
- Runner log: `tmp/tui-parity/logs/run-20260416-101433/status.tsv` (partial;
  stopped at T14).
- `bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md` — 1 253-item
  parity list (stale; see item 46 for a live regeneration plan).
- `CLAUDE.md` — canonical project status (refreshed post-PR-13; items 10–11
  remain the last two "What to work on" blockers).

## Out-of-scope for this catalogue refresh

No code changes were made. No git operations ran. No PR body edits. No batch
prompts were created outside this directory. Phases C–F of the execution plan
(`11-execution-plan.md`) flag work but don't start it.

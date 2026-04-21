# Stale Docs — Terminology, Paths, Banners

> **Status (post-PR-13)**: items 61, 62, 63 closed; 64–67 still open + 1 new
> stale-plans-audit item. Refreshed 2026-04-16.
>
> **Re-audit 2026-04-20**: 0 additional items closed. 5 items still open (64, 65, 66, 67, 67a).
> Note: items 65 and 66 are partially addressed (grimoire/styx/clade removed from CLAUDE.md
> and live code, but linger in tmp/ runner prompts; death concepts removed from CLAUDE.md
> but roko-daimon still has mortality.rs). These remain open until fully swept.

## Summary

Documentation-only items. All read-only modifications to markdown files; no
code changes. Most are 5-minute fixes that prevent a newcomer from being
misled. Batch as one PR to avoid many tiny commits.

## Items

### 61. [DONE] CLAUDE.md "TUI is text-only" blocker line

**Resolved in**: CLAUDE.md refresh (`b1bba746`). The "Known blockers" section
no longer claims TUI is text-only; the "Status table" now explicitly lists
`Interactive TUI (ratatui) — Wired`.

**Status**: ✅ DONE.

---

### 62. [DONE] CLAUDE.md "Text dashboard | Scaffold" row

**Resolved in**: Same CLAUDE.md refresh. The status row now reads
"Interactive TUI (ratatui) | Wired | crates/roko-cli/src/tui/, F1–F7 tabs,
roko dashboard".

**Status**: ✅ DONE.

---

### 63. [DONE] CLAUDE.md Key-crates table: `bardo-primitives` → `roko-primitives`

**Resolved in**: CLAUDE.md Key-crates table now lists `roko-primitives` with
the descriptor `HDC vectors, tier routing` and the wiring note `Tier wired in
orchestrate/neuro/learn; HDC fingerprint-per-episode pending`.

**Status**: ✅ DONE.

---

### 64. `bardo-backup/tmp/roko-progress/*.md` — add stale-snapshot banners

**Evidence**: See item 47. Entire directory is a frozen pre-PR-13 snapshot;
reads as live.

**Direction**: Mass sed to prepend:
```
> ⚠ Historical snapshot from <date>. **Not** kept in sync with the current code.
> For current state, see `CLAUDE.md` or `tmp/ux-followup/`.
```

**Fix scope**: 30 minutes including a spot check.

**Priority**: P1.

---

### 65. Terminology: `grimoire` → `neuro`, `styx` → `Korai`, `clade` → `fleet`

**Evidence**: User auto-memory `~/.claude/projects/.../memory/feedback_naming_conventions.md`
confirms the renames. Search for old names in live docs:
```
grep -rn 'grimoire\|styx\|clade' tmp/ CLAUDE.md README.md --include='*.md'
```

**Current state**: Old names linger in `bardo-backup/` (expected) and may
appear in `tmp/`. Sweep + rename in live docs only.

**Direction**: Replace each occurrence with the canonical current vocabulary.
Leave `bardo-backup/` alone.

**Fix scope**: 1 hour.

**Priority**: P1.

---

### 66. Death concepts removed — ensure no stragglers

**Evidence**: User auto-memory: "Death concepts removed." Some older design
docs referenced "mortal", "death", "reincarnation".

**Current state**: Unknown presence in CLAUDE.md or `tmp/`.

**Direction**: Grep and replace any live references with the current
vocabulary (e.g. "tier progression", "retirement").

**Fix scope**: 1 hour.

**Priority**: P1.

---

### 67. MORI-PARITY-CHECKLIST.md stale paths

**Evidence**: `bardo-backup/tmp/roko-progress/MORI-PARITY-CHECKLIST.md`
references `apps/mori/*` paths which do not exist under `crates/roko-*`.

**Current state**: Newcomers follow the path and get confused.

**Direction**: Either (a) leave as-is with a banner (item 64), or (b) invest
in a live regeneration of the checklist (item 46).

**Fix scope**: See items 46 and 64.

**Priority**: P1.

---

### 67a. `tmp/implementation-plans/` stale "pending" markers

**Evidence**: `tmp/implementation-plans/00-INDEX.md` (per CLAUDE.md
"Implementation plans" path) lists implementation plans whose status fields
were last refreshed pre-PR-13. T1–T19 completion is not reflected; items
that landed under PR #13 may still show as "pending" or "in-flight".

**Current state**: Stale status markers in the index obscure real progress.

**Direction**: Walk `tmp/implementation-plans/00-INDEX.md` and mark items now
completed by PR #13 / the merge of `e792e649`. Cross-ref the post-PR-13 delta
in `00-INDEX.md` of this catalogue.

**Fix scope**: 1 hour audit + edit.

**Priority**: P1.

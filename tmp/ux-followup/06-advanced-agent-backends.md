# Advanced Agent Backends — Codex / Cursor / Streaming / Test Parity

> **Status (post-PR-13)**: 5 original items still open + 1 new (cascade router
> integration testing). Refreshed 2026-04-16.

## Summary

`roko-agent` supports 8+ backends (refreshed LOC counts):

| Backend / surface | File | LOC |
|------------------|------|-----|
| Tool loop core | `tool_loop/mod.rs` | 1 686 |
| OpenAI-compat provider | `provider/openai_compat.rs` | 1 387 |
| Dispatcher | `dispatcher/mod.rs` | 1 069 |
| Provider trait | `provider/mod.rs` | 1 051 |
| Multi-pool | `multi_pool.rs` | 1 007 |
| OpenAI-compat backend | `openai_compat_backend.rs` | 973 |
| Codex agent | `codex_agent.rs` | 966 |
| Claude API agent | `claude_agent.rs` | 966 |
| Task runner | `task_runner.rs` | 929 |
| Gemini native | `gemini/native.rs` | 883 |
| Claude CLI agent | `claude_cli_agent.rs` | 878 |
| Cursor agent | `cursor_agent.rs` | 763 |
| OpenAI agent | `openai_agent.rs` | 611 |
| Exec / generic | `exec.rs` | ~600 |

The dispatcher is wired, but coverage across backends is uneven — some have
streaming, some don't; tests are concentrated in `claude_*`; tool-loop
integration varies. Items below bring the non-Claude backends to parity.

## Items

### 36. Codex backend — full protocol verification

**Evidence**: `crates/roko-agent/src/codex_agent.rs` (966 LOC). Codex's newest
protocol (multi-reasoning, draft-frame semantics) shipped late 2025. No audit
confirms support for the full frame set.

**Current state**: Codex requests go through; exhaustive feature parity uncertain.

**Gap**: Build a conformance test harness that replays a captured Codex session
log and diffs against observed responses. Start with a 10-turn fixture.

**Fix scope**: 2 days. One new test file, one fixture.

**Priority**: P1.

---

### 37. Cursor backend — streaming path

**Evidence**: `crates/roko-agent/src/cursor_agent.rs` (763 LOC). Cursor's
agent API supports streaming; unclear whether our implementation calls the
streaming endpoint or polls.

**Current state**: Works non-streaming; streaming either incomplete or not wired.

**Gap**: Verify `send_turn_streaming` is implemented and plumbed via the
`LlmBackend` trait.

**Fix scope**: 2 days. If missing, port the Claude streaming pattern.

**Priority**: P1.

---

### 38. Test parity across backends

**Evidence**: `ls crates/roko-agent/src/` shows backend files of similar LOC,
but a grep for `mod tests` density inside each shows Claude backends are more
tested than Cursor/Codex (by simple line count comparison).

**Current state**: Test coverage uneven — dispatcher shim passes for all, but
backend-specific edge cases (tool calls, reasoning frames, partial chunks)
mostly live in Claude tests.

**Gap**: Replicate the Claude test suite shape for Codex and Cursor. At
minimum: happy path, streaming path, tool-call path, error-path,
session-continuation.

**Fix scope**: 2–3 days. Boilerplate-heavy.

**Priority**: P1.

---

### 39. `ExecAgent` — generic CLI backend, production readiness

**Evidence**: `crates/roko-agent/src/exec.rs` (~600 LOC) plus 8+ tests. The
dispatcher wires it. CLAUDE.md lists it as part of the Claude CLI + ExecAgent
agent dispatch.

**Current state**: Works for any CLI-based model (e.g. local models behind a
`llama.cpp` wrapper), but no streaming, no tool-call support, no MCP injection.

**Gap**: Document which backends should choose `ExecAgent` vs `ClaudeCliAgent`
(overlapping purpose). Possibly delete one.

**Fix scope**: 1 day. Decision + consolidation.

**Priority**: P1.

---

### 40. Gemini / Perplexity / Ollama — directory presence without unit-level parity

**Evidence**: `crates/roko-agent/src/gemini/`, `perplexity/`, `ollama/` are
directories (not single files). For Gemini: `gemini/native.rs` is 883 LOC; for
Perplexity: `chat.rs` 678, `deep_research.rs` 629, `tool_loop.rs` 628,
`search.rs` 627. Ollama splits between `ollama_agent.rs` and
`ollama_backend.rs` suggesting duplication.

**Current state**: Unclear delineation between backend file vs `mod.rs` for
each multi-file backend.

**Gap**: Unify. One file per backend OR one directory per backend with a
clear `mod.rs` — don't do both.

**Fix scope**: 1–2 days. Refactor + test stabilisation.

**Priority**: P2 (nice-to-have; no user-visible bug if left as-is).

---

### 40a. Cascade / model router untested against real backends

**Evidence**: `crates/roko-learn/src/cascade_router.rs` (4 766 LOC) +
`model_router.rs` (2 170 LOC). Both have unit tests against mock dispatchers;
neither has an integration test that drives an end-to-end selection through a
real backend (or a tightly-mocked dispatcher that matches a real backend's
trait impl).

**Current state**: Selection algorithm tested in isolation; the wiring that
turns a routing decision into an actual provider call is exercised only
manually.

**Gap**: Add `tests/cascade_router_integration.rs` (or sibling) that boots
`Dispatcher` with two mock backends, runs cascade until one wins, and asserts
the persisted state at `.roko/learn/cascade-router.json` reflects the winner.

**Fix scope**: 2 days. Reuses `roko-std::mock` dispatcher.

**Priority**: P1. Cross-ref item 86 in `14-observability-gaps.md`.

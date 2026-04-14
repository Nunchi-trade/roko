# Critical Integration Gap: SafetyLayer → CLI Pipeline

> **Layer**: L3 Harness (safety enforcement) → L4 Orchestration (CLI invocation)
>
> **Crate**: `roko-agent` (SafetyLayer, ToolDispatcher) and `roko-cli` (orchestrate.rs)
>
> **Synapse traits**: All safety traits (Gate, Policy) are built, but not yet invoked from every production code path
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md)


> **Implementation**: Specified

---

## Overview

This document describes the **#1 remaining integration gap** in the Roko safety architecture: the SafetyLayer is fully built and wired to the ToolDispatcher, but that pipeline is not yet universal across the CLI/runtime branches that actually run agents. Routed HTTP provider paths now reach the ToolDispatcher for OpenAI-compatible providers, Anthropic API, Gemini compat models, Gemini-native non-grounding tool-capable models, and Perplexity tool-capable chat, but known-protocol subprocess paths and native or specialty endpoints still bypass it.

This is not a design gap — the components exist and are connected. It is a **wiring gap** — some of the connected components are still not called from important execution paths. The pattern is documented in the implementation plan at `tmp/implementation-plans/11-inconsistencies.md` and remains a top-priority item for making the safety architecture universally effective.

---

## The Gap in Detail

### What Is Built

The safety infrastructure is substantial and well-tested:

**SafetyLayer** (`roko-agent/src/safety/mod.rs`, ~285 lines): A composite struct that chains six safety guards:

| Guard | Module | What It Does |
|---|---|---|
| BashPolicy | `safety/bash.rs` | Deny-pattern matching on shell commands (rm -rf, sudo, curl pipe, fork bombs, 8192 char limit) |
| GitPolicy | `safety/git.rs` | Block force push, hard reset, branch deletion on protected branches |
| NetworkPolicy | `safety/network.rs` | Scheme filtering, private network blocking (RFC1918, link-local, loopback), host allow/deny |
| PathPolicy | `safety/path.rs` | Worktree sandbox via canonicalization, escape prevention, optional symlink denial |
| ScrubPolicy | `safety/scrub.rs` | 9 default regex patterns scrubbing secrets (API keys, JWTs, private keys, env assignments) |
| RateLimiter | `safety/rate_limit.rs` | Sliding-window counter keyed by (role, tool), default 60 calls/60s |

Each guard has comprehensive tests (the safety module has 50+ tests across the six sub-modules).

**ToolDispatcher** (`roko-agent/src/dispatcher/mod.rs`, ~1070 lines): A dispatch pipeline that integrates the SafetyLayer:

```rust
pub struct ToolDispatcher {
    registry: ToolRegistry,
    resolver: Box<dyn HandlerResolver>,
    max_result_bytes: usize,
    safety: Option<SafetyLayer>,
    // ...
}

impl ToolDispatcher {
    /// Wire the SafetyLayer into the dispatch pipeline.
    pub fn with_safety(mut self, layer: SafetyLayer) -> Self {
        self.safety = Some(layer);
        self
    }
}
```

The `dispatch()` method runs a 7-stage pipeline:

```
1. validate       → is this a valid tool call?
2. tool_filter    → is this tool allowed for this role?
3. permission     → does the agent have ToolPermission?
4. safety pre-exec → SafetyLayer.check_pre_execution()
5. handler        → execute the actual tool
6. truncate       → enforce max_result_bytes
7. safety scrub   → SafetyLayer.scrub_output()
```

Each stage emits an audit Engram via `emit_audit()`.

### What Is Not Wired

The primary routed branch in **orchestrate.rs** (`roko-cli/src/orchestrate.rs`) now goes through provider resolution and can reach `ToolLoop` + `ToolDispatcher` for supported HTTP backends. The remaining non-wired branches are the intentional known-protocol subprocess paths and backend-specific special cases that still dispatch agents like this:

```rust
// orchestrate.rs — agent creation (simplified)
let mut agent = ExecAgent::new(
    &self.config.agent.command,
    self.config.agent.args.clone(),
);
// ... set up prompt, system message, etc.
let result = agent.run(&prompt).await?;
```

`ExecAgent` spawns a subprocess (Claude CLI or another protocol command) and captures its output. It does **not** go through the ToolDispatcher. In those remaining subprocess branches this means:

- **No BashPolicy** — the agent's bash commands are not checked against deny patterns
- **No GitPolicy** — force pushes and hard resets are not blocked
- **No NetworkPolicy** — private network access is not filtered
- **No PathPolicy** — worktree escape is not prevented by Roko (though the underlying CLI may have its own sandboxing)
- **No ScrubPolicy** — secrets in agent output are not scrubbed
- **No RateLimiter** — there are no per-role, per-tool rate limits
- **No audit emissions** — tool calls are not logged to the audit chain

The SafetyLayer and ToolDispatcher are no longer completely unreachable from the CLI runtime, but they are still not constructed or called in every execution branch.

### Why This Happened

This is a documented pattern in the Roko codebase: "built but never connected" (see CLAUDE.md rule #2: "WIRE, don't build"). The safety infrastructure was developed in the `roko-agent` crate, while the orchestration loop was developed in `roko-cli`. The two were never connected because:

1. `ExecAgent` wraps a subprocess (Claude CLI). Tool dispatch happens **inside** the subprocess, not in Roko's process. The ToolDispatcher is designed for in-process tool dispatch.
2. The `ClaudeCliAgent` variant (which uses the Claude API directly) does have a system prompt but no ToolDispatcher integration.
3. There was no clear integration point: the ToolDispatcher expects to receive tool call requests from an LLM response parser, but orchestrate.rs delegates that parsing to the subprocess.

### The Architecture Mismatch

```
CURRENT FLOW (no safety):
  orchestrate.rs → ExecAgent::run() → subprocess (Claude CLI)
                                        ↓
                                   Claude CLI handles its own
                                   tool dispatch internally
                                        ↓
                                   Raw output returned to
                                   orchestrate.rs

INTENDED FLOW (with safety):
  orchestrate.rs → ToolDispatcher::dispatch() → SafetyLayer → tool handler
                   ↑                                              ↓
                   └─── audit Engrams emitted at each stage ──────┘
```

The mismatch: orchestrate.rs delegates to a subprocess that does its own tool dispatch, while the SafetyLayer is designed for in-process tool dispatch. Bridging this gap requires either:

**Option A: Hook into the subprocess.** Pass safety configuration to Claude CLI via `--settings` or `--allowed-tools`, relying on Claude CLI's own safety mechanisms. This is partial — it delegates enforcement to the subprocess and loses Roko's audit chain.

**Option B: Intercept subprocess I/O.** Parse Claude CLI's tool call output, run it through the ToolDispatcher, and feed the result back. This is complex and fragile.

**Option C: Switch to in-process dispatch.** Replace ExecAgent (subprocess) with direct API calls (ClaudeCliAgent) where tool dispatch happens in-process through the ToolDispatcher. This is the architecturally correct solution but requires significant refactoring.

**Option D: Pre/post hooks.** Run SafetyLayer checks on the agent's prompt (pre-execution) and output (post-execution) without intercepting individual tool calls. This provides partial coverage: ScrubPolicy on output, PathPolicy on file paths mentioned in plans, but no per-tool-call enforcement.

---

## Impact Assessment

### What Is At Risk Without the Safety Pipeline

| Risk | Severity | Mitigation Without Roko Safety |
|---|---|---|
| Agent runs `rm -rf /` or destructive bash | Critical | Relies on Claude CLI's own safety settings |
| Agent force-pushes to protected branch | High | Relies on Git server-side protections |
| Agent accesses private networks | Medium | Relies on OS/network-level firewalls |
| Agent escapes worktree sandbox | High | Relies on Claude CLI's `--worktree` flag |
| Agent leaks API keys in output | High | No mitigation unless Claude CLI scrubs |
| Agent exceeds rate limits | Medium | No mitigation — cost runaway possible |
| Tool calls not audited | High | No forensic replay capability |

### What Works Without the ToolDispatcher

Not everything is broken. Several safety mechanisms operate independently:

- **Gate pipeline**: CompileGate, TestGate, ClippyGate run after agent execution and catch broken code
- **Conductor circuit breaker**: Monitors health metrics and can abort sessions
- **ProcessSupervisor**: Manages agent subprocess lifecycle (timeouts, kill signals)
- **Adaptive risk thresholds**: Gate thresholds adjust based on historical pass rates
- **Episode logging**: Agent turns and gate results are recorded (at the orchestrate.rs level)

These provide partial safety but miss the per-tool-call enforcement that the ToolDispatcher provides.

---

## Resolution Path

### Phase 1: Pre/Post Safety Hooks (Tier 1)

Wire SafetyLayer into orchestrate.rs as pre/post hooks around agent execution:

```rust
// Before agent execution
let safety = SafetyLayer::from_config(&config.safety);
safety.check_pre_execution(&prompt)?;

// After agent execution
let scrubbed_output = safety.scrub_output(&raw_output);
```

This provides:
- ScrubPolicy on agent output (secret scrubbing)
- PathPolicy validation on file paths in the prompt
- RateLimiter on overall agent invocations (not per-tool-call)

### Phase 2: Claude CLI Settings Passthrough (Tier 1)

Pass Roko's safety configuration to Claude CLI via settings:

```rust
let mut agent = ExecAgent::new(&config.agent.command, args);
agent.with_settings(safety_to_claude_settings(&config.safety));
// This generates --settings or --allowed-tools flags
```

This delegates per-tool-call enforcement to Claude CLI but ensures Roko's safety intent is communicated.

### Phase 3: In-Process Tool Dispatch (Tier 2)

For the ClaudeCliAgent path (direct API calls), wire the ToolDispatcher into the agent's tool loop:

```rust
let dispatcher = ToolDispatcher::new(registry, resolver)
    .with_safety(safety_layer);

// In the agent's tool loop:
loop {
    let response = llm.complete(&messages).await?;
    if let Some(tool_call) = response.tool_use {
        let result = dispatcher.dispatch(&tool_call).await?;
        // result includes audit emissions, scrubbing, rate limiting
    }
}
```

This is the architecturally correct solution. It requires moving from subprocess-based agent execution to in-process execution, which is tracked as a separate implementation priority.

---

## Current Implementation Plan Reference

The gap is documented in:
- `tmp/implementation-plans/11-inconsistencies.md` § "CRITICAL: Still-present gaps in nunchi codebase" items 1-5
- `tmp/implementation-plans/03-safety-hooks.md` Phase B (wire safety guards)
- CLAUDE.md "What to work on" section (now completed for several items, but this gap persists)

The implementation plan recommends:
1. Update orchestrate.rs to create ToolDispatcher with SafetyLayer (Phase B)
2. Replace ExecAgent with ClaudeCliAgent for the primary execution path (deferred)
3. Wire SystemPromptBuilder templates into agent prompts (completed)
4. Add new checklist items for the ToolDispatcher gap

---

## Self-Check

This document exists because the writing rules require that the #1 integration gap be flagged prominently. The gap is:

- **Real**: orchestrate.rs does not construct or call ToolDispatcher
- **Verified**: confirmed by reading the active codebase (`crates/roko-cli/src/orchestrate.rs`, `crates/roko-agent/src/dispatcher/mod.rs`, `crates/roko-agent/src/safety/mod.rs`)
- **Documented**: tracked in implementation plans and CLAUDE.md
- **Impactful**: all six safety guards are inactive in the production code path
- **Solvable**: the components exist and are tested — this is a wiring task, not a design task

Until this gap is closed, the safety architecture described in this topic (00-defense-in-depth through 15-forensic-ai) represents the **target state**, not the **current state**. The current state has Gates (post-execution verification) and ProcessSupervisor (lifecycle management) active. The per-tool-call safety pipeline (pre-execution checks, real-time scrubbing, rate limiting, audit emissions) is built but dormant.

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md) — The overall safety architecture that this gap undermines
- [04-permits-allowlists.md](04-permits-allowlists.md) — ToolPermission system that is not enforced without ToolDispatcher
- [05-loop-detection.md](05-loop-detection.md) — RateLimiter that is not active without ToolDispatcher
- [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md) — Engram Syscalls require universal enforcement via ToolDispatcher

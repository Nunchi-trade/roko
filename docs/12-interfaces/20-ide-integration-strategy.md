# IDE integration strategy

> Architecture decision document for exposing Roko's plan execution, gate pipeline, learning loops, and dashboard inside code editors.

> **Status**: Proposed
> **Decision**: ACP-first, MCP as universal adapter, VS Code extension when warranted. Never fork.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [02-agents/02-provider-adapters.md](../02-agents/02-provider-adapters.md) (ACP adapter, protocol families), [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) (existing HTTP surface)
**Key sources**: `tmp/run-anywhere/ide.md` (research), ACP specification, VS Code AI extensibility docs

---

## Context

Roko's differentiator is not the IDE shell. It is plan execution, the gate pipeline, learning loops, and self-improvement. Other agents offer chat-in-editor. Roko offers a full cognitive loop with verification, routing, and feedback -- capabilities that no other agent surfaces in an IDE.

The question is how to expose those capabilities without building an editor.

This document evaluates five approaches, recommends a phased strategy, and specifies the protocol extensions Roko needs.

---

## The five approaches

| # | Approach | Effort | Time to MVP | Maintenance | IDE support |
|---|----------|--------|-------------|-------------|-------------|
| 1 | MCP server | ~1 week | Days | Minimal | VS Code Copilot, Cursor, Continue, any MCP client |
| 2 | ACP agent (`roko acp`) | ~2-3 weeks | Weeks | Low | Zed, JetBrains (all IDEs), Neovim, Emacs, future VS Code |
| 3 | VS Code extension (chat participant) | ~1-2 months | Month | Medium | VS Code only |
| 4 | VS Code extension (full) | ~3-6 months | Months | Medium-High | VS Code only |
| 5 | VS Code fork | ~6-12+ months | Half year+ | Extreme | Fork only |

### Evaluation matrix

| Criterion | MCP server | ACP agent | VS Code chat | VS Code full | VS Code fork |
|-----------|-----------|-----------|-------------|-------------|-------------|
| **Capability ceiling** | Tool-level only (stateless calls) | Full agent lifecycle (sessions, streaming, permissions) | Deep VS Code integration (chat panel, diff view) | Full editor control (sidebar, decorations, CodeLens) | Unlimited (modify editor internals) |
| **User experience** | Tools appear in existing chat | Dedicated agent panel in supported IDEs | `@roko` in VS Code chat with slash commands | Custom dashboard, plan tree view, inline gate results | Custom everything |
| **Multi-IDE support** | Any MCP client | Zed, JetBrains, Neovim, Emacs, Cursor | VS Code only | VS Code only | Fork only |
| **Implementation cost** | ~500 LOC Rust | ~1-2K LOC Rust | ~5-10K LOC TypeScript + Rust ACP | ~10-20K LOC TypeScript + Rust ACP | 100K+ LOC, ongoing rebase |
| **Maintenance burden** | Near zero (MCP is stable) | Low (ACP is stable, agent is standalone) | Medium (VS Code API changes quarterly) | Medium-High (more API surface, more breakage) | Extreme (monthly rebase, security patches) |
| **Time to first user** | Days | 2-3 weeks | 1-2 months | 3-6 months | 6-12 months |

---

## Decision: ACP-first with MCP as universal adapter

The recommended path is:

1. **MCP server now** -- Expose Roko's commands as MCP tools. Works in every MCP-capable editor today. Zero extension code.
2. **ACP agent next** -- Build `roko acp` for full agent lifecycle. One implementation, five+ editor families.
3. **VS Code extension later** -- Build only when ACP proves insufficient for a specific VS Code capability (diff preview, WebView dashboard, inline decorations).
4. **Never fork VS Code** -- The maintenance cost is unsustainable at any scale below Cursor's.

### Why ACP first

**Multi-IDE with one codebase.** ACP is a JSON-RPC 2.0 protocol over stdio. One Rust implementation works in Zed, JetBrains (IntelliJ, PyCharm, GoLand, WebStorm, RustRover -- all of them), Neovim, and Emacs. This is the same "write once, run everywhere" advantage that LSP brought to language servers.

**Natural mapping to Roko's architecture.** ACP sessions map to plan runs. ACP tool calls map to file edits. ACP permission requests map to gate approvals. ACP session/load maps to `--resume` from executor snapshots. The protocol was designed for exactly this type of agent.

**Growing adoption.** Zed created ACP in October 2025. JetBrains co-authored it. Cursor joined the ACP registry in March 2026. Goose, Kimi Code, and Kiro all ship `acp` subcommands. The protocol is reaching LSP-level ubiquity for agents.

**Rust SDK exists.** The `agent-client-protocol` crate on crates.io provides the schema types and serialization. Roko already has a `CursorAcpAdapter` in `crates/roko-agent/src/provider/cursor_acp.rs` -- the protocol is not foreign to this codebase.

**Low maintenance.** The ACP spec is versioned and stable. The agent is an independent process. IDE updates do not break the agent.

### Why MCP as the universal adapter

MCP and ACP serve different roles:

- **MCP**: Agent discovers and uses tools. Stateless. Request-response.
- **ACP**: IDE communicates with agent. Stateful sessions. Bidirectional streaming.

They compose. When an IDE opens an ACP session, it passes MCP server configs to the agent. The agent uses MCP for tool access and ACP for IDE interaction:

```
IDE
 |
 +-- ACP --> roko (agent)
 |             |
 |             +-- MCP --> code search server
 |             +-- MCP --> documentation server
 |             +-- MCP --> database server
 |
 +-- MCP --> roko (as tool provider to Copilot)
```

Roko appears twice in this diagram. As an ACP agent, it drives its own cognitive loop and streams results back to the IDE. As an MCP server, it exposes its commands as tools that Copilot or any other MCP client can invoke. Both roles coexist. Both are useful.

---

## ACP protocol lifecycle

### Transport

JSON-RPC 2.0 over stdio. The IDE spawns `roko acp` as a subprocess and communicates via newline-delimited JSON on stdin/stdout. No HTTP server, no port allocation, no firewall issues. The buffer limit is 50MB by default.

Future transports (HTTP, WebSocket) are specified in the ACP spec for remote deployments but are not needed for the initial implementation.

### Handshake

```
IDE (Client)                          roko (Agent)
  |--- initialize ------------------>|
  |    { protocolVersion: "2025-1",  |
  |      clientInfo: { name: "zed" },|
  |      capabilities: { ... } }     |
  |                                   |
  |<-- initialize response ----------|
  |    { agentInfo: { name: "roko" },|
  |      capabilities: { ... } }     |
```

Roko advertises its capabilities:

```json
{
  "agentInfo": {
    "name": "roko",
    "version": "0.1.0",
    "description": "Self-developing agent with plan execution, gates, and learning"
  },
  "capabilities": {
    "sessions": {
      "new": true,
      "load": true,
      "list": true,
      "modes": ["run", "plan", "research"],
      "configOptions": ["model", "effort", "budget"]
    },
    "mcpCapabilities": { "http": false, "sse": false },
    "promptCapabilities": {
      "audio": false,
      "image": false,
      "embeddedContext": true
    }
  }
}
```

The IDE advertises what it can provide:

```json
{
  "capabilities": {
    "fileSystem": { "readTextFile": true, "writeTextFile": true },
    "terminal": { "create": true, "output": true, "waitForExit": true, "kill": true },
    "prompts": { "audio": false, "image": true, "embeddedContext": true }
  }
}
```

Capability negotiation lets Roko adapt its behavior. If the IDE supports `terminal/create`, Roko runs gate checks (compile, test, clippy) in the IDE's terminal panel where the user can see output. If the IDE does not support terminals, Roko falls back to spawning shell commands directly.

### Session lifecycle

```
IDE                                   roko
  |--- session/new ----------------->|  cwd, MCP server configs
  |<-- session ID -------------------|
  |                                   |
  |--- session/prompt -------------->|  user text + context
  |<-- session/update (notification) |  AgentThoughtChunk (reasoning)
  |<-- session/update (notification) |  AgentMessageChunk (response)
  |<-- session/update (notification) |  ToolCallStart (editing file)
  |<-- session/update (notification) |  ToolCallResult (edit applied)
  |<-- session/update (notification) |  _roko.dev/gate/result
  |<-- session/update (notification) |  _roko.dev/plan/status
  |<-- PromptResponse ---------------|  stopReason: done
  |                                   |
  |--- session/prompt -------------->|  follow-up
  |<-- ... continued streaming       |
  |                                   |
  |--- session/cancel -------------->|  user cancels
  |    (roko drains, checkpoints)    |
```

### Method mapping

**IDE to Roko (agent methods):**

| ACP method | Roko behavior |
|---|---|
| `initialize` | Return version, capabilities, feature flags |
| `session/new` | Set working directory, load `roko.toml`, initialize `LearningRuntime` |
| `session/load` | Resume from executor snapshot (`--resume`) |
| `session/prompt` | Dispatch to universal loop (`run.rs`) or plan executor (`orchestrate.rs`) |
| `session/cancel` | Set shutdown flag, drain active agents, checkpoint state |
| `session/setMode` | Switch between `run`, `plan`, and `research` modes |
| `session/setModel` | Override default model in config |

**Roko to IDE (client methods):**

| ACP method | Roko usage |
|---|---|
| `fs/readTextFile` | Read unsaved editor buffers (supplements direct filesystem reads) |
| `fs/writeTextFile` | Write through the IDE for diff preview (supplements direct writes) |
| `session/requestPermission` | Gate approval prompts, destructive edit confirmation |
| `session/update` | Stream plan progress, gate results, agent output, learning metrics |
| `terminal/create` | Run compile/test/clippy gates in the IDE's terminal panel |
| `terminal/waitForExit` | Block until gate command completes |
| `terminal/kill` | Abort a hung gate or agent process |

### Streaming via session/update

ACP defines several update kinds for streaming agent output:

| Update kind | Content |
|---|---|
| `AgentMessageChunk` | Streamed response text |
| `AgentThoughtChunk` | Reasoning trace (extended thinking) |
| `ToolCallStart` | Tool invocation starting (name, arguments) |
| `ToolCallProgress` | Intermediate tool output |
| `ToolCallResult` | Final tool result or error |
| `Plan` | Multi-step plan with step descriptions |

Roko uses all of these plus custom extensions prefixed with `_roko.dev/` (see next section).

---

## Roko-specific ACP extensions

Standard ACP covers basic agent operations. Roko's unique capabilities -- plan DAG execution, gate pipelines, learning feedback, routing decisions -- need custom extensions. ACP supports this through the `_` prefix convention for vendor-specific methods.

### Plan progress (`_roko.dev/plan/status`)

Sent as `session/update` notifications whenever task status changes:

```json
{
  "method": "session/update",
  "params": {
    "kind": "_roko.dev/plan/status",
    "data": {
      "plan_id": "05-glm-integration",
      "total_tasks": 14,
      "completed": 7,
      "in_progress": 2,
      "failed": 0,
      "pending": 5,
      "tasks": [
        { "id": "2D.01", "status": "completed", "model": "glm-5.1" },
        { "id": "2D.02", "status": "in_progress", "model": "glm-5.1", "iteration": 2 }
      ]
    }
  }
}
```

IDEs render this as a task checklist, progress bar, or tree view depending on their UI capabilities. Zed might show it in the agent panel. JetBrains might render it as a tool window. The data is the same; the presentation varies.

### Gate results (`_roko.dev/gate/result`)

Sent after each gate pipeline run:

```json
{
  "method": "session/update",
  "params": {
    "kind": "_roko.dev/gate/result",
    "data": {
      "task_id": "2D.02",
      "pipeline": [
        { "gate": "compile", "passed": true, "duration_ms": 3200 },
        { "gate": "test", "passed": false, "duration_ms": 15400,
          "details": "3 tests failed: test_glm_thinking, test_glm_cache, test_glm_error" },
        { "gate": "clippy", "passed": true, "duration_ms": 8100 }
      ],
      "overall_passed": false,
      "iteration": 2,
      "conductor_action": "retry_with_error_digest"
    }
  }
}
```

The `conductor_action` field tells the IDE what Roko decided to do about the failure. The IDE can display pass/fail badges, show failed test output, or highlight the specific files that broke the build.

### Learning feedback (`_roko.dev/learn/routing`)

Explains the model routing decision for the current task:

```json
{
  "method": "session/update",
  "params": {
    "kind": "_roko.dev/learn/routing",
    "data": {
      "selected_model": "glm-5.1",
      "provider": "zai",
      "stage": "ucb",
      "score": 0.847,
      "candidates": [
        { "model": "glm-5.1", "score": 0.847, "pass_rate": 0.82, "cost_norm": 0.04 },
        { "model": "kimi-k2.5", "score": 0.791, "pass_rate": 0.78, "cost_norm": 0.02 },
        { "model": "claude-sonnet", "score": 0.723, "pass_rate": 0.88, "cost_norm": 0.42 }
      ],
      "total_observations": 423
    }
  }
}
```

This data is optional for rendering but valuable for transparency. A VS Code extension could show routing decisions in a sidebar. A terminal-based editor might ignore it. The extension mechanism ensures backward compatibility -- IDEs that do not recognize `_roko.dev/` extensions discard them silently.

### Episode events (`_roko.dev/episode/event`)

Per-turn cost and performance tracking:

```json
{
  "method": "session/update",
  "params": {
    "kind": "_roko.dev/episode/event",
    "data": {
      "episode_id": "ep_abc123",
      "task_id": "2D.02",
      "model": "glm-5.1",
      "provider": "zai",
      "input_tokens": 12500,
      "output_tokens": 3400,
      "cost_usd": 0.032,
      "wall_ms": 8400,
      "gate_passed": false,
      "iteration": 2,
      "tool_calls": ["Read", "Edit", "Bash"]
    }
  }
}
```

### Dashboard snapshot (`_roko.dev/dashboard/snapshot`)

Periodic snapshot of the full dashboard state for WebView rendering:

```json
{
  "method": "session/update",
  "params": {
    "kind": "_roko.dev/dashboard/snapshot",
    "data": {
      "provider_health": [
        { "id": "zai", "state": "healthy", "p50_ms": 1200, "error_rate": 0.01 },
        { "id": "moonshot", "state": "probing", "p50_ms": null, "error_rate": 0.15 }
      ],
      "model_comparison": [
        { "model": "glm-5.1", "pass_rate": 0.82, "avg_cost": 0.19, "observations": 203 }
      ],
      "experiments": [
        { "id": "glm-vs-kimi-impl", "status": "running", "variants": 2, "trials": 27 }
      ],
      "budget": { "used": 4.20, "limit": 10.00 },
      "router_stage": "ucb",
      "total_observations": 423
    }
  }
}
```

This is the same data that powers `roko dashboard` text mode and the `GET /api/learn/*` HTTP endpoints. The VS Code extension (when built) renders it in a WebView panel. Other IDEs consume what they can and ignore the rest.

---

## MCP server specification

The MCP server is the fastest path to IDE integration. It requires no extension code and works in any MCP-capable editor today.

### Implementation

New file: `crates/roko-serve/src/mcp_server.rs` (~500 LOC).
New subcommand option: `roko serve --mcp` (stdio mode) or `roko serve --mcp --port 9090` (HTTP mode).

The MCP server implements two methods:

- `tools/list` -- Returns tool definitions for all Roko CLI commands.
- `tools/call` -- Executes a Roko command and returns the result.

### Tool definitions

| MCP tool | What it does | Maps to |
|---|---|---|
| `roko_run` | Execute a prompt through the cognitive loop | `roko run` |
| `roko_plan_run` | Execute a plan with gate verification | `roko plan run` |
| `roko_plan_list` | List available plans | `roko plan list` |
| `roko_plan_show` | Show plan details | `roko plan show` |
| `roko_status` | Get signal/episode counts | `roko status` |
| `roko_research` | Deep research with citations | `roko research topic` |
| `roko_prd_idea` | Capture a work item | `roko prd idea` |
| `roko_prd_list` | List PRDs | `roko prd list` |
| `roko_dashboard` | Get dashboard data as JSON | `roko dashboard --json` |
| `roko_provider_health` | Get provider health status | New endpoint |

### Configuration

VS Code (`.vscode/mcp.json`):

```json
{
  "servers": {
    "roko-tools": {
      "command": "roko",
      "args": ["serve", "--mcp"]
    }
  }
}
```

The tools appear in Copilot's agent mode. Users select them via "Configure Tools" in the chat panel. No extension install needed.

### Limitations

MCP tools are stateless request-response calls. They cannot stream intermediate results, maintain session state, or request user permissions. For plan execution that takes minutes and generates hundreds of intermediate events, MCP is insufficient. That is why ACP is the primary protocol and MCP is the adapter layer.

---

## Why not fork VS Code

Forking VS Code is a trap for any team that is not Cursor.

### The maintenance burden

VS Code releases monthly. Each release touches hundreds of files across the editor core. A fork must rebase on every release to pick up bug fixes and security patches. The rebase conflicts with custom modifications. The conflicts compound.

Cursor employs ~300 engineers and is still 4 minor versions behind current VS Code. Their Chromium ships with 80-94 known CVEs. OX Security reported in November 2025 that both Cursor and Windsurf had weaponizable Chromium vulnerabilities affecting 1.8 million developers. Cursor patched within a day. Windsurf did not respond.

### The marketplace problem

VS Code forks cannot access the VS Code Marketplace. They use OpenVSX, which has fewer extensions and documented security issues -- a vulnerability allowed malicious actors to squat on extension namespaces and serve compromised packages that appeared as "recommended."

### The graveyard

Void Editor, a YC-backed open-source VS Code fork, abandoned active development. The maintenance burden was unsustainable for a small team. Windsurf (Codeium's fork) was acquired piecemeal -- CEO to Google, IP to Cognition -- after the fork strategy proved unviable.

### The only viable case

Cursor's fork works because Cursor has $2B ARR, 300 engineers, and venture funding at a $29.3B valuation. The fork is the product. For Roko, the fork would be a distraction. The differentiating capabilities (plan execution, gates, learning, self-improvement) live in the Rust backend. The IDE is a display layer. Building that display layer through ACP and a thin extension costs 1-2 orders of magnitude less effort than maintaining a fork.

---

## ACP agent implementation

### New file and subcommand

```
crates/roko-cli/src/acp.rs  (~1-2K LOC)
```

New subcommand: `roko acp`

The ACP server listens on stdin, writes to stdout. It handles the ACP lifecycle methods and delegates to existing Roko runtime functions.

### Mapping ACP to existing code

| ACP operation | Roko code path |
|---|---|
| `session/new` | Load `roko.toml`, init `LearningRuntime`, create `FileSubstrate` |
| `session/prompt` (run mode) | `run.rs::run_once()` |
| `session/prompt` (plan mode) | `orchestrate.rs::run_plan()` |
| `session/prompt` (research mode) | `research.rs::research_topic()` |
| `session/load` | Load from `.roko/state/executor.json` |
| `session/cancel` | `PlanRunner::shutdown()` |
| `session/update` streaming | `EpisodeLogger` events + gate results from `orchestrate.rs` |

The ACP server reuses the same runtime as the CLI. It does not duplicate logic. It wraps existing functions with ACP message serialization.

### IDE configuration

**Zed** (`~/.config/zed/settings.json`):

```json
{
  "agent_servers": {
    "Roko": {
      "type": "custom",
      "command": "roko",
      "args": ["acp", "--workdir", "."]
    }
  }
}
```

**JetBrains** (`~/.jetbrains/acp.json`):

```json
{
  "agent_servers": {
    "Roko": {
      "command": "/path/to/roko",
      "args": ["acp"]
    }
  }
}
```

### Permission flow

When Roko needs to perform a destructive operation (overwrite a file, run a shell command, apply a plan), it sends a `session/requestPermission` request to the IDE. The IDE shows a confirmation dialog. The user approves or rejects. This maps directly to Roko's existing gate approval model -- the IDE becomes the approval interface that the TUI currently provides.

---

## VS Code extension architecture (phase 3)

Build this only after ACP proves insufficient for a specific VS Code capability. The extension communicates with Roko through ACP -- it is a thin UI layer, not a second agent implementation.

### Structure

```
extensions/vscode-roko/  (~5-10K TypeScript)
  src/
    extension.ts          -- activate/deactivate, process lifecycle
    chatParticipant.ts    -- @roko chat handler with /plan, /run, /status
    acpClient.ts          -- JSON-RPC client for roko subprocess
    processManager.ts     -- spawn, kill, restart roko acp
    diffProvider.ts       -- TextDocumentContentProvider for edit previews
    dashboardPanel.ts     -- WebviewPanel rendering dashboard data
    planTreeView.ts       -- TreeDataProvider for plan DAG in sidebar
    statusBar.ts          -- StatusBarItem showing progress + model
  webview/
    dashboard/            -- React or Svelte app for dashboard WebView
```

### Communication

```
User types @roko in chat
  -> Chat participant handler receives request
  -> Forwards to ACP client
  -> ACP client sends session/prompt to roko subprocess
  -> roko streams session/update notifications:
       AgentThoughtChunk    -> rendered in chat as thinking
       ToolCallStart        -> rendered as "Editing file.rs..."
       ToolCallResult       -> rendered as code block
       _roko.dev/gate/result -> rendered as pass/fail badge
       _roko.dev/plan/status -> rendered as task checklist
  -> Final PromptResponse
  -> Chat participant renders final output
```

### File edit flow

1. Roko generates a file edit through its tool loop.
2. ACP notification: `ToolCallResult` with file path and content.
3. Extension creates a `TextDocumentContentProvider` with the proposed content.
4. Opens VS Code's diff editor: `vscode.commands.executeCommand('vscode.diff', originalUri, proposedUri)`.
5. User reviews the inline diff.
6. Accept: `workspace.applyEdit()` writes to disk.
7. Reject: discard proposed content, notify Roko.

This is the same pattern Claude Code, Cline, and Continue use. Users already expect it.

### Dashboard WebView

The extension creates a WebView panel that renders the same data Roko exposes through `_roko.dev/dashboard/snapshot`:

```typescript
const panel = vscode.window.createWebviewPanel(
  'rokoDashboard', 'Roko Dashboard', vscode.ViewColumn.Two,
  { enableScripts: true, retainContextWhenHidden: true }
);

acpClient.on('_roko.dev/dashboard/snapshot', (data) => {
  panel.webview.postMessage({ type: 'dashboard-update', data });
});
```

The WebView inherits VS Code's theme via CSS variables (`var(--vscode-editor-foreground)`). If ROSEDUST theming is desired, it overrides these variables to match the void-black palette defined in `07-rosedust-design-language.md`.

---

## Migration path

### Phase 1: MCP server (week 1)

Expose Roko's CLI commands as MCP tools. ~500 LOC in `roko-serve`. Works in VS Code Copilot, Cursor, and Continue immediately. No extension code.

**Milestone**: A user types `@workspace /roko_status` in Copilot chat and sees Roko's signal and episode counts.

### Phase 2: ACP agent (weeks 2-4)

Build `roko acp` subcommand. ~1-2K LOC in `roko-cli`. Handle session lifecycle, prompt dispatch, and streaming. Define `_roko.dev/` extensions.

**Milestone**: A user opens Zed, types a prompt in the agent panel, and Roko executes a plan with streamed gate results.

### Phase 3: VS Code extension (months 2-3)

Build the VS Code extension as a thin ACP client with chat participant, diff provider, and dashboard WebView. ~5-10K LOC TypeScript.

**Milestone**: A user types `@roko /plan run plans/` in VS Code chat and sees a live plan progress view with pass/fail gate badges.

### Phase 4: Enhanced integration (ongoing)

- Register Roko as a Language Model Chat Provider (expose CascadeRouter to VS Code's model picker).
- Register Roko's tools with the Language Model Tools API (Copilot agent mode can invoke `roko_plan_run`).
- Inline completions from the skill library.
- Problems panel integration (gate errors as VS Code diagnostics).
- CodeLens for plan tasks (show task status inline in code files).

Each phase 4 item is independent and built only when user demand justifies it.

---

## Protocol comparison

| Aspect | ACP | LSP | MCP | DAP |
|---|---|---|---|---|
| Purpose | Agent <-> IDE | Language server <-> IDE | Agent <-> tools/data | Debugger <-> IDE |
| Direction | Bidirectional | Mostly client to server | Client to server | Bidirectional |
| Sessions | Multi-turn conversations | Document state | Stateless tool calls | Debug sessions |
| Streaming | Yes (notifications) | Limited | No | Events |
| Permissions | Native (`requestPermission`) | No | No | No |
| Tool calls | Native (plan/tool lifecycle) | No | Native (`tools/call`) | No |
| Transport | stdio, HTTP (future) | stdio, TCP | stdio, HTTP, SSE | stdio, TCP |

ACP is to agents what LSP is to language servers: a protocol that makes one implementation work in every editor. The parallel is exact. Before LSP, every editor needed a custom language plugin. Before ACP, every editor needs a custom agent extension. ACP eliminates that duplication.

---

## Competitive position

| Feature | Claude Code | Cline | Continue | Cursor | Roko (proposed) |
|---|---|---|---|---|---|
| Protocol | MCP (custom) | gRPC-over-postMessage | Custom messenger | Proprietary | ACP (open standard) |
| IDE support | VS Code | VS Code | VS Code, JetBrains | Cursor only | Zed, JetBrains, VS Code, Neovim, Emacs |
| Multi-plan execution | No | No | No | No | Yes (DAG executor) |
| Gate pipeline | No | No | No | No | Yes (compile/test/clippy/diff) |
| Learning/routing | No | No | No | Yes (model router) | Yes (LinUCB bandit + Thompson) |
| Self-improvement | No | No | No | No | Yes (knowledge distillation) |
| Dashboard | No | No | No | No | Yes (13 pages) |
| Cost tracking | Basic | Basic | No | Credit-based | Full CostTable + budget guardrails |
| Provider health | No | No | No | No | Circuit breaker + latency tracking |
| A/B experiments | No | No | No | No | Prompt + model experiments |

Roko's IDE story is not about competing on editor features. It is about exposing capabilities that no other agent has -- plan execution, verification gates, learning loops, self-improvement -- through a standard protocol that works in every editor.

---

## Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| ACP adoption stalls | Low | Medium | MCP fallback covers the same IDEs with less capability |
| ACP spec changes breaking | Low | Low | Version negotiation in `initialize`, Roko supports multiple versions |
| VS Code drops ACP support | Medium | Low | VS Code extension communicates over ACP anyway (extension spawns `roko acp`) |
| Custom extensions ignored by IDEs | Expected | Low | Extensions are optional. Core functionality uses standard ACP methods. |
| User expects Cursor-level UX | Medium | Medium | Set expectations: Roko's value is the backend, not the editor chrome |

---

## Implementation tasks

Ordered by priority. Each task is independent unless noted.

**MCP server (phase 1):**
- [ ] Add `--mcp` flag to `roko serve` subcommand
- [ ] Implement `tools/list` returning tool definitions for 10 CLI commands
- [ ] Implement `tools/call` dispatching to CLI command handlers
- [ ] Test with VS Code Copilot agent mode
- [ ] Test with Cursor

**ACP agent (phase 2):**
- [ ] Add `acp` subcommand to `roko-cli/src/main.rs`
- [ ] Implement ACP server initialization (capabilities, version negotiation)
- [ ] Implement `session/new` (config loading, `LearningRuntime` init)
- [ ] Implement `session/prompt` dispatch to `run.rs` and `orchestrate.rs`
- [ ] Implement `session/update` streaming for agent output
- [ ] Implement `session/update` streaming for gate results (`_roko.dev/gate/result`)
- [ ] Implement `session/update` streaming for plan progress (`_roko.dev/plan/status`)
- [ ] Implement `session/load` (resume from executor snapshot)
- [ ] Implement `session/cancel` (graceful shutdown with checkpoint)
- [ ] Implement `session/requestPermission` for edit approval
- [ ] Define and document all `_roko.dev/` extension messages
- [ ] Write integration test with mock IDE client
- [ ] Test in Zed
- [ ] Test in JetBrains

**VS Code extension (phase 3):**
- [ ] Scaffold extension with `yo code`
- [ ] Implement `ProcessManager` (spawn/kill/restart `roko acp`)
- [ ] Implement `AcpClient` (JSON-RPC over stdio)
- [ ] Register `@roko` chat participant with `/plan`, `/run`, `/status` commands
- [ ] Implement `DiffProvider` for edit previews
- [ ] Implement dashboard WebView panel
- [ ] Implement plan tree view in sidebar
- [ ] Implement status bar item (plan progress, current model, budget)
- [ ] Publish to VS Code Marketplace

---

## References

1. ACP specification: [agentclientprotocol.com](https://agentclientprotocol.com)
2. ACP GitHub: [github.com/agentclientprotocol/agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol)
3. ACP Rust SDK: [crates.io/crates/agent-client-protocol](https://crates.io/crates/agent-client-protocol)
4. VS Code AI extensibility: [code.visualstudio.com/api/extension-guides/ai/ai-extensibility-overview](https://code.visualstudio.com/api/extension-guides/ai/ai-extensibility-overview)
5. VS Code MCP support: [code.visualstudio.com/docs/copilot/customization/mcp-servers](https://code.visualstudio.com/docs/copilot/customization/mcp-servers)
6. JetBrains ACP: [jetbrains.com/acp](https://www.jetbrains.com/acp/)
7. Zed ACP: [zed.dev/acp](https://zed.dev/acp)
8. OX Security fork vulnerability report (November 2025)
9. Roko provider adapters: `docs/02-agents/02-provider-adapters.md`
10. Roko HTTP API: `docs/12-interfaces/05-http-api-roko-serve.md`

# 01 — Tool Architecture as Cell Protocol

> Every tool is a Cell implementing the Connect protocol. The ToolDef pattern maps to Cell
> metadata. Trust tiers map to capability declarations. The safety hook chain is a Pipeline
> of Verify Cells gating execution.

**Parent spec**: [14-TOOLS.md](../../unified/14-TOOLS.md), [02-CELL.md](../../unified/02-CELL.md)

---

## 1. Core Insight

A tool is the mechanism by which an agent affects the world outside its reasoning loop. In Roko's
unified model, every tool is a **Cell conforming to the Connect protocol**. The Connect protocol
defines lifecycle-managed external I/O: `connect / query / execute / disconnect`. A tool Cell
receives a typed input Signal (the tool parameters), executes against an external system (filesystem,
shell, network, chain), and produces a typed output Signal (the result).

This is not a metaphor. The ToolDef pattern — a compile-time constant declaring name, schema,
capabilities, and cost — is Cell metadata. The three trust tiers (Read, Write, Privileged) are
capability declarations that the Graph evaluator checks before routing execution. The safety hook
chain (seven hooks) is a Pipeline of Verify Cells that must pass before the Connect Cell fires.

The result: tools are not a special subsystem bolted onto the side of the agent architecture. They
are ordinary Cells, composable with every other Cell in the system, subject to the same predict-
publish-correct learning, the same telemetry via Observe, and the same safety boundary via Verify.

---

## 2. ToolDef as Cell Metadata

Every Cell declares typed I/O, capabilities, protocol conformance, cost estimates, and version.
The `ToolDef` struct is the concrete expression of Cell metadata for Connect Cells:

```rust
/// Cell metadata for a Connect Cell (tool).
/// Compile-time constant — one per tool module.
pub struct ToolDef {
    // --- Cell identity ---
    pub name: &'static str,           // Cell ID (kebab-case or snake_case)
    pub description: &'static str,    // Connect protocol: what this Cell connects to

    // --- Protocol conformance ---
    pub category: Category,           // Semantic grouping for Route Cell filtering
    pub capability: CapabilityTier,   // Read | Write | Privileged — capability declaration
    pub risk_tier: RiskTier,          // Layer1 | Layer2 | Layer3 — Verify pre-check depth

    // --- Cost estimation (Cell contract) ---
    pub tick_budget: TickBudget,      // Fast (<1s) | Medium (1-5s) | Slow (5-15s)

    // --- LLM interface (Compose protocol integration) ---
    pub prompt_snippet: &'static str,     // Injected into Compose output
    pub prompt_guidelines: &'static [&'static str],  // Phase-conditional usage hints

    // --- Surface integration (Observe protocol) ---
    pub progress_steps: &'static [&'static str],  // Named steps for Lens rendering
    pub sprite_trigger: SpriteTrigger,            // TUI animation state
}
```

### Mapping to Cell Trait

| ToolDef field | Cell trait method | Purpose |
|---|---|---|
| `name` | `Cell::id()` | Unique identifier within the Graph |
| `description` | `Cell::description()` | Human/LLM-readable purpose statement |
| `capability` | `Cell::capabilities()` | What permissions this Cell requires |
| `risk_tier` | `Cell::protocols()` → Verify depth | How much pre-execution checking |
| `tick_budget` | `Cell::cost_estimate()` | Route Cell uses this for EFE calculation |
| `prompt_snippet` + `prompt_guidelines` | (Compose integration) | Injected when this Cell is available |
| `progress_steps` | (Observe integration) | Lens Cell reads these for rendering |

---

## 3. The 16 Built-in Connect Cells

Roko ships 16 built-in tools as Connect Cells in `crates/roko-std/src/tool/builtin/`. They
decompose into five functional groups:

### File I/O Cells (6)

| Cell | Capability | Connect Target | Input Signal | Output Signal |
|---|---|---|---|---|
| `read_file` | Read | Local filesystem | `{ file_path, offset?, limit? }` | File contents with line numbers |
| `write_file` | Write | Local filesystem | `{ file_path, content }` | Write confirmation |
| `edit_file` | Write | Local filesystem | `{ file_path, old_string, new_string }` | Edit confirmation |
| `multi_edit` | Write | Local filesystem | `{ edits: [{ file_path, old_string, new_string }] }` | Batch edit confirmation |
| `notebook_edit` | Write | Local filesystem (.ipynb) | `{ notebook_path, cell_number, new_source }` | Cell update confirmation |
| `apply_patch` | Write | Local filesystem | `{ patch: unified_diff }` | Patch application result |

### Search Cells (3)

| Cell | Capability | Connect Target | Notes |
|---|---|---|---|
| `glob` | Read | Filesystem index | Pattern matching, returns paths sorted by mtime |
| `grep` | Read | File contents (ripgrep) | Regex search, multiple output modes |
| `ls` | Read | Directory listing | Quick exploration before deeper search |

### Execution Cells (2)

| Cell | Capability | Connect Target | Notes |
|---|---|---|---|
| `bash` | Write | System shell | Persistent working directory, captures stdout/stderr |
| `run_tests` | Write | Test runner | Dispatches to cargo/npm/etc based on project type |

### Web Cells (2)

| Cell | Capability | Connect Target | Notes |
|---|---|---|---|
| `web_fetch` | Read | HTTP endpoint | GET request, returns body |
| `web_search` | Read | Search provider | Returns structured results with citations |

### Orchestration Cells (3)

| Cell | Capability | Connect Target | Notes |
|---|---|---|---|
| `todo_write` | Write | Session task store | Internal task tracking |
| `task` (task_agent) | Write | Agent spawner | Delegates to sub-agent, returns result |
| `exit_plan_mode` | Write | Plan state machine | Transitions agent from planning to approval |

---

## 4. Three Trust Tiers as Capability Declarations

The Cell trait includes a `capabilities()` method returning the set of capabilities the Cell
requires from its execution environment. For tool Cells, this maps to three tiers:

### Tier Structure

```rust
/// Capability declaration for Connect Cells.
/// Determines what authorization is required before execution.
pub enum CapabilityTier {
    /// No authorization needed. Cannot modify external state.
    /// ~60% of tools (all Search, Web, read_file).
    Read,

    /// Requires a Capability<T> token — consumed on use.
    /// Rust ownership enforces single-use at compile time.
    /// ~35% of tools (write_file, edit_file, bash, run_tests, task_agent).
    Write,

    /// Requires Capability<T> token PLUS owner approval signal.
    /// Admin operations, strategy changes. ~5% of tools.
    Privileged,
}
```

### Compile-Time Safety via Rust Ownership

The key innovation is that `Capability<T>` is a **move-only token**:

```rust
/// Unforgeable, single-use, scoped execution permit.
/// - Cannot be created outside the safety system (pub(crate) constructor)
/// - Cannot be reused (no Clone, no Copy — Rust moves it)
/// - Cannot be forged (no Default implementation)
/// - Cannot outlive its tick (expiry checked at execution time)
pub struct Capability<T> {
    pub value_limit: f64,
    pub expires_at: u64,
    pub policy_hash: [u8; 32],
    pub permit_id: String,
    _marker: PhantomData<T>,
}
```

The handler signatures enforce this at the type level:

```rust
// Read Cell — no token needed, can be speculatively executed:
async fn execute_read(&self, params: Value, ctx: &ToolContext) -> Result<ToolResult>;

// Write Cell — token consumed by move, impossible to call without authorization:
async fn execute_write(&self, params: Value, ctx: &ToolContext,
                       capability: Capability<Self>) -> Result<ToolResult>;

// Privileged Cell — token + owner approval, both consumed:
async fn execute_privileged(&self, params: Value, ctx: &ToolContext,
                            capability: Capability<Self>,
                            owner_approval: OwnerApproval) -> Result<ToolResult>;
```

This means speculative execution (prefetching likely-next-read tools) is **structurally safe**:
the speculation engine literally cannot produce the types needed to call a Write Cell. This is not a
runtime policy check that could be bypassed — it is a compile-time type constraint.

---

## 5. Safety Hook Chain as Pipeline of Verify Cells

Before any Write or Privileged Cell executes, the safety hook chain runs. In unified terms, this
is a **Pipeline pattern** — a linear Graph of Verify Cells where each can approve, modify, or
reject the execution request.

### The 7 Verify Cells in Order

```
[1] PolicyCage ─→ [2] AllowlistGuard ─→ [3] SpendingLimiter ─→ [4] RateLimiter
        │                  │                     │                      │
        v                  v                     v                      v
 Behavioral state    Token/contract        Per-tick/day USD       Operations/window
 enforcement         allowlist check       budget enforcement     rate enforcement
        │                  │                     │                      │
        └──────────────────┴─────────────────────┴──────────────────────┘
                                       │
                                       v
[5] RevmSimulator ─→ [6] HallucinationDetector ─→ [7] ResultFilter
        │                       │                        │
        v                       v                        v
 Pre-flight EVM          Address/amount            Output sanitization,
 fork simulation         ground-truth check        response size cap
```

### Pipeline Execution Semantics

Each Verify Cell returns one of three verdicts:

```rust
pub enum HookDecision {
    Allow,                          // Pass to next Cell in Pipeline
    AllowModified(Value),           // Pass modified params to next Cell
    Reject(String),                 // Pipeline early-exit, tool blocked
}
```

The Pipeline has **early-exit semantics**: any Reject stops the chain immediately. Modified
parameters flow forward — each subsequent Cell sees the potentially-transformed input.

### Capability Minting

If all seven Verify Cells pass, the Pipeline produces an `ActionPermit` which is used to mint
the `Capability<T>` token. This is the **only code path** that can create a Capability — the
constructor is `pub(crate)`:

```rust
pub async fn run_safety_pipeline(
    verify_cells: &[Box<dyn SafetyHook>],
    tool: &ToolDef,
    mut params: Value,
    ctx: &ToolContext,
) -> Result<(Value, Capability<T>)> {
    // Pipeline of Verify Cells
    for cell in verify_cells {
        match cell.on_tool_call(tool, &params, ctx).await? {
            HookDecision::Allow => continue,
            HookDecision::AllowModified(new_params) => { params = new_params; }
            HookDecision::Reject(reason) => {
                return Err(SafetyError::Rejected { tool: tool.name, reason });
            }
        }
    }
    // All passed — mint the token
    let permit = ActionPermit::create(tool, &params, ctx)?;
    let capability = Capability::new(permit.value_limit, permit.expires_at,
                                     permit.policy_hash, permit.id);
    Ok((params, capability))
}
```

---

## 6. Role-Based Filtering as Graph-Level Capability Intersection

The `StaticToolRegistry` filters tools by agent role. In unified terms, this is **capability
intersection at the Graph level** — the Graph definition declares which Cells are available based
on the Agent's role (a Space-level property).

| Role | Available Cells | Rationale |
|---|---|---|
| **Implementer** | All 16 | Full read + write + execution access |
| **Reviewer** | Read-only subset (8) | Can inspect but not modify |
| **Researcher** | Read + Web (10) | Can search and fetch, no write |
| **Architect** | Read + Search (8) | Can inspect and plan |
| **Scribe** | Read + Write (12) | Can read and write docs, no execution |
| **Auditor** | Read-only (7) | Strictest: inspection only |

This filtering happens at Graph construction time (agent boot), not at execution time. The
filtered cells are simply not present in the agent's tool registry — the routing cannot select
what does not exist.

---

## 7. ToolResult as Output Signal

Every Connect Cell produces a `ToolResult` — the output Signal:

```rust
pub struct ToolResult {
    pub data: Value,                         // Cell output (serialized)
    pub is_error: bool,                      // Whether execution failed
    pub schema_version: u32,                 // Signal schema version
    pub expected_outcome: Option<String>,    // Prediction (for predict-publish-correct)
    pub actual_outcome: Option<String>,      // Reality (for calibration)
    pub ground_truth_source: Option<String>, // Evidence source
}
```

The `expected_outcome` and `actual_outcome` fields close the predict-publish-correct loop:
- Write Cell publishes its prediction as `expected_outcome`
- The Verify step (Gate pipeline) compares expected vs actual
- Divergence triggers calibration: the episode is tagged for Dream replay and heuristic revision

This makes every Write Cell a learner by construction — it predicts what will happen, reality
provides the ground truth, and the error feeds back through the standard calibration channel.

---

## 8. Event Bus Integration (Observe Protocol)

Every tool Cell emits structured Pulses on Bus during execution:

| Pulse topic | Payload | When |
|---|---|---|
| `tool.{name}.start` | `{ params_hash, tick }` | Handler entry |
| `tool.{name}.step` | `{ step_name, step_index, total_steps }` | Each progress step |
| `tool.{name}.end` | `{ success, duration_ms, result_summary }` | Handler exit |
| `tool.{name}.error` | `{ error_code, error_message }` | Handler failure |

These Pulses are consumed by:
- **Lens Cells** (TUI rendering: progress bars, animation states)
- **React Cells** (budget enforcement: abort if cost exceeds estimate)
- **Calibration Cells** (learning: duration predictions, success rate tracking)

---

## 9. Tool Composition Patterns

Connect Cells compose like any other Cell. Common patterns:

### Sequential Pipeline (Edit Loop)
```toml
[[nodes]]
id = "read"
cell = "read_file@^1"
[[nodes]]
id = "edit"
cell = "edit_file@^1"
[[nodes]]
id = "verify"
cell = "run_tests@^1"
[[edges]]
from = "read"
to = "edit"
[[edges]]
from = "edit"
to = "verify"
```

### Parallel Fan-out (Research)
```toml
[[nodes]]
id = "search-a"
cell = "web_search@^1"
[nodes.params]
query = "rust async patterns"
[[nodes]]
id = "search-b"
cell = "web_search@^1"
[nodes.params]
query = "tokio task spawning"
[[nodes]]
id = "merge"
cell = "greedy-composer@^1"
[[edges]]
from = "search-a"
to = "merge"
[[edges]]
from = "search-b"
to = "merge"
```

### Speculation (Prefetch)

Read Cells support speculative execution via co-occurrence patterns:
- `glob` followed by `read_file` (0.92 probability)
- `grep` followed by `read_file` (0.88 probability)
- `web_search` followed by `web_fetch` (0.75 probability)

The speculation engine fires likely-next Read Cells in parallel. Results are cached for the
tick duration and discarded if unused.

---

## What This Enables

1. **Uniform composition** — tools compose with gates, routers, composers, and every other Cell
   type using the same Graph definition language.
2. **Structural safety** — Capability<T> makes unauthorized execution impossible to express in
   valid Rust, not merely "checked at runtime."
3. **Automatic learning** — every tool participates in predict-publish-correct via ToolResult's
   expected/actual fields.
4. **Progressive trust** — the same Pipeline pattern (7 Verify Cells) scales from "allow all reads"
   to "simulate + approve + audit" for high-value writes.
5. **Domain extensibility** — new tools are new Connect Cells with the same metadata pattern,
   composable with all existing infrastructure.

---

## Feedback Loops

- **Predict-publish-correct on duration**: each tool Cell publishes predicted execution time
  (from `tick_budget`), actual duration is measured, calibration adjusts future cost estimates.
- **Success rate calibration**: `tool.{name}.end` success/failure ratio feeds a Beta-Binomial
  tracker that adjusts Route Cell selection (avoid tools with declining reliability).
- **Speculation accuracy**: co-occurrence cache hit rate feeds speculation threshold adjustment —
  if prefetched results are rarely used, the threshold increases.
- **Safety Pipeline learning**: rejection rates per hook feed adaptive threshold tuning — a hook
  that rejects 95% of calls may indicate a misconfigured policy, surfaced via Lens.

---

## Open Questions

1. **Tool versioning and Cell versioning alignment** — should `ToolDef` carry a `semver::Version`
   field that maps to Cell's `@^version` syntax in Graph definitions?
2. **Cross-domain tool discovery** — when a Graph needs a tool from a domain plugin not yet loaded,
   should the runtime auto-load the plugin (Tier 3 manifest) or require explicit configuration?
3. **Speculative execution budget** — how many parallel prefetch slots per tick? Current design
   uses co-occurrence threshold (0.7) but no hard cap.
4. **Write Cell idempotency** — should `edit_file` be idempotent (no-op if already applied)?
   Current design requires unique `old_string` which prevents re-application.

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| ToolDef struct with Cell metadata fields | `crates/roko-std/src/tool/builtin/mod.rs` | Shipped |
| StaticToolRegistry with role-based filtering | `crates/roko-std/src/tool/registry.rs` | Shipped |
| Safety hook chain (7 hooks) | `crates/roko-agent/src/safety/` | Shipped |
| Capability<T> token system | `crates/roko-agent/src/safety/` | Shipped |
| Handler dispatch | `crates/roko-std/src/tool/handlers.rs` | Shipped |
| Speculation engine (co-occurrence prefetch) | `crates/roko-std/src/tool/` | Planned |
| Tool Cell versioning with semver | `crates/roko-std/src/tool/` | Planned |
| Predict-publish-correct integration for ToolResult | `crates/roko-std/src/tool/` | Planned |

# Mori Reference Patterns for TUI Parity

This pack documents key Mori patterns that Roko TUI should match.
Reference source: `/Users/will/dev/uniswap/bardo/apps/mori/`

## 1. Agent Output Segment Parsing

**Mori file**: `src/tui/widgets/agent_output.rs` (lines 715-904)

Mori parses raw agent output into 8 semantic segment types:

```rust
enum SegmentKind {
    Thinking,      // Natural language reasoning
    Heading,       // Markdown headers (# ## ###)
    ToolUse,       // Tool invocations (▸, >, $, Running, Reading, Writing, Editing, Created)
    Code,          // Code blocks (``` fenced)
    Success,       // Positive outcomes (✓, PASS, APPROVE, ok)
    Error,         // Failures (ERROR, FAILED, REVISE, error[)
    Blank,         // Empty lines
    TurnMarker,    // Turn boundaries (──── turn N)
}
```

**Parsing strategy**:
1. Line-by-line processing with preprocessing
2. Insert newlines before "Now ", "Let ", "I'll " to break blob output
3. Split long lines (>120 chars) at sentence boundaries
4. Track code block state (toggle on ``` fences)
5. Tag each line by content patterns (regex-free: starts_with / contains)
6. Group consecutive same-kind segments into `SegmentGroup`
7. Render each group with theme-specific styling

**Key patterns for tagging**:
- `Thinking`: default for non-tool, non-code, non-heading lines
- `Heading`: starts with `# ` or `## ` or `### `
- `ToolUse`: starts with `▸ `, `> `, `$ `, or `Running `, `Reading `, `Writing `, `Editing `, `Created `
- `Code`: between ``` fences, or starts with 4+ spaces when preceded by blank
- `Success`: contains `✓`, `PASS`, `APPROVE`, `ok` (case sensitive patterns)
- `Error`: contains `ERROR`, `FAILED`, `error[`, starts with `error`
- `TurnMarker`: matches `────` turn marker pattern

## 2. CachedRender

```rust
pub struct CachedRender {
    pub last_len: usize,           // Last content byte length
    pub lines: Vec<Line<'static>>, // Pre-rendered ratatui Lines
}
```

Cache parsed + styled lines to avoid re-parsing on every frame:
- On agent output update: check if `output.len()` changed
- If changed: re-parse → group → style → store in CachedRender
- On render: use cached lines directly (no re-parsing)

## 3. Approval Flow

**Mori files**: `src/agent/events.rs`, `src/app/parallel.rs`, `src/state/mod.rs`

```rust
// Event carries opaque JSON-RPC request ID
AgentEvent::ApprovalRequested {
    role: AgentRole,
    instance: Option<String>,
    command: String,
    approval_id: serde_json::Value,  // Echo verbatim in response
}

// State tracking
pub struct PendingApproval {
    pub role: AgentRole,
    pub command: String,
    pub approval_id: serde_json::Value,
}
```

Flow:
1. Agent process emits ApprovalRequested event via stdout JSON-RPC
2. Orchestrator receives event, creates PendingApproval
3. TUI shows approval modal with command details
4. User presses y/n
5. Response sent back to agent via stdin with echoed approval_id
6. In parallel mode, Mori auto-approves (configurable)

**Roko adaptation**: Use `mpsc::Sender<ApprovalResponse>` + `oneshot::Sender<bool>`
for the IPC channel between PlanRunner and TUI.

## 4. Process Supervision Display

Mori has a "P:Procs" sub-tab showing:
- Live cargo build output from all parallel plans
- Per-plan process status (running, stopped, failed)
- sccache stats

**Roko adaptation**: Use `sysinfo` crate to collect per-PID metrics:
- `ProcessSupervisor` in roko-runtime already tracks PIDs
- Add `active_pids() -> Vec<u32>` method
- Sys metrics thread polls sysinfo for CPU%, MEM per PID
- Render as table: PID | Role | CPU% | MEM | State | Uptime

## 5. Execution Waves

```rust
pub struct RunState {
    pub execution_waves: Vec<(usize, Vec<String>)>,  // (wave_index, plan_bases)
    pub current_wave: usize,
    pub wave_expanded: HashSet<usize>,
}
```

Waves group plans by dependency level (topological sort):
- Wave 0: no dependencies
- Wave 1: depends only on Wave 0
- Wave N: depends only on Waves 0..N-1
- UI shows collapsible wave headers
- Shift+Left/Right navigates between waves

## 6. NervViz (State-Driven Visualization)

**Mori file**: `src/tui/nerv_viz.rs`

```rust
pub struct VizContext {
    pub task_progress: f64,      // 0-1
    pub plan_progress: f64,      // 0-1
    pub context_pressure: f64,   // 0-1
    pub token_rate: f64,         // 0-1 normalized
    pub agent_active: bool,
    pub iteration: u32,
    pub error_state: bool,
}

// Three core visualization primitives:
pub fn progress_field(area, buf, elapsed, progress: f64);
pub fn activity_ripples(area, buf, elapsed, activity: f64);
pub fn data_rain(area, buf, elapsed, throughput: f64);
pub fn state_viz(area, buf, elapsed, ctx: &VizContext);  // Composites all layers
```

**Key design rules**:
- Only write to empty cells (spaces). Never overwrite text.
- Use braille characters (U+2800 range) for sub-pixel detail.
- All animations driven by `elapsed` seconds from `atmosphere.elapsed()`.
- Colors stay in ROSEDUST palette (rose/violet hues).
- No interactive UI — just visualization overlays.
- VizContext is built from live state each frame.

## 7. Agent Spawn Pattern

**Mori file**: `src/agent/connection.rs` (lines 2444-2620)

Key patterns for reference (already implemented in roko-agent):
- `--bare` flag saves ~92% prompt overhead
- Tool restrictions per role (Conductor = read-only, Implementer = write)
- MCP auto-discovery fallback
- Cargo jobs capped at 2 per agent
- Subprocess killed on Drop
- stdout/stderr parsed for JSON-RPC events

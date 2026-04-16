# Section F: TUI & Interfaces

Source: `tmp/integrate-prds/06-BUILD-SEQUENCE.md`, `docs/07-interfaces/`
Target: `crates/roko-cli/src/tui/`

The TUI scaffold already exists (~80 files) with ratatui dependency and basic page/widget structure. Needs wiring to live data and interactive features.

---

## F.01 — Interactive TUI Dashboard

**Status**: SCAFFOLD
**Priority**: P0 (highest self-hosting priority)
**Estimated LOC**: ~300
**Dependencies**: None

### Files to modify

- `crates/roko-cli/src/tui/app.rs` — Main app loop, event handling
- `crates/roko-cli/src/tui/dashboard.rs` — Dashboard layout and rendering
- `crates/roko-cli/src/tui/state.rs` — Live state management
- `crates/roko-cli/src/tui/event.rs` — Terminal event handling
- `crates/roko-cli/src/main.rs` — Wire `dashboard` command to TUI app

### Context

The TUI scaffold has ~80 files with views, modals, widgets, and page layouts. `ratatui` is already a dependency. The `dashboard` command currently renders text-only output. Need to wire it into an interactive terminal UI.

Existing structure:
- Views: `agents_view.rs`, `config_view.rs`, `dashboard_view.rs`, `logs_view.rs`, `plans_view.rs`, `git_view.rs`, `context_view.rs`
- Modals: `help.rs`, `confirm.rs`, `approval.rs`, `task_detail.rs`, `plan_detail.rs`, `wave_overview.rs`, etc.
- Widgets: `header_bar.rs`, `status_bar.rs`, `task_progress.rs`, `token_sparkline.rs`, `wave_progress.rs`, `plan_tree.rs`, etc.
- Pages: `efficiency.rs`, `operations.rs`

### Implementation details

1. In `app.rs`, implement the main event loop:
   ```rust
   pub async fn run_tui(state: AppState) -> anyhow::Result<()> {
       let mut terminal = setup_terminal()?;
       loop {
           terminal.draw(|f| render(f, &state))?;
           if let Event::Key(key) = crossterm::event::read()? {
               match key.code {
                   KeyCode::Char('q') | KeyCode::Esc => break,
                   KeyCode::Tab => state.next_tab(),
                   KeyCode::Char('j') => state.scroll_down(),
                   KeyCode::Char('k') => state.scroll_up(),
                   KeyCode::Enter => state.select(),
                   KeyCode::Char('?') => state.toggle_help(),
                   _ => {}
               }
           }
           state.refresh_data().await?; // poll live data
       }
       restore_terminal(terminal)?;
       Ok(())
   }
   ```
2. In `state.rs`, add live data polling:
   - Read `.roko/state/executor.json` for plan execution state
   - Read `.roko/episodes.jsonl` for recent episodes
   - Read `.roko/learn/cascade-router.json` for model routing stats
   - Read `.roko/learn/efficiency.jsonl` for efficiency metrics
   - Poll every 2 seconds
3. In `dashboard.rs`, wire the tab-based layout:
   - Tab 1: Dashboard overview (health, active plans, recent episodes)
   - Tab 2: Plans (list + detail)
   - Tab 3: Agents (pool status, model distribution)
   - Tab 4: Logs (live episode stream)
   - Tab 5: Config (current settings)
4. Ensure terminal restoration on panic (set panic hook)
5. Wire `dashboard` CLI command to `run_tui()`

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
# Manual test: cargo run -p roko-cli -- dashboard
# Should show interactive TUI, press 'q' to quit
```

---

## F.02 — TUI Plan Execution View

**Status**: SCAFFOLD
**Priority**: P0
**Estimated LOC**: ~100
**Dependencies**: F.01

### Files to modify

- `crates/roko-cli/src/tui/views/plans_view.rs` — Plan list and detail rendering
- `crates/roko-cli/src/tui/widgets/plan_tree.rs` — DAG visualization
- `crates/roko-cli/src/tui/widgets/task_progress.rs` — Per-task progress bars

### Context

The plan tree widget exists but needs to show live execution state: which tasks are running, completed, failed, blocked. The plan view should be the primary monitoring interface during `plan run`.

### Implementation details

1. In `plans_view.rs`:
   - List all discovered plans with status indicators (pending/running/complete/failed)
   - On select: show plan detail with task DAG
   - Show current wave, completed tasks, remaining tasks
2. In `plan_tree.rs`:
   - Render DAG as indented tree with status colors:
     - Green: completed, Yellow: running, Red: failed, Gray: pending, Blue: blocked
   - Show task name, model tier, duration (if completed)
   - Highlight critical path
3. In `task_progress.rs`:
   - For running tasks: show elapsed time, model being used, token count
   - For completed tasks: show duration, gate verdict, cost

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
```

---

## F.03 — TUI Agent Pool View

**Status**: SCAFFOLD
**Priority**: P1
**Estimated LOC**: ~80
**Dependencies**: F.01

### Files to modify

- `crates/roko-cli/src/tui/views/agents_view.rs` — Agent list and status
- `crates/roko-cli/src/tui/modals/agent_pool_modal.rs` — Agent detail modal

### Context

Show live agent pool status: which agents are active, their model backends, current task, token usage, cost.

### Implementation details

1. In `agents_view.rs`:
   - Table: agent_id | model | status (idle/busy/error) | current_task | tokens_used | cost
   - Color-coded status: green=idle, yellow=busy, red=error
   - Sort by status (busy first) then by name
2. In `agent_pool_modal.rs`:
   - On Enter: show agent detail — full history, model routing stats, recent episodes
   - Show per-agent C-Factor contribution if available
3. Data source: read from ProcessSupervisor state or `.roko/state/` files

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
```

---

## F.04 — TUI Efficiency Dashboard

**Status**: SCAFFOLD
**Priority**: P1
**Estimated LOC**: ~60
**Dependencies**: F.01

### Files to modify

- `crates/roko-cli/src/tui/pages/efficiency.rs` — Efficiency metrics display
- `crates/roko-cli/src/tui/widgets/token_sparkline.rs` — Token usage sparkline

### Context

Show efficiency metrics over time: tokens per task, cost per task, success rate, model distribution. Data exists in `.roko/learn/efficiency.jsonl`.

### Implementation details

1. In `efficiency.rs`:
   - Top row: summary stats (total tokens, total cost, avg tokens/task, success rate)
   - Middle: token sparkline over last N tasks (using `token_sparkline.rs` widget)
   - Bottom: model distribution bar chart (how many tasks per tier: T0/T1/T2)
2. In `token_sparkline.rs`:
   - Braille-based sparkline showing token usage trend
   - Configurable window (last 10/50/100 tasks)
3. Data source: parse `.roko/learn/efficiency.jsonl` for per-turn events

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
```

---

## F.05 — TUI Log Viewer

**Status**: SCAFFOLD
**Priority**: P1
**Estimated LOC**: ~50
**Dependencies**: F.01

### Files to modify

- `crates/roko-cli/src/tui/views/logs_view.rs` — Live log stream
- `crates/roko-cli/src/tui/event_sources.rs` — Log file tailing

### Context

Stream episodes and events in real-time during plan execution. Currently logs go to `.roko/episodes.jsonl` but there's no live viewer.

### Implementation details

1. In `event_sources.rs`:
   - Tail `.roko/episodes.jsonl` using file watcher (notify crate or polling)
   - Parse each line as JSON, extract: timestamp, agent, action, verdict
2. In `logs_view.rs`:
   - Scrollable list of log entries with auto-scroll (latest at bottom)
   - Color-coded by type: agent turn (blue), gate pass (green), gate fail (red), system (gray)
   - Filter by: agent, type, time range
   - Keyboard: `/` to filter, `f` to toggle auto-scroll, `j/k` to scroll

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
```

---

## F.06 — HTTP API Endpoints (roko-serve)

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~150
**Dependencies**: None

### Files to modify

- `crates/roko-serve/src/routes/status.rs` — Add missing endpoints
- `crates/roko-serve/src/routes/learning.rs` — Add learning endpoints
- `crates/roko-serve/src/routes/mod.rs` — Wire new routes

### Context

Several HTTP endpoints specified in docs are missing from roko-serve. Existing routes cover: run, ws, sse, agents, config, plans, prds, research, status, providers, templates, learning, webhooks, subscriptions, deployments. Missing: gate summary/history, some learning sub-routes, health, metrics/summary, WebSocket progress events.

### Implementation details

1. Add to `routes/status.rs`:
   - `GET /api/health` — simple health check (200 OK + uptime + version)
   - `GET /api/metrics/summary` — combined metrics (C-Factor, cost, efficiency, active plans)
2. Add to `routes/learning.rs`:
   - `GET /api/learning/cascade` — CascadeRouter state dump
   - `GET /api/learning/experiments` — active/concluded experiments
   - `GET /api/learning/efficiency` — recent efficiency events
   - `GET /api/learning/adaptive-thresholds` — gate threshold state
3. Add gate endpoints (new file or extend `routes/status.rs`):
   - `GET /api/gates/summary` — per-rung pass/fail counts
   - `GET /api/gates/history` — recent gate verdicts with details
4. Wire all routes in `mod.rs`

### Verify command

```bash
cargo build -p roko-serve 2>&1 | tail -5
# Then: curl http://localhost:6677/api/health | jq
# Then: curl http://localhost:6677/api/metrics/summary | jq
```

---

## F.07 — Observability: Tracing

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-cli/src/main.rs` — Initialize tracing subscriber
- `crates/roko-cli/src/orchestrate.rs` — Add `#[instrument]` spans

### Context

`tracing-subscriber` is not wired. No `#[instrument]` spans exist in orchestrate.rs. Need structured logging for debugging plan execution.

### Implementation details

1. In `main.rs`, initialize tracing:
   ```rust
   tracing_subscriber::fmt()
       .with_env_filter(EnvFilter::from_default_env())
       .with_target(false)
       .init();
   ```
2. Add `#[instrument(skip(...))]` to key functions in `orchestrate.rs`:
   - `run_plan()`, `dispatch_task()`, `run_gate_pipeline()`, `handle_gate_result()`
3. Add `tracing::info!` / `tracing::warn!` at key decision points:
   - Model selection, gate verdict, replan trigger, task completion
4. Configure via `RUST_LOG` env var (default: `roko=info`)

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
RUST_LOG=roko=debug cargo run -p roko-cli -- status 2>&1 | grep -c 'INFO\|DEBUG'
```

---

## F.08 — Cost Aggregation

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/costs_log.rs` — Aggregation queries
- `crates/roko-cli/src/status.rs` — Display cost summary

### Context

Cost data is logged per-turn but no aggregation exists. Need: total cost, cost per plan, cost per model tier, cost trend.

### Implementation details

1. In `costs_log.rs`, add aggregation methods:
   - `fn total_cost(&self) -> f64`
   - `fn cost_by_model(&self) -> HashMap<String, f64>`
   - `fn cost_by_plan(&self) -> HashMap<String, f64>`
   - `fn daily_cost(&self, days: usize) -> Vec<(String, f64)>` — per-day breakdown
2. In `status.rs`, add cost section to `roko status` output:
   ```
   Cost Summary:
     Total:    $12.34
     Today:    $3.21
     By model: opus=$8.10, sonnet=$3.50, haiku=$0.74
     By plan:  wire-safety=$4.20, fix-routing=$3.10, ...
   ```

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo run -p roko-cli -- status 2>&1 | grep -i cost
```

---

## F.09 — Daemon Mode

**Status**: NOT DONE
**Priority**: P3
**Estimated LOC**: ~80
**Dependencies**: None

### Files to modify

- `crates/roko-cli/src/daemon.rs` — Daemon infrastructure (exists, partial)
- `crates/roko-cli/src/daemon/launchd.rs` — macOS launchd plist generation (exists)
- `crates/roko-cli/src/main.rs` — Add `daemon` subcommand

### Context

`daemon.rs` and `daemon/launchd.rs` exist but are not wired to CLI. Need `roko daemon install/start/stop/status` subcommands.

### Implementation details

1. Add `Daemon` CLI variant with subcommands: `install`, `uninstall`, `start`, `stop`, `status`
2. `install`:
   - macOS: generate launchd plist at `~/Library/LaunchAgents/com.nunchi.roko.plist`
   - Linux: generate systemd unit at `~/.config/systemd/user/roko.service`
3. `start`: `launchctl load` (macOS) or `systemctl --user start roko` (Linux)
4. `stop`: `launchctl unload` or `systemctl --user stop roko`
5. `status`: query launchd/systemd for running state
6. Daemon runs `roko serve` + periodic `roko plan run` + periodic health checks

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo run -p roko-cli -- daemon --help 2>&1 | head -5
```

---

## F.10 — Code Intelligence MCP Server

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~150
**Dependencies**: None

### Files to modify

- `crates/roko-mcp-code/` — **NEW CRATE** (or extend existing MCP crates)
- `crates/roko-index/src/` — Expose index query API

### Context

`roko-index` has parser + graph + HDC indexing (4 modules, 30 tests) but no MCP server. Need an MCP server that exposes code intelligence to agents: symbol lookup, call graph, import resolution, semantic search.

`crates/roko-mcp-github/`, `crates/roko-mcp-scripts/`, `crates/roko-mcp-slack/`, `crates/roko-mcp-stdio/` already exist as reference implementations.

### Implementation details

1. Create `roko-mcp-code` crate following existing MCP crate patterns
2. Expose tools:
   - `symbol_lookup(name: String) -> Vec<SymbolInfo>` — find symbol definitions
   - `call_graph(function: String, depth: u32) -> CallGraph` — callers/callees
   - `imports(file: String) -> Vec<Import>` — file's import graph
   - `semantic_search(query: String, limit: u32) -> Vec<SearchResult>` — HDC-powered semantic search
3. Initialize `roko-index` on startup, index workspace
4. Add to default MCP config in `roko.toml` auto-discovery

### Verify command

```bash
cargo build -p roko-mcp-code 2>&1 | tail -5
# Then test via MCP protocol
```

---

## F.11 — Auto Plan Generation on PRD Promote

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~20
**Dependencies**: None

### Files to modify

- `crates/roko-cli/src/prd.rs` — Wire promote → plan generation
- `crates/roko-serve/src/routes/prds.rs` — API-side promote trigger

### Context

`prd draft promote` should automatically trigger `prd plan <slug>` to generate an implementation plan. The handoff path exists but may not be fully wired in all code paths.

### Implementation details

1. In `prd.rs`, after successful `draft promote`:
   - Call `plan_generate()` with the promoted PRD slug
   - Print: "Plan generated: plans/{slug}/tasks.toml"
   - If generation fails, warn but don't fail the promote
2. In `routes/prds.rs`, if the API-side promote endpoint exists:
   - Trigger plan generation asynchronously after promote response
3. Ensure `plan_generate.rs` has access to the promoted PRD content

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
# Test: cargo run -p roko-cli -- prd draft promote <slug>
# Should see "Plan generated:" in output
```

---

## F.12 — PlaybookStore Integration

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/playbook.rs` — PlaybookStore (exists)
- `crates/roko-cli/src/orchestrate.rs` — Wire PlaybookStore into PlanRunner

### Context

`PlaybookStore` exists in roko-learn but was never added to PlanRunner. Playbooks are successful task patterns that should be injected into agent context for similar future tasks.

### Implementation details

1. In `orchestrate.rs`:
   - Initialize `PlaybookStore` from `.roko/learn/playbooks.json`
   - Before dispatch: query `playbook_store.relevant(task_category, 3)` for matching playbooks
   - Pass playbooks to `SystemPromptBuilder::with_playbooks()`
   - After successful task: call `playbook_store.record(task, outcome)` to update
2. Ensure `PlaybookStore::record()` extracts patterns from successful task completions
3. Ensure `PlaybookStore::relevant()` returns playbooks ranked by relevance and recency

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo test -p roko-learn --lib -- playbook 2>&1 | tail -10
```

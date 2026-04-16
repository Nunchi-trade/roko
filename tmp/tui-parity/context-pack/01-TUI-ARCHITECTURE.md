# TUI Architecture Context Pack

## Data flow

```
                    ┌─────────────────────────────────────────────┐
                    │             Connected Mode (new)            │
                    │                                             │
 orchestrate.rs ──► │  StateHub ──► watch::Sender<Snapshot>       │
                    │                     │                       │
                    │              borrow_and_update()             │
                    │                     ▼                       │
                    │  ┌──────────────────────────────────────┐   │
                    │  │ App.snapshot_rx: Option<watch::Rx>   │   │
                    │  └──────────────────────────────────────┘   │
                    └─────────────┬───────────────────────────────┘
                                  │
                    ┌─────────────┼───────────────────────────────┐
                    │             │  Standalone Mode (existing)   │
                    │             │                               │
 .roko/ files ────► │  DashboardData.load_best_effort()          │
                    │             │                               │
                    └─────────────┼───────────────────────────────┘
                                  │
                                  ▼
                    ┌─────────────────────────────────────────────┐
                    │         update_from_snapshot()               │
                    │                                             │
                    │  TuiState (406 fields)                      │
                    │  ├── plans: Vec<PlanEntry>                  │
                    │  ├── agents: Vec<AgentRow>                  │
                    │  ├── execution_waves: Vec<Wave>             │
                    │  ├── gate_results: Vec<GateResultEntry>     │
                    │  ├── phase_pipeline: Vec<PhaseStep>         │
                    │  └── ... (navigation, scroll, modals, etc.) │
                    └─────────────┬───────────────────────────────┘
                                  │
                                  ▼
                    ┌─────────────────────────────────────────────┐
                    │  Views (F1-F7) + Widgets + Modals           │
                    │  Read TuiState, write to Frame buffer       │
                    └─────────────┬───────────────────────────────┘
                                  │
                                  ▼
                    ┌─────────────────────────────────────────────┐
                    │  ratatui::Frame → Terminal                  │
                    └─────────────────────────────────────────────┘
```

## File map

### Core TUI files
| File | Purpose |
|------|---------|
| `tui/mod.rs` | Module exports, re-exports |
| `tui/app.rs` | App struct, event loop, background threads |
| `tui/state.rs` | TuiState (406 fields), PlanEntry, AgentRow, Wave, etc. |
| `tui/dashboard.rs` | DashboardData loader, DashboardScaffold, Theme |
| `tui/event.rs` | EventHandler, Event enum (Key, Mouse, Resize, Tick) |
| `tui/input.rs` | InputMode, FocusZone, key dispatch, TuiAction |
| `tui/tabs.rs` | Tab enum (F1-F7), tab navigation |
| `tui/layout.rs` | Ratatui layout helpers |
| `tui/scroll.rs` | ScrollAccel for held-key scrolling |
| `tui/ansi.rs` | ANSI color/style utilities |
| `tui/atmosphere.rs` | Breathing/heartbeat animations |
| `tui/effects_config.rs` | PostFX pipeline configuration |
| `tui/postfx.rs` | Post-processing effects |
| `tui/postfx_pipeline.rs` | Effect chain runner |
| `tui/hit_test.rs` | Mouse hit-zone testing |

### Views (one per F-key tab)
| File | Tab | Content |
|------|-----|---------|
| `tui/views/dashboard_view.rs` | F1 | Overview gauges, plan progress, cost |
| `tui/views/plans_view.rs` | F2 | Plan tree, task progress, wave overview |
| `tui/views/agents_view.rs` | F3 | Agent output, diffs, token burn |
| `tui/views/git_view.rs` | F4 | Branch tree, commit graph, worktrees |
| `tui/views/logs_view.rs` | F5 | Log viewer with filtering |
| `tui/views/config_view.rs` | F6 | Config editor |
| `tui/views/context_view.rs` | F7 | Engram DAG, episode replay |

### Widgets
| File | Purpose |
|------|---------|
| `tui/widgets/header_bar.rs` | Top status bar |
| `tui/widgets/status_bar.rs` | Bottom status bar |
| `tui/widgets/plan_tree.rs` | Hierarchical plan tree |
| `tui/widgets/task_progress.rs` | Task checklist |
| `tui/widgets/parallel_pool.rs` | Parallel agent pool |
| `tui/widgets/phase_compact.rs` | Phase pipeline progress |
| `tui/widgets/token_sparkline.rs` | Token burn sparkline |
| `tui/widgets/sys_metrics.rs` | CPU/memory/disk/network gauges |
| `tui/widgets/diff_panel.rs` | Diff viewer |
| `tui/widgets/error_digest.rs` | Error summary |
| `tui/widgets/branch_tree.rs` | Git branch visualization |
| `tui/widgets/rosedust.rs` | ROSEDUST theme + effects |
| `tui/widgets/braille.rs` | Braille/block drawing |

### Modals
| File | Purpose |
|------|---------|
| `tui/modals/approval.rs` | Agent command approval |
| `tui/modals/confirm.rs` | Destructive action confirmation |
| `tui/modals/inject.rs` | Free-text injection |
| `tui/modals/notification.rs` | Toast notifications |
| `tui/modals/quit.rs` | Quit confirmation |
| `tui/modals/wave_overview.rs` | Wave browser |
| `tui/modals/queue_overview.rs` | Milestone queue |
| `tui/modals/agent_pool_modal.rs` | Agent roster |
| `tui/modals/task_picker.rs` | Task selection |
| `tui/modals/batch_review.rs` | Batch results review |
| `tui/modals/plan_detail.rs` | Plan detail overlay |

## Key types

```rust
// state.rs
pub struct TuiState { /* 406 fields */ }
pub struct PlanEntry { id, name, status, active, phase, tasks_total, tasks_done, tasks_failed, elapsed_secs, wave, expanded, tasks: Vec<TaskEntry> }
pub struct AgentRow { id, active, role, model, input_tokens, output_tokens, context_limit, current_plan, current_task, last_output_line }
pub struct Wave { index, plans: Vec<String>, done, total, expanded }
pub struct TaskRow { id, title, status: TaskRowStatus, elapsed_secs }
pub struct PhaseStep { name, status: PhaseStatus, elapsed_secs, pct }
pub struct GateResultEntry { gate, plan_id, passed, output }

// app.rs
pub struct App { workdir, tui_state, atmosphere, fx_config, active_modal, notifications, data, running, sys_rx, data_rx, git_rx, ... }

// dashboard.rs
pub struct DashboardData { root, executor_state, plans, active_tasks, agents, gate_results, efficiency, ... }
```

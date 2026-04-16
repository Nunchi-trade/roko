# TUI State Context Pack

This pack documents the current TuiState and App structs so batch agents
can modify them accurately without reading 1000+ lines of source.

## TuiState (`crates/roko-cli/src/tui/state.rs`)

### Major field groups

**Orchestrator state** (lines ~304-314):
- `orchestrator_state: String`
- `plans: Vec<PlanEntry>`
- `current_plan_idx: usize`
- `current_iteration: u32`
- `current_phase: String`

**Phase pipeline** (lines ~316-318):
- `phase_pipeline: Vec<PhaseStep>`

**Execution waves** (lines ~320-322):
- `execution_waves: Vec<Wave>`

**Task checklist** (lines ~324-326):
- `current_task_checklist: Vec<TaskRow>`

**Gate results** (lines ~328-330):
- `gate_results: Vec<GateResultEntry>`

**Agent roster** (lines ~332-338):
- `agents: Vec<AgentRow>` — active agent list
- `agents_by_id: HashMap<String, AgentState>` — **DEAD, remove in T7**
- `parallel_agents: Vec<ParallelAgentState>`

**Navigation** (lines ~340-352):
- `active_tab: Tab`
- `selected_plan: usize`
- `selected_agent: usize`
- `focus: FocusZone`

**Animation** (lines ~354-356):
- `atmosphere: Atmosphere`

**Input** (lines ~358-368):
- `input_mode: InputMode`
- `message_input: String`
- `filter_text: String`
- `filter_active: bool`

**Scroll positions** (lines ~370-392):
- 11 scroll offset fields + `log_auto_tail: bool`

**Modal visibility** (lines ~394-408):
- 7 `show_*` boolean flags

**Approval/confirm** (lines ~410-414):
- `pending_approval: Option<PendingApproval>`
- `pending_confirm: Option<PendingConfirm>`

**Git state** (lines ~416-434):
- Branch tree, commit graph, worktree list, summary lines

**Plans & content** (lines ~439-445):
- `plan_detail_content: String` — **DEAD, remove in T7**
- `plan_summary_content: String` — **DEAD, remove in T7**
- `pipeline_run_state: bool`

**Cost/tokens** (lines ~453-469):
- Token & cost aggregates, cumulative totals, burn rate

**Token history** (lines ~471-475):
- `token_burn_history: HashMap<String, VecDeque<u64>>` — **DEAD, remove in T7**

**System metrics** (lines ~477-479):
- `sys_metrics: SysMetrics` (CPU, memory, disk, network)

**Timing** (lines ~481-483):
- `run_started: Option<Instant>`

**Wave navigation** (lines ~485-487):
- `selected_wave_idx: usize`

**Config editor** (lines ~489-501):
- Cursor, scroll, pending edits, editing state

**Agent pane** (lines ~503-505):
- `agent_pane_group: usize`

### Key methods

```rust
impl TuiState {
    pub fn new() -> Self;
    pub fn from_dashboard_data(data: &DashboardData) -> Self;
    pub fn update_from_snapshot(&mut self, data: &DashboardData);
    pub fn task_counts(&self) -> (usize, usize);
    pub fn elapsed_secs(&self) -> f64;
    pub fn wave_count(&self) -> usize;
    pub fn current_wave(&self) -> Option<&Wave>;
    pub fn active_agent_count(&self) -> usize;
    pub fn filter_ref(&self) -> Option<&str>;
}
```

## App (`crates/roko-cli/src/tui/app.rs`)

### Key fields

```rust
pub struct App {
    pub workdir: PathBuf,
    pub tui_state: TuiState,
    pub atmosphere: Atmosphere,
    pub fx_config: EffectsConfig,
    pub active_modal: Option<ModalState>,
    pub notifications: Vec<Notification>,
    pub scroll_accel: ScrollAccel,
    pub current_page: PageId,
    pub data: DashboardData,
    pub scaffold: DashboardScaffold,
    pub running: bool,
    pub last_refresh: Instant,
    pub sys_rx: Option<mpsc::Receiver<SysMetrics>>,
    pub data_rx: Option<mpsc::Receiver<DashboardData>>,
    pub git_rx: Option<mpsc::Receiver<GitBgData>>,
    pub frame_counter: u64,
    pub last_input: Instant,
    pub terminal_size: (u16, u16),
}
```

### Key methods

```rust
impl App {
    pub fn new(root: impl AsRef<Path>) -> Self;
    pub fn new_with_page(root, initial_page: Option<PageId>) -> Self;
    pub fn run(mut self) -> Result<()>;        // Main event loop (sync)
    pub async fn run(terminal, app) -> ...;     // Async variant
}
```

### Background threads pattern

App spawns background threads for non-blocking I/O:

```rust
// In App::new():
let (sys_tx, sys_rx) = mpsc::channel(16);
std::thread::spawn(move || { /* poll sysinfo every 2s, send via sys_tx */ });

let (data_tx, data_rx) = mpsc::channel(4);
std::thread::spawn(move || { /* DashboardData::load_best_effort() every 1s, send via data_tx */ });

let (git_tx, git_rx) = mpsc::channel(4);
std::thread::spawn(move || { /* git status/log/worktree every 5s, send via git_tx */ });
```

### Main loop pattern

```rust
loop {
    // 1. Receive background data
    if let Ok(sys) = self.sys_rx.try_recv() { self.tui_state.sys_metrics = sys; }
    if let Ok(data) = self.data_rx.try_recv() { self.data = data; self.tui_state.update_from_snapshot(&self.data); }
    if let Ok(git) = self.git_rx.try_recv() { self.tui_state.update_git(git); }

    // 2. Handle events (keyboard, mouse, tick)
    match event_handler.next()? {
        Event::Key(key) => self.handle_key(key),
        Event::Tick => { /* adaptive frame rate */ },
        ...
    }

    // 3. Render
    terminal.draw(|f| { self.render(f); })?;

    if !self.running { break; }
}
```

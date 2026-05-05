# Real-time visualization

**Default answer: extend the existing ratatui TUI.** It's already wired
for the data we want to show. See `08-reuse-map.md` for the full inventory.

This file covers (a) the recommended approach — adding a `F11 Bench` tab —
and (b) external alternatives if you want a web/projector view in addition.

## The chart vocabulary (audience-agnostic)

Regardless of medium, these are what we want to display:

1. **Live cost meter** — running USD per backend, ticking up as tasks complete
2. **Live pass-rate** — pass / completed per backend, sparkline
3. **Token waterfall** — for the in-flight task, tokens streaming as bars
4. **Pareto frontier** — final scatter: x=USD/task, y=pass-rate per backend
5. **Per-task heatmap** — rows=tasks, cols=backends, color by pass/fail, shade by cost
6. **Trace timeline** — Gantt of LLM/tool/gate events for a single task
7. **USD per success** — total spend / pass count; the headline metric
8. **C-factor bar** — composite score per backend (roko-only metric, big differentiator)

## Primary: extend roko's existing ratatui TUI

The TUI at `crates/roko-cli/src/tui/` already has 10 tabs (F1-F10), a
file-watcher data plane (`fs_watch.rs`), a JSONL tailer with cursor
(`jsonl_tailer.rs`), and renders with `Table`, `Sparkline`, `BarChart`,
`Gauge`, `Chart`, `Paragraph` — the full chart vocabulary above.

It already tails `.roko/learn/efficiency.jsonl` for the F1 Dashboard and
F10 Learning tabs. **Every metric you need is already streaming through
the TUI's data layer** — you just need to render it grouped by `backend`
instead of by `agent_id`.

### What to add

```
crates/roko-cli/src/tui/views/bench_view.rs    NEW (~300 lines)
crates/roko-cli/src/tui/tabs.rs                EDIT (1 enum variant)
crates/roko-cli/src/tui/mod.rs                 EDIT (1 dispatch arm)
```

`tabs.rs` already declares `Tab::Dashboard`, `Tab::Plans`, ..., `Tab::Learning`
with helpers `ALL`, `fkey()`, `from_key()`, `label()`, `index()`, `next()`.
Adding `Tab::Bench` is the same shape as adding any other tab.

### Layout sketch (ratatui)

```
┌─ F11 Bench ─────────────────────────────────────────────────────┐
│ ┌─ Scoreboard ─────────────────────────────────────────────────┐│
│ │ backend      pass@1  USD     USD/pass  TTFT(ms)  C-factor    ││
│ │ roko         0.85    $4.10   $4.82     420       0.78        ││
│ │ anthropic    0.70    $3.20   $4.57     380       0.55        ││
│ │ langgraph    0.78    $5.60   $7.18     510       0.51        ││
│ │ crewai       0.65    $9.20   $14.15    640       0.42        ││
│ └──────────────────────────────────────────────────────────────┘│
│ ┌─ Live cost (BarChart) ────┬─ Pass-rate (Sparkline per row) ──┐│
│ │ ▓▓▓░░░░░ roko             │ roko       ▁▂▃▅▇█▇▇▇▇             ││
│ │ ▓▓░░░░░░ anthropic        │ anthropic  ▁▂▃▄▅▆▆▆▆▆             ││
│ │ ▓▓▓▓▓░░░ langgraph        │ langgraph  ▁▂▄▅▆▇▇▆▇▇             ││
│ │ ▓▓▓▓▓▓▓░ crewai           │ crewai     ▁▂▃▄▅▅▆▅▆▆             ││
│ └───────────────────────────┴────────────────────────────────────┘│
│ ┌─ Pareto (Chart scatter) ──┬─ Heatmap (Table colored cells) ─────┐│
│ │ ↑ pass-rate               │      r  a  l  c                    ││
│ │ │     • roko              │ t01  ▓░ ▓░ ▓░ ░░                   ││
│ │ │ • anthropic   • langgr  │ t02  ▓░ ▓░ ░░ ░░                   ││
│ │ │           • crewai      │ t03  ▓░ ░░ ▓░ ░░                   ││
│ │ └─────────→ USD/task      │ ...                                ││
│ └───────────────────────────┴────────────────────────────────────┘│
└──────────────────────────────────────────────────────────────────┘
```

All four panels use widgets already in the codebase:

- Scoreboard → `Table` (existing pattern in `plans_view.rs`)
- Live cost → `BarChart` (existing pattern in `dashboard_view.rs`)
- Pass-rate → `Sparkline` (existing pattern in `learning_view.rs`)
- Pareto → `Chart` with `GraphType::Scatter` (existing in `learning_view.rs`)
- Heatmap → `Table` with cell-level `Style::bg()` (existing pattern)

### Data wiring

Reuse `tui/dashboard.rs::FileStamp` for change detection on
`.roko/learn/efficiency.jsonl`:

```rust
// illustrative; matches the pattern in dashboard.rs
struct BenchData {
    by_backend: HashMap<String, BackendStats>,
    by_task:    HashMap<String, HashMap<String, TaskOutcome>>,
    pareto:     Vec<(String, f64, f64)>,  // (backend, usd_avg, pass_rate)
}

impl BenchData {
    fn refresh(&mut self, efficiency_path: &Path, episodes_path: &Path) {
        // EpisodeLogger::read_all(episodes_path) — already exists
        // parse efficiency.jsonl line by line — same shape as F10
        // group by backend, compute aggregates
    }
}
```

The same notify watcher already running for F1 and F10 fires when the
file grows; `bench_view.rs::on_event()` calls `refresh()`. No new
infrastructure.

### Live cost rendering

For the "live race" feel, render the BarChart with current cumulative
spend per backend:

```rust
// illustrative
let bars: Vec<Bar> = bench_data.by_backend.iter()
    .map(|(name, stats)| {
        Bar::default()
            .label(name.into())
            .value(stats.total_usd_cents)  // BarChart needs u64
            .style(color_for_backend(name))
    })
    .collect();

let chart = BarChart::default()
    .block(Block::bordered().title("USD spend (live)"))
    .bar_width(8)
    .data(BarGroup::default().bars(&bars));
f.render_widget(chart, area);
```

Refresh on every notify event = effectively realtime.

### Token rain (per-task drill-down)

For the in-flight task, stream output tokens as they arrive. roko-serve
already publishes `AgentOutput { content, done }` events on `/ws`. The
existing F3 Agents tab already renders this for a single agent. Replicate
the same pattern with one column per backend.

For the bench tab, this could be a "drill-down on Enter" — selecting a
task in the heatmap opens a side-by-side view with one column per backend
showing token-by-token output. Closes the loop visually.

### Effort estimate

- Tab scaffold: 1 hour
- Scoreboard table: 2 hours
- BarChart + Sparkline panels: 2 hours
- Pareto Chart: 2 hours
- Heatmap: 3 hours (cell coloring is fiddly)
- Token-rain drill-down: 4 hours
- Polish + tests: 4 hours

**~2 days** for a feature-complete bench tab.

## Headless mode for CI

The same tab can run headless via roko's existing TUI test pattern (see
`crates/roko-cli/tests/tui_tabs.rs`). For nightly bench runs without a
terminal:

```bash
cargo run -p roko-cli -- bench report --output reports/$(date +%Y-%m-%d).json
```

Where `bench report` is a thin CLI wrapper that constructs the same
`BenchData` and serializes it. Same data path, no rendering.

## Recording for distribution

The TUI records cleanly with:

- **asciinema** — `asciinema rec demo.cast`, plays in browser via the
  asciinema-player JS library
- **vhs** ([charmbracelet/vhs](https://github.com/charmbracelet/vhs)) —
  scripted terminal sessions → reproducible .gif/.mp4. Works well for
  README headers and slide embeds. Example tape:

```vhs
# demo/demo-research/01-five-frame/demo.tape
Output demo.gif
Set FontSize 14
Set Width 1200
Set Height 700

Type "roko bench run --tasks tasks/roko-bench.toml --backends roko,anthropic,langgraph,crewai"
Enter
Sleep 30s

Type "F11"  # switch to bench tab
Sleep 5s

# tasks complete one by one, bars grow
Sleep 60s
```

This gives you a 90-second .gif suitable for README/landing page.

## Secondary: web dashboard for projector / screen-share

If a meeting demo needs a web view (projector, screen share, embedded in
a deck), roko-serve already has the infrastructure:

### Option B1: roko-serve `/ws` event stream + simple HTML

`crates/roko-serve/src/routes/ws.rs` already streams `ServerEvent`s with
back-pressure handling. A small static HTML page can subscribe and render
with [Plotly.js](https://plotly.com/javascript/) — single file, no build:

```html
<!-- demo/demo-research/03-investor/dashboard.html -->
<script src="https://cdn.plot.ly/plotly-2.x.min.js"></script>
<div id="cost-bar"></div>
<div id="pareto"></div>
<script>
const ws = new WebSocket("ws://localhost:6677/ws");
ws.onopen = () => ws.send(JSON.stringify({
    subscribe: ["topic:efficiency.event", "topic:gate_result"]
}));
const state = { byBackend: {} };
ws.onmessage = (e) => {
    const evt = JSON.parse(e.data);
    update(state, evt);
    Plotly.react("cost-bar", [/* ... */]);
    Plotly.react("pareto", [/* ... */]);
};
</script>
```

That's the entire web dashboard. It opens a WebSocket to roko-serve, gets
backfilled events on connect (the `/ws` endpoint replays from the ring
buffer), then live updates. No Streamlit, no Grafana, no extra service.

### Option B2: nunchi-dashboard (already exists)

CLAUDE.md mentions `dashboard-quickstart` — a separate dashboard project
that already speaks to roko-serve. If this is fleshed out, point it at
the same `/ws` and add benchmark-specific panels.

### Option B3: just project the TUI

Often overlooked: a tmux'd ratatui session at large font-size on a
projector is *more* impressive than a web dashboard, because the live
event stream feels fast and dense. For an in-room demo this is the
recommended path.

```bash
# big font, no chrome
ITERM_PROFILE=Demo tmux new-session 'roko dashboard'
```

## When external tools are worth it

Skip these for v1. Consider only if the demo specifically calls for them:

| External | Add only if |
|---|---|
| Streamlit | You need non-engineers to update the dashboard layout themselves |
| Grafana + Prometheus | You're operating a long-lived nightly service and want time-series alerting |
| Langfuse | You want a hosted, multi-user trace browser for a remote team |
| Marimo | You want a notebook-style "explore the data" UI alongside the live view |

For the four recipes in `06-recipes.md`, none of these is required.

## Recommended primary + secondary

| Recipe | Primary live display | Secondary artifact |
|---|---|---|
| 01 five-frame live demo | tmux split with TUI bench tab + asciinema record | vhs-generated .gif |
| 02 nightly | headless `bench report` → JSON + Slack ping on regression | Static HTML report from JSON |
| 03 investor | TUI bench tab projected, recorded with OBS | Static plotly HTML report |
| 04 swe-bench public | n/a (submission, not display) | Predictions JSON for leaderboard |

Every primary is roko-native. Every secondary is a single file or a
single command. No external service runs in the critical path.

## Visual differentiation tricks worth implementing

A few specific moves that pay off for a live audience:

### Race-bar effect

Hold the BarChart at 0 for 1s, then animate growth as tasks complete. The
"slow build" creates anticipation. Implementation: keep `bar.value` at 0
until the first event arrives per backend, then ease to actual.

### Color semantics

- **Green** = roko (the protagonist)
- **Blue** = anthropic-direct (the friendly baseline)
- **Yellow/orange** = langgraph (the adequate competitor)
- **Red** = autogen (the expensive one)

ratatui `Color::Green/Blue/Yellow/Red` — already in scope.

### "Receipts" mode

Press `D` (drill-down) on a task row → split-screen of all backends' full
output for that task, side by side, with cost/time annotations. This is
what convinces a skeptic: they can see exactly what each framework
generated.

Implementation: read `.roko/memory/episodes.jsonl`, filter by
`task_id`, group by `backend`, render each as a `Paragraph`.

### "Why" panel

When hovering over a task in the heatmap, show in a side panel:
- Which gate failed (from `Episode.gate_verdicts`)
- How many iterations
- Final tool calls
- Cost breakdown

All these fields are already in `Episode` — no new data needed.

### C-factor as "trust score"

Big number in the corner: `roko: 0.78` vs `langgraph: 0.51`. C-factor is
roko's composite quality metric (8 components incl. cost efficiency, gate
pass rate, error recovery rate). Other frameworks score lower mechanically
because they don't have gates → `gate_pass_rate=0` for them, dragging the
composite down. This is *correct* — they really don't gate-verify their
outputs — but it's also a metric only roko knows how to compute.

For the demo, present it honestly: "roko's C-factor reflects gate-verified
work; other frameworks don't gate, so their score is bounded above by
this missing component." That's a true and persuasive statement.

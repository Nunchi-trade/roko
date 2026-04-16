# TUI Parity Batches

8 batches to bring Roko TUI to Mori parity.

## Dependency graph

```
T7 (cleanup)          — independent
T8 (effects)          — independent
T1 (StateHub)         — independent (foundation)
T2 (segments)         — independent
  ├── T5 (pool+wave)  — depends on T1
  ├── T3 (approval)   — depends on T1
  │   └── T4 (procs)  — depends on T1, T3
  └── T6 (metrics)    — depends on T1, T2
```

## Serial execution order

Respects dependency DAG, interleaves independent batches:

```
T1 → T7 → T2 → T5 → T3 → T6 → T4 → T8
```

## Batch manifest

| Batch | Title | Group | Deps | ~LOC | Write scope |
|-------|-------|-------|------|------|-------------|
| T1 | StateHub subscription (replace polling with streaming) | streaming | — | ~350 | app.rs, state.rs, main.rs |
| T2 | Agent output segment parsing | display | — | ~500 | segment.rs (new), agents_view.rs, state.rs, mod.rs |
| T3 | Approval flow IPC | streaming | T1 | ~350 | approval_ipc.rs (new), app.rs, input.rs, orchestrate.rs, main.rs |
| T4 | Process supervision display | streaming | T1, T3 | ~300 | state.rs, app.rs, dashboard_view.rs, roko-runtime/process.rs |
| T5 | Parallel pool + wave ribbon | pool | T1 | ~350 | parallel_pool.rs, state.rs, dashboard_view.rs, input.rs |
| T6 | Context metrics + route display | display | T1, T2 | ~300 | state.rs, agents_view.rs, dashboard_view.rs |
| T7 | Dead field cleanup | cleanup | — | ~-200 | state.rs, app.rs, views/*, widgets/* |
| T8 | Visual effects (NervViz + particles) | effects | — | ~450 | postfx.rs, postfx_pipeline.rs, effects_config.rs, input.rs |

## Conflict groups

- **streaming**: T1, T3, T4 (touch app.rs, state.rs, orchestrate.rs)
- **display**: T2, T6 (touch agents_view.rs, state.rs)
- **pool**: T5 (touches parallel_pool.rs, dashboard_view.rs)
- **cleanup**: T7 (touches state.rs broadly)
- **effects**: T8 (touches postfx files)

## Verification gates

| Batch | Commands |
|-------|----------|
| T1 | `cargo check -p roko-cli && cargo test -p roko-cli --lib` |
| T2 | `cargo check -p roko-cli && cargo test -p roko-cli --lib -- tui::segment` |
| T3 | `cargo check -p roko-cli && cargo test -p roko-cli --lib -- tui::approval_ipc` |
| T4 | `cargo check -p roko-cli -p roko-runtime` |
| T5 | `cargo check -p roko-cli` |
| T6 | `cargo check -p roko-cli` |
| T7 | `cargo check -p roko-cli && cargo clippy -p roko-cli --no-deps -- -D warnings` |
| T8 | `cargo check -p roko-cli` |

# TUI Parity Context Pack: Always Read First

You are running as an unattended Codex batch from `tmp/tui-parity`.

## Core rules

1. Do not assume any prior chat history. This prompt pack must be sufficient.
2. Work only from repository reality plus the files named in the prompt.
3. Must compile with `cargo check -p roko-cli` — zero errors.
4. Must not break existing keyboard shortcuts (F1-F7, Tab, Shift+Tab, etc.).
5. Must work both standalone (polling) and connected (streaming). If the
   `watch::Receiver` is `None`, fall back to the existing disk-polling path.
6. Respect the ROSEDUST theme from `crates/roko-cli/src/tui/widgets/rosedust.rs`.
   Use rose/violet hues (325° base) for all new UI elements.
7. Do not touch files outside your write scope unless a small adjacent fix is
   required to make the batch compile.
8. Use subagents to explore + implement in parallel where beneficial.
9. Run the listed verify commands yourself before declaring success.
10. Keep changes inside the batch write scope. If you need to modify a shared
    file (state.rs, app.rs), only touch the fields/methods relevant to your batch.

## Batch completion bar

A batch is only complete when:

- the listed tasks for that batch are implemented
- the code compiles under the batch verification gate
- new files are wired into the module tree (`mod.rs` exports)
- any new public types are documented with `///` doc comments

## Failure behavior

If a batch is too large, finish the highest-dependency work first and leave a
precise note in the final message about what remains. Do not stop at analysis.

## Key architectural notes

- **TuiState** (`state.rs`) is the single aggregation point for all TUI data.
  Views/widgets read from TuiState, never from raw files.
- **DashboardData** (`dashboard.rs`) loads from disk files on tick.
  `update_from_snapshot()` syncs DashboardData → TuiState.
- **StateHub** (`roko-core/src/state_hub.rs`) is the streaming alternative.
  It publishes `DashboardEvent` → materializes into `DashboardSnapshot` →
  TUI reads via `watch::Receiver` at 60fps with zero-copy borrows.
- **Views are decoupled**: each takes `&DashboardData`, `&TuiState`, `&Theme`.
- **Background I/O**: `sys_rx`, `data_rx`, `git_rx` channels keep I/O off the
  main thread.

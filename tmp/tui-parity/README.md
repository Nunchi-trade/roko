# TUI Mori Parity Runner

Overnight Codex batch runner to bring Roko TUI to Mori parity. Follows the
same pattern as `tmp/ux-refactoring/`.

## Quick start

```bash
# List all batches
bash tmp/tui-parity/run-tui-parity.sh --list

# Dry run (preview prompts, no Codex)
bash tmp/tui-parity/run-tui-parity.sh --dry-run

# Run all batches
bash tmp/tui-parity/run-tui-parity.sh --model gpt-5.4 --reasoning high

# Run specific batches
bash tmp/tui-parity/run-tui-parity.sh --only T1,T2,T7

# Resume a prior run
bash tmp/tui-parity/run-tui-parity.sh --continue last

# Verify only (re-run gates on existing worktree)
bash tmp/tui-parity/run-tui-parity.sh --verify-only --continue last
```

## Architecture

```
tmp/tui-parity/
├── run-tui-parity.sh           # Main runner
├── README.md                    # This file
├── BATCHES.md                   # Batch manifest + dependency graph
├── lib/
│   ├── common.sh                # Batch metadata, deps, verify commands
│   ├── spawn.sh                 # Codex exec wrapper with delegation + retry
│   └── verify.sh                # Compile/test/clippy gate + commit
├── context-pack/
│   ├── 00-TUI-PARITY-RULES.md  # Rules all batches follow
│   ├── 01-TUI-ARCHITECTURE.md  # State flow diagram, file map
│   ├── 02-STATE-HUB-PACK.md    # StateHub + DashboardSnapshot + EventBus
│   ├── 03-TUI-STATE-PACK.md    # TuiState + App struct + main_loop
│   └── 04-MORI-REFERENCE.md    # Key Mori patterns to match
├── prompts/
│   ├── T1.prompt.md             # StateHub subscription
│   ├── T2.prompt.md             # Agent output segment parsing
│   ├── T3.prompt.md             # Approval flow IPC
│   ├── T4.prompt.md             # Process supervision display
│   ├── T5.prompt.md             # Parallel pool + wave ribbon
│   ├── T6.prompt.md             # Context metrics + route display
│   ├── T7.prompt.md             # Dead field cleanup
│   └── T8.prompt.md             # Visual effects (NervViz + particles)
└── logs/                        # Created at runtime
```

## Batches

See [BATCHES.md](BATCHES.md) for the full manifest and dependency graph.

| ID | Title | Deps |
|----|-------|------|
| T1 | StateHub subscription | — |
| T2 | Agent output segment parsing | — |
| T3 | Approval flow IPC | T1 |
| T4 | Process supervision display | T1, T3 |
| T5 | Parallel pool + wave ribbon | T1 |
| T6 | Context metrics + route display | T1, T2 |
| T7 | Dead field cleanup | — |
| T8 | Visual effects (NervViz + particles) | — |

## Execution order

```
T1 → T7 → T2 → T5 → T3 → T6 → T4 → T8
```

T1 is the foundation (streaming). T7, T2, T8 are independent and interleaved
between dependent batches.

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--model` | `gpt-5.4` | Codex model |
| `--reasoning` | `high` | Reasoning effort level |
| `--timeout` | `5400` | Per-batch timeout (seconds) |
| `--retries` | `2` | Max automatic retries per batch |
| `--base-ref` | `HEAD` | Git ref for new worktree |
| `--only` | all | Comma-separated batch IDs |
| `--continue` | — | Resume prior run (ID or 'last') |
| `--dry-run` | — | Preview without running Codex |
| `--force` | — | Re-run successful batches |
| `--verify-only` | — | Re-run verification gates only |
| `--list` | — | Show batch manifest |

## Verification

After all 8 batches complete:

1. `cargo check -p roko-cli` — zero errors
2. `cargo test -p roko-cli --lib` — all tests pass
3. `cargo clippy -p roko-cli --no-deps -- -D warnings` — no warnings
4. `roko dashboard` launches, shows real data
5. F1-F7 tabs render rich content
6. Agent output shows parsed segments (thinking, code, tool_use)
7. Ctrl-E enables PostFX effects
8. `v` key cycles VFX presets

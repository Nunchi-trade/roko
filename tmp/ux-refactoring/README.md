# UX Refactoring Overnight Runner

This directory now contains both the task source material and a Codex-native
overnight harness for implementing it in bounded batches.

## Entry point

```bash
bash tmp/ux-refactoring/run-ux-refactoring.sh --list
bash tmp/ux-refactoring/run-ux-refactoring.sh --dry-run --only A1
bash tmp/ux-refactoring/run-ux-refactoring.sh
```

## Defaults

- Model: `gpt-5.4`
- Reasoning: `high`
- Execution mode: one isolated git worktree, one batch at a time
- Retry policy: bounded automatic retries with previous failure context
- Verification: batch-specific cargo or forge commands after each Codex run

## Why it is single-lane by default

`tmp/tui` is already running in parallel, and most UX refactoring work overlaps
heavily in `roko-cli`, `roko-serve`, and `mirage-rs`. This harness optimizes
for completion and recoverability rather than raw concurrency.

## What is in here

- [`00-INDEX.md`](00-INDEX.md): top-level task list plus overnight batch order
- [`BATCHES.md`](BATCHES.md): canonical batch manifest for the runner
- [`SOURCE-INDEX.md`](SOURCE-INDEX.md): code anchors for each track
- `context-pack/`: shared self-contained context each Codex batch should read
- `prompts/`: one prompt per implementation batch
- `lib/`: shared shell helpers
- `run-ux-refactoring.sh`: the actual overnight runner

## Output and state

Each run creates:

- `tmp/ux-refactoring/logs/<run_id>/manifest.env`
- `tmp/ux-refactoring/logs/<run_id>/<batch>.log`
- `tmp/ux-refactoring/logs/<run_id>/<batch>.result`
- `.roko/worktrees/ux-refactoring-<run_id>/`

Successful batches are auto-committed inside the dedicated UX refactoring
worktree branch so the run can resume cleanly after a crash or timeout.

## Important behavior

- The runner creates its worktree from committed `HEAD`.
- Uncommitted changes in the main repo are intentionally not imported into the
  overnight worktree. This prevents collisions with other active runners.
- On a failed attempt, the runner resets the dedicated worktree back to the last
  successful commit for that branch and retries with the previous failure
  summary appended to the next prompt.

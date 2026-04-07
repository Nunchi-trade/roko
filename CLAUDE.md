# Roko

Roko is a Rust toolkit for building agents. 18 crates + 2 apps, ~177K LOC, 3,039+ passing tests.

## Quick orientation

- **Workspace root**: `/Users/will/dev/nunchi/roko/roko/`
- **Full project context**: `/Users/will/dev/nunchi/roko/CONTEXT.md` — read this for the complete picture (what roko is, where it came from, all crates, all source material)
- **PRD migration plan**: `/Users/will/dev/nunchi/roko/roko/tmp/prd-migration/` — checklist for generating new docs from original PRDs

## Architecture

1 noun (Signal) + 6 verb traits (Substrate, Scorer, Gate, Router, Composer, Policy). Universal loop: query → score → route → compose → act → verify → write → react.

## Key crates

| Crate | Path | What |
|---|---|---|
| roko-core | `/Users/will/dev/nunchi/roko/roko/crates/roko-core/` | Signal + 6 traits, types, config, tools, errors |
| roko-agent | `/Users/will/dev/nunchi/roko/roko/crates/roko-agent/` | 5 LLM backends, pools, MCP, tool loop, safety |
| roko-orchestrator | `/Users/will/dev/nunchi/roko/roko/crates/roko-orchestrator/` | Plan DAG, parallel executor, merge queue, safety |
| roko-gate | `/Users/will/dev/nunchi/roko/roko/crates/roko-gate/` | 11 gates, 6-rung pipeline |
| roko-compose | `/Users/will/dev/nunchi/roko/roko/crates/roko-compose/` | Prompt assembly, 9 templates, enrichment |
| roko-conductor | `/Users/will/dev/nunchi/roko/roko/crates/roko-conductor/` | 10 watchers, circuit breaker |
| roko-learn | `/Users/will/dev/nunchi/roko/roko/crates/roko-learn/` | Episodes, playbooks, bandits, model routing |
| roko-cli | `/Users/will/dev/nunchi/roko/roko/crates/roko-cli/` | CLI binary: REPL, oneshot, pipe, daemon, plan, run, orchestrate, status |
| roko-fs | `/Users/will/dev/nunchi/roko/roko/crates/roko-fs/` | FileSubstrate (JSONL), GC, layout (with state/ for sessions) |
| roko-std | `/Users/will/dev/nunchi/roko/roko/crates/roko-std/` | Defaults, 19 builtin tools, mock dispatcher |
| roko-chain | `/Users/will/dev/nunchi/roko/roko/crates/roko-chain/` | Chain client/wallet traits |
| roko-index | `/Users/will/dev/nunchi/roko/roko/crates/roko-index/` | Code intelligence, symbol graph, PageRank |

## Known issues (resolved)

1. ~~**1 failing test**~~ — FIXED: test now passes 3 observations to meet the `decide()` threshold
2. ~~**Safety unwired**~~ — FIXED: `SafetyLayer` in `roko-agent/src/safety/mod.rs` aggregates all 6 policies; `ToolDispatcher.with_safety()` wires pre-execution checks + post-execution scrubbing
3. ~~**No end-to-end orchestration loop**~~ — FIXED: `roko-cli/src/orchestrate.rs` connects CLI → orchestrator → agent → gates; `roko plan run <dir>` executes plans end-to-end
4. ~~**No session persistence**~~ — FIXED: auto-save of executor + event log snapshots to `.roko/state/`; `--resume` flag loads from snapshots; `roko-fs` layout now includes `state/` directory

## Building

```bash
cd /Users/will/dev/nunchi/roko/roko
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --no-deps -- -D warnings
```

## Reference material (read-only, do not modify)

- Original apps roko replaces: `/Users/will/dev/uniswap/bardo/apps/` (especially `apps/mori/` — 108K LOC orchestrator)
- Original crates: `/Users/will/dev/uniswap/bardo/crates/` (36 crates, 137K LOC)
- Implementation plans: `/Users/will/dev/uniswap/bardo/.mori/plans/` (171 plans with TOML task definitions)
- PRD documents: `/Users/will/dev/nunchi/roko/bardo-backup/prd/` (359 files, 26 sections)
- Research docs: `/Users/will/dev/nunchi/roko/bardo-backup/tmp/` (mori-refactor, mori-agents, death, agent-chain)

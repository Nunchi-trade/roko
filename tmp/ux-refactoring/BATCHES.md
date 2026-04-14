# UX Refactoring Batch Manifest

This is the execution contract for `run-ux-refactoring.sh`. Each batch is
deliberately smaller than the raw section docs so Codex can complete it without
hidden chat history.

| Batch | Tasks | Purpose | Primary write scope | Verify focus |
|------|-------|---------|---------------------|--------------|
| `A1` | `A.01-A.05` | Agent owner/skills/stats/artifacts/messaging backbone | `apps/mirage-rs`, `crates/roko-serve` | mirage + serve compile and route tests |
| `A2` | `A.06-A.10` | ISFR proxy, predictions, chat CLI, research intent, task feedback | `apps/mirage-rs`, `crates/roko-serve`, `crates/roko-cli` | mirage + serve + cli compile |
| `B1` | `B.01-B.06` | Demo ABI/contracts/providers/events/yield-routing spine | `crates/roko-demo`, `contracts`, `demo/` | `roko-demo` build + forge tests |
| `B2` | `B.07-B.18` | Demo integration, benchmarking, autonomy, persistence, TUI mode | `crates/roko-demo`, `contracts`, `demo/` | `roko-demo` scenario smoke runs |
| `C1` | `C.01-C.05` | New `roko-agent-server` crate, feature gating, aggregator, auth, card wiring | `crates/roko-agent-server`, `crates/roko-serve`, `apps/mirage-rs` | new crate builds and aggregator tests |
| `C2` | `C.06-C.08` | mirage cleanup, dashboard migration, WS multiplexer | `apps/mirage-rs`, `crates/roko-serve`, docs | minimal mirage build + serve build |
| `D1` | `D.02-D.17` | Core attestation, lineage, tiering, daimon, routing, provider/runtime gaps | `crates/roko-core`, `roko-chain`, `roko-neuro`, `roko-daimon`, `roko-agent`, `roko-cli` | focused unit slices by keyword |
| `E1` | `E.01-E.08` | Health/cost/latency/skills/experiments feedback into routing and prompts | `crates/roko-learn`, `roko-conductor`, `roko-compose`, `roko-orchestrator`, `roko-cli` | router, conductor, replanning, experiment tests |
| `D2` | `D.18-D.33` | DAG optimization, mutation, composition, supervision, security, dream mid-layer | `crates/roko-orchestrator`, `roko-agent`, `roko-runtime`, `roko-dreams` | orchestrator and dreams keyword tests |
| `D3` | `D.34-D.54` | Long-horizon learning, heartbeat, daemon/WASM, pheromone/active inference | `crates/roko-neuro`, `roko-learn`, `roko-daimon`, `roko-dreams`, `roko-compose`, `roko-cli` | learning, heartbeat, safety, compose tests |
| `F1` | `F.01-F.06` | Interactive TUI plus missing serve endpoints | `crates/roko-cli/src/tui`, `crates/roko-serve` | `roko-cli` TUI build + status routes |
| `F2` | `F.07-F.12` | Tracing, cost aggregation, daemon CLI, code MCP, promote hook, playbooks | `crates/roko-cli`, `crates/roko-mcp-*`, `crates/roko-learn` | `roko-cli`, MCP, playbook tests |

## Dependencies

| Batch | Depends on |
|------|------------|
| `A1` | none |
| `A2` | `A1` |
| `B1` | none |
| `B2` | `B1` |
| `C1` | none |
| `C2` | `C1` |
| `D1` | none |
| `E1` | `D1` |
| `D2` | `D1`, `E1` |
| `D3` | `D2` |
| `F1` | none |
| `F2` | `F1` |

## Conflict groups

- `backend`: `A1`, `A2`, `C1`, `C2`
- `demo`: `B1`, `B2`
- `cli`: `D1`, `E1`, `D2`, `D3`, `F1`, `F2`

Those groups are here to make future multi-worktree scheduling possible. The
current runner stays single-lane for reliability.

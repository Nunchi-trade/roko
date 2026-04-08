# mirage-rs: real chain fork, task stats, cognitive traces, and dashboard polish

## Summary

Wire the dashboard to real data — fork block/URL from upstream, task completion stats on agents, cognitive trace recording/display, and visual improvements across the board.

- **Real fork info**: `MirageStatus` now reports `forkBlock` and `forkUrl` from the upstream RPC at fork time; dashboard shows `FORK: 24,837,959` instead of `FORK: ?`
- **Task stats flow**: `complete_task`/`fail_task` HTTP handlers increment `tasks_completed`/`tasks_failed` on `AgentStats`; dashboard columns read the correct fields
- **Cognitive traces**: new `POST /api/agents/{id}/trace` endpoint; agent simulation posts 4-phase traces (Retrieve → Reason → Act → Verify) per cycle; dashboard has expandable trace rows per agent
- **Cache-busting**: `/dashboard` static files served with `Cache-Control: no-cache, must-revalidate`
- **Pheromones slowed**: reduced forces (centroid attraction 0.0008→0.0003, repulsion 30→12, damping 0.92→0.78) for ambient drift instead of frantic motion
- **Insight graph enlarged**: container 460px→700px, repulsion 4x, node cap 200→400, ideal edge distances increased, search fetches 60 hits
- **Topology spread**: repulsion 6x (3000→18000), ideal spring 120→200, canvas 400→600px, weaker center gravity
- **Task automation**: watchers create tasks 50% (was 20%), validators claim 70% (was 30%), security agents also claim tasks

## Files changed (16)

### Rust backend (9 files)

| File | Change |
|---|---|
| `src/chain/agent.rs` | Add `tasks_completed`, `tasks_failed` to `AgentStats` + `add_stats_delta` |
| `src/fork.rs` | Add `fork_block`, `fork_url` to `ForkState` and `MirageStatus`; `mine_block` advances timestamp |
| `src/provider.rs` | Add `http_url()` accessor on `UpstreamRpc` |
| `src/http_api/task.rs` | Wire `complete_task` → `tasks_completed`, `fail_task` → `tasks_failed` on agent stats |
| `src/http_api/agent.rs` | New `POST /api/agents/{id}/trace` handler with `AgentEvent::Trace` broadcast |
| `src/http_api/mod.rs` | Register `.post(post_agent_trace)` on trace route |
| `src/http_api/knowledge.rs` | Existing — unchanged except unstaged from prior work |
| `src/rpc.rs` | Cache-control middleware for `/dashboard`; fix pre-existing test assertions (`"agents"`→`"items"`) |
| `examples/agent_simulation.rs` | Post 4-phase cognitive traces per cycle; increase task creation/completion rates |

### Frontend (7 files)

| File | Change |
|---|---|
| `static/index.html` | Add "Traces" column to agent registry (9 cols); update styling |
| `static/js/polling.js` | Fix `tasks_completed`/`tasks_failed` columns; add `toggleTraceRow`/`fetchAndRenderTraces`; increase search/entry limits |
| `static/js/main.js` | Fix fork chip to read camelCase `forkBlock`/`forkUrl` |
| `static/js/pheromones.js` | Reduce all force constants for slower ambient drift |
| `static/js/graph.js` | 4x repulsion, increased ideal distances, node cap 400 |
| `static/js/topology.js` | 6x repulsion, ideal spring 200, weaker center gravity |
| `static/style.css` | Insight graph 700px, topology 600px, detail panel 700px |

## Test plan

- [x] `cargo check -p mirage-rs --features chain,roko` passes
- [x] `cargo test -p mirage-rs --features chain,roko` — 347 tests pass (including previously-failing `agent_http_endpoints_via_full_server`)
- [x] Start server with `--rpc-url https://eth.llamarpc.com --block-interval-ms 50`
- [x] `mirage_status` returns real `forkBlock` (24837959) and `forkUrl`
- [x] Dashboard shows `FORK: 24,837,959` with green chip
- [x] Agent simulation registers 20 agents, posts traces, creates/completes tasks
- [x] Agent registry shows non-zero Tasks OK / Tasks Fail
- [x] Click VIEW on agent row — traces expand showing Retrieve→Reason→Act→Verify
- [x] Pheromone particles drift slowly instead of jittering rapidly
- [x] Insight graph fills larger container with spread-out nodes
- [x] Agent topology has readable spacing between nodes
- [x] Hard refresh not needed — cache-busting headers prevent stale files

# UX Refactoring Context Pack: Repo Map

## Core runtime crates

- `crates/roko-cli`: primary CLI, orchestration entrypoints, TUI, daemon hooks
- `crates/roko-orchestrator`: DAGs, execution, replanning
- `crates/roko-agent`: agent adapters, tool loop, dispatcher, safety
- `crates/roko-compose`: prompt/system composition
- `crates/roko-learn`: routing, experiments, provider health, costs, playbooks
- `crates/roko-neuro`: knowledge store, context assembly, tiering
- `crates/roko-daimon`: affect, somatics, motivation signals
- `crates/roko-dreams`: sleep/replay/dream loops
- `crates/roko-conductor`: interventions and routing pressure

## HTTP / substrate

- `apps/mirage-rs`: EVM fork simulator plus accumulated dashboard REST state
- `crates/roko-serve`: HTTP server for runs, plans, providers, research, status
- `crates/roko-chain`: chain reads/writes and wallet abstractions

## Demo and interface surfaces

- `crates/roko-demo`: contracts + scenarios + demo runners
- `contracts/`: Solidity sources and Forge tests
- `demo/`: manifests, scenario fixtures, prompt assets
- `crates/roko-mcp-*`: existing MCP server patterns

## Planned new surfaces

- `crates/roko-agent-server`: not yet present; expected in C batches
- `crates/roko-mcp-code`: not yet present; may be added in F2

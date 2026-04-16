# UX Refactoring Source Index

This file freezes the main local references the overnight prompts depend on.

## Global

- `Cargo.toml`
- `docs/STATUS.md`
- `tmp/ux-refactoring/*.md`

## A track: dashboard backend

- `apps/mirage-rs/src/chain/agent.rs`
- `apps/mirage-rs/src/chain/task.rs`
- `apps/mirage-rs/src/http_api/mod.rs`
- `apps/mirage-rs/src/http_api/agent.rs`
- `apps/mirage-rs/src/http_api/task.rs`
- `crates/roko-serve/src/routes/agents.rs`
- `crates/roko-serve/src/routes/research.rs`
- `crates/roko-serve/src/routes/learning.rs`
- `crates/roko-serve/src/events.rs`
- `tmp/sdb-spec/`

## B track: demo

- `crates/roko-demo/src/main.rs`
- `crates/roko-demo/src/lib.rs`
- `crates/roko-demo/src/bindings.rs`
- `crates/roko-demo/src/deploy.rs`
- `crates/roko-demo/src/verify.rs`
- `crates/roko-demo/src/scenarios/`
- `contracts/src/`
- `contracts/test/`
- `demo/manifest.toml`
- `demo/scenarios/`
- `demo/prompts/`
- `tmp/demo/DEMO-IMPLEMENTATION-PLAN.md`
- `tmp/demo/tasks/ERRATA.md`

## C track: architecture migration

- `apps/mirage-rs/Cargo.toml`
- `apps/mirage-rs/src/http_api/`
- `apps/mirage-rs/src/chain/`
- `crates/roko-serve/src/routes/mod.rs`
- `crates/roko-serve/src/routes/agents.rs`
- `crates/roko-serve/src/state.rs`
- `tmp/ux/00-architecture-overview.md`
- `tmp/ux/01-agent-server-design.md`
- `tmp/ux/02-mirage-extraction.md`
- `tmp/ux/03-auth-and-discovery.md`
- `tmp/ux/04-dashboard-migration.md`
- `tmp/ux/05-build-phases.md`
- `tmp/ux/06-open-questions.md`

## D/E track: runtime gaps and feedback loops

- `crates/roko-cli/src/orchestrate.rs`
- `crates/roko-orchestrator/src/dag.rs`
- `crates/roko-orchestrator/src/executor/mod.rs`
- `crates/roko-orchestrator/src/replan.rs`
- `crates/roko-core/src/attestation.rs`
- `crates/roko-core/src/engram.rs`
- `crates/roko-agent/src/dispatcher/`
- `crates/roko-agent/src/tool_loop/`
- `crates/roko-agent/src/gemini/`
- `crates/roko-neuro/src/`
- `crates/roko-learn/src/`
- `crates/roko-daimon/src/lib.rs`
- `crates/roko-dreams/src/`
- `crates/roko-compose/src/system_prompt_builder.rs`
- `crates/roko-conductor/src/conductor.rs`
- `tmp/integrate-prds/08-DEEP-ARCHITECTURAL-GAPS.md`
- `tmp/integrate-prds/09-REFACTORING-PRD-ADDITIONS.md`
- `tmp/integrate-prds/06-BUILD-SEQUENCE.md`

## F track: interfaces and TUI

- `crates/roko-cli/src/main.rs`
- `crates/roko-cli/src/tui/`
- `crates/roko-cli/src/daemon.rs`
- `crates/roko-cli/src/daemon/`
- `crates/roko-serve/src/routes/status.rs`
- `crates/roko-serve/src/routes/learning.rs`
- `crates/roko-serve/src/routes/prds.rs`
- `crates/roko-mcp-stdio/`
- `crates/roko-mcp-github/`
- `crates/roko-mcp-slack/`
- `crates/roko-mcp-scripts/`

## Known reality constraints

- `crates/roko-agent-server/` does not exist yet. Batches `C1` and `C2` are
  expected to create it and add it to the workspace.
- `crates/roko-mcp-code/` does not exist yet. `F2` may either create it or
  extend an existing MCP crate if that is the cleaner fit after inspection.
- The overnight runner should treat the current repository state as the source
  of truth when the docs drift. If implementation reality differs from a task
  doc, follow the code and update the relevant docs in the same batch.

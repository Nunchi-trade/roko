---
plan: W01-wire-system-prompts
depends_on: []
parallel_with: []
crates_touched: [roko-cli, roko-compose, roko-agent]
estimated_tasks: 4
estimated_parallel_width: 1
estimated_minutes: 30
---

# W01: Wire SystemPromptBuilder into orchestrate.rs

## Context

`orchestrate.rs` has inline `role_system_prompt()` returning 1-sentence strings.
Meanwhile `roko-compose` has a fully-tested 6-layer `SystemPromptBuilder` and 9
role templates that are never called. This plan wires them together.

## References

- Current inline prompts: `crates/roko-cli/src/orchestrate.rs:660-720`
- SystemPromptBuilder: `crates/roko-compose/src/system_prompt_builder.rs`
- Templates: `crates/roko-compose/src/templates/`
- Mori reference: `/Users/will/dev/uniswap/bardo/apps/mori/src/agent/connection.rs:427-500`

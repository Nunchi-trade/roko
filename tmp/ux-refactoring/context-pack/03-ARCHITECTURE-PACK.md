# UX Refactoring Context Pack: Architecture Pack

This pack condenses the UX architecture docs for C/F work.

## Intended architecture

- `mirage-rs` should return to being an EVM fork simulator plus JSON-RPC.
- Agent-specific runtime state should move into a new `roko-agent-server` crate.
- `roko-serve` should host an aggregator surface that preserves the dashboard's
  existing `/api/*` response shapes while sourcing data from agent servers,
  chain reads, and serve state.
- The dashboard should eventually switch one base URL rather than being
  rewritten around a new schema.

## Migration phases

1. Additive phase: backend endpoints continue to work, nothing breaks.
2. Extraction phase: agent-server crate exists, mirage REST is feature-gated,
   aggregator proxies the old shapes.
3. Cleanup phase: old mirage REST surfaces can be removed or reduced to core
   EVM-only endpoints.

## Files that define the architecture

- `tmp/ux/00-architecture-overview.md`
- `tmp/ux/01-agent-server-design.md`
- `tmp/ux/02-mirage-extraction.md`
- `tmp/ux/03-auth-and-discovery.md`
- `tmp/ux/04-dashboard-migration.md`
- `tmp/ux/05-build-phases.md`
- `tmp/ux/06-open-questions.md`

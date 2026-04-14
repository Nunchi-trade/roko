# UX Refactoring Context Pack: TUI and Interfaces Pack

This pack condenses the TUI/interface work for F batches.

## Primary source docs

- `tmp/ux-refactoring/F-tui-interfaces.md`
- `tmp/integrate-prds/06-BUILD-SEQUENCE.md`
- `docs/07-interfaces/`

## Current reality

- `crates/roko-cli/src/tui/` already contains a large scaffold of views,
  widgets, modals, and page modules.
- `dashboard` command behavior is not yet fully interactive.
- Some serve endpoints already exist; F.06 is about filling gaps and matching
  interface expectations, not inventing a second API.
- `F.10` can either add a new `roko-mcp-code` crate or extend an existing MCP
  crate if that yields a cleaner code-intelligence server.

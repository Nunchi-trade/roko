# 2026-04-26 Verified Implementation Archive

These docs were moved here after the implementation pass produced code, tests, and no-mock runtime proof for their completed slices.

- `05-PROMPT-ASSEMBLY.md`: composition-backed runner and `roko run` prompts, bounded gate feedback.
- `06-OBSERVABILITY.md`: persisted runner artifact queryability and HTTP projection/status endpoints.
- `14-FAILURE-RETRY.md`: structured gate failure kind and retry policy metadata.
- `15-SAFETY-EXTENSIONS.md`: bundled non-permissive safety contracts for additional roles.

Not all original scope in these files is globally complete. Remaining cross-cutting work is still tracked in the active `tmp/mori-diffs` docs, especially provider abstraction, live TUI polish, feedback writeback, extensions, and plugin execution.

For the current handoff list of everything still open across the whole package, start here:

- [../../29-CURRENT-RUNTIME-GAP-LEDGER.md](../../29-CURRENT-RUNTIME-GAP-LEDGER.md)
- [../../23-HANDOFF-OPEN-ITEMS.md](../../23-HANDOFF-OPEN-ITEMS.md)

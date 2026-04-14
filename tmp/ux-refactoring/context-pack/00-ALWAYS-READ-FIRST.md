# UX Refactoring Context Pack: Always Read First

You are running as an unattended Codex batch from `tmp/ux-refactoring`.

## Core rules

1. Do not assume any prior chat history. This prompt pack must be sufficient.
2. Work only from repository reality plus the files named in the prompt.
3. Prefer shipping code, tests, and docs together when a task changes behavior.
4. If the docs and code disagree, trust the codebase after inspection and then
   update the relevant docs to match.
5. Do not touch `tmp/tui/` or assume anything from the concurrently running TUI
   harness.
6. Keep changes inside the batch write scope unless a small adjacent fix is
   required to make the batch compile.
7. Run the listed verify commands yourself before declaring success.

## Batch completion bar

A batch is only complete when:

- the listed tasks for that batch are implemented or consciously reconciled
- the code compiles under the batch verification gate
- new files are wired into the workspace/module tree as needed
- any new public CLI/API behavior is documented in the touched task docs

## Failure behavior

If a batch is too large, finish the highest-dependency work first and leave a
precise note in the final message about what remains. Do not stop at analysis.

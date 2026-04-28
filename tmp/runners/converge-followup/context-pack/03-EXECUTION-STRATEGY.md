# Execution Strategy

## Use Maximum Parallelism

You have access to subagents. **Use them aggressively** to complete this batch as fast as possible.

### How to parallelize

1. **Read all scope files and also_read files simultaneously** at the start — don't read them one at a time.
2. **If the batch modifies multiple files**, plan all changes first, then apply them in parallel using subagents — one per file or logical change group.
3. **Run verification commands in parallel** (e.g., `cargo check -p crate1` and `cargo check -p crate2` at the same time).
4. **Don't wait for one edit to complete before starting the next** if they're in different files.

### Speed guidelines

- Do NOT run `cargo test`, `cargo clippy`, or full `cargo check --workspace` — only run per-crate `cargo check -p <crate>` if you need to verify your changes compile.
- Do NOT write tests unless the batch prompt explicitly asks for tests.
- Do NOT add doc comments, type annotations, or refactor surrounding code.
- Make the minimal changes described in the batch prompt. Nothing more.
- Prefer `cargo check -p <specific-crate>` over `cargo check --workspace` — it's 10x faster.

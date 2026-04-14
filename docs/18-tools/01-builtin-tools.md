# 01 — Built-in Tools (roko-std)

> The 16 built-in tools shipped with `roko-std` — the domain-agnostic tool set available to
> every agent regardless of domain configuration.


> **Implementation**: Shipping

---

## Overview

`roko-std` ships 16 built-in tools that form the **universal tool set** available to all
agents. These tools are domain-agnostic — they handle file I/O, search, shell execution, web
access, notebook editing, task management, and planning. Domain-specific tools (chain, research,
ops) are loaded separately via domain plugins.

The built-in tools are defined in `crates/roko-std/src/tool/builtin/mod.rs` and registered
via `LazyLock` initialization. They are accessed through the `StaticToolRegistry` which
implements role-based filtering.

**Crate location:** `crates/roko-std/src/tool/`

**Current tool count:** 16 (confirmed by `TOOL_COUNT` constant and registry tests)

---

## Tool Registry

### StaticToolRegistry

The `StaticToolRegistry` is a zero-sized struct that implements the `ToolRegistry` trait. It
provides three operations:

```rust
pub struct StaticToolRegistry;

impl ToolRegistry for StaticToolRegistry {
    /// Get a tool by name.
    fn get(&self, name: &str) -> Option<&ToolDef>;

    /// Get all registered tools.
    fn all(&self) -> &[ToolDef];

    /// Validate arguments against a tool's schema.
    fn validate_args(&self, name: &str, args: &serde_json::Value) -> Result<()>;
}
```

### Role-Based Filtering

The `for_role()` method filters tools based on agent role:

```rust
impl StaticToolRegistry {
    /// Return tools available for a given role.
    pub fn for_role(&self, role: &str) -> Vec<&ToolDef> { /* ... */ }
}
```

Role-based access control:

| Role | Available Tools | Rationale |
|---|---|---|
| **Implementer** | read + write + exec tools | Full access for code writing |
| **Reviewer** | read-only tools (no write, no exec) | Can inspect but not modify |
| **Researcher** | read + web tools | Can search and read, no write |
| **Architect** | read + search tools | Can inspect and plan |
| **Scribe** | read + write tools | Can read and write docs |
| **Auditor** | read-only tools | Strictest: read-only inspection |

Tests in `registry.rs` confirm 16 tools registered and verify role-based filtering produces
correct subsets (e.g., Implementer gets read+write+exec, Auditor gets read-only).

---

## The 16 Built-in Tools

### 1. `read_file`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Read |
| **Description** | Read the contents of a file at the given path |

Reads a file from the local filesystem and returns its contents. Supports offset and limit
parameters for reading specific line ranges — necessary for large files that exceed context
window limits. Returns file contents with line numbers in `cat -n` format.

Used in the PERCEIVE step of the cognitive loop when the agent needs to understand existing
code or configuration.

### 2. `write_file`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Write |
| **Description** | Write content to a file, creating it if necessary |

Creates or overwrites a file at the specified path with the provided content. The agent must
have read the file first (if it exists) before writing — this is enforced to prevent blind
overwrites.

Used in the ACT step when the agent produces new code, configuration, or documentation.

### 3. `edit_file`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Write |
| **Description** | Replace exact string matches in a file |

Performs exact string replacement in files. The `old_string` must be unique in the file
(or `replace_all` must be set). Preserves indentation and surrounding context. Preferred over
`write_file` for targeted modifications because it sends only the diff — smaller token cost
and clearer intent.

### 4. `multi_edit`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Write |
| **Description** | Apply multiple edits to one or more files in a single operation |

Batched version of `edit_file` that applies multiple string replacements across one or more
files atomically. Reduces round-trips when an agent needs to make coordinated changes across
several locations.

### 5. `glob`

| Property | Value |
|---|---|
| **Category** | Search |
| **Trust Tier** | Read |
| **Description** | Find files matching glob patterns |

Fast file pattern matching. Supports patterns like `**/*.rs` or `src/**/*.ts`. Returns
matching file paths sorted by modification time. Used during PERCEIVE to discover relevant
files before reading them.

### 6. `grep`

| Property | Value |
|---|---|
| **Category** | Search |
| **Trust Tier** | Read |
| **Description** | Search file contents with regular expressions |

Content search built on ripgrep. Supports full regex syntax, file type filtering, glob
filtering, and multiple output modes (`content`, `files_with_matches`, `count`). The primary
tool for codebase exploration — finding function definitions, usage sites, and patterns.

### 7. `bash`

| Property | Value |
|---|---|
| **Category** | Execution |
| **Trust Tier** | Write |
| **Description** | Execute shell commands and return output |

Executes a bash command and returns stdout/stderr. The working directory persists between
commands, but shell state does not. Used for build commands (`cargo build`, `cargo test`),
git operations, and any system interaction that doesn't have a dedicated tool.

### 8. `ls`

| Property | Value |
|---|---|
| **Category** | Search |
| **Trust Tier** | Read |
| **Description** | List directory contents |

Lists files and directories at a given path. Used for quick directory exploration before
deeper investigation with `glob` or `grep`.

### 9. `web_fetch`

| Property | Value |
|---|---|
| **Category** | Web |
| **Trust Tier** | Read |
| **Description** | Fetch content from a URL |

HTTP GET for web resources. Returns the response body. Used by research agents for gathering
external information, documentation, API responses.

### 10. `web_search`

| Property | Value |
|---|---|
| **Category** | Web |
| **Trust Tier** | Read |
| **Description** | Search the web for information |

Web search via configured search provider. Returns search results with titles, snippets, and
URLs. Primary tool for research agents investigating topics, finding documentation, and
gathering citations.

### 11. `notebook_edit`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Write |
| **Description** | Edit Jupyter notebook cells |

Replaces, inserts, or deletes cells in `.ipynb` files. Supports both code and markdown cell
types. Handles the JSON structure of Jupyter notebooks transparently.

### 12. `todo_write`

| Property | Value |
|---|---|
| **Category** | Planning |
| **Trust Tier** | Write |
| **Description** | Write todo items for task tracking |

Creates and manages todo items for tracking work within a session. Used by agents to
maintain their own task lists during complex multi-step operations.

### 13. `task` (task_agent)

| Property | Value |
|---|---|
| **Category** | Orchestration |
| **Trust Tier** | Write |
| **Description** | Spawn a sub-agent to handle a delegated task |

Spawns a sub-agent with a specific prompt and returns the result. Used for task delegation
within multi-agent orchestration — the parent agent can break work into pieces and delegate
to specialized sub-agents.

### 14. `exit_plan_mode`

| Property | Value |
|---|---|
| **Category** | Planning |
| **Trust Tier** | Write |
| **Description** | Signal completion of plan mode and request user approval |

Used when an agent has finished writing a plan and is ready for user review. Transitions the
agent from plan mode to awaiting approval.

### 15. `apply_patch`

| Property | Value |
|---|---|
| **Category** | File I/O |
| **Trust Tier** | Write |
| **Description** | Apply a unified diff patch to files |

Applies a unified diff patch to one or more files. Used for applying pre-computed changes,
especially useful when the agent has generated a diff and wants to apply it atomically.

### 16. `run_tests`

| Property | Value |
|---|---|
| **Category** | Execution |
| **Trust Tier** | Write |
| **Description** | Run the project's test suite |

Executes the project's test suite and returns results. Invokes the appropriate test runner
based on project type (Cargo for Rust, npm/yarn for TypeScript, etc.). Used in the VERIFY
step of the cognitive loop to validate changes.

---

## Tool Registration and Initialization

Tools are registered via `LazyLock` initialization in `builtin/mod.rs`:

```rust
pub static ROKO_BUILTIN_TOOLS: LazyLock<Vec<ToolDef>> = LazyLock::new(|| {
    vec![
        read_file::TOOL_DEF,
        write_file::TOOL_DEF,
        edit_file::TOOL_DEF,
        multi_edit::TOOL_DEF,
        glob::TOOL_DEF,
        grep::TOOL_DEF,
        bash::TOOL_DEF,
        ls::TOOL_DEF,
        web_fetch::TOOL_DEF,
        web_search::TOOL_DEF,
        notebook_edit::TOOL_DEF,
        todo_write::TOOL_DEF,
        task_agent::TOOL_DEF,
        exit_plan_mode::TOOL_DEF,
        apply_patch::TOOL_DEF,
        run_tests::TOOL_DEF,
    ]
});

pub const TOOL_COUNT: usize = 16;
```

Additionally, a `sandbox` module exists for WASM-sandboxed tool execution (see
`04-safety-hooks.md` for the WASM sandbox architecture), though it is not counted in the
`TOOL_COUNT` array as it is a meta-capability rather than a user-facing tool.

---

## Module Structure

```
crates/roko-std/src/tool/
├── mod.rs              # Module structure, re-exports
├── builtin/
│   └── mod.rs          # 16 tool definitions + LazyLock registration
├── registry.rs         # StaticToolRegistry + role-based filtering
├── handlers.rs         # Handler dispatch
├── expand_pointer.rs   # JSON pointer expansion
└── mock_dispatcher.rs  # MockToolDispatcher for testing
```

Re-exports from `tool/mod.rs`:

- `ROKO_BUILTIN_TOOLS` — the static tool array
- `TOOL_COUNT` — compile-time tool count (16)
- `HandlerRegistry` — handler lookup
- `handler_for` — resolve handler by name
- `MockToolDispatcher` — test double for tool dispatch
- `StaticToolRegistry` — the registry implementation

---

## Tool Categories (Built-in)

The 16 built-in tools fall into five categories:

| Category | Tools | Count |
|---|---|---|
| **File I/O** | read_file, write_file, edit_file, multi_edit, notebook_edit, apply_patch | 6 |
| **Search** | glob, grep, ls | 3 |
| **Execution** | bash, run_tests | 2 |
| **Web** | web_fetch, web_search | 2 |
| **Planning / Orchestration** | todo_write, task, exit_plan_mode | 3 |

These categories are distinct from the 17 chain domain categories (data, trading, LP, vault,
lending, staking, restaking, derivatives, yield, safety, intelligence, memory, identity,
wallet, streaming, testnet, bootstrap). The built-in categories are universal; the chain
categories are domain-specific (see `02-tool-categories.md`).

---

## Relationship to Domain Plugin Tools

The 16 built-in tools are **always available** regardless of domain configuration. Domain
plugins add their own tools on top:

```
Built-in (16 tools)          ← roko-std, always loaded
  + Chain domain plugin      ← 423+ DeFi tools, loaded when domain = "chain"
  + Research domain tools    ← web_search enhancements, citation tracking
  + Ops domain tools         ← event source management, monitoring
  + MCP tools                ← dynamically discovered via MCP servers
  = Agent's full tool set    ← filtered by profile + role
```

The agent's final tool set is the union of built-in tools plus domain tools, filtered by
the agent's profile and role. A coding agent gets the 16 built-in tools filtered by role
(Implementer sees all 16, Reviewer sees read-only subset). A chain agent gets the 16 built-in
tools plus whichever chain tools match its profile.

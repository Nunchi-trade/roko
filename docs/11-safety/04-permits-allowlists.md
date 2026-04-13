# Permits and Allowlists: Tool Permission System

> **Layer**: L1 Framework (tool authorization), L3 Harness (task-level filtering)
>
> **Crate**: `roko-core` (ToolPermission, ToolDef), `roko-agent` (ToolDispatcher, SafetyLayer)
>
> **Synapse traits**: `Gate` (verify permissions), `Router` (select tool based on role capabilities)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [01-capability-tokens.md](01-capability-tokens.md)


> **Implementation**: Shipping

---

## Overview

The Roko tool permission system operates at three levels:

1. **Role-based permissions**: Each agent role (Implementer, Auditor, Researcher, Planner, Reviewer) has a set of permission flags (read, write, exec, git, network) that define what categories of tools it can use.
2. **Tool-level requirements**: Each tool definition declares what permissions it requires. A tool requiring `write: true` cannot be executed by a role that only grants `read: true`.
3. **Task-level filters**: Individual tasks can further restrict which tools are available via allowed and denied tool lists.

These three levels compose: a tool call succeeds only if the role grants the required permissions AND the tool is not on the task's deny list AND (if an allow list exists) the tool is on the allow list.

---

## Role-Based Permission Model

### Permission Flags

```rust
/// What a tool requires to execute.
pub struct ToolPermission {
    pub read: bool,      // Can read files and query state
    pub write: bool,     // Can modify files
    pub exec: bool,      // Can execute commands (bash)
    pub git: bool,       // Can perform git operations
    pub network: bool,   // Can make network requests
}

/// Convenience constructors.
impl ToolPermission {
    pub fn read_only() -> Self {
        Self { read: true, write: false, exec: false, git: false, network: false }
    }
    pub fn writes() -> Self {
        Self { read: true, write: true, exec: false, git: false, network: false }
    }
    pub fn full() -> Self {
        Self { read: true, write: true, exec: true, git: true, network: true }
    }
}
```

### Role Permission Matrix

| Role | read | write | exec | git | network | Rationale |
|------|------|-------|------|-----|---------|-----------|
| Implementer | yes | yes | yes | yes | no | Needs to write code, run builds/tests, commit |
| Auditor | yes | no | yes | no | no | Reviews code, runs analysis tools, never modifies |
| Researcher | yes | no | no | no | yes | Reads code, queries external resources |
| Planner | yes | no | no | no | no | Only reads plans and codebase state |
| Reviewer | yes | no | yes | no | no | Reads code, runs verification tools |

### Satisfaction Check

The `ToolDispatcher` verifies that the role's granted permissions satisfy the tool's requirements:

```rust
impl ToolPermission {
    /// Returns true if `granted` satisfies all of self's requirements.
    pub fn satisfied_by(&self, granted: &ToolPermissions) -> bool {
        (!self.read || granted.read)
            && (!self.write || granted.write)
            && (!self.exec || granted.exec)
            && (!self.git || granted.git)
            && (!self.network || granted.network)
    }
}
```

When a permission check fails, the dispatcher emits an audit signal with phase `permission` and status `denied`, including both the required and granted permission sets for debugging.

---

## Task-Level Tool Filters

Beyond role permissions, individual tasks can specify which tools are available:

### Allowed Tools

When `allowed_tools` is set on the `ToolContext`, only tools in this list can execute. All other tools return `ToolError::PermissionDenied` with a clear error message explaining the allowlist.

```rust
// In ToolContext:
pub allowed_tools: Option<Vec<String>>,
```

Use cases:
- A "read-only audit" task that restricts an Implementer role to only `read_file`, `glob`, `grep`
- A "git-only" task that limits tools to `git_status`, `git_diff`, `git_log`
- A "test-only" task that allows only `bash` (for running tests) and `read_file`

### Denied Tools

When `denied_tools` is set, tools in this list are blocked regardless of role permissions.

```rust
// In ToolContext:
pub denied_tools: Option<Vec<String>>,
```

Use cases:
- Blocking `bash` during a review task to prevent code execution
- Blocking `write_file` during analysis to prevent accidental modifications
- Blocking network tools during offline tasks

### Evaluation Order

The deny list is evaluated before the allow list. A tool on both lists is blocked. The full evaluation order in the dispatcher:

1. **Schema validation**: Check tool call arguments against the registry's JSON schema
2. **Tool existence**: Verify the tool exists in the registry
3. **Deny list check**: If the tool is in `denied_tools`, reject with "blocked because it is listed in denied_tools"
4. **Allow list check**: If `allowed_tools` is non-empty and the tool is not in it, reject with "blocked because it is not listed in allowed_tools"
5. **Permission check**: Verify `def.permission.satisfied_by(&role_perms)`
6. **Safety layer check**: Run SafetyLayer pre-execution checks (if attached)

---

## Tool Registry and Definitions

### ToolDef Structure

Every tool in Roko is registered via a `ToolDef` that declares its name, category, permission requirements, and concurrency policy:

```rust
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub category: ToolCategory,
    pub permission: ToolPermission,
    pub concurrency: ToolConcurrency,
    pub schema: Option<serde_json::Value>,
}
```

### Tool Categories

```rust
pub enum ToolCategory {
    FileRead,      // read_file, glob, grep
    FileWrite,     // write_file, edit_file
    Shell,         // bash
    Git,           // git operations
    Network,       // web_fetch, web_search
    Meta,          // list_tools, show_plan
    Custom,        // user-defined tools
}
```

### Concurrency Policy

Tools declare whether they can run in parallel or must be serialized:

```rust
pub enum ToolConcurrency {
    Parallel,  // Safe to run concurrently (read_file, glob, grep)
    Serial,    // Must run sequentially (bash, write_file, edit_file)
}
```

The `ToolDispatcher::dispatch_batch()` groups calls by concurrency policy: `Parallel` tools run via `futures::future::join_all`, while `Serial` tools run sequentially to preserve shell-state ordering and avoid write-write races.

---

## Configuration via roko.toml

Tool permissions and filters can be configured in `roko.toml`:

```toml
[agent]
# Default role for agent dispatch
default_role = "Implementer"

# MCP server configuration (tools from external servers)
mcp_config = ".roko/mcp-config.json"

[safety]
# Additional bash deny patterns
bash_deny_patterns = ["npm run deploy", "cargo publish"]

# Network allowlist (empty = any host)
network_allow_hosts = [".github.com", ".crates.io"]

# Rate limit override
rate_limit_calls = 120
rate_limit_window_secs = 60
```

---

## Integration with the Orchestrator

The `orchestrate.rs` module in `roko-cli` assigns roles and permissions when dispatching agents for plan execution:

1. Each task in a plan has an assigned role (Implementer, Auditor, etc.)
2. The role determines which `ToolPermissions` are granted
3. The task may specify additional tool filters via `allowed_tools` / `denied_tools`
4. The `RoleSystemPromptSpec` in `roko-compose` generates a system prompt that includes the role's capabilities, ensuring the LLM knows what tools are available

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Dennis & Van Horn (1966) | Capability-based security model |
| Saltzer & Schroeder (1975) | Principle of least privilege |
| Anderson (2008) | Security Engineering — comprehensive access control treatment |

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md) — Overall safety architecture
- [01-capability-tokens.md](01-capability-tokens.md) — Compile-time capability enforcement
- [06-sandboxing.md](06-sandboxing.md) — Process-level isolation
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — ToolDispatcher not invoked from CLI

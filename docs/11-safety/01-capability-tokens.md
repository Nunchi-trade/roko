# Capability-Gated Tools: Compile-Time Safety Enforcement

> **Layer**: L1 Framework (tool authorization), L3 Harness (gate verification)
>
> **Crate**: `roko-agent` (current: `ToolPermission` in `roko-core`; target: `Capability<T>`)
>
> **Synapse traits**: `Gate` (verify tool calls against permissions), `Router` (select appropriate capability tier)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md)


> **Implementation**: Specified

---

## The Problem: Runtime Checks Are Fragile

The standard agent safety pattern is: LLM proposes a tool call, a safety hook checks it, blocks if it violates policy. But this is a runtime check. If the safety hook has a bug, if the hook chain is bypassed by an unexpected code path, if a race condition opens a window between the check and the execution — the tool executes anyway.

Dennis & Van Horn (1966) established capability-based security: access rights should be unforgeable tokens verified at the type level, not runtime guards. The WASM Component Model (Haas et al., 2017) instantiates this principle for sandboxed execution. The key insight: if the type system prevents the unsafe code from compiling, no runtime bug can bypass the safety check.

---

## Current Implementation: ToolPermission

The current Roko codebase implements capability-based authorization through the `ToolPermission` system in `roko-core`. Every tool definition (`ToolDef`) declares a required permission level, and every execution context (`ToolContext`) carries a set of granted permissions based on the agent's role.

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

/// What a role grants.
pub struct ToolPermissions {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
    pub git: bool,
    pub network: bool,
}
```

### Authorization Check

The `ToolDispatcher` checks permissions at dispatch time:

```rust
// In ToolDispatcher::dispatch():
let role_perms = ToolPermissions {
    read: ctx.capabilities.read,
    write: ctx.capabilities.write,
    exec: ctx.capabilities.exec,
    git: ctx.capabilities.git,
    network: ctx.capabilities.network,
};
if !def.permission.satisfied_by(&role_perms) {
    return ToolResult::err(ToolError::PermissionDenied(format!(
        "{} requires {:?}, role grants {:?}",
        call.name, def.permission, role_perms
    )));
}
```

### Task-Level Tool Filters

Beyond role-based permissions, the dispatcher supports per-task tool filtering via allowed and denied tool lists:

```rust
// In ToolContext:
pub allowed_tools: Option<Vec<String>>,  // If set, only these tools can execute
pub denied_tools: Option<Vec<String>>,   // If set, these tools are blocked
```

The deny list is evaluated before the allow list — a tool on both lists is blocked. This enables fine-grained control: an Auditor role might have read+exec permissions but a specific task might further restrict it to only `read_file` and `grep`.

---

## Three Tool Tiers (Design Target)

The target design splits tools into three trust tiers, each with different access requirements enforced by the Rust type system:

### Tier 1: Read-Only Tools

No capability token needed. These tools cannot modify any state — they only observe.

Examples in the current codebase: `read_file`, `glob`, `grep`, `list_signals`, `show_plan`.

```rust
/// TIER 1: Read-only tools. No capability needed.
/// Cannot modify any on-chain state, filesystem state, or spend funds.
pub trait ReadTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute_read(&self, params: serde_json::Value) -> Result<serde_json::Value>;
}
```

### Tier 2: Write Tools

**Require a `Capability<Self>` token.** The capability is consumed (ownership transferred) on execution. After one use, the capability no longer exists — Rust's move semantics prevent reuse.

Examples: `write_file`, `edit_file`, `bash` (when modifying state), `git commit`, `git push`.

```rust
/// TIER 2: Write tools. REQUIRE a Capability<Self> token.
/// The capability is CONSUMED (ownership transferred) on execution.
pub trait WriteTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;

    /// The capability parameter takes ownership — it is consumed.
    /// After this call, the capability cannot be used again.
    async fn execute_write(
        &self,
        params: serde_json::Value,
        capability: Capability<Self>,
    ) -> Result<serde_json::Value>
    where
        Self: Sized;
}
```

### Tier 3: Privileged Tools

Require capability **plus** owner approval. These are almost never called autonomously — they require explicit operator steer or multi-sig approval.

Examples: modifying `roko.toml` configuration, changing safety policy parameters, adjusting gate thresholds.

```rust
/// TIER 3: Privileged tools. Require capability + owner approval.
pub trait PrivilegedTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute_privileged(
        &self,
        params: serde_json::Value,
        capability: Capability<Self>,
        owner_approval: OwnerApproval,
    ) -> Result<serde_json::Value>
    where
        Self: Sized;
}
```

---

## The Capability Token

The `Capability<T>` struct is the core of compile-time safety enforcement:

```rust
use std::marker::PhantomData;

/// A capability token. Unforgeable, typed, scoped, single-use.
///
/// Properties (all enforced at compile time):
/// 1. Cannot be created outside the safety crate (pub(crate) constructor)
/// 2. Cannot be used twice (moved on use — Rust's ownership system)
/// 3. Cannot be forged (no Default, no Clone, no Copy)
/// 4. Cannot be used after expiry (checked at execution time)
///
/// The token proves that the safety layer was checked, the risk engine
/// approved, and the permit was created — all before the tool can execute.
pub struct Capability<T> {
    /// Maximum USD value this token authorizes (for chain-domain agents).
    pub value_limit: f64,
    /// Invalid after this tick.
    pub expires_at: u64,
    /// SHA-256 of the policy state at check time.
    pub policy_hash: [u8; 32],
    /// Links to audit trail.
    pub permit_id: String,
    /// Ties the token to a specific tool type.
    _marker: PhantomData<T>,
}

impl<T> Capability<T> {
    /// Constructor is pub(crate) — only the safety crate can create these.
    /// No other code in the system can mint a capability token.
    pub(crate) fn new(
        value_limit: f64,
        expires_at: u64,
        policy_hash: [u8; 32],
        permit_id: String,
    ) -> Self {
        Self {
            value_limit,
            expires_at,
            policy_hash,
            permit_id,
            _marker: PhantomData,
        }
    }

    pub fn is_valid(&self, current_tick: u64) -> bool {
        current_tick <= self.expires_at
    }
}
```

### Why This Is Stronger Than Runtime Checks

The Rust ownership system provides four properties that runtime checks cannot:

1. **Unforgeability**: The `Capability<T>` constructor is `pub(crate)`. No code outside the safety module can create one. There is no `Default`, `Clone`, or `Copy` implementation. The only way to obtain a capability is through the safety layer's approval process.

2. **Single-use**: When a `Capability<T>` is passed to `execute_write()`, Rust's move semantics transfer ownership. The caller no longer has the capability — it has been consumed. Double-spending a capability is a compile error, not a runtime check.

3. **Type-safety**: `Capability<SwapTool>` cannot be used with `execute_write()` on a `DepositTool`. The `PhantomData<T>` marker binds the capability to a specific tool type. Using the wrong capability is a type error.

4. **Temporal validity**: The `expires_at` field provides a runtime check for temporal validity. Even if a capability is somehow held past its intended use window, it becomes invalid after its tick expires.

### Interaction with Speculative Execution

The dual-process cognition system (see the cognitive architecture documentation) can only speculate on Tier 1 (read-only) tools. Speculating on a Tier 2 tool is not "checked at runtime and rejected" — it is **impossible to write the code**, because `execute_write()` requires a `Capability<Self>` parameter that no speculative code path can produce:

```rust
// This compiles — read tools don't need capabilities:
async fn speculate_read(tool: &dyn ReadTool) {
    let _ = tool.execute_read(serde_json::Value::Null).await;
}

// This does NOT compile — there is no way to construct the Capability:
// async fn speculate_write(tool: &dyn WriteTool) {
//     tool.execute_write(serde_json::Value::Null, ???).await;
//     //                                          ^^^ no capability to pass
// }
```

---

## Capability Lifecycle Events

Every capability lifecycle event is emitted as an Engram through the audit sink:

| Event | Description | Engram Kind |
|-------|-------------|-------------|
| `PermitCreated` | A new capability token was minted by the safety layer | `Kind::ToolInvocation` with tag `phase=permit_created` |
| `PermitConsumed` | A capability was consumed by a write tool | `Kind::ToolInvocation` with tag `phase=permit_consumed` |
| `PermitExpired` | A capability expired without being used | `Kind::ToolInvocation` with tag `phase=permit_expired` |
| `PermitDenied` | A capability request was rejected by the safety layer | `Kind::ToolInvocation` with tag `phase=permit_denied` |

These events form part of the content-addressed audit DAG (see [02-audit-chain.md](02-audit-chain.md)), enabling forensic reconstruction of every safety decision.

---

## Implementation Status and Roadmap

### Currently Implemented

The `ToolPermission` + `ToolPermissions` system in `roko-core` provides runtime capability checking:

- Five permission flags (read, write, exec, git, network)
- Per-tool permission requirements in `ToolDef`
- Per-context granted permissions in `ToolContext`
- `satisfied_by()` check in `ToolDispatcher`
- Task-level tool filters (allowed_tools, denied_tools)
- Full audit trail via `AuditSink` signals at every dispatch phase

### Planned (Tier 2)

The compile-time `Capability<T>` system described above is a Tier 2 implementation target:

- `Capability<T>` struct with ownership-based single-use
- Three tool tiers (ReadTool, WriteTool, PrivilegedTool)
- `pub(crate)` constructor enforcement
- Integration with the PolicyCage for chain-domain agents
- Speculative execution restriction to read-only tools

### Implementation Plan Reference

See `implementation-plans/03-safety-hooks.md`:
- **Phase A**: Claude CLI settings hooks (tool filtering via `settings.json`)
- **Phase B**: Wire safety guards into dispatch (current state — implemented)
- **Phase C**: Capability tokens + sandboxing (the `Capability<T>` design)

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Dennis & Van Horn (1966) | Capability-based security — unforgeable tokens verified at type level |
| Haas et al. (2017) | WASM Component Model — sandboxed execution via capabilities |
| Lampson (1974) | Protection — formal model of access control in operating systems |
| Levy (1984) | Capability-Based Computer Systems — comprehensive treatment |
| Miller et al. (2003) | Robust Composition — object-capability patterns for secure composition |
| Watson et al. (2015) | CHERI — hardware-enforced capabilities in modern processors |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Overall safety architecture
- [04-permits-allowlists.md](04-permits-allowlists.md) — Tool permission system details
- [06-sandboxing.md](06-sandboxing.md) — Process-level isolation
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — ToolDispatcher not invoked from CLI pipeline

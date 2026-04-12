# 04 — Safety Hooks & Capability Tokens

> Capability<T> flow, safety hook chain, Revm simulation, WASM sandbox for untrusted tools,
> TaintedString for sensitive data handling.


> **Implementation**: Shipping

---

## Overview

Safety in Roko's tool system operates at two levels:

1. **Compile-time** — The `Capability<T>` token system uses Rust's ownership and move
   semantics to make unauthorized tool execution impossible to express in valid Rust code.
2. **Runtime** — The safety hook chain applies a pipeline of checks (PolicyCage, allowlists,
   spending limits, rate limits, simulation, hallucination detection, output filtering) before
   any write tool executes.

These two levels are complementary. The compile-time system provides the structural guarantee
("this tool CANNOT run without authorization"). The runtime system provides the policy
enforcement ("this tool SHOULD NOT run under these conditions").

---

## Capability<T> Token System

The `Capability<T>` token is the core safety mechanism. It proves that the PolicyCage was
checked, the risk engine approved, and an ActionPermit was created — all before the tool can
execute. Even if every other safety mechanism fails, the tool physically cannot run without
this token.

### Token Definition

```rust
use std::marker::PhantomData;

/// Unforgeable, single-use, scoped capability token.
/// The safety system mints it. The tool handler consumes it.
/// No other path exists.
pub struct Capability<T> {
    pub value_limit: f64,           // Max USD authorized
    pub expires_at: u64,            // Tick expiry
    pub policy_hash: [u8; 32],      // SHA-256 of PolicyCage state at check time
    pub permit_id: String,          // Links to audit trail
    _marker: PhantomData<T>,        // Ties token to specific tool type
}

impl<T> Capability<T> {
    /// Only the safety system can create capability tokens.
    /// No other code in the system can mint one.
    pub(crate) fn new(
        value_limit: f64,
        expires_at: u64,
        policy_hash: [u8; 32],
        permit_id: String,
    ) -> Self {
        Self { value_limit, expires_at, policy_hash, permit_id, _marker: PhantomData }
    }

    pub fn is_valid(&self, current_tick: u64) -> bool {
        self.expires_at > current_tick
    }
}
```

### Properties Enforced at Compile Time

1. **Cannot be created** outside the safety system (`pub(crate)` constructor)
2. **Cannot be reused** (moved on use — Rust's ownership system)
3. **Cannot be forged** (no `Default`, no `Clone`, no `Copy`)
4. **Cannot be used after expiry** (checked at execution time)

### Capability Flow (8 Steps)

```
1. LLM proposes action
       |
       v
2. Safety system: check PolicyCage, check behavioral state, check spending limits
       |
       v
3. Risk engine: assess risk tier, check allowlist, simulate via Revm
       |
       v
4. ActionPermit created (permit_id links to audit chain)
       |
       v
5. Capability<T> minted with value_limit, expiry, policy_hash
       |
       v
6. Tool handler receives Capability<T> — consumed by move semantics
       |
       v
7. Tool executes (the only code path that can reach execution)
       |
       v
8. Audit chain records: PermitCreated, ToolCall, PermitConsumed
```

### Compile-Time Safety Demonstration

The speculative execution engine can only speculate on `ReadTool` types. Speculating on a
`WriteTool` is not "checked at runtime and rejected" — it is **impossible to compile**:

```rust
// This compiles — read tools don't need capabilities:
async fn speculate_read(tool: &dyn ReadTool) {
    tool.execute_read(serde_json::Value::Null, &ctx).await;
}

// This does NOT compile — no way to construct the Capability:
// async fn speculate_write(tool: &dyn WriteTool) {
//     tool.execute_write(serde_json::Value::Null, &ctx, ???).await;
//     //                                               ^^^ No capability to pass
// }
```

---

## Safety Hook Chain

All write operations pass through the safety hook chain before execution. The chain is
implemented as lifecycle hooks that fire in dependency order.

### SafetyHook Trait

```rust
/// Safety hook chain — each hook can approve, reject, or modify the tool call.
pub trait SafetyHook: Send + Sync {
    async fn on_tool_call(
        &self,
        tool: &ToolDef,
        params: &serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<HookDecision>;
}

pub enum HookDecision {
    /// Allow the tool call to proceed.
    Allow,
    /// Allow with modified parameters.
    AllowModified(serde_json::Value),
    /// Reject the tool call with a reason.
    Reject(String),
}
```

### Default Hook Chain Order

1. **PolicyCage** — Behavioral state enforcement. Agents in Struggling state can only unwind.
   Agents in Resting state have limited operations. (Note: the original source framed this in
   terms of mortality phases — Conservation, Declining, Terminal. The new architecture uses
   Daimon behavioral states which are cyclical. The mechanism is identical; the framing changes
   from existential to practical — see `refactoring-prd/08-translation-guide.md`.)

2. **AllowlistGuard** — Token and contract allowlist check. Only pre-approved tokens and
   contracts can be interacted with.

3. **SpendingLimiter** — Per-tick and per-day USD spending limits. Prevents runaway spending
   from a single agent loop iteration or accumulated over a day.

4. **RateLimiter** — Max operations per time window. Prevents rapid-fire transactions that
   could be MEV-exploited or indicate hallucination.

5. **RevmSimulator** — Pre-flight simulation in Revm fork. Executes the transaction in a
   local EVM fork to verify it will succeed and to measure expected outcomes.

6. **HallucinationDetector** — Verify addresses and amounts against known state. Catches
   LLM-hallucinated token addresses, impossible amounts, and other nonsensical parameters.

7. **ResultFilter** — Sanitize output. Strip sensitive data, cap response size, ensure the
   result is safe to include in the LLM context.

Each hook emits events to the event bus for TUI rendering of safety check progress.

### Hook Chain Execution

The hooks execute in order. If any hook returns `Reject`, the chain stops and the tool call is
blocked. If a hook returns `AllowModified`, subsequent hooks see the modified parameters.

```rust
pub async fn run_safety_chain(
    hooks: &[Box<dyn SafetyHook>],
    tool: &ToolDef,
    mut params: serde_json::Value,
    ctx: &ToolContext,
) -> Result<(serde_json::Value, ActionPermit)> {
    for hook in hooks {
        match hook.on_tool_call(tool, &params, ctx).await? {
            HookDecision::Allow => continue,
            HookDecision::AllowModified(new_params) => {
                params = new_params;
                continue;
            }
            HookDecision::Reject(reason) => {
                return Err(SafetyError::Rejected { tool: tool.name, reason }.into());
            }
        }
    }

    // All hooks passed — create ActionPermit and mint Capability<T>
    let permit = ActionPermit::create(tool, &params, ctx)?;
    Ok((params, permit))
}
```

---

## WASM Sandbox for Untrusted Tools

The 423+ native chain domain tools run unsandboxed at full Rust speed — they're part of the
reviewed, compiled codebase. But untrusted tools (user-provided, marketplace-purchased,
third-party MCP tools) run inside a WASM sandbox using Wasmtime.

### Resource Limits

**Fuel metering**: Each WASM instruction consumes "fuel." When fuel runs out, execution halts.
This prevents infinite loops and runaway computation. Default: 10 million fuel units (~100ms
of computation).

**Epoch interruption**: A wall-clock timeout enforced by a background tokio task that
increments the Wasmtime engine epoch. This catches cases where fuel metering alone doesn't
prevent long execution (tight loops consuming little fuel per iteration). Default: 5 seconds.

### Sandbox Implementation

```rust
pub struct WasmSandbox {
    engine: wasmtime::Engine,
    fuel_limit: u64,         // default: 10_000_000
    timeout: Duration,       // default: 5s
    memory_limit: usize,     // default: 256MB
}

impl WasmSandbox {
    pub async fn execute(
        &self,
        wasm_bytes: &[u8],
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let module = wasmtime::Module::new(&self.engine, wasm_bytes)?;
        let mut store = wasmtime::Store::new(&self.engine, ());
        store.set_fuel(self.fuel_limit)?;

        // Epoch-based timeout
        let engine = self.engine.clone();
        let timeout = self.timeout;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            engine.increment_epoch();
        });

        let instance = wasmtime::Instance::new(&mut store, &module, &[])?;
        let execute_fn = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "execute")?;

        // Marshal params → WASM memory, call, unmarshal result
        // ...
        Ok(result)
    }
}
```

### Sandbox Security Properties

- Sandboxed tools receive a **restricted interface — read operations only**
- Any write operation must be returned as a request that the host validates through the normal
  safety hook chain before executing
- **No filesystem access** — cannot read or write files on the host
- **No network access** — cannot make HTTP requests or open sockets
- **No wallet key access** — cannot access the agent's signing keys
- Every sandbox execution emits `WasmToolStart` and `WasmToolComplete` events for audit trail

---

## TaintedString: Sensitive Data Handling

Sensitive data (private keys, API keys, session tokens) is wrapped in `TaintedString`, which
provides automatic zeroization on drop and information flow control:

```rust
pub struct TaintedString {
    value: zeroize::Zeroizing<String>,
    labels: HashSet<TaintLabel>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaintLabel {
    WalletSecret,          // Never leaves process
    OwnerSecret,           // Never to LLM context or mesh
    StrategyConfidential,  // Never to collective knowledge
    UserPII,               // Never to collective without anonymization
    UntrustedExternal,     // Must validate before use
}

impl TaintedString {
    pub fn can_flow_to(&self, sink: DataSink) -> bool {
        match sink {
            DataSink::LlmContext => {
                !self.labels.contains(&TaintLabel::WalletSecret)
                && !self.labels.contains(&TaintLabel::OwnerSecret)
            }
            DataSink::EventBus => {
                !self.labels.contains(&TaintLabel::WalletSecret)
            }
            DataSink::CollectiveMesh => {
                !self.labels.contains(&TaintLabel::StrategyConfidential)
                && !self.labels.contains(&TaintLabel::UserPII)
                && !self.labels.contains(&TaintLabel::WalletSecret)
            }
        }
    }
}
```

### Flow Control Rules

| Taint Label | LLM Context | Event Bus | Collective Mesh | Description |
|---|---|---|---|---|
| `WalletSecret` | BLOCKED | BLOCKED | BLOCKED | Private keys never leave process |
| `OwnerSecret` | BLOCKED | Allowed | BLOCKED | API keys, owner credentials |
| `StrategyConfidential` | Allowed | Allowed | BLOCKED | Proprietary trading strategies |
| `UserPII` | Allowed | Allowed | BLOCKED (unless anonymized) | Personal data |
| `UntrustedExternal` | Allowed (after validation) | Allowed | Allowed (after validation) | External input |

---

## Roko-Agent Safety Layer (Current Implementation)

The current implementation in `roko-agent` (`crates/roko-agent/src/safety/`) provides a
domain-agnostic safety layer integrated into the ToolDispatcher:

- **Role authorization**: Agents can only execute tools permitted by their role
- **Pre-call checks**: Parameter validation, rate limiting, budget enforcement
- **Post-call checks**: Output sanitization, sensitive data filtering

This safety layer is wired into the ToolDispatcher and runs for every tool call regardless of
domain. The chain-domain-specific safety hooks (PolicyCage, AllowlistGuard, RevmSimulator,
etc.) extend this base layer with chain-specific checks.

---

## Audit Trail

Every safety decision produces an audit record:

```rust
pub struct SafetyAuditRecord {
    pub timestamp: i64,
    pub tool_name: String,
    pub hook_name: String,
    pub decision: HookDecision,
    pub params_hash: String,        // Hash of input params (not the params themselves)
    pub permit_id: Option<String>,  // If a permit was created
    pub reason: Option<String>,     // If rejected, why
}
```

Audit records are stored as Engrams (the universal data type — see
`docs/01-synapse-architecture/`) with `Kind::Custom("safety.audit")` and
lineage linking back to the tool call Engram. This creates a complete provenance chain:

```
Tool Call Engram
    → Safety Audit Engram (PermitCreated)
    → Tool Execution Engram
    → Safety Audit Engram (PermitConsumed)
    → Outcome Verification Engram
```

This audit chain supports the Forensic AI innovation (see `docs/09-innovations/`) — every
action can be causally replayed from Engram lineage for regulatory compliance or debugging.

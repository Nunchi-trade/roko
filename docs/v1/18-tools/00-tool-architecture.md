# 00 — Tool Architecture

> Roko's tool system: ToolDef pattern, ToolContext, ToolResult, ToolExecutor, and the
> principles governing how tools compose within the Synapse Architecture.
> See also [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md).


> **Implementation**: Shipping

---

## Overview

Every capability in Roko is mediated by **tools** — typed, described, trust-tiered functions
that an agent can invoke during the ACT step of the universal cognitive loop. Tools are the
bridge between the agent's reasoning (LLM output) and the external world (filesystems, chains,
APIs, memory stores).

The tool architecture is designed around five principles:

1. **Static declaration** — tools are described at compile time via `ToolDef` constants.
2. **Trust-tiered execution** — three tiers (Read, Write, Privileged) enforced by the Rust
   type system, not runtime checks.
3. **Profile-filtered loading** — only relevant tools are registered at boot, based on
   agent configuration.
4. **Role-based access** — the `StaticToolRegistry` filters tools per agent role (Implementer,
   Reviewer, Researcher, Architect, Scribe).
5. **Domain-agnostic core** — the kernel tool types (`ToolDef`, `ToolContext`, `ToolResult`)
   are domain-agnostic. Domain-specific tools (e.g., chain/DeFi tools) are one domain plugin
   among many.

---

## The ToolDef Pattern

Every tool in Roko is a module exporting a `ToolDef` static constant. `ToolDef` is the
canonical declaration format — a compile-time constant that describes the tool's name,
description, trust tier, category, risk classification, and handler function.

### Core ToolDef Struct

The target `ToolDef` struct in `roko-core`:

```rust
/// Static tool definition — compile-time constant per tool.
pub struct ToolDef {
    /// Tool name: `<prefix>_<action>_<subject>` convention.
    /// Examples: "read_file", "uniswap_get_pool_info", "aave_supply_collateral".
    pub name: &'static str,

    /// LLM-facing description: when to call, what it returns, what it does NOT do.
    /// Serves two audiences: the LLM selecting which tool to call, and the LLM
    /// filling in parameters.
    pub description: &'static str,

    /// Semantic category — drives profile filtering (17 categories for chain domain,
    /// plus categories for coding, research, ops domains).
    pub category: Category,

    /// Trust tier: Read | Write | Privileged.
    /// Determines which handler trait the tool implements.
    pub capability: CapabilityTier,

    /// Risk classification: Layer1 (read) | Layer2 (bounded write) | Layer3 (unbounded write).
    pub risk_tier: RiskTier,

    /// Expected execution time: Fast (<1s) | Medium (1-5s) | Slow (5-15s).
    pub tick_budget: TickBudget,

    /// Named execution steps for TUI progress bar rendering.
    pub progress_steps: &'static [&'static str],

    /// Animation state trigger for the Spectre visualization.
    pub sprite_trigger: SpriteTrigger,

    /// Short string (~20-50 tokens) injected near the system prompt.
    /// Always present when the tool is loaded. Cached by prompt caching.
    pub prompt_snippet: &'static str,

    /// Array of usage hints injected with the tool schema.
    /// Phase-conditional guidelines that the LLM reads and self-enforces.
    pub prompt_guidelines: &'static [&'static str],
}
```

### ToolDef Field Semantics

| Field | Type | Purpose |
|---|---|---|
| `name` | `&'static str` | Tool name following `<prefix>_<action>_<subject>` convention |
| `description` | `&'static str` | LLM-facing: when to call, what it returns, what it does NOT do |
| `category` | `Category` | Drives profile filtering (17 categories in chain domain) |
| `capability` | `CapabilityTier` | `Read`, `Write`, or `Privileged` — determines handler trait |
| `risk_tier` | `RiskTier` | `Layer1` (read), `Layer2` (bounded write), `Layer3` (unbounded write) |
| `tick_budget` | `TickBudget` | `Fast` (<1s), `Medium` (1-5s), `Slow` (5-15s) |
| `progress_steps` | `&[&str]` | Named execution steps for TUI progress bar rendering |
| `sprite_trigger` | `SpriteTrigger` | Animation state: `Thinking`, `Executing`, `Success`, `Failure` |
| `prompt_snippet` | `&'static str` | Injected near system prompt, cached. 20-50 tokens. |
| `prompt_guidelines` | `&[&str]` | Phase-conditional usage hints injected with tool schema |

### Example ToolDef: Read Tool (Chain Domain Plugin)

```rust
use roko_tools::{ToolDef, ToolContext, ToolResult, Category, CapabilityTier, RiskTier, TickBudget};
use serde::{Deserialize, Serialize};

/// Input parameters for uniswap_get_pool_info.
#[derive(Debug, Deserialize)]
pub struct GetPoolInfoParams {
    /// Pool contract address (0x...).
    /// Use uniswap_get_pools_by_token_pair to find the address first.
    pub pool_address: String,
    /// Chain ID (default: 1 for Ethereum). Common: 8453 for Base, 42161 for Arbitrum.
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
}

fn default_chain_id() -> u64 { 1 }

/// Pool state returned by uniswap_get_pool_info.
#[derive(Debug, Serialize)]
pub struct PoolInfo {
    pub pool_address: String,
    pub chain_id: u64,
    pub version: String,           // "v3" | "v4"
    pub token0: TokenMeta,
    pub token1: TokenMeta,
    pub fee_tier: u32,
    pub sqrt_price_x96: String,
    pub tick: i32,
    pub liquidity: String,
    pub tvl_usd: f64,
    pub volume_24h_usd: f64,
    pub fee_apy_24h: f64,
}

pub static TOOL_DEF: ToolDef = ToolDef {
    name: "uniswap_get_pool_info",
    description: concat!(
        "Get current state of a Uniswap V3 or V4 pool: price, liquidity, TVL, volume, fees. ",
        "Use when the agent needs pool depth, TVL, current price, or fee APY. ",
        "Returns tick, sqrtPriceX96, liquidity (as token amounts), 24h volume, and fee tier.",
    ),
    category: Category::Data,
    capability: CapabilityTier::Read,
    risk_tier: RiskTier::Layer1,
    tick_budget: TickBudget::Fast,
    progress_steps: &["Fetching slot0", "Loading subgraph data", "Computing APY"],
    sprite_trigger: SpriteTrigger::Thinking,
    prompt_snippet: "Use uniswap_get_pool_info for pool state. \
                     Call uniswap_get_pools_by_token_pair first to get the pool address.",
    prompt_guidelines: &[
        "Prefer this over manual slot0 reads -- it normalizes V3/V4 differences.",
        "Cache results for 15s. Don't call twice in the same tick for the same pool.",
    ],
};

/// Handler implementation.
pub async fn handle(params: GetPoolInfoParams, ctx: &ToolContext) -> Result<ToolResult> {
    ctx.event_bus.emit_tool_start("uniswap_get_pool_info", &params);

    let provider = ctx.provider(params.chain_id)?;

    // Step 1: Read slot0
    ctx.event_bus.emit_tool_update("uniswap_get_pool_info", "Fetching slot0");
    let slot0 = read_slot0(&provider, params.pool_address.parse()?).await?;

    // Step 2: Subgraph data
    ctx.event_bus.emit_tool_update("uniswap_get_pool_info", "Loading subgraph data");
    let subgraph = ctx.subgraph_client.query_pool(params.pool_address.parse()?).await?;

    // Step 3: Compute APY
    ctx.event_bus.emit_tool_update("uniswap_get_pool_info", "Computing APY");
    let apy = compute_fee_apy(&slot0, &subgraph);

    let result = PoolInfo { /* ... */ };
    ctx.event_bus.emit_tool_end("uniswap_get_pool_info", true);

    Ok(ToolResult::read(result))
}
```

---

## Three Trust Tiers

Tools implement one of three traits based on their trust tier. The Rust type system enforces
that write tools cannot execute without a capability token — this is a **compile-time
guarantee**, not a runtime check.

### ReadTool (~60% of chain domain tools)

No capability token required. Cannot modify on-chain state. Examples: check price, read
balance, query pool state, get gas price, read health factor.

```rust
#[async_trait]
pub trait ReadTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute_read(
        &self,
        params: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult>;
}
```

### WriteTool (~35% of chain domain tools)

Require a `Capability<Self>` token that is **consumed (moved)** on use. Rust's ownership
system prevents reuse at compile time. Examples: swap tokens, add liquidity, deposit into
vault, stake ETH.

```rust
#[async_trait]
pub trait WriteTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute_write(
        &self,
        params: serde_json::Value,
        ctx: &ToolContext,
        capability: Capability<Self>,  // Moved (consumed) on use
    ) -> Result<ToolResult>
    where Self: Sized;
}
```

### PrivilegedTool (~5% of chain domain tools)

Require a capability token plus owner approval. Admin operations, strategy changes. Almost
never called autonomously — requires explicit owner steer or multi-sig approval.

```rust
#[async_trait]
pub trait PrivilegedTool: Send + Sync {
    fn id(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute_privileged(
        &self,
        params: serde_json::Value,
        ctx: &ToolContext,
        capability: Capability<Self>,
        owner_approval: OwnerApproval,
    ) -> Result<ToolResult>
    where Self: Sized;
}
```

### Speculative Execution and Trust Tiers

The speculative execution engine can only speculate on `ReadTool` types. Speculating on a
`WriteTool` is not "checked at runtime and rejected" — it is **impossible to write the code**,
because `execute_write` requires a `Capability<Self>` parameter that no speculative code path
can produce:

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

## ToolResult Format

All tools return `ToolResult`, which includes expected/actual fields for ground truth
verification on write tools:

```rust
#[derive(Debug, Serialize)]
pub struct ToolResult {
    /// The tool output data, serialized as JSON.
    pub data: serde_json::Value,
    /// Whether the tool execution failed.
    pub is_error: bool,
    /// Schema version for response format evolution.
    pub schema_version: u32,
    /// For write tools: what the tool expected to happen.
    pub expected_outcome: Option<String>,
    /// For write tools: what actually happened (from receipt/balance check).
    pub actual_outcome: Option<String>,
    /// Ground truth verification source.
    pub ground_truth_source: Option<String>,
}

impl ToolResult {
    /// Convenience for read-only results.
    pub fn read<T: Serialize>(data: T) -> Self {
        Self {
            data: serde_json::to_value(data).unwrap(),
            is_error: false,
            schema_version: 1,
            expected_outcome: None,
            actual_outcome: None,
            ground_truth_source: None,
        }
    }

    /// Convenience for write results with ground truth.
    pub fn write<T: Serialize>(
        data: T,
        expected: impl Into<String>,
        actual: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            data: serde_json::to_value(data).unwrap(),
            is_error: false,
            schema_version: 1,
            expected_outcome: Some(expected.into()),
            actual_outcome: Some(actual.into()),
            ground_truth_source: Some(source.into()),
        }
    }
}
```

The `expected_outcome` and `actual_outcome` fields feed into the Neuro knowledge system
(formerly Grimoire). When expected and actual diverge, the episode is tagged for Dream
replay and heuristic revision.

---

## ToolContext Interface

The runtime injects a `ToolContext` providing access to chain providers, safety pipeline,
event bus, memory, and configuration:

```rust
pub struct ToolContext {
    /// Alloy provider for the specified chain (chain domain plugin).
    pub fn provider(&self, chain_id: u64) -> Result<Arc<dyn Provider>>;
    /// Alloy signer for write operations (chain domain plugin).
    pub fn signer(&self, chain_id: u64) -> Result<Arc<dyn Signer>>;
    /// Revm fork for pre-flight simulation (chain domain plugin).
    pub fn revm_fork(&self, chain_id: u64) -> Result<RevmFork>;
    /// Event bus for TUI/surface rendering.
    pub event_bus: Arc<EventBus>,
    /// Neuro knowledge store (optional, active when learning profile on).
    pub neuro: Option<Arc<NeuroStore>>,
    /// Subgraph client for historical data (chain domain plugin).
    pub subgraph_client: Arc<SubgraphClient>,
    /// Current session config.
    pub config: Arc<ToolConfig>,
    /// Uniswap Trading API client (optional, chain domain plugin).
    pub trading_api: Option<Arc<TradingApiClient>>,
    /// TypeScript sidecar for Uniswap SDK math (chain domain plugin).
    pub sidecar: Arc<SidecarClient>,
}
```

Note that chain-specific fields (`provider`, `signer`, `revm_fork`, `subgraph_client`,
`trading_api`, `sidecar`) are injected by the chain domain plugin. A coding agent or research
agent would have a `ToolContext` with different (or absent) chain fields. The context is
parameterized by domain.

That domain parameterization is the bridge to `TypedContext` from the refinement path:
structured context tells tools which situation they are operating in, and write paths can
emit a `Custody` record to capture who acted, why the action was taken, and what evidence
backed it. In practice, tool docs should describe both the category filter and the structured
context keys the tool expects.

---

## Tool Annotation Semantics

| Annotation | Meaning | Safety Effect |
|---|---|---|
| `CapabilityTier::Read` | No on-chain state modification | Safety skips simulation, no capability needed |
| `CapabilityTier::Write` | Broadcasts transactions | Requires `Capability<Self>`, full simulation |
| `CapabilityTier::Privileged` | Admin/ownership operations | Requires capability + owner approval |
| `RiskTier::Layer1` | Read-only | No ActionPermit |
| `RiskTier::Layer2` | Bounded write (value < limit) | Standard ActionPermit |
| `RiskTier::Layer3` | Unbounded write | Elevated ActionPermit, full simulation |

---

## LLM-Optimized Tool Descriptions

Tool descriptions serve two audiences: the LLM selecting which tool to call, and the LLM
filling in parameters.

**Selection guidance** (the `description` field): answers "when should I call this tool?"

- Starts with the tool's purpose in a single phrase
- Lists specific intents that map to this tool
- States what it does NOT do (disambiguates from similar tools)
- Mentions prerequisites ("get pool address first via uniswap_get_pools_by_token_pair")

**Parameter documentation** (serde `#[serde(rename)]` + doc comments): answers "how do I fill
this in?"

- Format requirements ("0x-prefixed hex address")
- Common values or examples
- Defaults and when to omit

**Anti-patterns** that degrade LLM tool selection accuracy:

- Generic descriptions: "Interact with Uniswap" (does not help selection)
- No parameter docs: missing doc comments cause hallucinated values
- Ambiguous scope: two similar tools with indistinguishable descriptions
- Response payloads exceeding ~25,000 tokens — implement pagination

---

## promptSnippet and promptGuidelines

`ToolDef` includes two fields for zero-cost context engineering:

- **`prompt_snippet`**: Short string injected near the system prompt. Always present when the
  tool is loaded. ~20-50 tokens, cached by the provider's prompt caching system.
- **`prompt_guidelines`**: Array of usage hints injected as part of the tool schema. Also
  cached alongside tool definitions.

These fields replace stuffing tool usage instructions into the system prompt. Each tool carries
its own instructions, present only when the tool is loaded.

### Phase-Conditional Guidelines (Chain Domain Plugin)

Guidelines can reference behavioral states directly. The LLM reads them and self-enforces — no
runtime branching needed:

```rust
pub static COMMIT_ACTION_DEF: ToolDef = ToolDef {
    name: "commit_action",
    // ...
    prompt_snippet: "Executes a previewed action. Requires a valid, unexpired permit ID. \
                     After committing, ALWAYS verify the outcome with query_state.",
    prompt_guidelines: &[
        "NEVER call commit_action without a valid permitId from preview_action.",
        "If more than 3 minutes have passed since the preview, re-preview first.",
        "After committing, call query_state to verify the state change occurred.",
        "If commit fails, DO NOT retry immediately -- check query_state first.",
        // Behavioral state-conditional (LLM reads and self-enforces):
        "In Struggling state: only commit close/unwind actions. New positions will be blocked.",
        "In Resting state: only commit settlement actions. The system will block all other commits.",
    ],
};
```

Note: The original legacy source framed these phase-conditional guidelines in terms of
mortality phases (Conservation → Terminal). The new architecture uses Daimon behavioral states
(Engaged → Struggling → Coasting → Exploring → Focused → Resting) which are cyclical — there
is no terminal destination (see `refactoring-prd/08-translation-guide.md`).

### Token Savings Analysis

| Configuration | Tool tokens per turn | Context savings vs baseline |
|---|---|---|
| All 423+ tools directly exposed (baseline) | ~38,000 | — |
| 8 adapter-facing tools (two-layer model) | ~1,200 | **94% reduction** |
| 8 adapter-facing + 5 skill descriptions (dormant) | ~1,450 | **92% reduction** |
| 8 adapter-facing + 2 skills active (loaded) | ~2,800 | **85% reduction** |

The savings compound across an agent's lifetime. 19,000 tokens saved per turn, roughly 20 T1
turns per day, at $0.001/1K tokens = ~$0.38/day. Over 30 days that's $11.40 — enough to
significantly extend an agent's operating budget.

---

## DecisionCycleRecord Integration

Tools contribute data to the agent's 9-step cognitive loop via the `DecisionCycleRecord`.
Every tool execution produces an `ActionRecord` that becomes part of the tick's permanent
record:

```rust
pub struct ActionRecord {
    pub action_type: String,           // "swap", "rebalance", "deposit"
    pub tool_name: String,             // The ToolDef.name that executed
    pub permit_id: Option<String>,     // Links to capability token
    pub tx_hash: Option<String>,       // On-chain transaction hash
    pub status: ActionStatus,          // Executed, Blocked, Deferred
    pub block_reason: Option<String>,  // Why it was blocked (if blocked)
    pub gas_cost: f64,                 // Gas cost in USD
}

pub struct OutcomeRecord {
    pub verified: bool,
    pub expected: String,              // From ToolResult.expected_outcome
    pub actual: String,                // From ToolResult.actual_outcome
    pub pnl_impact: Option<f64>,       // P&L change from this action
    pub ground_truth_source: String,   // "receipt", "balance_check", "log_comparison"
}
```

The `OutcomeRecord` feeds back into the Neuro knowledge system — if expected and actual
diverge, the episode is tagged for Dream replay and heuristic revision.

---

## Speculation Engine

Read tools support prefetching via co-occurrence patterns. When a tool is called, the
speculation engine checks historical co-occurrence data and prefetches likely-next-read
tools in parallel:

```rust
pub struct SpeculationEngine {
    /// Tool co-occurrence matrix (tool_a, tool_b) -> probability.
    co_occurrence: HashMap<(&'static str, &'static str), f64>,
    /// Minimum probability to trigger prefetch.
    threshold: f64, // default: 0.7
}

impl SpeculationEngine {
    pub fn on_tool_call(&self, tool_name: &str) -> Vec<PrefetchTask> {
        self.co_occurrence
            .iter()
            .filter(|((a, _), prob)| *a == tool_name && **prob >= self.threshold)
            .map(|((_, b), _)| PrefetchTask { tool_name: b })
            .collect()
    }
}
```

Example co-occurrences (chain domain plugin):
- `uniswap_get_pool_info` → `data_get_token_price` (0.85)
- `uniswap_get_quote` → `safety_simulate_transaction` (0.92)
- `aave_get_health_factor` → `data_get_token_price` (0.78)

Prefetched results are cached for the duration of the tick. If the agent doesn't use them,
they're discarded at tick end. This is one component of the 16 T0 Probes innovation
(see `docs/09-innovations/`).

---

## Event Bus Integration

Every tool emits typed events through the event bus for TUI rendering, telemetry, and
surface updates.

### Tool Lifecycle Events

| Event | Payload | When |
|---|---|---|
| `tool:start` | `{ tool_name, params_hash, tick }` | Handler entry |
| `tool:update` | `{ tool_name, step_name, step_index, total_steps }` | Each progress step |
| `tool:end` | `{ tool_name, success, duration_ms, result_summary }` | Handler exit |
| `tool:error` | `{ tool_name, error_code, error_message }` | Handler failure |

### TUI Rendering Contract

The TUI subscribes to `tool:*` events and renders them according to the tool's metadata:

- **`progress_steps`**: Drives a step-by-step progress bar. Each `tool:update` event advances
  the bar.
- **`sprite_trigger`**: Sets the Spectre animation state (`Thinking` for reads, `Executing`
  for writes, `Success`/`Failure` on completion).
- **`tick_budget`**: The TUI uses this to estimate expected duration and show appropriate
  loading states.

### Event Emission Pattern

```rust
pub async fn handle(params: P, ctx: &ToolContext) -> Result<ToolResult> {
    ctx.event_bus.emit(Subsystem::Tools, EventPayload::ToolExecutionStart {
        tool_name: TOOL_DEF.name.into(),
        params_hash: hash_params(&params),
    });

    // Step 1
    ctx.event_bus.emit_tool_update(TOOL_DEF.name, "Fetching data");
    let data = fetch(&ctx.provider(chain_id)?).await?;

    // Step 2
    ctx.event_bus.emit_tool_update(TOOL_DEF.name, "Processing");
    let result = process(data)?;

    ctx.event_bus.emit(Subsystem::Tools, EventPayload::ToolExecutionComplete {
        tool_name: TOOL_DEF.name.into(),
        success: true,
        duration_ms: elapsed.as_millis() as u64,
    });

    Ok(ToolResult::read(result))
}
```

---

## Relationship to the Synapse Architecture

Tools operate at **Layer 1 (Framework)** in the five-layer architecture. They are invoked
during the ACT step (step 5) of the universal cognitive loop:

```
1. PERCEIVE      → Substrate.query()       What is happening?
2. EVALUATE      → Scorer.score()          How relevant/important?
3. ATTEND        → Router.select()         What matters most?
4. INTEGRATE     → Composer.compose()      Build context under budget
5. ACT           → Agent.execute()         ← TOOLS INVOKED HERE
6. VERIFY        → Gate.verify()           Did it work?
7. PERSIST       → Substrate.put()         Store output with lineage
8. ADAPT         → Policy.decide()         What patterns emerged?
9. META-COGNIZE  → Daimon.assess()         Am I doing this well?
```

The VERIFY step (step 6) uses `ToolResult.expected_outcome` vs `ToolResult.actual_outcome` to
produce a `Verdict` — the Gate's ground truth check. This closes the perception-action loop:
the agent acts via tools, verifies via gates, and adapts via policies.

---

## OpenAPI 3.1 Tool Definitions

Roko tool schemas are JSON Schema Draft 2020-12 compliant, aligning with OpenAPI 3.1.
This means any Roko `ToolDef` can be exported as an OpenAPI operation, and any OpenAPI spec
can be imported as Roko tools. The bridge enables interoperability with Claude's function
calling API (`input_schema`), OpenAI's function calling, and the broader API ecosystem.

### Schema Generation from Rust Types

The `schemars` crate derives JSON Schema from Rust parameter structs at compile time,
closing the loop between Rust types and tool parameter schemas:

```rust
use schemars::JsonSchema;
use serde::Deserialize;

/// Input parameters — schemars derives JSON Schema automatically.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// Absolute path to the file to read.
    pub file_path: String,
    /// Line number to start reading from (1-indexed).
    #[schemars(range(min = 1))]
    pub offset: Option<u32>,
    /// Number of lines to read.
    #[schemars(range(min = 1, max = 10000))]
    pub limit: Option<u32>,
}

/// Generate the JSON Schema at compile time.
pub fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ReadFileParams)).unwrap()
}
```

### Claude/OpenAI/MCP Function Calling Compatibility

Roko tool definitions map directly to LLM provider function calling formats:

| Roko Field | Claude API | OpenAI API | MCP |
|---|---|---|---|
| `name` | `name` | `name` | `name` |
| `description` | `description` | `description` | `description` |
| `input_schema` (JSON Schema) | `input_schema` | `parameters` | `inputSchema` |

The conversion is mechanical — no information is lost.

### OpenAPI Import

```rust
/// Convert an OpenAPI 3.1 spec into Roko ToolDefs.
/// Each path+method pair becomes a separate tool.
pub fn import_openapi(spec: &openapiv3::OpenAPI) -> Vec<ToolDef> {
    let mut tools = Vec::new();
    for (path, item) in &spec.paths.paths {
        for (method, operation) in item.iter() {
            let name = operation.operation_id
                .clone()
                .unwrap_or_else(|| format!("{}_{}", method, sanitize_path(path)));
            let input_schema = extract_request_schema(operation);
            tools.push(ToolDef {
                name: Box::leak(name.into_boxed_str()),
                description: Box::leak(
                    operation.summary.clone().unwrap_or_default().into_boxed_str()
                ),
                category: Category::Custom("openapi".into()),
                capability: match method {
                    "get" | "head" | "options" => CapabilityTier::Read,
                    _ => CapabilityTier::Write,
                },
                risk_tier: RiskTier::Layer2,
                // ... remaining fields with defaults
            });
        }
    }
    tools
}
```

---

## Tool Versioning

Tools evolve over time. Parameter schemas change, behaviors shift, new capabilities are added.
Tool versioning prevents breaking changes from silently corrupting agent behavior.

### Semantic Versioning for Tools

```rust
/// Extended ToolDef with versioning support.
pub struct VersionedToolDef {
    pub def: ToolDef,
    /// Semantic version of this tool.
    pub version: semver::Version,
    /// Deprecation notice (if this version is deprecated).
    pub deprecated: Option<DeprecationNotice>,
    /// Changelog entries for this version.
    pub changelog: &'static [&'static str],
}

pub struct DeprecationNotice {
    pub since: &'static str,
    pub replacement: Option<&'static str>,
    pub removal_target: Option<&'static str>,
    pub migration: &'static str,
}
```

### Version Semantics

| Change Type | Version Bump | Examples |
|---|---|---|
| Breaking schema change | **Major** | Remove required param, change param type |
| New optional param | **Minor** | Add `include_metadata: Option<bool>` |
| New output field | **Minor** | Add `created_at` to response |
| Bug fix | **Patch** | Fix edge case, improve error message |

### Version Resolution

```rust
impl VersionedToolRegistry {
    /// Resolve a tool by name and version constraint.
    /// Returns the highest version matching the constraint.
    pub fn resolve(
        &self,
        name: &str,
        constraint: &semver::VersionReq,
    ) -> Option<&VersionedToolDef> {
        self.tools.get(name)
            .and_then(|versions| {
                versions.iter()
                    .filter(|v| constraint.matches(&v.version))
                    .max_by(|a, b| a.version.cmp(&b.version))
            })
    }
}
```

### Configuration

```toml
# roko.toml — pin tool versions
[tools.versions]
"read_file" = ">=1.0.0, <2.0.0"
"web_search" = "^2.1.0"
```

---

## Tool Composition and Dataflow

Tools compose into pipelines — sequential chains, parallel fans, and DAGs.

### Composition Patterns

**Sequential (pipe):** Output of tool A feeds input of tool B.
```
read_file → grep → edit_file
```

**Parallel (fan-out):** Multiple independent tools execute concurrently, results merged.
```
┌─ web_search("rust async") ──┐
│                              ├─ merge → compose_answer
└─ web_search("tokio guide") ─┘
```

**Iterative (loop):** Tool called repeatedly until condition met.
```
loop { edit_file → run_tests → if pass then break }
```

### ToolPipeline Definition

```rust
/// A composable pipeline of tool invocations.
pub struct ToolPipeline {
    pub stages: Vec<PipelineStage>,
    pub timeout: Duration,
    pub fail_fast: bool,
}

pub enum PipelineStage {
    /// Single tool invocation.
    Single {
        tool: String,
        params: serde_json::Value,
        /// JSONPath expression to extract from prior stage output.
        input_mapping: Option<String>,
    },
    /// Parallel fan-out — all tools execute concurrently.
    Parallel {
        tools: Vec<(String, serde_json::Value)>,
        merge_strategy: MergeStrategy,
    },
    /// Conditional — select tool based on prior output.
    Conditional {
        condition: String,
        if_true: Box<PipelineStage>,
        if_false: Option<Box<PipelineStage>>,
    },
}

pub enum MergeStrategy {
    Array,         // Collect all results into an array
    MergeObjects,  // Merge objects (later overrides earlier)
    FirstSuccess,  // Take the first successful result
}
```

### ReAct-Style Interleaved Composition

The predominant composition pattern in Roko agents is ReAct (Yao et al., 2023): interleaved
reasoning traces and tool actions. The LLM decides at each step which tool to call based on
prior observations. This requires no explicit pipeline definition — the LLM IS the pipeline
controller. The `ToolPipeline` struct above is for programmatic (non-LLM) composition such
as gate pipelines, automated test sequences, and batch operations.

---

## Tool Discovery Service

Beyond the static registry and MCP discovery, Roko supports capability-based tool discovery
via semantic search. When an agent encounters a task that no loaded tool handles, it can query
the discovery service for tools matching the capability description.

```rust
/// Tool discovery via embedding-based semantic search.
pub struct ToolDiscoveryService {
    /// Embedding index over all registered tool descriptions.
    index: VectorIndex,
    registry: Arc<dyn ToolRegistry>,
    embedder: Arc<dyn Embedder>,
}

impl ToolDiscoveryService {
    /// Find tools matching a natural-language capability description.
    pub async fn discover(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<DiscoveredTool>> {
        let query_embedding = self.embedder.embed(query).await?;
        let results = self.index.search(&query_embedding, top_k);
        results.into_iter().map(|hit| {
            let tool = self.registry.get(&hit.id)
                .ok_or_else(|| anyhow!("Tool not in registry: {}", hit.id))?;
            Ok(DiscoveredTool {
                tool,
                similarity: hit.score,
            })
        }).collect()
    }

    /// Rebuild the index when tools are added or removed.
    pub async fn reindex(&mut self) -> Result<()> {
        let tools = self.registry.all();
        let descriptions: Vec<&str> = tools.iter().map(|t| t.description).collect();
        let embeddings = self.embedder.embed_batch(&descriptions).await?;
        self.index = VectorIndex::build(
            tools.iter().map(|t| t.name.to_string()).zip(embeddings)
        );
        Ok(())
    }
}

pub struct DiscoveredTool<'a> {
    pub tool: &'a ToolDef,
    pub similarity: f32,
}
```

### Configuration

```toml
[tools.discovery]
enabled = true
embedding_model = "text-embedding-3-small"
similarity_threshold = 0.75
reindex_on_tool_change = true
max_results = 10
```

### Test Criteria

- Discovery returns `read_file` for query "read contents of a file".
- Discovery returns empty for query with no matching tool.
- Similarity threshold correctly filters low-confidence matches.
- Index rebuilds when MCP tools are added at runtime.
- Discovery latency < 50ms for 500 tools.

---

## Current Implementation Status

The tool architecture is implemented in `roko-std` (`crates/roko-std/src/tool/`):

- **`builtin/mod.rs`**: 16 built-in tools (see `01-builtin-tools.md`)
- **`registry.rs`**: `StaticToolRegistry` with role-based filtering
- **`handlers.rs`**: Handler dispatch
- **`mod.rs`**: Module structure and re-exports

The chain domain plugin tools (423+ DeFi tools) are specified in the legacy PRD
(`bardo-backup/prd/07-tools/`) and will be implemented as a separate `roko-domain-chain`
crate following the domain plugin pattern described in `refactoring-prd/10-developer-guide.md`.

---

## References

- **OpenAPI 3.1.0** (OAI, 2021) — JSON Schema 2020-12 alignment.
  [spec](https://github.com/oai/openapi-specification/blob/main/versions/3.1.0.md)
- **schemars** — Derive JSON Schema from Rust types.
  [crate](https://crates.io/crates/schemars)
- **ReAct** (Yao et al., 2023) — Interleaved reasoning and tool actions.
  [paper](https://arxiv.org/abs/2210.03629)
- **AGNTCY Agent Directory** (Cisco, 2025) — Distributed tool discovery.
  [paper](https://arxiv.org/abs/2509.18787)

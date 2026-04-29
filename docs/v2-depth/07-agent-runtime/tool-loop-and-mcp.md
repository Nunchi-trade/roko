# Tool Loop and MCP Integration

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). The agent tool loop as a Hot Flow implementing the perceive-think-act-verify cycle, MCP integration as a Connect protocol providing dynamic tool discovery, and the ToolDispatcher as a Pipeline Cell enforcing safety at every invocation.

---

## 1. The ToolLoop as a Hot Flow

The spec defines the cognitive loop as a **Loop pattern** (Graph with feedback edge). At the implementation level, the ToolLoop is a concrete realization of this pattern for HTTP-based agents. It drives the iterative cycle:

```
prompt -> LLM -> tool_calls? -> dispatch -> results -> LLM -> ...
         ^                                               |
         +----------- feedback (results as context) -----+
```

This is a Hot Flow: it stays resident between firings (turns), maintains accumulated state (message history), and re-fires per tick until termination.

### Construction

```rust
pub struct ToolLoop {
    translator: Arc<dyn Translator>,     // Wire format conversion
    dispatcher: Arc<ToolDispatcher>,     // Safety + execution pipeline
    backend: Arc<dyn LlmBackend>,        // Single-turn LLM interface
    max_iterations: usize,               // Default: 25
    context_token_limit: usize,          // Context growth guard
}
```

Three dependencies, all injected via `Arc`:

1. **Translator** -- Converts between canonical tools and the backend's wire format. Selected based on `ModelProfile::tool_format` (see [chat-types-and-streaming.md](chat-types-and-streaming.md)).
2. **ToolDispatcher** -- Runs tool calls through the safety + execution Pipeline (section 3).
3. **LlmBackend** -- Sends conversation turns to the LLM (section 2).

### The core loop

```rust
async fn run_inner(&self, mut messages, mut iterations, tools, ctx)
    -> ToolLoopOutput
{
    let rendered_tools = self.translator.render_tools(tools);

    loop {
        // 1. Check iteration cap (Verify: resource budget)
        if max_iter::is_exhausted(iterations, self.max_iterations) {
            return checkpoint + MaxIterations;
        }

        // 2. Check cancellation (external termination signal)
        if ctx.is_cancelled() {
            return checkpoint + Cancelled;
        }

        // 3. Send turn to LLM (Connect: external system I/O)
        let response = self.backend.send_turn(&messages, &rendered_tools).await?;

        // 4. Parse tool calls (Score: evaluate response structure)
        let calls = self.translator.parse_calls(&response)?;

        // 5. No tool calls = final answer (Loop termination)
        if calls.is_empty() {
            return ToolLoopOutput { final_text: response.extract_text(), ... };
        }

        // 6. Inject assistant message into history
        if let Some(msg) = self.translator.render_assistant_message(&response) {
            messages.push(msg);
        }

        // 7. Dispatch tool calls (Pipeline: validate -> authorize -> execute)
        let results = self.dispatcher.dispatch_batch(calls.clone(), ctx).await;

        // 8. Format results as messages (Compose: assemble context)
        let rendered = self.translator.render_results(&results);
        result_msg::append_results(&mut messages, rendered);

        // 9. Prune context if needed (Resource management)
        prune::prune_if_needed(&mut messages, self.context_token_limit);

        iterations += 1;
    }
}
```

### Termination conditions

The loop terminates on one of four conditions:

| Condition | Meaning | Output |
|---|---|---|
| **Stop** | LLM returns response with no tool calls | Final answer extracted |
| **MaxIterations** | Iteration cap reached (default: 25) | Checkpoint for resume |
| **Cancelled** | External cancel token tripped between turns | Checkpoint for resume |
| **BackendError** | LLM returns an error | Checkpoint + error |

### Checkpoint and resume

When the loop stops for any reason other than `Stop`, it produces a `Checkpoint` capturing the full conversation state:

```rust
pub struct Checkpoint {
    pub iterations: usize,
    pub tool_calls: Vec<ToolCall>,
    pub messages: Vec<serde_json::Value>,
}
```

This enables resume: `tool_loop.resume(checkpoint, &tools, &ctx)` continues from the exact state where it left off. Critical for long-running tasks that hit the iteration cap or experience transient backend errors.

### Context pruning

The `prune` submodule prevents the conversation from exceeding the model's context window:

```rust
pub fn prune_if_needed(messages: &mut Vec<Value>, token_limit: usize) {
    // Estimate tokens from message byte length
    // Keep system + first user message (always)
    // Drop oldest tool results, preserving most recent
    // Keep at least head + tail
}
```

The strategy is conservative: the system prompt and initial user message are always preserved (they contain task instructions). The most recent messages are preserved (they contain the latest context). Oldest tool results are dropped first.

---

## 2. LlmBackend: The Single-Turn Interface

The `LlmBackend` trait is intentionally lower-level than the `Agent` trait:

```rust
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send the current conversation state to the backend.
    /// Returns the raw wire response for the Translator to parse.
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError>;
}
```

| Trait | Granularity | Who drives the loop? |
|---|---|---|
| `Agent::run()` | Complete multi-turn run | The Agent implementation |
| `LlmBackend::send_turn()` | Single request-response | The ToolLoop |

The ToolLoop calls `send_turn()` once per iteration, inspects the response for tool calls, dispatches them, and calls `send_turn()` again. This gives Roko full control over the tool dispatch pipeline (safety, caching, batching) for HTTP-based agents.

### Existing implementations

- **`OllamaLlmBackend`** (`crates/roko-agent/src/ollama_backend.rs`) -- Implements `LlmBackend` for the Ollama HTTP API. Proves the pattern works.

### Missing implementations

The critical gap: no `OpenAiCompatBackend` or `AnthropicApiBackend` implementations exist yet. HTTP-based agents (OpenAI, ZhipuAI, Gemini, Perplexity) currently go through `Agent::run()` which does a single-shot call, bypassing the ToolLoop entirely. The implementation plan documents this as the highest-priority integration gap.

```rust
/// Proposed OpenAI-compatible LlmBackend implementation
pub struct OpenAiCompatBackend {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: Option<u64>,
}

#[async_trait]
impl LlmBackend for OpenAiCompatBackend {
    async fn send_turn(
        &self,
        messages: &[Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError> {
        let body = json!({
            "model": self.model,
            "messages": messages,
            "tools": tools.as_json_array(),
            "max_tokens": self.max_tokens,
        });
        let resp = self.client
            .post(&format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(BackendResponse::Json(resp.json().await?))
    }
}
```

---

## 3. ToolDispatcher: The Safety Pipeline

The `ToolDispatcher` processes every tool call through an 8-step Pipeline. In unified terms, this is a **Pipeline pattern** (linear Graph of Verify + Route + Compose Cells) with early exit at each step:

```
1. VALIDATE   -- Args against JSON schema from registry
2. RESOLVE    -- Look up the ToolDef for the canonical name
3. FILTER     -- Task-level allowed/denied tool lists
4. AUTHORIZE  -- role_permissions.satisfied_by(&tool_permissions)
5. SAFETY     -- SafetyLayer pre-execution checks
6. EXECUTE    -- handler.execute() with timeout + cancellation
7. TRUNCATE   -- Oversized output capped to max_result_bytes
8. SCRUB      -- SafetyLayer post-execution secret scrubbing
```

Each step emits audit Signals via the `AuditSink`, creating a full trace of the dispatch decision chain. If any step fails, the pipeline short-circuits with an error result.

### Batch dispatch (parallel + serial)

Tool calls are partitioned by concurrency safety:

```rust
pub async fn dispatch_batch(&self, calls: Vec<ToolCall>, ctx: &ToolContext)
    -> Vec<(ToolCall, ToolResult)>
{
    let (parallel, serial) = partition_by_concurrency(calls, &self.registry);

    // Parallel-safe tools (read_file, grep, glob): fan out with join_all
    let par_results = futures::future::join_all(
        parallel.map(|c| self.dispatch(c, ctx))
    ).await;

    // Serial tools (bash, write_file): sequential to preserve ordering
    for call in serial {
        let result = self.dispatch(call, ctx).await;
    }
}
```

### SafetyLayer: Six policy families

The `SafetyLayer` composes six policy families that form a Verify pipeline before and after tool execution:

```rust
pub struct SafetyLayer {
    pub bash_policy: BashPolicy,       // Command allowlist/denylist
    pub git_policy: GitPolicy,         // Branch protection, force-push blocking
    pub network_policy: NetworkPolicy, // Outbound destination allowlist
    pub path_policy: PathPolicy,       // Worktree escape prevention
    pub scrub_policy: ScrubPolicy,     // Secret scrubbing from outputs
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub role: String,
}
```

| Policy | Pre-execution check | Post-execution check |
|---|---|---|
| `BashPolicy` | Command against allowlist/denylist | -- |
| `GitPolicy` | Block force-push to main, `reset --hard` | -- |
| `NetworkPolicy` | URL against destination allowlist (blocks localhost, private IPs) | -- |
| `PathPolicy` | Canonicalize path, block worktree escapes | -- |
| `ScrubPolicy` | -- | Remove API keys, tokens, secrets from output |
| `RateLimiter` | Per-role, per-tool call rate check | -- |

### Integration gap

The SafetyLayer is wired into the ToolDispatcher. The ToolDispatcher is used by the ToolLoop. But **the ToolDispatcher is never called from `orchestrate.rs`**. The orchestrator constructs `ClaudeCliAgent` instances directly, and the Claude CLI drives its own internal tool loop. Roko's full safety pipeline is bypassed for the primary execution path.

This is documented as the number-one integration gap. The fix is wiring HTTP backends through `create_agent_for_model` -> adapter -> `LlmBackend` -> `ToolLoop` -> `ToolDispatcher` -> `SafetyLayer`.

---

## 4. MCP Integration as Connect Protocol

The Model Context Protocol (MCP) is a standard for connecting agents to external tools via JSON-RPC over stdio. In unified terms, MCP integration is a **Connect protocol** operation: discover external tools at runtime, convert their schemas, and integrate them into the tool registry.

### Architecture

```
crates/roko-agent/src/mcp/
    client.rs           -- JSON-RPC stdio transport (Connect protocol)
    config.rs           -- .mcp.json discovery and parsing (Trigger protocol)
    dedup.rs            -- Multi-server tool deduplication (Route protocol)
    dynamic_registry.rs -- Composes static + MCP tools (Compose protocol)
    to_tool_def.rs      -- MCP schema -> ToolDef conversion (Score protocol)
```

### MCP client

The `McpClient` manages JSON-RPC connections to MCP servers:

```rust
pub struct McpClient {
    transport: Box<dyn Transport>,
}

pub trait Transport: Send + Sync {
    fn send(&mut self, request: McpRequest) -> Result<McpResponse>;
    fn receive(&mut self) -> Result<McpResponse>;
}
```

The primary transport is `StdioTransport`: spawn the MCP server as a child process, communicate via stdin/stdout JSON-RPC messages.

### Tool discovery (tools/list)

At startup, the client sends `tools/list` and receives the server's tool catalog:

```rust
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,  // JSON Schema
}
```

### Tool invocation (tools/call)

When the agent requests a tool, the client sends `tools/call`:

```json
{
    "jsonrpc": "2.0",
    "id": 42,
    "method": "tools/call",
    "params": { "name": "read_file", "arguments": { "path": "/src/main.rs" } }
}
```

### Tool conversion: MCP -> ToolDef

MCP tool definitions are converted to Roko's canonical `ToolDef` format:

```rust
pub fn mcp_to_tool_def(mcp_tool: &McpToolDef) -> ToolDef {
    ToolDef::new(
        &mcp_tool.name,
        &mcp_tool.description,
        ToolCategory::Custom,        // MCP tools are always custom
        ToolPermission::read_only(), // Conservative default (untrusted)
    )
    .with_schema(mcp_tool.input_schema.clone())
}
```

MCP tools default to `read_only()` permissions because external tools are untrusted. Higher permissions must be explicitly granted in config.

### Multi-server deduplication

When multiple MCP servers expose tools with the same name:

1. If unique across servers: keep as-is
2. If name collision: prefix with server name (`filesystem:read_file` vs `github:read_file`)
3. If collision with built-in: built-in takes precedence, MCP tool is prefixed

### DynamicToolRegistry

Composes static built-in tools with dynamically discovered MCP tools:

```rust
pub struct DynamicToolRegistry {
    static_tools: Vec<ToolDef>,
    mcp_tools: Vec<ToolDef>,
}

impl ToolRegistry for DynamicToolRegistry {
    fn get(&self, name: &str) -> Option<&ToolDef> {
        // Static tools take precedence, then MCP tools
        self.static_tools.iter().find(|t| t.name == name)
            .or_else(|| self.mcp_tools.iter().find(|t| t.name == name))
    }
}
```

The ToolDispatcher uses `DynamicToolRegistry` transparently -- it does not know whether a tool came from the built-in catalog or an MCP server.

### Config discovery

`.mcp.json` is searched in priority order:

1. Current working directory
2. Project root (from `roko.toml`)
3. User home directory (`~/.mcp.json`)

The `roko.toml` config can also specify an explicit path:

```toml
[agent]
mcp_config = ".mcp.json"
```

### Two MCP paths

| Agent type | MCP path | Who drives tool calls? |
|---|---|---|
| Claude CLI agents | Passthrough via `--mcp-config` flag | Claude CLI's internal MCP client |
| HTTP-based agents | Roko's `McpClient` via `DynamicToolRegistry` + `ToolLoop` | Roko's ToolDispatcher |

Both paths produce the same observable behavior (agent can call MCP tools). The difference is plumbing: Claude CLI has its own battle-tested MCP client, so Roko passes the config through. HTTP agents need Roko's MCP client.

### MCP in the ToolLoop pipeline

```
1. discover_mcp_servers()       -> Vec<McpServerConfig>
2. connect_and_list_tools()     -> Vec<(server_name, McpToolDef)>
3. dedup_tools()                -> Vec<McpToolDef>
4. mcp_to_tool_def()            -> Vec<ToolDef>
5. DynamicToolRegistry::new()   -> merges built-in + MCP tools
6. ToolLoop::new(translator, dispatcher, backend)
7. loop.run(system, user, all_tools, ctx)
```

---

## 5. Reasoning Pattern Hierarchy

The ToolLoop implements the **ReAct** pattern (Yao et al., 2023, arXiv:2210.03629). Research identifies a hierarchy of reasoning patterns:

| Pattern | Quality | Cost | Loop structure |
|---|---|---|---|
| **Direct** | Low | 1 call | No loop |
| **ReAct** | Medium | N calls | `think -> act -> observe` (current ToolLoop) |
| **Reflexion** | High | 2N calls | ReAct + self-reflection on failure |
| **Tree-of-Thought** | Higher | K x N calls | Explore K branches, evaluate, pick best |
| **MCTS/LATS** | Highest | K^2 x N calls | Full tree search with value backpropagation |

The CascadeRouter could select the reasoning pattern (not just the model) based on task complexity. Mechanical tasks use Direct, standard implementation uses ReAct, gate-failure retries use Reflexion, architecture decisions use MCTS.

### Reflexion integration with gate feedback

When a gate rejects an agent's output, the gate result should be converted to a verbal reflection and injected into the next dispatch:

```rust
pub struct ReflexionContext {
    pub reflections: Vec<Reflection>,
    pub max_reflections: usize,  // Default: 3
}

pub struct Reflection {
    pub attempt_number: usize,
    pub gate_name: String,
    pub failure_reason: String,
    pub verbal_reflection: String,  // LLM-generated "what went wrong"
}
```

Reflexion achieves 91% pass@1 on HumanEval vs GPT-4's 80% (Shinn et al., 2023, NeurIPS 2023). This maps directly to Roko's gate-failure-replan mechanism.

---

## 6. Tool Selection Optimization

### Tool RAG (Retrieval-Augmented Tool Selection)

Instead of including all tool definitions in context, retrieve only relevant tools per query using HDC embeddings. Across 121 tools from 5 MCP servers: **99.6% token reduction** while maintaining 97.1% hit rate (Red Hat, 2025, arXiv:2603.20313).

### AutoTool: Graph-Based Tool Prediction

Tool usage follows predictable sequential patterns (search -> read -> edit). A transition graph mined from `.roko/episodes.jsonl` can predict likely next tools. Reduces LLM call count by 15-25% and tokens by 10-40% (arXiv:2511.14650, AAAI 2026).

### Speculative Tool Execution (PASTE)

Run predicted tools in parallel with the LLM's reasoning. If the actual tool call matches a speculated result, use the cached output. Reduces average task completion time by 48.5% (Microsoft Research, arXiv:2603.18897). Safety constraint: only speculate read-only tools.

### Tool Result Caching

Cache tool results by (tool_name, args_hash) with per-tool TTL policies:

| Tool category | Cacheable? | TTL | Invalidation |
|---|---|---|---|
| Pure read (file read, search) | Yes | Minutes | On source change |
| Computed/deterministic (math) | Yes | Long/infinite | Never |
| State-querying (git status) | Yes | Seconds | On any write operation |
| Write/mutating (file write) | Never | N/A | Invalidates related reads |

Achieves 1.69x latency speedup without accuracy loss (ToolCacheAgent, ICLR 2026).

---

## What This Enables

- **Multi-turn tool calling for all HTTP backends** through a single, tested ToolLoop implementation
- **Unified safety enforcement** via the 8-step ToolDispatcher Pipeline regardless of provider
- **Dynamic tool discovery** through MCP without recompilation or config changes
- **Resumable execution** via checkpoints for long-running or interrupted tasks
- **Context management** through automatic pruning that prevents context overflow

## Feedback Loops

1. **ToolLoop -> EpisodeLogger**: Every tool call and result becomes an episode entry. The episode history feeds the ToolTransitionGraph for predicting future tool usage.
2. **Gate failure -> Reflexion -> ToolLoop**: When a gate rejects output, the failure is converted to a verbal reflection and injected into the next ToolLoop run. The loop improves with each attempt.
3. **Tool dispatch metrics -> CascadeRouter**: Tool call patterns and success rates feed back into model routing. Models that produce better tool calls get preferred.
4. **MCP discovery -> DynamicToolRegistry -> ToolLoop**: New MCP servers discovered at startup expand the available tool set without code changes. The ToolLoop adapts automatically.
5. **Context pruning -> Token budget**: Pruning decisions affect which context the model sees on the next turn, creating a feedback loop between context management and output quality.

## Open Questions

1. **LlmBackend implementations**: The OpenAiCompatBackend and AnthropicApiBackend do not exist yet. Only OllamaLlmBackend is implemented. This blocks the ToolLoop from serving the majority of HTTP-based agents.

2. **Reasoning strategy routing**: Should the CascadeRouter select reasoning patterns (Direct, ReAct, Reflexion, MCTS) alongside model selection? This would make the routing decision multi-dimensional: model x reasoning_strategy x effort_level.

3. **ToolLoop vs Claude CLI internal loop**: Claude CLI drives its own tool loop, bypassing Roko's ToolDispatcher and SafetyLayer. Should Roko intercept Claude CLI's tool calls (via stream-JSON event parsing) and apply its own safety policies, or trust Claude CLI's internal safety?

4. **Speculative execution safety**: PASTE only speculates read-only tools, but "read-only" classification depends on the tool registry being accurate. A miscategorized tool could cause side effects from speculative execution.

---

## Citations

1. Yao, S. et al. (2023). "ReAct: Synergizing Reasoning and Acting." ICLR 2023. arXiv:2210.03629.
2. Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal Reinforcement Learning." NeurIPS 2023. arXiv:2303.11366.
3. Zhou, A. et al. (2024). "Language Agent Tree Search." ICML 2024. arXiv:2310.04406.
4. arXiv:2511.14650 (2025). "AutoTool: Efficient Tool Selection." AAAI 2026.
5. arXiv:2603.18897 (2025). Microsoft Research. "PASTE: Pattern-Aware Speculative Tool Execution."
6. arXiv:2603.20313 (2025). Red Hat. "Tool RAG: Next Breakthrough in Scalable AI Agents."
7. `crates/roko-agent/src/tool_loop/mod.rs` -- ToolLoop implementation (769 lines).
8. `crates/roko-agent/src/dispatcher/mod.rs` -- ToolDispatcher 8-step pipeline (1070 lines).
9. `crates/roko-agent/src/safety/mod.rs` -- SafetyLayer, 6 policy families.
10. `crates/roko-agent/src/mcp/` -- MCP client, config, dedup, dynamic_registry, to_tool_def.

# 07 — Tool Loop

> Sub-doc 07 of **02-agents** · Roko Documentation
>
> **Critical:** The ToolLoop already exists and works. It does NOT need to be
> rebuilt. What is missing is `LlmBackend` implementations for the HTTP-based
> providers so they can use the existing loop.
>
> This document describes the `ToolLoop` multi-turn driver, the `LlmBackend`
> trait, the `ToolDispatcher` 7-step pipeline, the `SafetyLayer` composition,
> and the integration gap.

---

## The ToolLoop Already Exists

The `ToolLoop` is a fully implemented, fully tested multi-turn tool-calling
driver at `crates/roko-agent/src/tool_loop/mod.rs`. It drives the iterative
cycle:

```
prompt → LLM → tool_calls? → dispatch → results → LLM → ...
```

The loop runs until one of four conditions:
1. **Stop** — The LLM returns a response with no tool calls (final answer).
2. **MaxIterations** — The iteration cap is reached (default: 25).
3. **Cancelled** — The cancel token is tripped between turns.
4. **BackendError** — The LLM returns an error.

The implementation is 263 lines of production code plus 500+ lines of tests.
It handles:
- Iteration cap enforcement (`max_iter` submodule, §36.54)
- Context-growth pruning (`prune` submodule, §36.55)
- Tool-result message construction (`result_msg` submodule, §36.56)
- Resumable checkpointing (`checkpoint` submodule, §36.57)
- Cancellation between turns (§36.45)
- Parallel/serial tool dispatch batching (§36.41)

**Do not rebuild this.** The gap is not the loop — it is the `LlmBackend`
implementations for HTTP providers.

---

## LlmBackend Trait

The `LlmBackend` trait at `crates/roko-agent/src/tool_loop/mod.rs:43` is
the interface between the ToolLoop and the LLM:

```rust
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// Send the current conversation state to the backend.
    ///
    /// `messages` is the accumulated message history (system, user,
    /// assistant, tool-result messages). `tools` is the pre-rendered
    /// tool spec from Translator::render_tools.
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
    ) -> Result<BackendResponse, LlmError>;
}
```

This is intentionally lower-level than the `Agent` trait:
- `Agent::run()` models a complete agent run (potentially many turns).
- `LlmBackend::send_turn()` models a single request-response round.

The ToolLoop calls `send_turn()` once per iteration, inspects the response
for tool calls via the `Translator`, dispatches any tool calls via the
`ToolDispatcher`, formats results, and calls `send_turn()` again.

### LlmError

```rust
pub enum LlmError {
    Backend(String),  // API error, non-success status
    Network(String),  // DNS, timeout, connection reset
}
```

---

## What Is Missing: LlmBackend Implementations

The ToolLoop works. The `Translator` implementations work. The
`ToolDispatcher` works. What is missing is the **bridge**: `LlmBackend`
implementations that wrap the HTTP-based agents.

An `OpenAiCompatBackend` implementation would look like:

```rust
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
        let response = self.client.post(&format!("{}/v1/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        // ...parse response...
        Ok(BackendResponse::Json(json))
    }
}
```

Implementation plan `modelrouting/14-integration-refinements.md` documents
this as the critical missing piece: "What's missing is NOT the loop — it's
the LlmBackend implementations for HTTP providers."

The `OllamaLlmBackend` at `crates/roko-agent/src/ollama_backend.rs` already
implements `LlmBackend` for the Ollama HTTP API, proving the pattern works.

---

## ToolLoop Internals

### Construction

```rust
pub struct ToolLoop {
    translator: Arc<dyn Translator>,
    dispatcher: Arc<ToolDispatcher>,
    backend: Arc<dyn LlmBackend>,
    max_iterations: usize,         // Default: 25
    context_token_limit: usize,    // Default from prune module
}
```

Three dependencies, all injected via `Arc`:
1. **Translator** — Converts between canonical tools and the backend's wire
   format. Selected based on `ModelProfile::tool_format`.
2. **ToolDispatcher** — Runs tool calls through the safety + execution pipeline.
3. **LlmBackend** — Sends conversation turns to the LLM.

### The core loop

```rust
async fn run_inner(&self, mut messages, mut iterations, mut all_calls, tools, ctx)
    -> ToolLoopOutput
{
    let rendered_tools = self.translator.render_tools(tools);

    loop {
        // 1. Check iteration cap
        if max_iter::is_exhausted(iterations, self.max_iterations) {
            return checkpoint + MaxIterations;
        }

        // 2. Check cancellation
        if ctx.is_cancelled() {
            return checkpoint + Cancelled;
        }

        // 3. Send turn to LLM
        let response = self.backend.send_turn(&messages, &rendered_tools).await?;

        // 4. Parse tool calls
        let calls = self.translator.parse_calls(&response)?;

        // 5. If no tool calls, return final answer
        if calls.is_empty() {
            return ToolLoopOutput { final_text: response.extract_text(), ... };
        }

        // 6. Inject assistant message into history
        if let Some(msg) = self.translator.render_assistant_message(&response) {
            messages.push(msg);
        }

        // 7. Dispatch tool calls (parallel + serial batching)
        let results = self.dispatcher.dispatch_batch(calls.clone(), ctx).await;
        all_calls.extend(calls);

        // 8. Format results as messages
        let rendered = self.translator.render_results(&results);
        result_msg::append_results(&mut messages, rendered);

        // 9. Prune context if needed
        prune::prune_if_needed(&mut messages, self.context_token_limit);

        iterations += 1;
    }
}
```

### Checkpoint and resume

When the loop stops for any reason other than `Stop` (final answer), it
produces a `Checkpoint`:

```rust
pub struct Checkpoint {
    pub iterations: usize,
    pub tool_calls: Vec<ToolCall>,
    pub messages: Vec<serde_json::Value>,
}
```

The checkpoint captures the full conversation state, allowing the loop to
be resumed later:

```rust
// Resume from where we left off
let output = tool_loop.resume(checkpoint, &tools, &ctx).await;
```

This is critical for long-running tasks that hit the iteration cap or
experience transient backend errors.

### Context pruning

The `prune` submodule implements context-growth guards that prevent the
conversation from exceeding the model's context window:

```rust
pub fn prune_if_needed(messages: &mut Vec<Value>, token_limit: usize) {
    // Estimate tokens from message byte length
    // Keep system + first user message
    // Drop oldest tool results, preserving most recent
    // Keep at least head + tail
}
```

The pruning strategy is conservative: it keeps the system prompt and initial
user message, preserves the most recent messages, and drops the oldest tool
results. This ensures the model always has the original instructions and the
most recent context.

---

## ToolDispatcher — The 7-Step Pipeline

The `ToolDispatcher` at `crates/roko-agent/src/dispatcher/mod.rs` processes
every tool call through a rigorous pipeline:

```
1. VALIDATE   — Args against JSON schema from registry (§36.42)
2. RESOLVE    — Look up the ToolDef for the canonical name
3. FILTER     — Task-level allowed/denied tool lists
4. AUTHORIZE  — def.permission.satisfied_by(&role_perms) (§36.46)
5. SAFETY     — SafetyLayer pre-execution checks (bash, git, network, path)
6. EXECUTE    — handler.execute() with timeout + cancellation (§36.40, §36.45)
7. TRUNCATE   — Oversized Ok content to max_result_bytes (§36.43)
8. SCRUB      — SafetyLayer post-execution secret scrubbing (§36.50)
```

Each step emits audit signals via the `AuditSink`, creating a full trace
of the dispatch decision chain. The phases are:
- `validation → passed/failed`
- `tool_filter → denied` (if filtered)
- `permission → granted/denied`
- `safety → blocked` (if SafetyLayer rejects)
- `handler → started/missing`
- `completion → succeeded/failed`

### Batch dispatch

The `dispatch_batch` method groups tool calls by concurrency policy:

```rust
pub async fn dispatch_batch(&self, calls: Vec<ToolCall>, ctx: &ToolContext)
    -> Vec<(ToolCall, ToolResult)>
{
    let (parallel, serial) = partition_by_concurrency(calls, self.registry.as_ref());

    // Parallel: fan out with join_all
    let par_results = futures::future::join_all(parallel.map(|c| self.dispatch(c, ctx))).await;

    // Serial: sequential loop (preserves shell-state ordering)
    for call in serial {
        let result = self.dispatch(call, ctx).await;
        // ...
    }
}
```

Parallel-safe tools (like `read_file`, `grep`, `glob`) run concurrently via
`join_all`. Serial tools (like `bash`, `write_file`) run sequentially to
preserve ordering and avoid write-write races.

---

## SafetyLayer

The `SafetyLayer` at `crates/roko-agent/src/safety/mod.rs` composes six
policy families:

```rust
pub struct SafetyLayer {
    pub bash_policy: BashPolicy,       // Command allowlist/denylist
    pub git_policy: GitPolicy,         // Branch protection
    pub network_policy: NetworkPolicy, // Outbound destination allowlist
    pub path_policy: PathPolicy,       // Worktree escape prevention
    pub scrub_policy: ScrubPolicy,     // Secret scrubbing from outputs
    pub rate_limiter: Option<Arc<RateLimiter>>,  // Per-tool rate limits
    pub role: String,                  // Role name for rate-limit keys
}
```

The `check_pre_execution` method applies policies based on tool name:

- **Bash/run_tests tools** → `BashPolicy` checks the command against
  allowlist/denylist; `GitPolicy` checks for destructive git operations
  (force push to main, `reset --hard`).
- **Network tools** (web_fetch, web_search) → `NetworkPolicy` checks the
  URL against the destination allowlist (blocks `127.0.0.1`, `localhost`,
  private IP ranges).
- **File tools** (read_file, write_file, etc.) → `PathPolicy` canonicalizes
  the path and blocks worktree escapes.
- **All tools** → `RateLimiter` checks per-role, per-tool call rate.

Post-execution, `scrub_output` runs the `ScrubPolicy` to remove API keys,
tokens, and other secrets from tool output before it enters the conversation
history.

---

## The Integration Gap

The SafetyLayer is wired into the ToolDispatcher. The ToolDispatcher is used
by the ToolLoop. The ToolLoop works. However:

**The ToolDispatcher is never called from `orchestrate.rs`.**

The orchestrator constructs `ClaudeCliAgent` instances directly, and the
Claude CLI drives its own internal tool loop. Roko's `ToolDispatcher` +
`SafetyLayer` + `ToolLoop` are bypassed entirely for the primary execution
path.

This is the **#1 integration gap** identified in implementation plan
`11-inconsistencies.md`. The gap exists because Claude CLI was the first
backend wired (and it handles tools internally), but HTTP-based backends
(which need Roko's ToolLoop) were added later.

The fix is to wire HTTP backends through `create_agent_for_model` → provider
adapter → `LlmBackend` → `ToolLoop` → `ToolDispatcher` → `SafetyLayer`.
This gives HTTP-based agents the same safety guarantees that Claude CLI
provides via its own internal safety mechanisms.

---

## Test Coverage

The ToolLoop has comprehensive tests covering all stop conditions:

- `zero_tool_calls_returns_immediately` — No tools → immediate final answer
- `single_tool_call_runs_to_completion` — One tool call → dispatch → final
- `max_iterations_returns_max_iterations` — Hits cap → checkpoint
- `cancellation_halts_loop` — Cancel token → stops between turns
- `backend_error_returns_backend_error` — LLM error → checkpoint
- `parallel_tool_calls_dispatched_in_one_batch` — Multiple calls → parallel
- `context_prune_drops_oldest_results_after_threshold` — Pruning works
- `tool_call_ids_flow_through_to_result_messages` — ID propagation
- `resume_continues_from_checkpoint` — Checkpoint → resume → continues

---

## Citations

1. `crates/roko-agent/src/tool_loop/mod.rs` — Full 769-line source with
   ToolLoop, LlmBackend, StopReason, ToolLoopOutput, Checkpoint.
2. `crates/roko-agent/src/dispatcher/mod.rs` — Full 1070-line source with
   ToolDispatcher, HandlerResolver, 7-step pipeline.
3. `crates/roko-agent/src/safety/mod.rs` — SafetyLayer, 6 policy families.
4. Implementation plan `modelrouting/14-integration-refinements.md` —
   "Wire EXISTING ToolLoop", OpenAiCompatBackend design.
5. Implementation plan `11-inconsistencies.md` — Gap #1: SafetyLayer wired
   to ToolDispatcher but Dispatcher never called from CLI/orchestrate.rs.
6. `crates/roko-agent/src/ollama_backend.rs` — OllamaLlmBackend, proving
   the LlmBackend pattern works.

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


> **Implementation**: Shipping

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

## Reasoning Pattern Taxonomy

The ToolLoop implements the basic **ReAct** pattern (Yao et al., 2023). Research
identifies a hierarchy of reasoning patterns, each building on the previous:

| Pattern | Quality | Cost | Best for |
|---|---|---|---|
| **Direct** | Low | 1 call | Simple classification, formatting |
| **ReAct** | Medium | N calls | Standard tool use (Roko's current loop) |
| **Reflexion** | High | 2N calls | Tasks with gate feedback (self-correction) |
| **Tree-of-Thought** | Higher | K×N calls | Plan generation, exploration |
| **MCTS/LATS** | Highest | K²×N calls | Hard debugging, architecture decisions |

### ReAct (Current Implementation)

Yao et al. (2023, arXiv:2210.03629, ICLR 2023). Interleaves reasoning traces
("Thought") with task-specific actions ("Action") and environment feedback
("Observation"). This is precisely what the ToolLoop implements: the LLM
reasons, picks a tool, gets output, continues.

### Reflexion: Self-Reflection on Gate Failures

Shinn et al. (2023, arXiv:2303.11366, NeurIPS 2023). After a failed task
attempt, the agent generates a verbal self-reflection summarizing what went
wrong and stores it in episodic memory. On the next attempt, this reflection
is included in context. Achieves 91% pass@1 on HumanEval (vs. GPT-4's 80%).

**Integration with Roko:** When a gate rejects an agent's output (compile
failure, test failure, clippy warnings), the gate result should be converted
to a verbal reflection and injected into the next agent dispatch. This
closes the feedback loop between gates and agents.

```rust
/// Reflexion integration: convert gate failures to verbal reflections
/// that improve the next agent attempt.
pub struct ReflexionContext {
    /// Previous attempt's gate results, converted to natural language.
    pub reflections: Vec<Reflection>,
    /// Maximum reflections to include in context (default: 3).
    pub max_reflections: usize,
}

pub struct Reflection {
    pub attempt_number: usize,
    pub gate_name: String,
    pub failure_reason: String,
    /// LLM-generated summary of what went wrong and what to try differently.
    pub verbal_reflection: String,
    /// Timestamp for ordering.
    pub timestamp: SystemTime,
}

impl ReflexionContext {
    /// Generate a reflection from a gate failure.
    /// The verbal_reflection is generated by asking the agent:
    /// "What went wrong and what should you do differently next time?"
    pub fn from_gate_failure(
        attempt: usize,
        gate_name: &str,
        gate_output: &str,
    ) -> Reflection {
        Reflection {
            attempt_number: attempt,
            gate_name: gate_name.to_string(),
            failure_reason: gate_output.to_string(),
            verbal_reflection: String::new(), // Filled by LLM reflection step
            timestamp: SystemTime::now(),
        }
    }

    /// Format reflections for injection into the system prompt.
    pub fn as_prompt_section(&self) -> String {
        let mut s = String::from("## Previous Attempt Reflections\n\n");
        for r in self.reflections.iter().take(self.max_reflections) {
            s.push_str(&format!(
                "Attempt {}: Gate '{}' failed.\nReason: {}\nReflection: {}\n\n",
                r.attempt_number, r.gate_name, r.failure_reason, r.verbal_reflection
            ));
        }
        s
    }
}
```

### MCTS/LATS for High-Stakes Tasks

Language Agent Tree Search (Zhou et al., 2024, arXiv:2310.04406, ICML 2024)
combines MCTS with LLM-powered value functions. Achieves 92.7% pass@1 on
HumanEval. The CascadeRouter could route high-complexity tasks (Delta speed,
Premium tier) to MCTS-style execution and simpler tasks to basic ReAct.

```rust
/// Reasoning strategy selection based on task complexity.
/// The CascadeRouter picks the reasoning pattern, not just the model.
#[derive(Clone, Debug)]
pub enum ReasoningStrategy {
    /// Single-shot: one LLM call, no tool loop. For trivial tasks.
    Direct,
    /// Standard ReAct: iterative tool loop (current ToolLoop).
    ReAct {
        max_iterations: usize,  // Default: 25
    },
    /// Reflexion: ReAct + self-reflection on failures.
    Reflexion {
        max_iterations: usize,  // Default: 25
        max_attempts: usize,    // Default: 3 (retry with reflection)
    },
    /// Tree-of-Thought: explore K branches, evaluate, pick best.
    TreeOfThought {
        branching_factor: usize,  // Default: 3
        max_depth: usize,         // Default: 5
        evaluation: EvaluationMethod,
    },
    /// MCTS/LATS: full tree search with value function and backpropagation.
    Mcts {
        simulations: usize,       // Default: 10
        exploration_weight: f64,  // Default: 1.414 (sqrt(2), UCB1)
        max_depth: usize,         // Default: 10
    },
}

pub enum EvaluationMethod {
    /// LLM scores each branch (0.0–1.0).
    LlmScore,
    /// Multiple LLMs vote on the best branch.
    Voting { voters: usize },
    /// Gate pipeline evaluates each branch.
    GatePipeline,
}
```

---

## Tool Selection Optimization

Research shows that intelligent tool selection before the LLM requests tools
can dramatically reduce token consumption and improve accuracy.

### Tool RAG (Retrieval-Augmented Tool Selection)

Instead of stuffing all tool definitions into context, use dense vector
embeddings to retrieve only relevant tools per query. Across 121 tools from
5 MCP servers: **99.6% token reduction** while maintaining 97.1% hit rate
and 0.91 MRR (Red Hat, 2025; arXiv:2603.20313).

```rust
/// Tool RAG: retrieve relevant tools per-task instead of including all tools.
/// Uses roko-index HDC embeddings for semantic similarity.
pub struct ToolRag {
    /// HDC embeddings for all registered tools.
    tool_embeddings: Vec<(String, Vec<f32>)>,
    /// Top-K tools to retrieve per query (default: 10).
    top_k: usize,
    /// Minimum similarity threshold (default: 0.3).
    min_similarity: f64,
}

impl ToolRag {
    /// Retrieve the top-K most relevant tools for a given task.
    pub fn retrieve(&self, task_embedding: &[f32]) -> Vec<String> {
        // 1. Compute cosine similarity between task and all tool embeddings
        // 2. Filter by min_similarity threshold
        // 3. Return top-K tool names
        todo!()
    }
}
```

### AutoTool: Graph-Based Tool Prediction

Tool usage exhibits *inertia* — tools follow predictable sequential patterns
(e.g., "search" → "read" → "edit"). AutoTool (arXiv:2511.14650, AAAI 2026)
builds a directed graph from historical trajectories where nodes = tools and
edges = transition probabilities. Reduces LLM call count by 15–25% and total
token consumption by 10–40%.

```rust
/// Tool transition graph: predict likely next tools based on history.
/// Mined from EpisodeLogger data in .roko/episodes.jsonl.
pub struct ToolTransitionGraph {
    /// Markov chain: tool_name -> [(next_tool, probability)]
    transitions: HashMap<String, Vec<(String, f64)>>,
    /// Minimum probability to include a tool in predictions (default: 0.1).
    min_probability: f64,
}

impl ToolTransitionGraph {
    /// Build from episode history.
    pub fn from_episodes(episodes: &[Episode]) -> Self {
        // Count (tool_a -> tool_b) transitions across all episodes
        // Normalize counts to probabilities
        todo!()
    }

    /// Predict likely next tools given the most recently used tool.
    pub fn predict_next(&self, current_tool: &str, k: usize) -> Vec<String> {
        self.transitions.get(current_tool)
            .map(|t| t.iter().take(k).map(|(name, _)| name.clone()).collect())
            .unwrap_or_default()
    }
}
```

### Speculative Tool Execution (PASTE)

Microsoft Research (arXiv:2603.18897, 2025) shows that tool call sequences
exhibit stable control flows. PASTE speculatively executes likely next tools
**in parallel** with the LLM's reasoning. Reduces average task completion time
by 48.5% and improves tool execution throughput by 1.8×.

```rust
/// Speculative tool execution: run predicted tools in parallel with LLM reasoning.
/// When the LLM's actual tool call matches a speculated result, use the cached output.
pub struct SpeculativeExecutor {
    /// Tool transition graph for prediction.
    graph: ToolTransitionGraph,
    /// Minimum transition probability to speculate (default: 0.7).
    speculation_threshold: f64,
    /// Cache of speculatively executed results.
    speculative_cache: HashMap<String, ToolResult>,
}

impl SpeculativeExecutor {
    /// Before sending a turn to the LLM, speculatively execute high-probability next tools.
    pub async fn speculate(
        &mut self,
        current_tool: &str,
        dispatcher: &ToolDispatcher,
        ctx: &ToolContext,
    ) {
        let predictions = self.graph.predict_next(current_tool, 3);
        for tool_name in predictions {
            // Only speculate read-only tools (safe to execute without side effects)
            if dispatcher.is_read_only(&tool_name) {
                // Execute speculatively and cache the result
                // If the LLM requests this tool, use cached result instead of re-executing
            }
        }
    }
}
```

---

## Tool Result Caching

Research (ToolCacheAgent, ICLR 2026 submission; arXiv:2601.15335) shows that
intelligent tool result caching achieves 1.69× latency speedup without
accuracy loss.

### Cacheability Classification

Each tool should be annotated with a cacheability policy:

| Tool Category | Cacheable? | TTL | Invalidation |
|---|---|---|---|
| **Pure read** (file read, search) | Yes | Moderate (minutes) | On source change |
| **Computed/deterministic** (math, parse) | Yes | Long/infinite | Never (pure function) |
| **State-querying** (git status, ps) | Yes | Short (seconds) | On any write operation |
| **Write/mutating** (file write, POST) | Never | N/A | Invalidates related reads |
| **Time-dependent** (current time, live data) | Short TTL only | Very short (seconds) | Time-based |

```rust
/// Tool result caching with per-tool cacheability policies.
pub struct ToolResultCache {
    /// Cache entries keyed by (tool_name, args_hash).
    entries: HashMap<(String, u64), CacheEntry>,
    /// Per-tool cacheability policies.
    policies: HashMap<String, CachePolicy>,
}

pub struct CacheEntry {
    pub result: ToolResult,
    pub created_at: Instant,
    pub ttl: Duration,
}

#[derive(Clone, Debug)]
pub struct CachePolicy {
    /// Is this tool's output safe to cache?
    pub cacheable: bool,
    /// Time-to-live for cached results.
    pub ttl: Duration,
    /// Tools whose execution invalidates this tool's cache.
    pub invalidated_by: Vec<String>,
}

impl ToolResultCache {
    /// Look up a cached result. Returns None if cache miss or expired.
    pub fn get(&self, tool_name: &str, args: &serde_json::Value) -> Option<&ToolResult> {
        let key = (tool_name.to_string(), hash_args(args));
        self.entries.get(&key).and_then(|entry| {
            if entry.created_at.elapsed() < entry.ttl {
                Some(&entry.result)
            } else {
                None
            }
        })
    }

    /// Invalidate all cache entries affected by a write tool execution.
    pub fn invalidate_for(&mut self, tool_name: &str) {
        let to_remove: Vec<_> = self.policies.iter()
            .filter(|(_, policy)| policy.invalidated_by.contains(&tool_name.to_string()))
            .map(|(name, _)| name.clone())
            .collect();

        self.entries.retain(|(name, _), _| !to_remove.contains(name));
    }
}

/// Default cacheability policies for Roko's 19 builtin tools.
pub fn default_cache_policies() -> HashMap<String, CachePolicy> {
    let mut m = HashMap::new();
    // Read-only tools: cacheable with moderate TTL
    m.insert("read_file".into(), CachePolicy {
        cacheable: true,
        ttl: Duration::from_secs(300),
        invalidated_by: vec!["write_file".into(), "edit_file".into()],
    });
    m.insert("glob".into(), CachePolicy {
        cacheable: true,
        ttl: Duration::from_secs(300),
        invalidated_by: vec!["write_file".into()],
    });
    m.insert("grep".into(), CachePolicy {
        cacheable: true,
        ttl: Duration::from_secs(300),
        invalidated_by: vec!["write_file".into(), "edit_file".into()],
    });
    // Write tools: never cached, invalidate read caches
    m.insert("write_file".into(), CachePolicy {
        cacheable: false,
        ttl: Duration::ZERO,
        invalidated_by: vec![],
    });
    // Gate tools: cacheable until source changes
    m.insert("run_tests".into(), CachePolicy {
        cacheable: true,
        ttl: Duration::from_secs(60),
        invalidated_by: vec!["write_file".into(), "edit_file".into()],
    });
    m
}
```

### Agentic Plan Caching

Agentic Plan Caching (arXiv:2506.14852, 2025) extracts, stores, and reuses
structured plan templates across semantically similar tasks. Reduces costs by
50.31% and latency by 27.28%. This maps directly to Roko's playbook system
in `roko-learn`.

---

## Tool Use Benchmarks: State of the Art

Current tool-use benchmarks and Roko's position:

| Benchmark | Scale | Key metric | SOTA | Roko relevance |
|---|---|---|---|---|
| **BFCL v4** | Multi-language | AST accuracy | 0.885 (Llama 405B) | Function calling quality |
| **ToolBench** | 16,464 APIs | Pass rate | ~70% (GPT-4) | Multi-tool selection |
| **WildToolBench** | Real-world | Session accuracy | <15% | Production gap indicator |
| **API-Bank** | 73 APIs | Plan+Retrieve+Call | GPT-4 best | Multi-step planning |
| **ACEBench** | ~4,500 APIs | Overall accuracy | 86% (GPT-4) | Agent scenarios hardest |
| **MCP-Bench** | 127 tasks | Task completion | Varies | MCP integration quality |

**Key insight from WildToolBench:** No model achieves >15% session accuracy
on real-world tool use. The gap between synthetic benchmarks and production is
enormous. This validates Roko's harness engineering approach (sub-doc 08):
the harness matters more than the model.

---

## Citations

1. Yao, S. et al. (2023). "ReAct: Synergizing Reasoning and Acting in Language
   Models." ICLR 2023. arXiv:2210.03629. — ReAct pattern.
2. Shinn, N. et al. (2023). "Reflexion: Language Agents with Verbal
   Reinforcement Learning." NeurIPS 2023. arXiv:2303.11366. — Self-reflection.
3. Zhou, A. et al. (2024). "Language Agent Tree Search Unifies Reasoning,
   Acting, and Planning." ICML 2024. arXiv:2310.04406. — LATS/MCTS.
4. Yao, S. et al. (2023). "Tree of Thoughts: Deliberate Problem Solving with
   Large Language Models." NeurIPS 2023. arXiv:2305.10601. — ToT.
5. arXiv:2511.14650 (2025). "AutoTool: Efficient Tool Selection for LLM
   Agents." AAAI 2026. — Graph-based tool prediction.
6. arXiv:2603.18897 (2025). Microsoft Research. "PASTE: Pattern-Aware
   Speculative Tool Execution." — 48.5% latency reduction.
7. ToolCacheAgent (2025). ICLR 2026 submission. — 1.69× speedup via caching.
8. arXiv:2506.14852 (2025). "Agentic Plan Caching." — 50.31% cost reduction.
9. Red Hat (2025). "Tool RAG: Next Breakthrough in Scalable AI Agents."
   — 99.6% token reduction.
10. Patil, S. et al. (2025). "BFCL: Berkeley Function Calling Leaderboard."
    ICML 2025. — Tool use benchmark.
11. arXiv:2604.06185 (2025). "WildToolBench: Benchmarking LLM Tool-Use in
    the Wild." ICLR 2026. — <15% session accuracy.
12. `crates/roko-agent/src/tool_loop/mod.rs` — Full 769-line source.
13. `crates/roko-agent/src/dispatcher/mod.rs` — Full 1070-line source.
14. `crates/roko-agent/src/safety/mod.rs` — SafetyLayer, 6 policy families.
15. `crates/roko-agent/src/ollama_backend.rs` — OllamaLlmBackend reference.

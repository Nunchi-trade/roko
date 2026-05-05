# Harness and Format Engineering

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). The agent harness as a Pipeline of Cells: input normalization, prompt injection, safety check, LLM call, output parsing, and format translation. Provider format differences are adapter Cells implementing a common protocol.

## The Meta-Harness Thesis

Research shows that the **harness** -- the scaffolding around an LLM -- contributes more to agent performance than the model itself (Lee et al., 2026, arXiv:2603.28052). A better harness on a weaker model often outperforms a worse harness on a stronger model:

| Benchmark | Harness impact | Notes |
|---|---|---|
| Text classification | +7.7 accuracy points | Same model, better harness |
| IMO math | +4.7 points | Structured tool access + validation |
| Token efficiency | 4x fewer tokens | Context pruning + right-sized prompts |
| SWE-bench mobile | 6x performance gap | ref [46]; harness vs. no harness |

The "6x gap" is a specific SWE-bench mobile result, not a universal claim. The +7.7 and +4.7 numbers are more representative of typical harness impact. The takeaway: harness quality is consistently the largest lever for agent performance.

## The Harness as a Pipeline Graph

In unified terms, the agent harness is a **Pipeline** (linear Graph of Cells) that processes every inference request. Each Cell can reject (Verify), transform (Compose), or redirect (Route):

```
Signal(Task)
    |
    v
+-------------------+
| InputNormalize     |  Cell: Compose protocol
| (validate schema,  |  Rejects malformed inputs early.
|  enforce types)    |
+--------+----------+
         |
+--------v----------+
| PromptAssemble     |  Cell: Compose protocol (9-layer SystemPromptBuilder)
| (role + task +     |  Budget-constrained VCG section assembly.
|  tools + history)  |
+--------+----------+
         |
+--------v----------+
| SafetyVerify       |  Cell: Verify protocol (pre-action)
| (role auth,        |  Can veto: returns Verdict.reject if policy violated.
|  bash guard,       |  6 policies: bash, git, network, path, scrub, rate.
|  path escape)      |
+--------+----------+
         |
+--------v----------+
| FormatTranslate    |  Cell: Compose protocol
| (canonical ->      |  Selects translator by ModelCapabilities.tool_format.
|  wire format)      |  Bidirectional: render_tools + parse_calls.
+--------+----------+
         |
+--------v----------+
| LLMCall            |  Cell: Connect protocol
| (send_turn via     |  The actual provider call. Returns BackendResponse.
|  LlmBackend)       |  Handles retries, timeouts, error classification.
+--------+----------+
         |
+--------v----------+
| OutputParse        |  Cell: Compose protocol
| (wire -> canonical |  parse_calls extracts tool invocations.
|  ToolCall + text)  |  extract_reasoning separates thinking from output.
+--------+----------+
         |
+--------v----------+
| ToolDispatch       |  Cell: Pipeline of 7 steps
| (validate -> auth  |  Only fires if LLM requested tool calls.
|  -> execute ->     |  Loops back to FormatTranslate for next turn.
|  audit)            |
+--------+----------+
         |
    Signal(Result)
```

### Pipeline Properties

**Early exit**: SafetyVerify can reject before the LLM call, saving cost. InputNormalize can reject before prompt assembly.

**Feedback edge**: ToolDispatch loops back to FormatTranslate for multi-turn tool use. This makes the harness a **Loop** (Graph with feedback edge), not just a Pipeline. The loop terminates when the LLM emits no tool calls (FinishReason::Stop) or the turn budget is exhausted.

**Predict-publish-correct**: Each Cell can publish predictions. SafetyVerify publishes its veto rate (predicted vs. actual safety violations). FormatTranslate publishes parse success rate. LLMCall publishes latency predictions. The Bus carries these Pulses to CalibrationPolicy Cells.

## Six Harness Principles as Cell Properties

The Meta-Harness paper identifies six principles. Each maps to a Cell behavior:

### 1. Tools for the Model, Not for Humans

The **FormatTranslate** Cell ensures each model gets tools in its preferred wire format. The `Translator` trait is the Cell's internal implementation:

```rust
/// A FormatTranslate Cell selects one Translator at construction time.
/// The Translator is pure (no I/O, no side effects), making the Cell
/// deterministic and testable.
pub trait Translator: Send + Sync {
    fn format(&self) -> ToolFormat;
    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools;
    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>>;
    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults;
}
```

Four translators exist, each an adapter Cell:

| Translator | Wire Format | Models | Key Behavior |
|---|---|---|---|
| OpenAiTranslator | `openai_json` | OpenAI, DeepSeek, Gemini-compat | JSON function calling. `arguments` arrives as string, parsed to Value. |
| ClaudeTranslator | `anthropic_blocks` | Claude CLI | Stream-JSON `tool_use` blocks. Results handled by backend (HandledByBackend). |
| OllamaTranslator | `ollama_json` | Ollama, local models | Like OpenAI but `message` instead of `choices[0].message`. |
| ReActTranslator | `react_text` | Models without tool support | Embeds tool schemas in system prompt. Parses `Action:` / `Input:` regex from text. |

### 2. Right Context, Not More Context

The **PromptAssemble** Cell uses the `SystemPromptBuilder` (9-layer Compose Cell):

| Layer | What | Budget Control |
|---|---|---|
| 0: Global | Project name, structure | Always included |
| 1: Task | Task description, dependencies | Always included |
| 2: Role | Role-specific constraints | Always included |
| 3: Tools | Only tools available to this role | Filtered by role permissions |
| 4: History | Relevant previous outputs | Pruned by context budget |
| 5: Meta | Budget, time constraints | Always included |
| 6: Knowledge | Neuro store entries (if consulted) | VCG-ranked by relevance |
| 7: Affect | Daimon state, somatic markers | Compact (3 floats + label) |
| 8: Experiment | A/B prompt variants (if active) | One variant selected |

The `prune` submodule enforces context-growth guards: drop oldest tool results when the conversation approaches the context limit, preserving system prompt, first user message, most recent N messages, and error-bearing tool results.

### 3. Validate Before Executing

The **SafetyVerify** and **ToolDispatch** Cells implement pre-action Verify:

```
ToolDispatch 7-Step Pipeline:
  1. Schema validation (JSON schema from ToolDef)
  2. Tool resolution (name -> handler lookup)
  3. Permission check (role -> allowed tools)
  4. SafetyLayer policies (6 policies: bash, git, network, path, scrub, rate)
  5. Rate limit check
  6. Handler execution
  7. Audit Signal emission
```

Steps 1-5 fire before step 6. Each can reject with a typed error Signal. The safety pipeline is a Pipeline within a Pipeline -- fractal composition.

### 4. Compress History Intelligently

The **PromptAssemble** Cell's pruning strategy:

- System prompt: always preserved
- First user message: always preserved
- Most recent N messages: tail window (configurable)
- Tool results with errors: preserved (diagnostic value)
- Old successful tool results: dropped first

The `Checkpoint` specialization saves full conversation state for resume, so aggressive pruning does not lose data permanently.

### 5. Graduate Autonomy Based on Confidence

Autonomy graduation operates at two levels:

**Role level** (static): Validator (read-only) < Reviewer (read + comment) < Implementer (read + write + exec) < Conductor (orchestration).

**Model level** (dynamic): The CascadeRouter (a Route Cell with a Loop pattern) starts cheap and escalates. Tasks start at the cheapest model tier and escalate only when confidence is low. See [dual-process-and-efe-routing.md](dual-process-and-efe-routing.md) for the full EFE routing model.

### 6. Close the Feedback Loop

Four feedback mechanisms form the predict-publish-correct Loop:

| Mechanism | What | Persistence |
|---|---|---|
| EpisodeLogger | Every agent turn + gate result | `.roko/episodes.jsonl` |
| Efficiency events | Per-turn token/cost/time metrics | `.roko/learn/efficiency.jsonl` |
| CascadeRouter | Model routing decisions + outcomes | `.roko/learn/cascade-router.json` |
| Adaptive gate thresholds | EMA per rung, adjusting pass criteria | `.roko/learn/gate-thresholds.json` |

## Format Translation as Adapter Cells

Format translation is the pattern where provider differences are hidden behind a common protocol. In unified terms, each Translator is an adapter Cell implementing the Compose protocol:

```rust
/// Selection: capabilities → translator (one-time, at agent construction).
fn translator_for(capabilities: &ModelCapabilities) -> Arc<dyn Translator> {
    match capabilities.tool_format {
        ToolFormat::OpenAiJson      => Arc::new(OpenAiTranslator),
        ToolFormat::OllamaJson      => Arc::new(OllamaTranslator),
        ToolFormat::AnthropicBlocks => Arc::new(ClaudeTranslator),
        ToolFormat::ReActText       => Arc::new(ReActTranslator),
    }
}
```

### Wire Format Types

Three enums carry the bidirectional format data:

```rust
/// What the LLM receives (canonical -> wire).
pub enum RenderedTools {
    JsonArray(Value),           // OpenAI, Ollama, HTTP APIs
    CliFlag(String),            // Claude CLI (--tools=Read,Edit,Bash,...)
    SystemPromptBlock(String),  // ReAct fallback (tool schemas in system prompt)
}

/// What the next turn receives (tool results).
pub enum RenderedResults {
    JsonMessages(Value),        // OpenAI, Ollama (role: "tool" messages)
    TextBlock(String),          // ReAct ("Observation: ...")
    HandledByBackend,           // Claude CLI (drives own tool loop)
}

/// What the LLM returns (wire -> canonical).
pub enum BackendResponse {
    Json(Value),                // Single JSON (HTTP APIs)
    StreamJson(Vec<Value>),     // Stream events (Claude CLI)
    Text(String),               // Plain text (ReAct)
}
```

### Format Switching and Tool Count Limits

Some models degrade when tool count exceeds their native capacity (Qwen3-coder above 5, smaller Llama above 10). The `ModelCapabilities.max_tools` field triggers truncation **before** the Translator sees the tools:

1. Tools explicitly requested by task definition
2. Tools matching agent's role permissions
3. Most frequently used tools from episode history

This truncation is a Route Cell decision (selecting which tools to include), not a Translate Cell decision.

### Reasoning Extraction

`BackendResponse.extract_reasoning()` handles four different reasoning wire formats across providers. The extracted reasoning serves three purposes:
- **ChatResponse.reasoning** field for downstream consumers
- **Episode logging** -- reasoning logged separately for analysis
- **Cost computation** -- reasoning tokens may have different pricing

## The ToolDispatch Loop

The ToolDispatch Cell deserves special attention because it is where the harness Pipeline becomes a Loop:

```
                   +--> FormatTranslate --> LLMCall --> OutputParse --+
                   |                                                  |
                   |    (if tool_calls present)                       |
                   +-- ToolDispatch <---------------------------------+
                        |
                        | (if no tool_calls or budget exhausted)
                        v
                   Final Signal(Result)
```

### ToolDispatcher: 7-Step Inner Pipeline

The ToolDispatcher itself is a Pipeline of 7 Cells:

| Step | Cell Type | What | Failure Mode |
|---|---|---|---|
| 1. Schema validate | Verify | Check args against ToolDef JSON schema | Reject with schema error |
| 2. Resolve handler | Route | Name -> handler lookup | Reject with unknown tool |
| 3. Permission check | Verify | Role -> allowed tools | Reject with permission denied |
| 4. Safety policies | Verify (6x) | Bash guard, git guard, network guard, path guard, scrub, rate | Reject with safety violation |
| 5. Rate limit | Verify | Per-tool rate limiter | Reject with rate exceeded |
| 6. Execute | Connect | Run the handler | Handler-specific errors |
| 7. Audit | Observe | Emit audit Signal | Never fails |

Each step's rejection is a Verdict with specific evidence kind, enabling downstream analysis of what blocks agents most.

## Mori-Diffs Reality Check

Per the affect-routing reality analysis (12-AFFECT-ROUTING.md):

**What is wired today:**
- SystemPromptBuilder (9-layer) is used for prompt assembly in orchestrate.rs
- ToolDispatcher + SafetyLayer are wired into ToolLoop
- Format translation works for 4 translator types
- Episode logging and efficiency events emit on every turn
- CascadeRouter persists and is consulted for model selection

**What is not wired:**
- ToolDispatcher is not universal across all runtime paths (Claude CLI drives its own loop)
- Temperament config field exists but is not propagated to gate thresholds, tool selection, or routing
- Role prompts are ~1 sentence each (Mori used ~2K tokens per role)
- Context pruning is byte-based, not semantic

The harness Pipeline is **real but incomplete**: the individual Cells work, the wiring exists for the primary path, but not all backends flow through the full Pipeline.

---

## What This Enables

1. **Provider-agnostic harness quality** -- Adding a new LLM provider requires only a new Translator adapter Cell, not changes to the safety, routing, or feedback machinery.
2. **Measurable harness impact** -- Because each Cell in the Pipeline emits Pulses, the system can measure where latency, rejections, and cost accumulate, enabling targeted harness improvement.
3. **Compositional safety** -- The SafetyVerify Cell sits at a fixed point in the Pipeline. An agent cannot bypass it because the Pipeline topology is defined in the Graph, not in the agent's code.

## Feedback Loops

1. **Tool rejection rate -> tool description refinement**: High rejection rates at Schema Validate suggest the tool schema does not match what the model expects. The predict-publish-correct loop on this Cell can trigger tool description updates.
2. **Format parse failure rate -> translator improvement**: Parse failures in OutputParse indicate the translator does not handle a provider's response format correctly. EMA tracking surfaces this.
3. **Safety veto rate -> role permission calibration**: High veto rates for a role suggest permissions are too tight or the role prompt does not discourage prohibited actions. This feeds into the adaptive threshold Loop.
4. **Harness latency -> Pipeline optimization**: If PromptAssemble dominates latency (large context), the system can route to models with larger context windows or prune more aggressively.

## Open Questions

1. **Semantic pruning**: The current byte-based context pruning does not understand message importance. Should the prune Cell use a Score protocol (rate message relevance) before deciding what to drop?
2. **Cross-backend SafetyLayer**: Claude CLI drives its own tool loop, bypassing the ToolDispatcher. Should safety policies be applied at the orchestrator level (pre-prompt validation) to cover all backends uniformly?
3. **Format switching**: Some models degrade above a tool count threshold, but the threshold is static (`max_tools` in config). Should the harness detect degradation dynamically and reduce tool count mid-session?
4. **Role prompt depth**: Current role prompts are ~1 sentence. How much of Mori's ~2K-token role prompts can be recovered without manual authoring? Could a Compose Cell generate role-specific behavioral instructions from examples in the episode log?

---

## Citations

1. Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM Agents." arXiv:2603.28052.
2. Jimenez, C. E. et al. (2024). "SWE-bench: Can Language Models Resolve Real-World GitHub Issues?"
3. `crates/roko-agent/src/dispatcher/mod.rs` -- 7-step ToolDispatcher pipeline.
4. `crates/roko-agent/src/translate/mod.rs` -- Translator trait and wire format types.
5. `crates/roko-compose/src/system_prompt_builder.rs` -- 9-layer prompt assembly.
6. `crates/roko-agent/src/safety/mod.rs` -- SafetyLayer (6 policies).
7. `crates/roko-agent/src/tool_loop/prune.rs` -- Context pruning.
8. See [02-CELL.md](../../unified/02-CELL.md) for protocol definitions (Verify, Compose, Route, Connect, Observe).

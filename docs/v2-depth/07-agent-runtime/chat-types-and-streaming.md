# Chat Types and Streaming

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). How provider-specific LLM response formats are normalized to canonical Signal types, how the translate layer converts between wire formats and the unified type system, and how streaming works across protocol families.

---

## 1. The Normalization Problem

Every LLM provider returns responses in a different format. Anthropic uses content blocks with typed entries. OpenAI uses a `choices` array with message objects. Ollama wraps content in a `message` field. Claude CLI emits a stream of JSON events over subprocess pipes. Perplexity adds `citations` alongside the response. Gemini adds `grounding_metadata`.

Without normalization, every consumer of agent output would need to understand every provider's wire format. The translate layer solves this by converting all wire formats to a single canonical type -- `ChatResponse` -- before any downstream processing occurs.

In unified terms, this is a set of adapter Cells implementing the **Score protocol** (evaluate and normalize) that sit between the raw Connect protocol output (provider HTTP responses) and the internal Signal processing pipeline.

---

## 2. ChatResponse: The Canonical Response Signal

`ChatResponse` is the normalized output of any LLM interaction, regardless of provider:

```rust
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// The assistant's text content (concatenated from all text blocks).
    pub content: String,
    /// Reasoning/thinking content (if model supports extended thinking).
    pub reasoning: Option<String>,
    /// Tool calls emitted by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage metrics.
    pub usage: Usage,
    /// Why the model stopped generating.
    pub finish_reason: FinishReason,
    /// Provider-specific metadata that doesn't fit the canonical model.
    pub metadata: ResponseMetadata,
}
```

### Field semantics

**`content`**: For providers returning structured content blocks (Anthropic), the translator extracts and concatenates all `type: "text"` blocks. For providers returning a single string (OpenAI, Ollama), it is extracted directly.

**`reasoning`**: Extended thinking / chain-of-thought output. Populated when `ModelProfile::supports_thinking` is true. Extracted by `BackendResponse::extract_reasoning()`, which handles four wire formats:

| Wire format | Provider | JSON path |
|---|---|---|
| OpenAI-style | DeepSeek, QwQ | `message.reasoning_content` |
| Anthropic-style | Claude (API) | `content[].type == "thinking"` |
| Stream-JSON events | Claude (CLI) | `delta.reasoning_content` or `delta.thinking` |
| Content block events | Claude (API streaming) | `content_block.type == "thinking"` |

**`tool_calls`**: Parsed tool invocations, normalized to `ToolCall { id, name, arguments }` from `roko-core::tool`. The `ToolCall` struct is the canonical Signal type for tool requests, independent of whether the provider uses OpenAI-style function calling, Anthropic-style content blocks, or text-based ReAct parsing.

**`finish_reason`**: See section 3 below.

**`metadata`**: See section 4 below.

---

## 3. FinishReason: Normalized Stop Conditions

Every provider names stop conditions differently. The `FinishReason` enum normalizes them:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FinishReason {
    #[default]
    Stop,           // Model finished generating (natural end)
    Length,         // Hit max_tokens limit
    ToolCalls,      // Model wants to call tools (loop continues)
    ContentFilter,  // Content policy triggered
    Error(String),  // Provider-specific error
}
```

### Cross-provider normalization table

| Canonical | OpenAI | Anthropic | ZhipuAI | Perplexity |
|---|---|---|---|---|
| `Stop` | `"stop"` | `"end_turn"` | `"stop"` | `"stop"` |
| `Length` | `"length"` | `"max_tokens"` | `"length"` | `"length"` |
| `ToolCalls` | `"tool_calls"` | `"tool_use"` | `"tool_calls"` | -- |
| `ContentFilter` | `"content_filter"` | -- | `"sensitive"` | -- |
| `Error(...)` | -- | -- | `"network_error"` | -- |

The `normalize_finish_reason()` function:

```rust
pub fn normalize_finish_reason(raw: &str) -> FinishReason {
    match raw {
        "stop" | "end_turn"             => FinishReason::Stop,
        "length" | "max_tokens"         => FinishReason::Length,
        "tool_calls" | "tool_use"       => FinishReason::ToolCalls,
        "content_filter" | "sensitive"  => FinishReason::ContentFilter,
        "network_error"                 => FinishReason::Error("network_error".into()),
        "model_context_window_exceeded" => FinishReason::Error("context_overflow".into()),
        other                           => FinishReason::Error(other.to_string()),
    }
}
```

This normalization is critical for the ToolLoop: when `finish_reason` is `ToolCalls`, the loop knows to dispatch tool calls and continue. When it is `Stop`, the loop extracts the final answer.

---

## 4. ResponseMetadata: Provider Extensions

Provider-specific fields that are too specialized to normalize yet live in `ResponseMetadata`:

```rust
#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    pub response_id: Option<String>,         // Unique ID from provider
    pub model_used: Option<String>,          // Actual model (may differ from requested)
    pub cached_tokens: Option<u64>,          // Tokens served from cache
    pub content_filter: Option<Value>,       // Provider-specific filter details
    pub web_search: Option<Value>,           // Perplexity citations, Gemini grounding
    pub provider_latency_ms: Option<u64>,    // Provider-reported latency
    pub raw_finish_reason: Option<String>,   // Pre-normalization string
}
```

Notable fields:

- **`model_used`**: When using OpenRouter, the actual model may differ from the requested one (OpenRouter routes across providers). This field captures the actual model for cost attribution and quality tracking.
- **`cached_tokens`**: Anthropic prompt caching reports tokens served from cache. These cost `cost_cache_read_per_m` (10% of normal input rate) instead of `cost_input_per_m`.
- **`web_search`**: Perplexity Sonar models return `citations`, `search_results`, and `annotations`. Gemini returns `grounding_metadata`. Both are captured as raw JSON for downstream consumers.

The extensibility pattern: add the field to metadata first as `Option<Value>`, prove it is useful across multiple consumers, then promote it to a first-class typed field.

---

## 5. BackendResponse: The Raw Wire Layer

Below `ChatResponse` sits `BackendResponse`, which represents raw bytes off the wire before normalization:

```rust
pub enum BackendResponse {
    /// Single JSON object (Ollama, OpenAI, Anthropic non-streaming).
    Json(serde_json::Value),
    /// Sequence of stream-JSON events (Claude CLI).
    StreamJson(Vec<serde_json::Value>),
    /// Plain-text completion (ReAct text models).
    Text(String),
}
```

### Text extraction across JSON shapes

```rust
impl BackendResponse {
    pub fn extract_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Json(v) =>
                v.pointer("/message/content")            // Ollama shape
                 .or_else(|| v.pointer("/choices/0/message/content")) // OpenAI shape
                 .and_then(|x| x.as_str())
                 .unwrap_or("")
                 .to_string(),
            Self::StreamJson(events) => {
                // Concatenate delta/text and content_block/text events
                // ...
            }
        }
    }
}
```

---

## 6. The Translate Pipeline

The flow from raw wire response to canonical `ChatResponse`:

```
Provider API Response (JSON / stream / text)
    |
    v
BackendResponse (raw wire representation)
    |
    +-- extract_text()      -> String
    +-- extract_reasoning() -> Option<String>
    |
    v
Translator::parse_calls() -> Vec<ToolCall>
    |
    v
ChatResponse {
    content:       extract_text(),
    reasoning:     extract_reasoning(),
    tool_calls:    parse_calls(),
    usage:         parsed from response usage block,
    finish_reason: normalize_finish_reason(raw),
    metadata:      provider-specific extensions,
}
```

This pipeline runs once per LLM response in the adapter layer. The `ToolLoop` receives `BackendResponse` from `LlmBackend::send_turn()` and uses the `Translator` to parse tool calls. Full `ChatResponse` assembly happens when the final result is surfaced to the orchestrator.

The `Translator` trait selects the right parsing strategy based on `ModelProfile::tool_format`:

| `tool_format` value | Translator impl | Used by |
|---|---|---|
| `"openai_json"` | OpenAI function calling parser | OpenAI, ZhipuAI, Perplexity, Gemini |
| `"anthropic_blocks"` | Anthropic content block parser | Anthropic API |
| `"ollama_json"` | Ollama message format parser | Ollama |
| `"react_text"` | Text-based ReAct parser (regex) | Legacy text-only models |

---

## 7. Streaming as Pulse Sequences

In unified terms, streaming maps to **Pulse sequences on Bus**. Each streaming event is an ephemeral Pulse that flows through the Bus fabric. The final assembled `ChatResponse` is a durable Signal that persists in Store.

### Protocol-specific streaming formats

| Protocol family | Streaming mechanism | Event format |
|---|---|---|
| Anthropic API | Server-Sent Events (SSE) | `message_start`, `content_block_delta`, `message_delta`, `message_stop` |
| Claude CLI | Stream-JSON over subprocess pipes | `system`, `assistant`, `result` JSON lines with `type` field |
| OpenAI Compat | Server-Sent Events (SSE) | `data: {...}` with `choices[0].delta` |
| Cursor ACP | JSON-RPC notifications | Event-type discriminated messages |

### Claude CLI stream-JSON protocol (primary path today)

The runner's `parse_stream_line()` function processes Claude CLI events:

```
system       -> SystemInit Pulse (model, capabilities)
assistant    -> MessageDelta Pulse (text content, tool calls)
result       -> TurnCompleted Pulse (final text, usage, session ID)
```

Each line is a JSON object with a `type` field. The runner accumulates deltas into a final `AgentEvent` that maps to `ChatResponse`.

### The layer problem

`ChatResponse` currently lives in `roko-agent::translate` rather than `roko-core`. This creates a dependency issue: `roko-compose` needs to reason about response shape when assembling multi-turn prompts (e.g., knowing if the last turn had reasoning blocks, cached token counts, finish reasons for continuations), but it cannot depend on `roko-agent` without creating a circular dependency.

```
Current:
  roko-compose  --depends-on--> roko-core  (OK)
  roko-agent    --depends-on--> roko-core  (OK)
  roko-compose  --CANNOT-->     roko-agent (circular!)

  ChatResponse lives in roko-agent::translate

Solution:
  Move ChatResponse, FinishReason, ResponseMetadata to roko-core::types
  Both roko-compose and roko-agent import from roko-core
```

Until this migration, `roko-compose` works with raw `Signal` metadata tags and JSON values rather than typed `ChatResponse` structs.

---

## 8. Mori-Diffs Reality

The mori-diff notes that the runner's stream parser (`parse_stream_line`) only understands Claude's `stream-json` protocol. For provider-agnostic dispatch, an `AgentEventStream` trait is needed:

```rust
/// Adapter between different agent output formats and the normalized event protocol.
pub trait AgentEventStream: Send {
    async fn next_event(&mut self) -> Option<AgentEvent>;
}
```

For Claude CLI, the existing `parse_stream_line()` becomes the implementation. For API-based backends, the `Agent::run()` return value would be mapped to synthetic `AgentEvent`s (Started -> MessageDelta -> TurnCompleted).

---

## What This Enables

- **Provider-agnostic consumption**: Any downstream system (orchestrator, TUI, episode logger) works with `ChatResponse` regardless of which provider generated it
- **Transparent reasoning extraction**: Extended thinking content from Claude, DeepSeek, o3, and Gemini is uniformly accessible via `reasoning: Option<String>`
- **Correct tool loop termination**: `FinishReason::ToolCalls` vs `FinishReason::Stop` drives the ToolLoop's continue/stop decision across all providers
- **Cost attribution**: `cached_tokens` and `model_used` enable accurate cost computation even when providers route or cache transparently

## Feedback Loops

1. **FinishReason -> ToolLoop control**: `ToolCalls` continues the loop; `Stop` terminates it. `Length` triggers context pruning and retry. This is a tight control loop where the normalized response directly governs execution flow.
2. **ResponseMetadata -> CascadeRouter**: `model_used` (for OpenRouter) and `cached_tokens` feed back into routing decisions. If caching is effective for a provider, the router can prefer it.
3. **Usage -> Demurrage**: Token costs flow through Usage into the efficiency tracker, which influences demurrage rates on knowledge Signals. Expensive-to-produce knowledge decays more slowly.
4. **ChatResponse -> Episode logging**: Every `ChatResponse` becomes rows in `.roko/episodes.jsonl` via the EpisodeLogger. The reasoning field, when present, is logged separately for reflection analysis.

## Open Questions

1. **ChatResponse migration to roko-core**: This is tracked as a Tier 1 task but not yet completed. The workaround (raw JSON values in roko-compose) is error-prone. Should this block other work?

2. **Streaming event normalization**: The mori-diff proposes `AgentEventStream` but it does not exist yet. For HTTP backends that return a complete response (not streaming), should they emit synthetic streaming events to unify the consumption pattern, or should the consumer handle both streaming and one-shot patterns?

3. **Reasoning content handling**: Different models produce reasoning content in different quality levels. Should `reasoning` be a scored Signal (with a Score protocol evaluation) rather than a raw string, so downstream consumers can assess reasoning quality?

4. **ResponseMetadata promotion criteria**: When should an `Option<Value>` metadata field be promoted to a first-class typed field? Current practice is ad-hoc. A threshold (used by 3+ consumers, or present in 3+ providers) would make this systematic.

---

## Citations

1. `crates/roko-agent/src/translate/mod.rs` -- ChatResponse, FinishReason, ResponseMetadata, BackendResponse, Translator trait.
2. `crates/roko-core/src/config/schema.rs` -- ModelProfile with supports_thinking, tool_format flags.
3. `crates/roko-cli/src/runner/agent_stream.rs` -- parse_stream_line for Claude CLI stream-JSON.
4. `tmp/mori-diffs/01-AGENT-DISPATCH.md` -- AgentEventStream proposal.
5. Anthropic API docs -- SSE streaming, content blocks, thinking parameter.
6. OpenAI API docs -- Chat completions streaming, structured outputs.

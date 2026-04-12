# 03 — Chat Types in roko-core

> Sub-doc 03 of **02-agents** · Roko Documentation
>
> This document explains why `ChatResponse`, `FinishReason`, and
> `ResponseMetadata` exist in `roko-agent/src/translate/mod.rs` rather than
> in `roko-core`, why they **must eventually live in roko-core**, and the
> current workaround. It also documents the canonical response types and
> their relationship to provider-specific wire formats.

---

## The Layer Problem

Roko's crate dependency graph enforces strict layering:

```
roko-core           (L0 — no agent dependencies)
    ↓
roko-compose        (L2 — assembles prompts, depends on roko-core)
    ↓
roko-agent          (L1/L2 — agent backends, depends on roko-core)
    ↓
roko-cli            (L4 — orchestration, depends on everything)
```

The `ChatResponse` type — a canonical representation of any LLM provider's
response — is currently defined in `roko-agent::translate`. This creates a
problem: `roko-compose` needs to reason about response shape when assembling
multi-turn prompts, but it cannot depend on `roko-agent` without creating a
circular dependency. The `SystemPromptBuilder` in `roko-compose` needs to
know about reasoning/thinking blocks, cached tokens, and finish reasons to
assemble context-aware prompts.

**The resolution:** `ChatResponse`, `FinishReason`, `ResponseMetadata`, and
the `normalize_finish_reason` function **must live in roko-core** so that
both `roko-compose` and `roko-agent` can depend on them. This migration is
tracked as part of the Tier 1 implementation priorities in the refactoring
PRD §07-implementation-priorities.

For now, the types live in `roko-agent::translate::mod.rs` and the compose
layer works around the limitation by operating on raw `Signal` metadata
rather than typed `ChatResponse` structs.

---

## ChatResponse — The Canonical Response Type

Defined at `crates/roko-agent/src/translate/mod.rs:55`:

```rust
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// The assistant's text content.
    pub content: String,
    /// Reasoning/thinking content, if the model supports it.
    pub reasoning: Option<String>,
    /// Tool calls emitted by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage metrics.
    pub usage: Usage,
    /// Why the model stopped generating.
    pub finish_reason: FinishReason,
    /// Provider-specific metadata.
    pub metadata: ResponseMetadata,
}
```

`ChatResponse` is the **canonical output** of any LLM interaction, regardless
of provider. Every adapter parses its provider's wire format into this struct
before any downstream processing occurs. This normalization is the fundamental
design principle of the translate layer: callers never deal with
provider-specific JSON shapes; they always work with `ChatResponse`.

### Fields in detail

**`content: String`** — The assistant's final text. For models that return
structured content blocks (like Anthropic's Messages API), the translator
extracts and concatenates all `type: "text"` blocks into this field.

**`reasoning: Option<String>`** — Extended thinking / chain-of-thought
output. Populated when `ModelProfile::supports_thinking` is true and the
model returns reasoning content. Extracted by `BackendResponse::extract_reasoning()`,
which handles three wire formats:
- OpenAI-style `reasoning_content` field on the message object
- Anthropic-style `content` blocks with `type: "thinking"`
- Stream-JSON events with `thinking_delta` types

**`tool_calls: Vec<ToolCall>`** — Parsed tool invocations. The `ToolCall`
struct is defined in `roko-core::tool` and carries `id`, `name`, and
`arguments` (as `serde_json::Value`).

**`usage: Usage`** — Token counts and cost. The `Usage` struct tracks
input tokens, output tokens, cache read/write tokens, estimated cost, and
wall-clock duration.

**`finish_reason: FinishReason`** — Why the model stopped. See below.

**`metadata: ResponseMetadata`** — Provider-specific extensions that don't
fit the canonical model. See below.

---

## FinishReason — Normalized Stop Conditions

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FinishReason {
    #[default]
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error(String),
}
```

Every provider has its own names for stop conditions:

| Roko canonical | OpenAI | Anthropic | ZhipuAI | Perplexity |
|---|---|---|---|---|
| `Stop` | `"stop"` | `"end_turn"` | `"stop"` | `"stop"` |
| `Length` | `"length"` | `"max_tokens"` | `"length"` | `"length"` |
| `ToolCalls` | `"tool_calls"` | `"tool_use"` | `"tool_calls"` | — |
| `ContentFilter` | `"content_filter"` | — | `"sensitive"` | — |
| `Error(...)` | — | — | `"network_error"` | — |

The `normalize_finish_reason` function at line 87 maps raw strings to
canonical variants:

```rust
pub fn normalize_finish_reason(raw: &str) -> FinishReason {
    match raw {
        "stop" | "end_turn"                    => FinishReason::Stop,
        "length" | "max_tokens"                => FinishReason::Length,
        "tool_calls" | "tool_use"              => FinishReason::ToolCalls,
        "content_filter" | "sensitive"         => FinishReason::ContentFilter,
        "network_error"                        => FinishReason::Error("network_error".into()),
        "model_context_window_exceeded"        => FinishReason::Error("context_overflow".into()),
        other                                  => FinishReason::Error(other.to_string()),
    }
}
```

This normalization is critical for the ToolLoop: when `finish_reason` is
`ToolCalls`, the loop knows to dispatch tool calls and continue iterating.
When it's `Stop`, the loop knows the model has finished and extracts the
final answer.

---

## ResponseMetadata — Provider Extensions

```rust
#[derive(Debug, Clone, Default)]
pub struct ResponseMetadata {
    /// Unique response ID from the provider.
    pub response_id: Option<String>,
    /// Actual model used (may differ from requested, e.g., OpenRouter routing).
    pub model_used: Option<String>,
    /// Number of cached tokens served (Anthropic prompt caching).
    pub cached_tokens: Option<u64>,
    /// Content filter details (provider-specific JSON).
    pub content_filter: Option<serde_json::Value>,
    /// Web search / grounding results (Perplexity citations, Gemini grounding).
    pub web_search: Option<serde_json::Value>,
    /// Provider-reported latency.
    pub provider_latency_ms: Option<u64>,
    /// Raw finish reason string before normalization.
    pub raw_finish_reason: Option<String>,
}
```

`ResponseMetadata` is intentionally loose — it uses `Option<Value>` for
fields that are too provider-specific to normalize yet. This follows the
extensibility principle: add the field to metadata first, prove it's useful,
then promote it to a first-class type.

Notable fields:

- **`model_used`** — When using OpenRouter, the actual model that served the
  request may differ from the requested one (e.g., OpenRouter may route
  `claude-opus-4-6` to a different provider's instance). This field captures
  the actual model for cost attribution and quality tracking.

- **`cached_tokens`** — Anthropic's prompt caching returns the number of
  tokens served from cache. This feeds into the `Usage` cost computation:
  cached tokens cost `cost_cache_read_per_m` instead of `cost_input_per_m`.

- **`web_search`** — Perplexity Sonar models return `citations`,
  `search_results`, and `annotations` alongside the response. Gemini returns
  `grounding_metadata`. Both are captured as raw JSON for downstream consumers.

---

## BackendResponse — The Raw Wire Layer

Below `ChatResponse` sits `BackendResponse`, which represents the raw bytes
off the wire before any normalization:

```rust
pub enum BackendResponse {
    /// Single JSON object (Ollama, OpenAI, Anthropic non-streaming).
    Json(serde_json::Value),
    /// Sequence of stream-json events (Claude CLI).
    StreamJson(Vec<serde_json::Value>),
    /// Plain-text completion (ReAct models).
    Text(String),
}
```

Each `Translator` implementation knows how to:
1. Extract text from its variant (`extract_text()`)
2. Extract reasoning from its variant (`extract_reasoning()`)
3. Parse tool calls from its variant (`parse_calls()`)

The `extract_text()` method handles the three main JSON shapes:

```rust
impl BackendResponse {
    pub fn extract_text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Json(v) => v.pointer("/message/content")        // Ollama shape
                .or_else(|| v.pointer("/choices/0/message/content")) // OpenAI shape
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            Self::StreamJson(events) => {
                // Concatenate delta/text and content_block/text events
                ...
            }
        }
    }
}
```

### Reasoning extraction

The `extract_reasoning()` method handles four different reasoning wire formats:

1. **OpenAI-style:** `message.reasoning_content` field (DeepSeek, QwQ)
2. **Anthropic-style:** `content` array with `type: "thinking"` blocks
3. **Stream-JSON events:** `delta.reasoning_content` or `delta.thinking`
4. **Content block events:** `content_block.type == "thinking"` with
   `thinking` or `text` sub-fields

This complexity is why reasoning extraction lives in the translate layer
rather than in individual adapters — it's a cross-cutting concern that
every HTTP backend needs.

---

## Why These Types Must Move to roko-core

The migration argument, in dependency terms:

```
Current:
  roko-compose  ──depends-on──→  roko-core  (OK)
  roko-agent    ──depends-on──→  roko-core  (OK)
  roko-compose  ──CANNOT──→  roko-agent     (circular!)

  ChatResponse lives in roko-agent::translate

Problem:
  roko-compose::SystemPromptBuilder needs ChatResponse to:
  - Know if the last turn had reasoning (to include/exclude thinking blocks)
  - Know cached_tokens (to decide prompt caching strategy)
  - Know finish_reason (to handle continuations vs fresh prompts)

Solution:
  Move ChatResponse, FinishReason, ResponseMetadata, normalize_finish_reason
  to roko-core::types (or roko-core::chat)

  Both roko-compose and roko-agent then import from roko-core.
```

The refactoring PRD §07-implementation-priorities tracks this as a Tier 1
task: "Chat types must live in roko-core (not roko-agent) because
roko-compose needs them."

Until the migration, `roko-compose` works with `Signal` metadata tags
and JSON values rather than typed `ChatResponse` structs, which is
error-prone but functional.

---

## The Translate Layer Pipeline

The flow from raw wire response to canonical `ChatResponse`:

```
Provider API Response (JSON/stream/text)
    │
    ▼
BackendResponse (raw wire representation)
    │
    ├── extract_text()      → String
    ├── extract_reasoning() → Option<String>
    │
    ▼
Translator::parse_calls()  → Vec<ToolCall>
    │
    ▼
ChatResponse {
    content:       extract_text(),
    reasoning:     extract_reasoning(),
    tool_calls:    parse_calls(),
    usage:         parsed from response usage block,
    finish_reason: normalize_finish_reason(raw),
    metadata:      provider-specific extensions,
}
```

This pipeline runs once per LLM response, in the adapter layer. The
`ToolLoop` receives `BackendResponse` from `LlmBackend::send_turn()`
and uses the `Translator` to parse tool calls. The full `ChatResponse`
assembly happens when the final result is surfaced to the orchestrator.

---

## Citations

1. Implementation plan `modelrouting/04-translator-extensions.md` —
   ChatResponse canonical type, FinishReason normalization, reasoning
   extraction, cached token parsing.
2. Refactoring PRD §07-implementation-priorities — Tier 1: Chat types must
   live in roko-core.
3. `crates/roko-agent/src/translate/mod.rs` — Full 548-line source with
   ChatResponse, FinishReason, ResponseMetadata, BackendResponse,
   Translator trait.
4. `crates/roko-core/src/config/schema.rs` — ModelProfile with
   supports_thinking, supports_search, supports_citations flags.
5. Refactoring PRD §01-synapse-architecture — Layer dependency rules.

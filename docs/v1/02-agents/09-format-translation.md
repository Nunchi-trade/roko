# 09 — Format Translation

> Sub-doc 09 of **02-agents** · Roko Documentation
>
> This document describes the `Translator` trait, the four translator
> implementations (Claude, Ollama, OpenAI, ReAct), the wire format types,
> and why format-aware translation is critical for agent performance.


> **Implementation**: Shipping

---

## Why Format Translation Matters

Research shows 5–30 accuracy points difference when using the wrong tool-call
format for a model. This is documented in:

- **Meta-Harness** (Lee et al., 2026, arXiv:2603.28052): Principle #1 —
  "Design tools for the model, not for humans."
- **WildToolBench**: Format-specific accuracy drops of 15–20% for models
  tested with non-native tool formats.
- **Qwen3-coder**: Documented format switch above 5 tools — performance
  degrades when the tool array exceeds the model's native tool-call capacity.

Each model family has a preferred wire format:

| Model family | Native format | `tool_format` value |
|---|---|---|
| Claude (API) | Anthropic content blocks | `anthropic_blocks` |
| Claude (CLI) | `--tools=Name,Name` flag | CLI flag |
| OpenAI / GPT | JSON function calling | `openai_json` |
| Ollama / Llama | OpenAI-compatible JSON | `ollama_json` |
| DeepSeek | OpenAI-compatible JSON | `openai_json` |
| Gemini | OpenAI-compatible JSON* | `openai_json` |
| Models without tool support | ReAct in system prompt | `react_text` |

*Gemini's native API has its own format, but the OpenAI-compatible endpoint
uses standard JSON function calling.

The `Translator` layer ensures each model gets tools in the format it
prefers. The `ModelProfile::tool_format` field is the selection key.

---

## The Translator Trait

Defined at `crates/roko-agent/src/translate/mod.rs:103`:

```rust
pub trait Translator: Send + Sync {
    /// Which wire format this translator emits/parses.
    fn format(&self) -> ToolFormat;

    /// Serialize the tool catalog into the backend's expected shape.
    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools;

    /// Parse the backend's response into canonical tool calls.
    fn parse_calls(&self, response: &BackendResponse)
        -> Result<Vec<ToolCall>, TranslatorError>;

    /// Serialize tool results for the next turn.
    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults;

    /// Extract assistant message for conversation history injection.
    fn render_assistant_message(&self, response: &BackendResponse)
        -> Option<serde_json::Value> {
        None  // Default: no-op
    }
}
```

Key design properties:
- **Sync and pure** — No I/O, no side effects. Given identical inputs,
  identical outputs. This makes translators easy to test.
- **One instance per backend** — The translator is selected once at agent
  construction time and used for all turns.
- **Bidirectional** — `render_tools` goes from canonical → wire;
  `parse_calls` goes from wire → canonical.

---

## The Four Translators

### 1. OpenAiTranslator (`translate/openai.rs`)

The workhorse translator. Handles the OpenAI chat completions tool format
used by most providers.

**`render_tools`** — Converts `ToolDef` to the OpenAI `functions` array:

```json
{
    "type": "function",
    "function": {
        "name": "read_file",
        "description": "Read a file from the filesystem",
        "parameters": { /* JSON Schema from ToolDef */ }
    }
}
```

**`parse_calls`** — Extracts tool calls from `choices[0].message.tool_calls`:

```json
{
    "tool_calls": [
        {
            "id": "call_abc123",
            "type": "function",
            "function": {
                "name": "read_file",
                "arguments": "{\"path\": \"/src/main.rs\"}"
            }
        }
    ]
}
```

Note: OpenAI returns `arguments` as a JSON string, not a parsed object.
The translator parses it into `serde_json::Value`.

**`render_results`** — Formats tool results as `role: "tool"` messages:

```json
{
    "role": "tool",
    "tool_call_id": "call_abc123",
    "content": "file contents here..."
}
```

### 2. OllamaTranslator (`translate/ollama.rs`)

Similar to OpenAI but handles Ollama's slightly different JSON structure
(messages are under `message` instead of `choices[0].message`).

### 3. ClaudeTranslator (`translate/claude.rs`)

Handles Claude CLI's stream-JSON protocol:

**`render_tools`** — Returns `RenderedTools::CliFlag("Read,Edit,Bash,...")`
for the `--tools` flag.

**`parse_calls`** — Parses `tool_use` blocks from stream-JSON events:

```json
{
    "type": "tool_use",
    "id": "toolu_abc123",
    "name": "read_file",
    "input": { "file_path": "/src/main.rs" }
}
```

**`render_results`** — Returns `RenderedResults::HandledByBackend` because
Claude CLI manages its own tool-call loop internally. Roko doesn't feed
results back.

### 4. ReActTranslator (`translate/react.rs`)

Fallback for models without native function calling support. Embeds tool
schemas directly in the system prompt and parses tool calls from the
model's natural language output.

**`render_tools`** — Returns `RenderedTools::SystemPromptBlock(...)`:

```text
You have access to the following tools:

### read_file
Read a file from the filesystem.
Parameters:
- path (string, required): The file path to read

To use a tool, respond with:
Action: tool_name
Input: {"param": "value"}
```

**`parse_calls`** — Uses regex to extract `Action:` and `Input:` lines
from the model's text output.

**`render_results`** — Returns `RenderedResults::TextBlock(...)`:

```text
Observation: [file contents here]
```

---

## Wire Format Types

### RenderedTools

```rust
pub enum RenderedTools {
    JsonArray(serde_json::Value),    // OpenAI, Ollama, HTTP APIs
    CliFlag(String),                 // Claude CLI (--tools=...)
    SystemPromptBlock(String),       // ReAct fallback
}
```

### RenderedResults

```rust
pub enum RenderedResults {
    JsonMessages(serde_json::Value), // OpenAI, Ollama (tool result messages)
    TextBlock(String),               // ReAct (Observation: ...)
    HandledByBackend,                // Claude CLI (drives own loop)
}
```

### BackendResponse

```rust
pub enum BackendResponse {
    Json(serde_json::Value),         // Single JSON (HTTP APIs)
    StreamJson(Vec<serde_json::Value>), // Stream events (Claude CLI)
    Text(String),                    // Plain text (ReAct)
}
```

---

## ModelCapabilities and Translator Selection

The `capability` submodule (`translate/capability.rs`) provides the bridge
between model profiles and translator selection:

```rust
pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_thinking: bool,
    pub supports_vision: bool,
    pub tool_format: ToolFormat,
    pub max_tools: Option<u32>,
}
```

The `translator_for` function selects the appropriate translator based on
capabilities:

```rust
pub fn translator_for(capabilities: &ModelCapabilities) -> Arc<dyn Translator> {
    match capabilities.tool_format {
        ToolFormat::OpenAiJson => Arc::new(OpenAiTranslator),
        ToolFormat::OllamaJson => Arc::new(OllamaTranslator),
        ToolFormat::AnthropicBlocks => Arc::new(ClaudeTranslator),
        ToolFormat::ReActText => Arc::new(ReActTranslator),
    }
}
```

The `capabilities_from_profile` function derives capabilities from a
`ModelProfile`, making the selection automatic based on config.

---

## Reasoning Extraction

The `BackendResponse` type provides `extract_reasoning()` which handles
four different reasoning wire formats (see sub-doc 03 for details). This
extraction is used by:

1. **ChatResponse construction** — The `reasoning` field is populated with
   extracted thinking content.
2. **Episode logging** — Reasoning content is logged separately from the
   main output for analysis.
3. **Cost computation** — Reasoning tokens may have different pricing
   (Anthropic charges differently for thinking vs. output tokens).

---

## Research Note: Format Switching

Some models exhibit "format switching" behavior — they perform well with
a given tool format up to a certain number of tools, then degrade. This is
documented for Qwen3-coder (above 5 tools) and some smaller Llama models
(above 10 tools).

The `max_tools` field in `ModelProfile` addresses this: when set, the
adapter truncates the tool array to the specified size, keeping only the
tools most relevant to the current task. The selection criteria are:
1. Tools explicitly requested by the task definition
2. Tools matching the agent's role permissions
3. Most frequently used tools from episode history

This truncation happens at the adapter level, before the Translator sees
the tools. The Translator always receives a tool array within the model's
comfortable range.

---

## Citations

1. Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM
   Agents." arXiv:2603.28052. — Principle #1: tools for the model.
2. `crates/roko-agent/src/translate/mod.rs` — Full 548-line source: Translator
   trait, ChatResponse, FinishReason, BackendResponse, wire format enums.
3. `crates/roko-agent/src/translate/openai.rs` — OpenAiTranslator.
4. `crates/roko-agent/src/translate/claude.rs` — ClaudeTranslator.
5. `crates/roko-agent/src/translate/ollama.rs` — OllamaTranslator.
6. `crates/roko-agent/src/translate/react.rs` — ReActTranslator.
7. `crates/roko-agent/src/translate/capability.rs` — ModelCapabilities,
   translator_for, capabilities_from_profile.
8. Implementation plan `modelrouting/04-translator-extensions.md` —
   BackendResponse reasoning extraction, FinishReason normalization.

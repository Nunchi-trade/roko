# 14 — Provider Integrations

> Sub-doc 14 of **02-agents** · Roko Documentation
>
> This document describes the specific provider integrations planned and
> partially implemented: Perplexity (Sonar), Gemini, ZhipuAI (GLM),
> Moonshot (Kimi), and OpenRouter. Each section covers the API surface,
> Roko-specific extensions, and integration status.


> **Implementation**: Shipping

---

## Perplexity (Sonar)

### API Surface

Perplexity exposes four API surfaces, all OpenAI-compatible:

1. **Chat Completions** (`/chat/completions`) — Primary interface. Standard
   OpenAI format with extensions for web search and citations.
2. **Agent/Responses API** — Newer agentic interface with built-in tool
   calling and web search integration.
3. **Search API** — Direct search without LLM generation (returns raw
   search results).
4. **Embeddings API** — Text embeddings for vector search.

### Sonar Models

| Model | Context | Features | Pricing |
|---|---|---|---|
| `sonar` | 128K | Search, citations | $1/M in, $1/M out |
| `sonar-pro` | 200K | Search, citations, extended | $3/M in, $15/M out |
| `sonar-reasoning` | 128K | Search, citations, CoT | $2/M in, $8/M out |
| `sonar-deep-research` | 128K | Async, multi-step | $2/M in, $8/M out + $5/req |

### Roko Integration

The `ModelProfile` struct includes Perplexity-specific fields:

```rust
pub supports_search: bool,           // Grounded web search
pub supports_citations: bool,        // Response citations
pub supports_async: bool,            // Async job API (deep research)
pub search_context_size: Option<String>,  // "low", "medium", "high"
pub cost_per_request: Option<f64>,   // Per-request fee
```

Example config:

```toml
[providers.perplexity]
kind = "openai_compat"
base_url = "https://api.perplexity.ai"
api_key_env = "PERPLEXITY_API_KEY"

[models.sonar-pro]
provider = "perplexity"
slug = "sonar-pro"
context_window = 200000
supports_tools = true
supports_search = true
supports_citations = true
tool_format = "openai_json"
cost_input_per_m = 3.00
cost_output_per_m = 15.00
cost_per_request = 0.005
search_context_size = "high"
```

### Response Extensions

Perplexity responses include additional fields:

```json
{
    "choices": [...],
    "citations": ["https://example.com/article1", "https://..."],
    "search_results": [
        {
            "url": "https://example.com/article1",
            "title": "Article Title",
            "snippet": "Relevant excerpt..."
        }
    ]
}
```

These are captured in `ResponseMetadata::web_search` as raw JSON for
downstream consumers (the research agent, citation formatter, etc.).

### Use Case in Roko

Perplexity Sonar is the ideal backend for the `Researcher` role. The
`roko research topic "<topic>"` command should route through Sonar for
web-grounded research with automatic citations. The `supports_citations`
flag enables the research agent to include verified citations in its output
without a separate verification step.

---

## Gemini

### API Surface

Google's Gemini provides two API endpoints:

1. **Native Gemini API** — Uses Google's own protocol with `Content` objects,
   `Part` arrays, and Gemini-specific features (grounding, code execution,
   thinking config).
2. **OpenAI-compatible endpoint** (`/v1beta/openai/`) — Standard chat
   completions format, usable with the `OpenAiCompatAdapter`.

### Key Features

| Feature | Details |
|---|---|
| Context window | **1M tokens** (2M for Gemini 1.5 Pro) |
| Free tier | 15 RPM, 1M TPM, 1500 RPD |
| Grounding | Verifies claims against Google Search |
| Code execution | Sandboxed Python execution |
| Thinking | Configurable `thinkingConfig` with token budget |

### Roko Integration

For the initial integration, Roko uses Gemini's OpenAI-compatible endpoint:

```toml
[providers.google]
kind = "openai_compat"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key_env = "GOOGLE_API_KEY"

[models.gemini-2-flash]
provider = "google"
slug = "gemini-2.0-flash"
context_window = 1048576
supports_tools = true
supports_thinking = true
supports_vision = true
tool_format = "openai_json"
cost_input_per_m = 0.075
cost_output_per_m = 0.30
```

The 1M context window makes Gemini particularly suitable for:
- **Large codebase analysis** — Can ingest entire modules without truncation
- **Long conversation histories** — The context pruning budget is enormous
- **Research synthesis** — Multiple research documents in a single prompt

### Grounding and Code Execution

Gemini's grounding feature (verifying claims against Google Search) and
code execution feature (running Python in a sandbox) are accessible through
the native API but **not through the OpenAI-compatible endpoint**. Future
integration may add a native Gemini adapter to expose these features.

---

## ZhipuAI (GLM)

### API Surface

ZhipuAI's GLM models use the OpenAI chat completions format:

| Model | Context | Features |
|---|---|---|
| GLM-5.1 | 200K | Tools, thinking, web search, code interpreter |
| GLM-4-Flash | 128K | Tools, fast, low cost |
| GLM-4-Air | 128K | Tools, balanced |

### Roko Integration

GLM models are a natural fit for the `OpenAiCompatAdapter`:

```toml
[providers.zai]
kind = "openai_compat"
base_url = "https://open.bigmodel.cn/api/paas/v4"
api_key_env = "ZHIPUAI_API_KEY"
timeout_ms = 60000

[models.glm-5-1]
provider = "zai"
slug = "glm-5.1"
context_window = 200000
max_output = 131072
supports_tools = true
supports_thinking = true
supports_web_search = true
tool_format = "openai_json"
cost_input_per_m = 1.40
cost_output_per_m = 4.40
```

This is the configuration used in the existing integration test at
`crates/roko-agent/src/provider/mod.rs:296`, which verifies the full
factory path with a mock ZhipuAI server.

### Finish Reason Normalization

GLM uses the same finish reason strings as OpenAI (`"stop"`, `"tool_calls"`,
`"length"`) plus ZhipuAI-specific ones (`"sensitive"` for content filtering,
`"network_error"` for internal errors). The `normalize_finish_reason`
function handles all of these.

---

## Moonshot (Kimi)

### API Surface

Moonshot's Kimi models use the OpenAI chat completions format with extensions
for file processing and web search:

| Model | Context | Features |
|---|---|---|
| moonshot-v1-128k | 128K | Tools, file processing |
| moonshot-v1-32k | 32K | Tools, standard |

### Roko Integration

```toml
[providers.moonshot]
kind = "openai_compat"
base_url = "https://api.moonshot.cn/v1"
api_key_env = "MOONSHOT_API_KEY"

[models.kimi-128k]
provider = "moonshot"
slug = "moonshot-v1-128k"
context_window = 128000
supports_tools = true
tool_format = "openai_json"
```

---

## OpenRouter

### API Surface

OpenRouter is a meta-provider that routes requests to 200+ models across
multiple underlying providers. It uses the OpenAI chat completions format
with routing extensions.

### Routing Configuration

OpenRouter-specific routing is controlled via the `ProviderRouting` struct
(sub-doc 01):

```toml
[models.claude-via-openrouter]
provider = "openrouter"
slug = "anthropic/claude-3.5-sonnet"
context_window = 200000
supports_tools = true
tool_format = "openai_json"

[models.claude-via-openrouter.provider_routing]
sort = "price"                    # price | throughput | latency
order = ["Anthropic", "AWS"]      # Provider preference
allow_fallbacks = true            # Auto-failover
max_price = 0.005                 # Cost ceiling per token
```

### Request Extensions

The `OpenAiCompatAdapter` injects OpenRouter-specific parameters when
`provider_routing` is set:

- `HTTP-Referer` header — Identifies the application to OpenRouter
- `X-Title` header — Application name for the OpenRouter dashboard
- `provider.order` in request body — Provider preference ordering
- `provider.allow_fallbacks` — Whether OpenRouter can use alternate providers

### Response Extensions

OpenRouter responses include `model` field indicating which actual model
served the request (may differ from the requested model when using
fallbacks). This is captured in `ResponseMetadata::model_used`.

### OpenRouter Metadata

The `openrouter_meta` module at `crates/roko-agent/src/provider/openrouter_meta.rs`
provides `fetch_model_metadata` for querying OpenRouter's model catalog:

```rust
pub async fn fetch_model_metadata(model_id: &str) -> Result<ModelMetadata> {
    // Queries https://openrouter.ai/api/v1/models/{model_id}
    // Returns pricing, context window, and capability information
}
```

This enables dynamic model discovery: Roko can query OpenRouter for model
capabilities at startup and populate the `[models.*]` registry automatically.

---

## Integration Status

| Provider | Config | Adapter | Tests | Production |
|---|---|---|---|---|
| Anthropic (API) | Done | Done | Done | Ready |
| Claude (CLI) | Done | Done | Done | Primary backend |
| OpenAI | Done | Done | Done | Ready |
| Ollama | Done | Done | Done | Ready |
| Cursor (ACP) | Done | Done | Partial | Ready |
| ZhipuAI (GLM) | Done | Done | Done | Integration test passes |
| OpenRouter | Done | Done | Partial | Ready |
| Perplexity | Config ready | Via OpenAiCompat | Not yet | Needs testing |
| Gemini | Config ready | Via OpenAiCompat | Not yet | Needs testing |
| Moonshot (Kimi) | Config ready | Via OpenAiCompat | Not yet | Needs testing |

---

## Citations

1. Implementation plan `modelrouting/20-perplexity-integration.md` — Sonar
   models, 4 API surfaces, response extensions.
2. Implementation plan `modelrouting/21-gemini-integration.md` — 1M context,
   free tier, grounding, code execution, thinking config.
3. Implementation plans `modelrouting/05-07` — GLM, Kimi, OpenRouter
   specifics.
4. `crates/roko-agent/src/provider/openrouter_meta.rs` —
   fetch_model_metadata.
5. `crates/roko-core/src/config/schema.rs:798` — ProviderRouting struct.
6. `crates/roko-core/src/config/schema.rs:819` — ModelProfile with
   Perplexity-specific fields.

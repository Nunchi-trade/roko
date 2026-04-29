# Provider Integrations and Domain Profiles

> Depth for [05-AGENT.md](../../unified/05-AGENT.md). Concrete provider integration details for 8+ backends, domain profiles as configuration bundles, and a reality check on what is wired versus aspirational.

## Provider Landscape

Roko integrates with 8+ LLM providers through a uniform Cell interface. Each provider is a **Connect protocol Cell** behind the `ProviderAdapter` trait. The adapter hides protocol differences; downstream Cells (ToolLoop, SafetyLayer, GatePipeline) are provider-agnostic.

### Integration Status

| Provider | Config | Adapter | Tests | Production Status |
|---|---|---|---|---|
| **Anthropic (API)** | Done | Done | Done | Ready |
| **Claude (CLI)** | Done | Done | Done | Primary backend |
| **OpenAI** | Done | Done | Done | Ready |
| **Ollama** | Done | Done | Done | Ready (local models) |
| **Cursor (ACP)** | Done | Done | Partial | Ready |
| **ZhipuAI (GLM)** | Done | Done | Done | Integration test passes |
| **OpenRouter** | Done | Done | Partial | Ready (meta-provider) |
| **Perplexity** | Done | Via shared factory for chat/search | Partial | Deep research adapter-specific |
| **Gemini** | Done | Via shared factory for compat models | Partial | Native grounding adapter-specific |
| **Moonshot (Kimi)** | Config ready | Via OpenAiCompat | Not yet | Needs testing |

### Architecture Pattern

All providers converge to the same runtime path:

```
Config (roko.toml)
    |
    v
resolve_model(slug) -> ResolvedModel
    |
    v
adapter_for_kind(provider_kind) -> &dyn ProviderAdapter
    |
    v
adapter.create_agent(provider, model, options) -> Box<dyn Agent>
    |
    v
Agent.run(signal, ctx) -> AgentResult
```

The key property: **agents never hold API keys**. Keys are resolved from environment variables at adapter construction time and injected into the backend. The InferenceHandle pattern (see [08-GATEWAY.md](../../unified/08-GATEWAY.md)) means the agent holds a channel sender, not credentials.

## Per-Provider Details

### Claude CLI

The primary backend for plan execution. Claude CLI drives its own internal tool loop, which means Roko's ToolDispatcher/SafetyLayer is bypassed.

**Wire format**: Stream-JSON events. The `ClaudeTranslator` parses `tool_use` blocks from the event stream.

**Key behavior**: `RenderedResults::HandledByBackend` -- Roko does not feed tool results back because Claude CLI manages its own loop. This is the single largest gap in safety coverage.

**Config**:
```toml
[providers.anthropic_cli]
kind = "claude_cli"
# No API key needed -- uses Claude CLI's own auth

[models.claude-sonnet]
provider = "anthropic_cli"
slug = "claude-sonnet-4-6"
```

### Anthropic API

Direct HTTP API with content blocks, thinking, and streaming.

**Wire format**: Anthropic content blocks. The `ClaudeTranslator` handles both CLI stream and API JSON.

**Key features**: Extended thinking with configurable token budget, vision support, system prompt caching.

### OpenAI / OpenAI-Compatible

The workhorse adapter. Handles OpenAI, DeepSeek, and any provider that speaks the chat completions protocol.

**Wire format**: JSON function calling. `arguments` field arrives as a string (not parsed JSON) -- the `OpenAiTranslator` parses it.

**Key pattern**: Most new providers can use this adapter with zero code changes, just config.

### Ollama

Local model execution. The `OllamaLlmBackend` is the reference implementation for the `LlmBackend` trait.

**Wire format**: OpenAI-compatible with minor differences (`message` instead of `choices[0].message`). The `OllamaTranslator` handles the delta.

**Key use**: Development, testing, offline operation, privacy-sensitive workloads.

### Perplexity (Sonar)

Research-optimized provider with built-in web search and citations.

**Four API surfaces**:
1. Chat Completions (`/chat/completions`) -- primary, OpenAI-compatible + search
2. Agent/Responses API -- agentic with built-in tool calling
3. Search API -- raw search results without LLM generation
4. Embeddings API -- text embeddings for vector search

**Models**:

| Model | Context | Pricing | Key Feature |
|---|---|---|---|
| sonar | 128K | $1/$1/M | Search, citations |
| sonar-pro | 200K | $3/$15/M | Extended search |
| sonar-reasoning | 128K | $2/$8/M | Chain-of-thought |
| sonar-deep-research | 128K | $2/$8/M + $5/req | Async, multi-step |

**Response extensions**: Perplexity responses include `citations` (URL array) and `search_results` (structured search data). Captured in `ResponseMetadata::web_search`.

**Roko-specific fields in ModelProfile**:
```rust
pub supports_search: bool,              // Grounded web search
pub supports_citations: bool,           // Response citations
pub supports_async: bool,               // Async deep research
pub search_context_size: Option<String>, // "low" | "medium" | "high"
pub cost_per_request: Option<f64>,      // Per-request fee (deep research)
```

**What is wired**: Search-grounded chat and citation paths use the shared factory/tool-loop. **What is not**: Deep research (async endpoint) and embeddings remain adapter-specific.

### Gemini

Google's models with massive context windows and optional grounding.

**Two endpoints**:
1. Native Gemini API -- Google's own protocol with grounding and code execution
2. OpenAI-compatible (`/v1beta/openai/`) -- standard chat completions

**Key features**:

| Feature | Value |
|---|---|
| Context window | 1M tokens (2M for Pro) |
| Free tier | 15 RPM, 1M TPM, 1500 RPD |
| Grounding | Claims verified against Google Search |
| Code execution | Sandboxed Python |
| Thinking | Configurable `thinkingConfig` with token budget |

**Best for**: Large codebase analysis (entire modules without truncation), long conversation histories, research synthesis.

**What is wired**: Simple OpenAI-compatible models use the shared factory path. **What is not**: Native grounding and code execution remain adapter-specific.

### ZhipuAI (GLM)

Chinese AI provider using OpenAI-compatible format.

| Model | Context | Features |
|---|---|---|
| GLM-5.1 | 200K | Tools, thinking, web search, code interpreter |
| GLM-4-Flash | 128K | Fast, low cost |
| GLM-4-Air | 128K | Balanced |

**Finish reason normalization**: GLM uses standard strings plus ZhipuAI-specific ones (`"sensitive"` for content filtering, `"network_error"` for internal errors). The `normalize_finish_reason` function handles all.

### OpenRouter

Meta-provider routing to 200+ models across multiple providers.

**Routing configuration**:
```toml
[models.claude-via-openrouter.provider_routing]
sort = "price"                    # price | throughput | latency
order = ["Anthropic", "AWS"]      # Provider preference
allow_fallbacks = true            # Auto-failover
max_price = 0.005                 # Cost ceiling per token
```

**Request extensions**: `HTTP-Referer`, `X-Title` headers, `provider.order` and `provider.allow_fallbacks` in request body.

**Response extensions**: `model` field indicating which actual model served the request (may differ with fallbacks). Captured in `ResponseMetadata::model_used`.

**Dynamic discovery**: `fetch_model_metadata` queries OpenRouter's catalog at startup, enabling automatic model registry population.

### Moonshot (Kimi)

| Model | Context | Features |
|---|---|---|
| moonshot-v1-128k | 128K | Tools, file processing |
| moonshot-v1-32k | 32K | Tools, standard |

Config ready, adapter via OpenAiCompat. Needs integration testing.

## Domain Profiles

A **domain profile** is an installable bundle that wraps roles, tools, gates, heuristics, and prompt templates into a coherent agent stack for a specific domain. In unified terms, a profile is a **Graph template** that instantiates a pre-configured Agent Space.

### Six Canonical Profiles

| Profile | Default Roles | Core Tools | Core Gates | Memory Shape |
|---|---|---|---|---|
| **Coding** | Researcher, Planner, Implementer, Reviewer, Tester | fs, git, language toolchains, code MCP | compile, unit, clippy, diff | Episodes, playbooks, build history |
| **Research** | Researcher, Analyst, Explorer, Reviewer | web, PDF, citation manager, notes | citation, factuality, novelty | Paper claims, replication ledger |
| **Blockchain** | Architect, Implementer, Reviewer, Operator | RPC, signer, explorer, compiler, simulator | simulation, gas, invariant, approval | Chain-of-custody, audit trail |
| **Data/ML** | Analyst, Implementer, Tester, Reviewer | SQL, notebooks, profiling | schema, sample-check, metric regression | Dataset fingerprints, lineage |
| **Ops/SRE** | Operator, Deployer, Monitor, Reviewer | kubectl, logs, metrics, runbooks, pager | dry-run, blast-radius, change-window | Incident archive, runbook library |
| **Writing** | DocWriter, Researcher, Reviewer | corpus search, style guide, fact-check, citations | style, fact, tone, plagiarism | Voice fingerprint, editorial archive |

### Profile Composition Rules

1. Roles selected first, then specialized by profile's prompts and tool allowlists
2. Tools merge by union when multiple profiles installed
3. Gates stack unless explicitly scoped to one profile
4. Heuristics coexist; routing chooses best fit for context
5. Profile collisions must be explicit (visible resolution policy)
6. Context must be structured (TypedContext), not free-form

### TypedContext

Structured situation record that domain profiles share, replacing ad-hoc free-text task summaries:

```rust
/// TypedContext is a structured Signal payload that gates and
/// heuristics can match on without parsing prose.
pub struct TypedContext {
    pub domain: Domain,
    pub fields: BTreeMap<ContextKey, ContextValue>,
}

pub enum ContextValue {
    String(String),
    Int(i64),
    Float(f64),
    Hash(EngramHash),
    Fingerprint(HdcVector),
    List(Vec<ContextValue>),
    Nested(BTreeMap<ContextKey, ContextValue>),
}
```

Typical keys by domain:

| Domain | Keys |
|---|---|
| Coding | `language`, `repo_root`, `file_set`, `last_gate` |
| Research | `question`, `source_ids`, `claim_set`, `corpus` |
| Blockchain | `chain`, `wallet`, `intent`, `simulation` |
| Data/ML | `dataset`, `notebook`, `metric`, `schema_version` |
| Ops/SRE | `service`, `incident_id`, `change_window`, `blast_radius` |
| Writing | `audience`, `tone`, `source_set`, `voice_target` |

### Custody

Chain-of-custody record for profile actions with external consequences:

```rust
pub struct Custody {
    pub action: ActionHash,
    pub who: PrincipalId,
    pub when: Timestamp,
    pub why: Vec<HeuristicId>,       // Which heuristics justified the action
    pub how: Vec<ClaimId>,           // Supporting claims
    pub approved_by: Option<PrincipalId>,
    pub simulation: Option<SimulationHash>,
    pub result: Option<ResultHash>,
    pub witness: Option<ChainWitness>,
}
```

Strongest need: Blockchain (transaction approval), Ops/SRE (deploys, rollbacks), Data/ML (lineage).

### Profile Installation

```toml
[profile.coding]
roles = ["researcher", "planner", "implementer", "reviewer"]
tools = ["fs.read", "fs.write", "git.status", "cargo.build", "cargo.test"]
gates = ["unit", "type", "style", "diff"]
heuristics = "@roko/coding-heuristics-starter"
templates = "@roko/coding-templates"
```

### Evaluation Suites

Each profile ships with a benchmark suite:
- Coding: bug-fix tasks with frozen SHAs and test outcomes
- Research: claim-to-source matching and follow-up paper detection
- Blockchain: vulnerable-contract detection and false-positive tracking
- Data/ML: dirty-dataset diagnosis and metric-regression handling
- Ops/SRE: simulated incidents and time-to-correct-diagnosis
- Writing: style-fidelity checks against known author corpus

## Reality Check: Current State vs. Aspirational

### Wired and Working

- **Provider adapters**: 6 adapters implemented and tested (Anthropic API, Claude CLI, OpenAI-compat, Ollama, Cursor, OpenRouter)
- **Factory path**: `create_agent_for_model` works end-to-end with all adapted providers
- **ToolLoop + ToolDispatcher**: Works for Ollama and OpenAI-compatible backends
- **Translator selection**: Automatic based on ModelCapabilities
- **CascadeRouter**: Persists, learns, and routes across sessions
- **Episode logging + efficiency tracking**: Emit on every turn

### Built But Partially Wired

- **Perplexity/Gemini**: Chat/search paths through shared factory, but deep research and native grounding are adapter-specific
- **ToolDispatcher universality**: Not all backends flow through it (Claude CLI bypasses)
- **Agent creation sites**: Primary paths consolidated, but prd.rs and some research paths still diverge

### Specified But Not Wired

- **Temperament propagation**: Config field exists, not read by runtime
- **Domain profiles**: Architecture designed, no installable bundles exist yet
- **TypedContext**: Type defined, not used by any gate or heuristic
- **Custody**: Type defined, not instantiated at runtime
- **Evaluation suites**: None implemented
- **Multi-profile composition**: No collision detection or resolution
- **Affect-routing**: AffectRoutingBias designed (12-AFFECT-ROUTING.md), not integrated into CascadeRouter

### Gap Severity

| Gap | Severity | Impact |
|---|---|---|
| Claude CLI safety bypass | Critical | Primary backend has no Roko safety enforcement |
| Remaining creation sites | High | CascadeRouter cannot intercept all model selection |
| LlmBackend coverage | High | Some HTTP providers bypass shared tool loop |
| Role prompt depth | Medium | ~1 sentence vs. ~2K tokens in Mori |
| Temperament propagation | Low | Operator dial exists but does nothing |
| Domain profiles | Low | Coding profile is implicit; others need bundling |

### Metrics

| Metric | Current | Target |
|---|---|---|
| Agent backends | 6 | 6 (stable) |
| Provider adapters | 6 | 6 (stable) |
| Translators | 4 | 4 (stable) |
| LlmBackend impls | 2 production families | Universal HTTP coverage |
| Creation sites consolidated | ~70% of production paths | 100% |
| Safety coverage | Partial | 100% of backends |
| Role prompt tokens | ~20 per role | ~2000 per role |
| Provider integrations tested | 4 | 8+ |
| Domain profiles installable | 0 | 6 |

---

## What This Enables

1. **Multi-provider fleet** -- The uniform adapter pattern means a deployment can route across Claude, OpenAI, Gemini, and local Ollama models simultaneously, using the CascadeRouter to select the best model per task.
2. **Domain-shaped defaults** -- Domain profiles eliminate the bootstrapping problem: a team installing Roko for blockchain gets a working agent stack without configuring 50 settings.
3. **Auditable actions** -- Custody records provide the provenance chain needed for regulated environments (blockchain, ops, data lineage).

## Feedback Loops

1. **Provider health -> routing -> cost**: When a provider degrades (latency spikes, error rate increase), the ProviderHealthRegistry deprioritizes it. Healthy providers receive more traffic. The CascadeRouter's pass-rate weighting closes this loop.
2. **Profile evaluation suite -> profile improvement**: Each profile's benchmark suite produces Verify Verdicts that feed back into the profile's heuristic calibration, improving default settings over time.
3. **TypedContext -> gate precision -> fewer false rejections**: Structured context allows gates to match on exact fields instead of parsing prose, reducing false positive rejections and improving gate pass rates.

## Open Questions

1. **Profile packaging format**: Should profiles be Cargo features, TOML bundles, or plugin archives? Each has different distribution and versioning implications.
2. **Provider-specific features**: Gemini grounding and Perplexity deep research are powerful but adapter-specific. Should they be exposed as domain-specific tools (Cells) within profiles, or as provider capabilities in the model registry?
3. **Dynamic model discovery**: OpenRouter's `fetch_model_metadata` can populate the registry at startup. Should this be automatic (query all available models) or opt-in (query only configured models)?
4. **Cross-provider routing**: When multiple providers offer the same model (e.g., Claude via Anthropic vs. Claude via OpenRouter), how should the CascadeRouter choose between them? Price? Latency? Pass rate? Provider health is the obvious answer, but the data collection is not yet wired.
5. **Profile evolution**: Should profiles have versions? If a profile's evaluation suite shows regression after a configuration change, should it auto-revert?

---

## Citations

1. `crates/roko-agent/src/provider/mod.rs` -- ProviderAdapter, create_agent_for_model, adapter_for_kind.
2. `crates/roko-agent/src/provider/openrouter_meta.rs` -- fetch_model_metadata.
3. `crates/roko-core/src/config/schema.rs` -- ProviderConfig, ModelProfile, ProviderRouting.
4. `crates/roko-agent/src/translate/mod.rs` -- Translator, wire format types.
5. `crates/roko-agent/src/ollama_backend.rs` -- Reference LlmBackend implementation.
6. `tmp/mori-diffs/12-AFFECT-ROUTING.md` -- Affect-routing gap analysis and implementation design.
7. See [05-AGENT.md](../../unified/05-AGENT.md) for Agent specialization definition.
8. See [08-GATEWAY.md](../../unified/08-GATEWAY.md) for InferenceHandle pattern.
9. See [19-CONFIG.md](../../unified/19-CONFIG.md) for config-as-Signal and domain profiles.

# 41 - Inference Gateway And Model-Call Service Audit

Date: 2026-04-27

Scope: this audit checks whether Roko has the unified inference gateway described in [../unified/08-GATEWAY.md](../unified/08-GATEWAY.md), whether every production model call flows through one shared runtime boundary, and what has to be rebuilt so provider calls, cost/cache accounting, provider proof, batch work, HTTP queries, and agent feedback are end-to-end reliable rather than route-local.

Bottom line: Roko does not yet have a real inference gateway. It has a useful runner-local dispatch facade, a server route named gateway, provider adapters, and several direct model-call paths. Those are not the same thing. The missing abstraction is a process-wide `ModelCallService` / `InferenceGateway` that owns the model-call pipeline after routing and before provider execution.

## Why This Matters

The docs describe the gateway as a central runtime primitive:

- Agents never hold API keys.
- Every inference request passes through a 9-cell pipeline.
- Cache, output budget, thinking caps, convergence, provider fallback, batch, and cost are runtime behavior, not route behavior.
- Stats are derived from request events, not from process-local counters.
- Batch requests use the same pipeline and durable operation state as real-time requests.
- Provider health, rate limits, and fallback metadata feed learning and later routing.

The current code has pieces of this, but the pieces are split:

- `crates/roko-cli/src/dispatch/` owns runner-local route, prompt assembly, and provider bridge seams.
- `crates/roko-agent/src/provider/` owns provider adapters and `create_agent_for_model`.
- `crates/roko-serve/src/routes/gateway.rs` exposes HTTP routes but mostly forwards a flattened prompt into `runtime.run_once`.
- `crates/roko-cli/src/commands/research.rs`, `crates/roko-dreams/src/runner.rs`, `crates/roko-cli/src/vision_loop/evaluator.rs`, `crates/roko-neuro/src/episode_completion.rs`, `crates/roko-std/src/tool/builtin/web_search.rs`, and `crates/roko-serve/src/routes/providers.rs` still make specialized model/provider calls through their own paths.
- `docker/gateway.Dockerfile` explicitly ships a placeholder because `crates/roko-gateway/` does not exist.

That means a successful provider test, a successful `roko research`, a successful HTTP `/api/inference/complete`, and a successful runner task do not prove the same subsystem. They prove different seams.

## Source-Verified Findings

| Finding | Source Evidence | Impact |
|---|---|---|
| The target gateway is specified as a 9-stage Pipeline Graph, not a single route handler. | [../unified/08-GATEWAY.md](../unified/08-GATEWAY.md) defines `LoopDetectCell`, `CacheLookupCell`, `ToolPruneCell`, `OutputBudgetCell`, `ThinkingCapCell`, `ConvergenceDetectCell`, `ProviderCallCell`, `CacheStoreCell`, and `CostTrackCell`. | The implementation must be a reusable service/pipeline, not duplicated endpoint logic. |
| The target crate does not exist. | `find crates -maxdepth 2 -name Cargo.toml | rg gateway` produced no `roko-gateway` crate. | Gateway code is embedded in `roko-serve` routes and other callers, so it cannot be the shared provider boundary. |
| The gateway container is intentionally a placeholder. | [../../docker/gateway.Dockerfile](../../docker/gateway.Dockerfile) says the `roko-gateway` crate does not yet exist and builds `--bin roko` as a stand-in. | Compose can start a container named gateway without proving a gateway implementation exists. |
| Docker compose wires the placeholder as a service. | [../../docker/docker-compose.yml](../../docker/docker-compose.yml) labels the `gateway` service as `roko-gateway (placeholder, see gateway.Dockerfile TODO)`. | Operator-level topology overstates runtime readiness. |
| HTTP inference bypasses a gateway service and calls `run_once`. | [../../crates/roko-serve/src/routes/gateway.rs](../../crates/roko-serve/src/routes/gateway.rs) formats chat messages into one prompt string and calls `state.runtime.run_once(state.workdir.as_path(), &prompt)`. | Structured messages, tools, budgets, provider fallback, cache, stream, and diagnostics are collapsed before dispatch. |
| Gateway stats are partly volatile process counters. | [../../crates/roko-serve/src/state.rs](../../crates/roko-serve/src/state.rs) stores `gateway_model_counters` in an in-memory `RwLock<HashMap<...>>`. | Stats reset on restart and cannot prove completed historical inference. |
| Cache metrics are placeholders. | [../../crates/roko-serve/src/routes/gateway.rs](../../crates/roko-serve/src/routes/gateway.rs) sets `cache_read_tokens: 0` and `cache_hits: 0`. | Cost/cache proof cannot be trusted. |
| Batch is local fan-out, not provider batch. | [../../crates/roko-serve/src/routes/gateway.rs](../../crates/roko-serve/src/routes/gateway.rs) uses `tokio::spawn`, `buffer_unordered(BATCH_CONCURRENCY)`, volatile `batch_progress`, and `runtime.run_once` for each item. | The advertised batch API does not prove provider batch discount, durable status, retry, or same-pipeline behavior. |
| Provider tests bypass the gateway. | [../../crates/roko-serve/src/routes/providers.rs](../../crates/roko-serve/src/routes/providers.rs) calls `create_agent_for_model` directly in `/api/providers/{id}/test`. | Provider proof results are not gateway proof results and do not emit full gateway events. |
| Research owns multiple provider paths. | [../../crates/roko-cli/src/commands/research.rs](../../crates/roko-cli/src/commands/research.rs) creates Perplexity/Gemini agents through research-specific config and reads `PERPLEXITY_API_KEY` directly for raw search. | Research success does not prove the same provider, credential, cache, budget, cost, or event path as runner tasks. |
| Dreams creates agents directly. | [../../crates/roko-dreams/src/runner.rs](../../crates/roko-dreams/src/runner.rs) calls `create_agent_for_model` from `DreamAgentConfig::build_agent`. | Dream consolidation can diverge from the runtime gateway, budget, and provider proof surfaces. |
| Vision loop creates agents directly. | [../../crates/roko-cli/src/vision_loop/evaluator.rs](../../crates/roko-cli/src/vision_loop/evaluator.rs) calls `create_agent_for_model` with local multimodal prompt packaging. | Vision evaluation has separate timeout, prompt, provider, and proof behavior. |
| Neuro distillation reads provider secrets directly. | [../../crates/roko-neuro/src/episode_completion.rs](../../crates/roko-neuro/src/episode_completion.rs) reads `ANTHROPIC_API_KEY` and calls `Distiller::with_claude`. | Cognitive background work violates "agents never hold API keys" and cannot be centrally observed. |
| Built-in web search reads provider secrets directly. | [../../crates/roko-std/src/tool/builtin/web_search.rs](../../crates/roko-std/src/tool/builtin/web_search.rs) reads `PERPLEXITY_API_KEY` and calls Perplexity search directly. | Tool execution bypasses gateway credential and observability policy. |
| Legacy orchestration still has search/provider paths. | [../../crates/roko-cli/src/orchestrate.rs](../../crates/roko-cli/src/orchestrate.rs) still contains direct Perplexity search creation and env reads. | Retaining `orchestrate.rs` as a production path keeps a second model-call runtime alive. |

## Current Architecture Diagnosis

There are three layers that should not be conflated:

1. `ProviderAdapter`: builds one provider-specific agent/client from config.
2. `AgentDispatcherV2` / `Dispatcher`: resolves runner tasks into a model, prompt, and provider-neutral outcome.
3. `InferenceGateway` / `ModelCallService`: owns every model call from every surface after routing policy has been selected.

The current implementation mostly has layers 1 and 2. It does not have layer 3.

`create_agent_for_model` is necessary but insufficient. It standardizes provider construction, but it does not guarantee central credential isolation, cache policy, output budgets, fallback chains, cost accounting, durable request events, batch behavior, or HTTP query parity.

`Dispatcher` is necessary but runner-scoped. It knows about `TaskDef`, runner prompt assembly, and `AgentOutcome`. It should not become the universal model-call service for research search, dream reviews, neuro distillation, provider probes, and HTTP inference. Instead, it should call the gateway for the provider-call phase.

`routes/gateway.rs` is named gateway but is currently an adapter. It should become a thin HTTP adapter over the real gateway service.

## Target Shape

Build a shared service boundary:

```rust
#[async_trait::async_trait]
pub trait ModelCallService: Send + Sync {
    async fn complete(&self, request: ModelCallRequest) -> Result<ModelCallResponse, ModelCallError>;

    async fn stream(
        &self,
        request: ModelCallRequest,
        sink: Box<dyn ModelStreamSink>,
    ) -> Result<ModelCallSummary, ModelCallError>;

    async fn submit_batch(
        &self,
        request: BatchModelCallRequest,
    ) -> Result<BatchOperationRef, ModelCallError>;

    async fn probe_provider(
        &self,
        request: ProviderProbeRequest,
    ) -> Result<ProviderProbeResult, ModelCallError>;
}
```

The production implementation should be `InferenceGateway`. It should be usable from CLI, runner, serve, dreams, neuro, tools, provider proof, and tests without pulling in HTTP route code.

Recommended home:

- If this must be embedded in the current process first: `crates/roko-runtime` or a new `crates/roko-inference` crate.
- If a separate gateway binary is still desired: a thin `crates/roko-gateway` binary depends on that library and exposes HTTP/metrics.
- Do not put the core service in `roko-serve`; server routes are adapters.
- Do not put the core service in `roko-cli`; CLI commands are adapters.
- Do not put provider-specific policy in domain crates like `roko-neuro`, `roko-dreams`, or `roko-std`.

## Core Request Contract

Every model call should carry these fields, even when some are defaulted:

```rust
pub struct ModelCallRequest {
    pub operation_id: OperationId,
    pub caller: ModelCallCaller,
    pub workdir: WorkspaceRef,
    pub messages: Vec<ModelMessage>,
    pub prompt_text: Option<String>,
    pub model: ModelSelectionRequest,
    pub routing_hints: RoutingHints,
    pub budget: BudgetPolicy,
    pub cache: CachePolicy,
    pub output: OutputPolicy,
    pub thinking: ThinkingPolicy,
    pub tool_policy: ToolPrunePolicy,
    pub redaction: RedactionPolicy,
    pub diagnostics: PromptDiagnosticsRef,
    pub credential_scope: CredentialScope,
    pub proof_mode: ProofMode,
}
```

Rules:

- [ ] A caller must set `caller` to one of `runner_task`, `provider_probe`, `research`, `research_search`, `dream_review`, `neuro_distillation`, `vision_loop`, `serve_http`, `tool_web_search`, `workflow_generation`, or `legacy_orchestrate`.
- [ ] `legacy_orchestrate` is allowed only during migration and must emit a warning event.
- [ ] `messages` is the canonical input. `prompt_text` is a compatibility field and must be converted into a single user message before the pipeline.
- [ ] Credentials are referenced by `credential_scope`; raw API keys never appear in `ModelCallRequest`.
- [ ] Every request receives an `operation_id`, even if the caller did not provide one.
- [ ] Every request emits `model_call.requested` before provider execution or cache lookup.
- [ ] Every terminal path emits exactly one of `model_call.completed`, `model_call.failed`, `model_call.cancelled`, or `model_call.cache_hit`.

## Pipeline Cells

The initial implementation should be deliberately explicit. Each cell has input, output, events, proof, and failure rules.

### Cell 1: LoopDetect

- [ ] Maintain bounded per-session recent-call state.
- [ ] Detect identical retry loops, oscillation, and long-output drift.
- [ ] Inject guidance through a structured system advisory, not by string-concatenating into route handlers.
- [ ] Emit `model_call.loop_detected` with `pattern`, `session_id`, and `guidance_inserted`.
- [ ] Prove guidance clears after one use.

### Cell 2: CacheLookup

- [ ] Normalize requests before hashing.
- [ ] Implement exact-match L1 cache with workspace/session namespace.
- [ ] Implement semantic L2 cache or explicitly ship L2 as `disabled` with a durable `model_call.cache_disabled` event.
- [ ] Emit `model_call.cache_lookup` with `hit`, `layer`, `cache_key_hash`, `namespace`, and `policy`.
- [ ] On cache hit, skip provider execution and still emit cost/savings events.

### Cell 3: ToolPrune

- [ ] Accept structured tool schemas.
- [ ] Preserve a never-prune allowlist for core tools.
- [ ] Track removed tool count and estimated token savings.
- [ ] Emit `model_call.tools_pruned`.
- [ ] Prove pruned tools are absent from provider payload and protected tools remain.

### Cell 4: OutputBudget

- [ ] Maintain per-model output token observations.
- [ ] Apply a cap only when request output policy allows it.
- [ ] Record original and effective `max_tokens`.
- [ ] Emit `model_call.output_budget_applied`.
- [ ] Prove explicit low caps are not raised and explicit high caps are reduced only by policy.

### Cell 5: ThinkingCap

- [ ] Apply provider/model-specific thinking defaults only when thinking is already enabled.
- [ ] Do not force thinking on.
- [ ] Do not override explicit user budget.
- [ ] Emit `model_call.thinking_cap_applied`.
- [ ] Prove Anthropic/OpenAI/Gemini-compatible payloads keep provider-specific semantics below the provider adapter.

### Cell 6: ConvergenceDetect

- [ ] Maintain bounded per-session response similarity state.
- [ ] Detect repeated near-identical responses.
- [ ] Inject one-shot divergence guidance on the next request.
- [ ] Emit `model_call.convergence_detected`.
- [ ] Prove dissimilar responses reset the counter.

### Cell 7: ProviderCall

- [ ] Resolve model/provider through the existing provider registry and `ProviderDispatchResolver`.
- [ ] Use `AgentDispatcherV2` / provider adapters for actual provider execution rather than duplicating HTTP clients.
- [ ] Implement fallback chain semantics for `rate_limited`, `temporarily_unavailable`, and timeout.
- [ ] Classify terminal status as `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, `provider_error`, `timeout`, or `cancelled`.
- [ ] Emit `model_call.provider_started`, `model_call.provider_event`, and `model_call.provider_finished`.
- [ ] Include fallback metadata: `original_model`, `served_model`, `fallback_used`, `fallback_reason`.

### Cell 8: CacheStore

- [ ] Store only successful cacheable responses.
- [ ] Exclude tool-use responses, errors, and too-short outputs.
- [ ] Write cache asynchronously but attach write result to the final event when possible.
- [ ] Emit `model_call.cache_store`.
- [ ] Prove a repeat request avoids provider execution.

### Cell 9: CostTrack

- [ ] Compute input, output, cache read, cache write, reasoning/thinking, and batch-discount costs from model profile pricing.
- [ ] Emit `model_call.cost_recorded` with actual cost, naive cost, and savings.
- [ ] Store cost records durably so stats survive restart.
- [ ] Publish per-agent, per-session, per-model, and per-provider projections.
- [ ] Prove HTTP stats match durable event totals.

## Event Contract

All gateway events should be durable and queryable. Do not rely on local atomic counters as the source of truth.

Required event names:

- [ ] `model_call.requested`
- [ ] `model_call.routing_resolved`
- [ ] `model_call.loop_detected`
- [ ] `model_call.cache_lookup`
- [ ] `model_call.cache_hit`
- [ ] `model_call.tools_pruned`
- [ ] `model_call.output_budget_applied`
- [ ] `model_call.thinking_cap_applied`
- [ ] `model_call.convergence_detected`
- [ ] `model_call.provider_started`
- [ ] `model_call.provider_stream_delta`
- [ ] `model_call.provider_finished`
- [ ] `model_call.provider_failed`
- [ ] `model_call.fallback_selected`
- [ ] `model_call.cache_store`
- [ ] `model_call.cost_recorded`
- [ ] `model_call.batch_submitted`
- [ ] `model_call.batch_flushed`
- [ ] `model_call.batch_polled`
- [ ] `model_call.batch_completed`
- [ ] `model_call.completed`
- [ ] `model_call.failed`
- [ ] `model_call.cancelled`

Required event fields:

- [ ] `operation_id`
- [ ] `request_id`
- [ ] `caller`
- [ ] `workspace_id`
- [ ] `session_id`
- [ ] `agent_id`
- [ ] `provider_id`
- [ ] `model_key`
- [ ] `model_slug`
- [ ] `attempt`
- [ ] `fallback_depth`
- [ ] `input_tokens`
- [ ] `output_tokens`
- [ ] `cache_read_tokens`
- [ ] `cache_write_tokens`
- [ ] `reasoning_tokens`
- [ ] `thinking_tokens`
- [ ] `latency_ms`
- [ ] `ttft_ms`
- [ ] `status`
- [ ] `error_kind`
- [ ] `cost_usd`
- [ ] `naive_cost_usd`
- [ ] `savings_usd`
- [ ] `prompt_diagnostics_id`
- [ ] `redaction_policy_id`

## Query And HTTP Contract

Server routes should become adapters over query services and command services.

Required command endpoints:

- [ ] `POST /api/gateway/inference` calls `ModelCallService::complete`.
- [ ] `POST /api/gateway/inference/stream` calls `ModelCallService::stream`.
- [ ] `POST /api/gateway/batch/submit` calls `ModelCallService::submit_batch`.
- [ ] `POST /api/gateway/batch/flush` calls `BatchModelCallService::flush`.
- [ ] `POST /api/providers/{id}/test` calls `ModelCallService::probe_provider`.

Required query endpoints:

- [ ] `GET /api/gateway/stats` reads from `GatewayProjection`, not from in-memory route counters.
- [ ] `GET /api/gateway/requests/{id}` returns request, cell actions, provider attempts, events, cost, and terminal output summary.
- [ ] `GET /api/gateway/batch/{id}` returns durable batch state after process restart.
- [ ] `GET /api/providers/{id}/health` reads provider health projection updated by gateway events.
- [ ] `GET /api/models` indicates which models are configured, reachable, unsupported, missing credentials, or unproven.
- [ ] `GET /api/proof/provider-matrix` returns the last proof status per provider/model pair.

Compatibility:

- [ ] Keep existing `/api/inference/complete` temporarily as an alias, but emit deprecation metadata in the response.
- [ ] Keep existing `/api/inference/batch/submit` temporarily as an alias, but route through the real batch service.
- [ ] Remove the alias only after proof scripts and UI use `/api/gateway/*`.

## Direct Call Site Migration

Each production call site below must move to `ModelCallService`. The checklist item is not complete when it merely uses `create_agent_for_model`; it is complete only when the call emits gateway events and is visible through gateway query endpoints.

### Runner Dispatch

- [ ] Keep `Dispatcher` responsible for route, prompt assembly, warm pool, and `AgentOutcome` normalization.
- [ ] Replace provider execution inside the dispatch bridge with `ModelCallService`.
- [ ] Preserve `AgentRuntimeEvent` streaming by adapting gateway stream events into `AgentRuntimeEvent`.
- [ ] Emit prompt diagnostics before the gateway call and attach `prompt_diagnostics_id` to `ModelCallRequest`.
- [ ] Prove one runner task appears in `GatewayProjection` with caller `runner_task`.

### HTTP Gateway Route

- [ ] Delete `format_messages_as_prompt` as the main gateway path.
- [ ] Replace route-local `state.runtime.run_once` with `ModelCallService::complete`.
- [ ] Replace route-local atomics with projection queries.
- [ ] Return structured `ModelCallResponse`.
- [ ] Prove `/api/gateway/stats` totals match the events for a request.

### Provider Test Route

- [ ] Replace direct `create_agent_for_model` call with `ModelCallService::probe_provider`.
- [ ] Return classified statuses: `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, `timeout`, `provider_error`.
- [ ] Include the gateway request id in the response.
- [ ] Prove Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI all use the same probe path.

### Research

- [ ] Convert Gemini grounding research to `ModelCallService` with caller `research`.
- [ ] Convert Perplexity chat/deep research to `ModelCallService` with caller `research`.
- [ ] Convert raw `research search` to either a typed `SearchService` capability backed by gateway credentials or a gateway `tool_web_search` call.
- [ ] Remove direct `PERPLEXITY_API_KEY` read from `research.rs`.
- [ ] Preserve citations and metadata in typed response extensions.
- [ ] Prove `roko research topic` and `roko research search` produce gateway events.

### Dreams

- [ ] Replace `DreamAgentConfig::build_agent` direct provider construction with a `DreamReviewModelClient`.
- [ ] Implement `DreamReviewModelClient` on top of `ModelCallService`.
- [ ] Preserve dream-specific role, effort, timeout, and fallback fields as request policies.
- [ ] Emit caller `dream_review`.
- [ ] Prove dream consolidation output appears as a gateway request and a cognitive feedback episode.

### Neuro Distillation

- [ ] Replace direct `ANTHROPIC_API_KEY` read with `ModelCallService`.
- [ ] Replace `Distiller::with_claude(api_key)` with a distiller trait that accepts a model-call client.
- [ ] Run distillation as a supervised operation, not detached unmanaged `tokio::spawn`.
- [ ] Emit caller `neuro_distillation`.
- [ ] Prove failed credentials appear as `missing_credentials` instead of silent no-op.

### Vision Loop

- [ ] Convert `VisionEvaluator` to use `ModelCallService`.
- [ ] Preserve structured multimodal messages; do not flatten image data into provider-specific text unless the provider adapter requires it.
- [ ] Attach vision capability requirement to `ModelSelectionRequest`.
- [ ] Emit caller `vision_loop`.
- [ ] Prove a vision request selects only models with `supports_vision`.

### Built-In Web Search Tool

- [ ] Replace direct `PERPLEXITY_API_KEY` read with a `SearchCapability` provided by runtime services.
- [ ] If Perplexity remains the backend, obtain credentials through `SecretService` and emit gateway/search events.
- [ ] Classify missing credentials, auth failures, and rate limits.
- [ ] Emit caller `tool_web_search`.
- [ ] Prove tool usage is visible in runtime projections.

### Legacy Orchestrate

- [ ] Freeze new model-call work in `orchestrate.rs`.
- [ ] Route any still-production legacy calls through `ModelCallService` with caller `legacy_orchestrate`.
- [ ] Emit a warning event whenever legacy calls the gateway.
- [ ] Remove direct Perplexity search clients and env reads from `orchestrate.rs` before claiming runtime convergence.
- [ ] Prove `rg -n "PerplexitySearchClient::new|std::env::var\\(\"PERPLEXITY_API_KEY\"\\)" crates/roko-cli/src/orchestrate.rs` returns no production call sites.

## Batch Redesign

The current batch endpoint should not be extended in place. Replace it with a durable gateway batch service.

Batch service contract:

- [ ] `submit_batch` creates a durable operation before returning.
- [ ] Each item receives a stable `custom_id`.
- [ ] Batch preprocessing runs LoopDetect, CacheLookup, ToolPrune, OutputBudget, ThinkingCap, and ConvergenceDetect before provider submission.
- [ ] Cache hits complete immediately and are not sent to provider batch.
- [ ] Provider-backed batch submissions use provider batch APIs where supported.
- [ ] Unsupported providers fall back to durable queued single calls and mark `batch_mode = emulated`.
- [ ] Batch results are stored durably and queryable after restart.
- [ ] Batch polling is supervised and cancellable.
- [ ] Batch cost applies provider discount only when provider batch API was actually used.
- [ ] The response exposes `batch_mode = provider_batch | emulated | cache_only`.

Provider support:

- [ ] Anthropic API: provider batch path.
- [ ] OpenAI-compatible providers: implement only where the provider supports batch; otherwise `emulated`.
- [ ] Perplexity: classify whether provider supports async/deep research separately from generic batch.
- [ ] Claude CLI: `unsupported` or `emulated`, never pretend provider batch discount.
- [ ] Codex CLI: `unsupported` or `emulated`, never pretend provider batch discount.

## Configuration And Secrets

This depends on [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md), but the gateway needs stricter local rules:

- [ ] `SecretService` is the only component that reads API key environment variables.
- [ ] `ModelCallService` receives credential handles, not raw secret strings.
- [ ] Domain crates cannot call `std::env::var("ANTHROPIC_API_KEY")`, `std::env::var("OPENAI_API_KEY")`, `std::env::var("MOONSHOT_API_KEY")`, `std::env::var("ZAI_API_KEY")`, `std::env::var("PERPLEXITY_API_KEY")`, or `std::env::var("GEMINI_API_KEY")`.
- [ ] HTTP responses never include secret values or full authorization headers.
- [ ] Proof artifacts redact secrets at write time.
- [ ] Missing credential is a first-class status, not a provider construction error string.

## Provider Matrix Proof

The provider proof harness should run through `ModelCallService::probe_provider` and nothing else.

Required matrix rows:

- [ ] Anthropic API
- [ ] OpenAI API
- [ ] Moonshot API
- [ ] Z.AI API
- [ ] Perplexity API
- [ ] Gemini API if configured
- [ ] Claude CLI
- [ ] Codex CLI

Required statuses:

- [ ] `proved`: live request succeeded and gateway events/projections match.
- [ ] `missing_credentials`: required credential absent.
- [ ] `auth_failed`: provider rejected credential.
- [ ] `rate_limited`: provider returned a rate limit status.
- [ ] `unsupported`: provider/model cannot satisfy the requested capability.
- [ ] `timeout`: request exceeded configured timeout.
- [ ] `provider_error`: provider failed for another classified reason.

Required proof artifact fields:

- [ ] `provider_id`
- [ ] `model_key`
- [ ] `model_slug`
- [ ] `transport`
- [ ] `status`
- [ ] `operation_id`
- [ ] `request_id`
- [ ] `gateway_event_count`
- [ ] `projection_seen`
- [ ] `latency_ms`
- [ ] `input_tokens`
- [ ] `output_tokens`
- [ ] `cost_usd`
- [ ] `redacted_error`
- [ ] `timestamp`

Proof command target:

```bash
tests/proof/mori-diffs/prove-provider-matrix-through-gateway.sh
```

That script should fail if a provider probe bypasses the gateway or if projections do not reflect the probe.

## Grep Gates

These commands define the no-regression surface for the migration. They should eventually be automated in the proof harness.

```bash
# HTTP gateway routes should not call the generic CLI runtime directly.
rg -n "run_once\\(" crates/roko-serve/src/routes/gateway.rs

# Gateway stats should not be fixed placeholder cache values.
rg -n "cache_hits: u64 = 0|cache_read_tokens: 0|cache_read_tokens: u64 = 0" crates/roko-serve/src/routes/gateway.rs

# Production call sites outside provider adapters/gateway should not construct agents directly.
rg -n "create_agent_for_model|spawn_agent_scoped|PerplexitySearchClient::new" \
  crates/roko-cli/src/commands \
  crates/roko-cli/src/vision_loop \
  crates/roko-dreams/src \
  crates/roko-neuro/src \
  crates/roko-std/src \
  crates/roko-serve/src/routes

# Domain crates and tools should not read provider secrets directly.
rg -n "std::env::var\\(\\\"(ANTHROPIC|OPENAI|MOONSHOT|ZAI|PERPLEXITY|GEMINI)_API_KEY\\\"" \
  crates/roko-dreams/src crates/roko-neuro/src crates/roko-std/src crates/roko-compose/src crates/roko-serve/src/routes crates/roko-cli/src/commands

# Gateway container should not be a placeholder.
rg -n "placeholder|does not yet exist|--bin roko|roko-gateway crate" docker/gateway.Dockerfile docker/docker-compose.yml

# There should be a concrete gateway/inference library or binary crate.
find crates -maxdepth 2 -name Cargo.toml | sort | rg "roko-(gateway|inference)"
```

During migration, these commands may still return known sites. A checklist item is complete only when the corresponding command is empty or every remaining hit is in an allowlisted provider adapter, test, or compatibility shim.

## Implementation Batches

### Batch A: Service Crate And Contract

- [ ] Create `ModelCallService` trait and request/response/error types in a crate that CLI, serve, dreams, neuro, tools, and proof code can depend on without dependency cycles.
- [ ] Create `InferenceGateway` production implementation with injected provider dispatcher, secret service, event store, cache, cost calculator, and clock.
- [ ] Create `NoopModelCallService` only for unit tests that do not prove runtime behavior; do not use it in proof scripts.
- [ ] Add typed caller enum and status enum.
- [ ] Add conversion from `AgentResult` / `AgentRuntimeEvent` to gateway response/events.
- [ ] Add conversion from structured chat messages to provider request input without flattening unless provider-specific code requires it.

Acceptance:

- [ ] `cargo check -p <service-crate>` passes.
- [ ] `cargo test -p <service-crate> model_call_contract` passes.
- [ ] No server route type leaks into the service crate.

### Batch B: Durable Events And Projections

- [ ] Add gateway event writer using the runtime event store from [34](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md).
- [ ] Add `GatewayProjection`.
- [ ] Add provider health projection updates from gateway provider events.
- [ ] Add request detail query by request id.
- [ ] Replace `gateway_model_counters` as source of truth.

Acceptance:

- [ ] A synthetic model call writes request, provider, cost, and completion events.
- [ ] Projection totals match event replay after process restart.
- [ ] `/api/gateway/stats` reads projection totals.

### Batch C: Provider Execution Cell

- [ ] Move provider execution under `ProviderCallCell`.
- [ ] Reuse `ProviderDispatchResolver` and `AgentDispatcherV2` rather than duplicating provider construction.
- [ ] Implement status classification.
- [ ] Implement fallback attempt loop.
- [ ] Emit provider lifecycle events.
- [ ] Add stream support with provider-neutral chunks.

Acceptance:

- [ ] One Anthropic/OpenAI-compatible live call through the gateway records provider lifecycle events.
- [ ] One missing credential probe returns `missing_credentials`.
- [ ] One unsupported capability request returns `unsupported`.

### Batch D: HTTP Gateway Adapter

- [ ] Replace `routes/gateway.rs` internals with calls to `ModelCallService`.
- [ ] Keep request/response compatibility shims where needed.
- [ ] Remove placeholder cache fields from response generation.
- [ ] Add request detail and projection-backed stats endpoints.
- [ ] Route provider test endpoint through `probe_provider`.

Acceptance:

- [ ] `curl /api/gateway/inference` emits durable gateway events.
- [ ] `curl /api/gateway/stats` changes after a request and remains correct after restart.
- [ ] `/api/providers/{id}/test` and `/api/gateway/inference` share request/projection code.

### Batch E: CLI And Runner Adapter

- [ ] Wire runner provider execution through `ModelCallService`.
- [ ] Preserve current runner dispatch outcomes and `AgentRuntimeEvent` stream.
- [ ] Add gateway event ids to runner task projection.
- [ ] Route one-shot `run`, PRD, plan, and workflow generation through the same call path when they need a model.
- [ ] Keep prompt assembly in `PromptAssembler`; gateway starts after prompt/messages are assembled.

Acceptance:

- [ ] `roko plan run` produces runner events and gateway events for the same task.
- [ ] `roko run "hello"` appears in gateway request query.
- [ ] Prompt diagnostics are linked from gateway request query.

### Batch F: Domain Caller Migration

- [ ] Convert research calls.
- [ ] Convert dream review calls.
- [ ] Convert neuro distillation calls.
- [ ] Convert vision evaluator calls.
- [ ] Convert built-in web search capability.
- [ ] Quarantine or remove legacy orchestrate direct provider/search calls.

Acceptance:

- [ ] Grep gates for direct env secret reads in domain/tool crates are empty or test-only.
- [ ] Grep gates for direct `create_agent_for_model` outside provider/gateway/dispatch are empty or allowlisted.
- [ ] Each caller enum has at least one integration proof.

### Batch G: Cache, Budget, Thinking, Convergence

- [ ] Implement L1 request cache.
- [ ] Implement or explicitly disable L2 semantic cache with events.
- [ ] Implement output budget cell.
- [ ] Implement thinking cap cell.
- [ ] Implement loop/convergence cells.
- [ ] Implement cost calculator with cache and batch fields.

Acceptance:

- [ ] Repeated request returns `cache_hit` and does not call provider.
- [ ] Output budget event records original/effective max tokens.
- [ ] Cost projection includes cache, reasoning/thinking, and savings fields.

### Batch H: Batch Service

- [ ] Replace route-local `tokio::spawn` fan-out with a supervised durable batch service.
- [ ] Implement provider-backed batch for supported providers.
- [ ] Implement emulated mode for unsupported providers.
- [ ] Add durable batch status and result queries.
- [ ] Add cancellation and restart recovery.

Acceptance:

- [ ] Batch survives process restart.
- [ ] Cache-only batch item completes without provider call.
- [ ] Provider-batch cost discount is applied only for provider-backed mode.
- [ ] Unsupported provider returns `unsupported` or `emulated` explicitly.

### Batch I: Docker And Deployment

- [ ] Decide whether gateway is an embedded service, standalone binary, or both.
- [ ] If standalone, create `crates/roko-gateway` binary that wraps the shared service crate.
- [ ] If embedded, remove or rename placeholder gateway container so it does not imply a false service.
- [ ] Update compose health checks to prove real gateway endpoints.
- [ ] Update Prometheus scrape targets to real metrics.

Acceptance:

- [ ] `docker/gateway.Dockerfile` has no placeholder TODO.
- [ ] Compose `gateway` health check calls a real endpoint.
- [ ] Gateway image does not build `--bin roko` as a stand-in.

## End-To-End Proof Matrix

Each row must have a reproducible script, a saved proof artifact, and a query assertion.

| Proof | Command | Required Evidence |
|---|---|---|
| Real provider request | `tests/proof/mori-diffs/prove-gateway-live-call.sh anthropic` | Request id, provider event, completion event, projection row. |
| Missing credential | `tests/proof/mori-diffs/prove-gateway-missing-credential.sh anthropic` | Status `missing_credentials`, no raw secret in output. |
| Provider matrix | `tests/proof/mori-diffs/prove-provider-matrix-through-gateway.sh` | One row per provider with classified status. |
| HTTP query proof | `tests/proof/mori-diffs/prove-gateway-http-query.sh` | `/api/gateway/requests/{id}` and `/api/gateway/stats` agree with event log. |
| Cache proof | `tests/proof/mori-diffs/prove-gateway-cache.sh` | Second request has `cache_hit = true` and provider attempt count unchanged. |
| Fallback proof | `tests/proof/mori-diffs/prove-gateway-fallback.sh` | First provider failure, fallback selected, served model differs from original. |
| Batch proof | `tests/proof/mori-diffs/prove-gateway-batch.sh` | Durable batch id, item status, result query, cost mode. |
| Restart proof | `tests/proof/mori-diffs/prove-gateway-restart-replay.sh` | Stats and request details survive server restart. |
| Runner proof | `tests/proof/mori-diffs/prove-runner-uses-gateway.sh` | One task id maps to one gateway request id. |
| Research proof | `tests/proof/mori-diffs/prove-research-uses-gateway.sh` | Research artifact plus gateway caller `research`. |
| Dream proof | `tests/proof/mori-diffs/prove-dream-uses-gateway.sh` | Dream report plus gateway caller `dream_review`. |
| Neuro proof | `tests/proof/mori-diffs/prove-neuro-distillation-uses-gateway.sh` | Distilled knowledge or classified failure plus gateway caller `neuro_distillation`. |
| Web search proof | `tests/proof/mori-diffs/prove-web-search-uses-gateway.sh` | Search results or classified failure plus gateway/tool event. |
| Vision proof | `tests/proof/mori-diffs/prove-vision-uses-gateway.sh` | Vision model capability assertion plus gateway caller `vision_loop`. |

## Definition Of Done

The gateway work is not done when a route returns text. It is done when:

- [ ] There is exactly one production `ModelCallService` used by runner, CLI one-shot, HTTP inference, provider tests, research, dreams, neuro distillation, vision, and web search.
- [ ] Every model call emits durable gateway events.
- [ ] Gateway stats are rebuilt from events/projections after restart.
- [ ] Provider proof matrix runs through the gateway and records classified statuses.
- [ ] Direct provider secret reads are eliminated from domain/tool crates.
- [ ] Route-local `run_once` calls are removed from inference gateway endpoints.
- [ ] Cache, output budget, thinking cap, provider fallback, and cost tracking are either implemented or explicitly disabled with durable events and proof statuses.
- [ ] Batch API is durable and honest about provider-backed versus emulated mode.
- [ ] Docker/compose no longer advertise a placeholder gateway as a working service.
- [ ] The grep gates above are automated and passing.
- [ ] The proof matrix above produces artifacts from a clean clone with credentials supplied only through the configured secret mechanism.

## Implementation Notes For The Next Agent

Start with the service boundary, not the HTTP route.

1. Implement the trait/types and event contract first.
2. Build a minimal `InferenceGateway` that supports `complete` with provider execution and durable events.
3. Replace the HTTP route internals with that service.
4. Replace provider test with `probe_provider`.
5. Add projections and proof.
6. Move runner dispatch onto the same service.
7. Migrate domain callers.
8. Add cache/budget/thinking/convergence cells.
9. Replace batch.
10. Fix Docker/compose.

Avoid these traps:

- Do not make `routes/gateway.rs` bigger.
- Do not solve research, dreams, and neuro with separate local clients.
- Do not count `create_agent_for_model` as gateway migration.
- Do not create new direct `std::env::var("*_API_KEY")` reads.
- Do not use in-memory counters as proof.
- Do not report cache savings while `cache_hits` is hardcoded to zero.
- Do not claim provider batch discount for local concurrent fan-out.

## Cross-Doc Links

- [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) should treat this as a P0 runtime convergence item.
- [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md) provides repository-wide evidence for duplicated runtime ownership.
- [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md) names `ModelCallService` as the intended dependency inversion boundary.
- [33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md](33-CONFIGURATION-PROVIDER-POLICY-AUDIT.md) covers config and secret policy that the gateway must consume.
- [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) covers event/projection/query design for gateway proof.
- [35-TASK-PROCESS-LIFECYCLE-AUDIT.md](35-TASK-PROCESS-LIFECYCLE-AUDIT.md) covers supervised background work needed by batch, provider probes, and neuro distillation.
- [38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md](38-COGNITIVE-FEEDBACK-LOOP-AUDIT.md) covers how gateway outcomes should feed learning, dreams, affect, and routing.
- [39-RUNNER-EXECUTION-POLICY-AUDIT.md](39-RUNNER-EXECUTION-POLICY-AUDIT.md) covers runner execution decisions that should call the gateway.
- [40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md](40-SERVE-TUI-RUNTIME-ADAPTER-AUDIT.md) covers why server routes and TUI must become adapters over shared runtime services.

## Self-Grade

Initial grade before this pass: 9.31 / 10.

Reason: prior docs mentioned `ModelCallService` and provider proof, but the problem was spread across config, serve, and dependency notes. An implementer could miss that `routes/gateway.rs` is a facade and that research, dreams, neuro, vision, web search, provider probes, batch, Docker, and HTTP stats all need to converge on one service.

Iteration changes made in this pass:

- Added source-verified evidence for the HTTP gateway route, placeholder Docker gateway, missing crate, direct provider call sites, direct secret reads, volatile counters, placeholder cache metrics, and local fan-out batch.
- Split `ProviderAdapter`, `Dispatcher`, and `ModelCallService` responsibilities so the target design does not create another monolith.
- Added concrete request, event, query, batch, config, proof, and migration contracts.
- Added per-call-site migration checklists for runner, HTTP, provider tests, research, dreams, neuro, vision, web search, and legacy orchestrate.
- Added grep gates and reproducible proof matrix rows.

Final grade after this pass: 9.86 / 10.

Remaining risk: this is still a design/audit document, not implementation. The main ambiguity left is crate placement (`roko-runtime`, `roko-inference`, or `roko-gateway`) because that should be decided with the actual dependency graph during implementation. The service boundary and proof requirements are concrete enough that another agent can implement in batches without reading the rest of the repository first.

Self-grade validation note: Current self-grade is `9.86 / 10`; this file is above the requested threshold and remains open until gateway/provider-matrix proof and direct-call-site migration pass.

# Runtime Convergence + Demo Batches

Runner: `tmp/runners/converge/run-converge.sh`
Strategy: Feature-gate `orchestrate.rs` behind `legacy-orchestrate`, new engine default, `--engine legacy` for old path.

## Track F: Foundation Fixes (resolve crate cycle + trait duplication from arch runner)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| F01 | Invert crate dep: roko-core must not depend on roko-runtime | `roko-core/Cargo.toml`, `roko-core/src/lib.rs` | — |
| F02 | Remove local trait copies from effect_driver, use roko-core | `roko-runtime/Cargo.toml`, `roko-runtime/src/effect_driver.rs` | F01 |
| F03 | Remove local RuntimeEvent from effect_driver, use roko-core | `roko-runtime/src/effect_driver.rs` | F02 |
| F04 | Update JsonlLogger to use roko-core EventConsumer + all RuntimeEvent variants | `roko-runtime/src/jsonl_logger.rs` | F03 |
| F05 | Update RuntimeProjection to parse all RuntimeEvent variants | `roko-runtime/src/projection.rs` | F03 |
| F06 | Remove WorkflowEvent/WorkflowEventConsumer, use roko-core EventConsumer | `roko-runtime/src/workflow_engine.rs` | F03 |

## Track S: Service Enhancement (make foundation services production-ready)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| S01 | ModelCallService: wire to existing provider dispatch (claude CLI backend) | `roko-agent/src/model_call_service.rs` | F02 |
| S02 | ModelCallService: add Anthropic API + OpenAI-compat backends | `roko-agent/src/model_call_service.rs` | S01 |
| S03 | ModelCallService: integrate CascadeRouter for model selection | `roko-agent/src/model_call_service.rs` | S01 |
| S04 | ModelCallService: add cost tracking + cost prediction | `roko-agent/src/model_call_service.rs` | S01 |
| S05 | ModelCallService: add MCP config passthrough | `roko-agent/src/model_call_service.rs` | S01 |
| S06 | PromptAssemblyService: wire full 9-layer SystemPromptBuilder | `roko-compose/src/prompt_assembly_service.rs` | F02 |
| S07 | PromptAssemblyService: add ContextSource for neuro/knowledge store | `roko-compose/src/prompt_assembly_service.rs` | S06 |
| S08 | PromptAssemblyService: add ContextSource for episodes + playbooks | `roko-compose/src/prompt_assembly_service.rs` | S06 |
| S09 | PromptAssemblyService: add section effectiveness scoring | `roko-compose/src/prompt_assembly_service.rs` | S06 |
| S10 | FeedbackService: add episode recording sink | `roko-learn/src/feedback_service.rs` | F02 |
| S11 | FeedbackService: add CascadeRouter bandit observation | `roko-learn/src/feedback_service.rs` | S10 |
| S12 | GateService: add remaining rungs 3-6 (diff, review, security, LLM judge) | `roko-gate/src/gate_service.rs` | F02 |
| S13 | GateService: add adaptive threshold integration | `roko-gate/src/gate_service.rs` | S12 |

## Track E: Engine Enhancement (make WorkflowEngine production-ready)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| E01 | PipelineStateV2: add TOML workflow config loading | `roko-runtime/src/pipeline_state.rs` | F06 |
| E02 | PipelineStateV2: add checkpoint serialization + resume | `roko-runtime/src/pipeline_state.rs` | E01 |
| E03 | EffectDriver: wire real agent spawn via ModelCallService | `roko-runtime/src/effect_driver.rs` | F03 S01 |
| E04 | EffectDriver: implement commit effect (git add + commit) | `roko-runtime/src/effect_driver.rs` | F03 |
| E05 | EffectDriver: implement checkpoint effect (state persistence) | `roko-runtime/src/effect_driver.rs` | E02 E04 |
| E06 | WorkflowEngine: add progress callback + event emission | `roko-runtime/src/workflow_engine.rs` | F06 E03 |
| E07 | WorkflowEngine: add cancellation via CancellationToken | `roko-runtime/src/workflow_engine.rs` | E06 |
| E08 | WorkflowEngine: add resume from checkpoint | `roko-runtime/src/workflow_engine.rs` | E02 E06 |

## Track W: Wiring (connect WorkflowEngine to live CLI paths)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| W01 | Add --engine v2/legacy flag to roko run CLI | `roko-cli/src/run.rs` | E06 |
| W02 | Wire WorkflowEngine as default for `roko run` | `roko-cli/src/run.rs` | W01 |
| W03 | Wire WorkflowEngine adapter construction in run.rs | `roko-cli/src/run.rs` | W02 S01 S06 S10 S12 |
| W04 | Wire WorkflowEngine to `roko plan run` entry point | `roko-cli/src/orchestrate.rs`, `roko-cli/src/run.rs` | W03 |
| W05 | Wire ACP bridge_events to use WorkflowEngine | `roko-acp/src/bridge_events.rs`, `roko-acp/src/runner.rs` | E06 |
| W06 | Wire unified.rs oneshot to use ModelCallService | `roko-cli/src/unified.rs`, `roko-cli/src/dispatch_direct.rs` | S01 |
| W07 | Wire roko-serve background tasks to WorkflowEngine events | `roko-serve/src/lib.rs` | E06 |
| W08 | Wire roko-serve SSE to WorkflowEngine RuntimeEvent stream | `roko-serve/src/adapters.rs`, `roko-serve/src/routes/mod.rs` | W07 |

## Track O: Observability (real-time visibility)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| O01 | Wire JsonlLogger as WorkflowEngine EventConsumer | `roko-cli/src/run.rs` | F04 W02 |
| O02 | Wire RuntimeProjection to roko-serve dashboard routes | `roko-serve/src/routes/mod.rs` | F05 |
| O03 | Bridge WorkflowEngine events to StateHub (TUI dashboard) | `roko-cli/src/run.rs` | W02 |
| O04 | Create CLI progress printer (Clack-style structured output) | `roko-cli/src/output_format.rs` (new) | — |
| O05 | Wire CLI progress printer into cmd_run workflow engine path | `roko-cli/src/run.rs` | O04 W02 |
| O06 | Wire efficiency event summary to CLI post-run output | `roko-cli/src/run.rs` | O05 S04 |

## Track R: Retirement (feature-gate orchestrate.rs)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| R01 | Add legacy-orchestrate feature to roko-cli Cargo.toml | `roko-cli/Cargo.toml` | — |
| R02 | Feature-gate orchestrate.rs module behind legacy-orchestrate | `roko-cli/src/lib.rs`, `roko-cli/src/orchestrate.rs` | R01 W04 |
| R03 | Feature-gate dispatch_helpers + agent_spawn behind legacy | `roko-cli/src/lib.rs` | R02 |
| R04 | Ensure cargo check passes without legacy-orchestrate feature | `roko-cli/src/run.rs` | R03 |
| R05 | Ensure cargo check passes with legacy-orchestrate feature | `roko-cli/src/run.rs` | R04 |

## Track C: CLI + Demo Features

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| C01 | Create output_format.rs — Clack-style symbols + ANSI colors | `roko-cli/src/output_format.rs` (new) | — |
| C02 | Identity line: agent name + model + routing decision | `roko-cli/src/output_format.rs` | C01 |
| C03 | Cost prediction line: estimated tokens + cost before execution | `roko-cli/src/output_format.rs` | C02 S04 |
| C04 | Knowledge loading line: facts loaded + confidence | `roko-cli/src/output_format.rs` | C02 |
| C05 | Cost actual + delta: real cost vs predicted after execution | `roko-cli/src/output_format.rs` | C03 |
| C06 | Gate results: formatted pass/fail with timing | `roko-cli/src/output_format.rs` | C01 |
| C07 | --share flag: generate share token + store run data | `roko-cli/src/run.rs`, `roko-core/src/lib.rs` | W02 |
| C08 | Share endpoint: GET /api/shared/:token in roko-serve | `roko-serve/src/routes/shared_runs.rs` | C07 |
| C09 | Agent list: formatted output for `roko agent list` | `roko-cli/src/output_format.rs` | C01 |
| C10 | Replay: formatted output for `roko replay` | `roko-cli/src/output_format.rs` | C01 |
| C11 | Dashboard SPA: wire knowledge + learning + agents API endpoints | `demo/demo-app/src/pages/` | O02 |
| C12 | Dashboard SPA: add share page at /share/:token | `demo/demo-app/src/pages/` | C08 |

## Track T: Integration Tests

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| T01 | Integration test: WorkflowEngine express workflow | `roko-runtime/src/workflow_engine.rs` | E06 |
| T02 | Integration test: WorkflowEngine standard workflow with gate | `roko-runtime/src/workflow_engine.rs` | T01 |
| T03 | Integration test: WorkflowEngine checkpoint + resume | `roko-runtime/src/workflow_engine.rs` | E08 |
| T04 | Integration test: CLI --engine v2 flag parses correctly | `roko-cli/src/run.rs` | W01 |
| T05 | Integration test: share URL generation + retrieval | `roko-serve/src/routes/shared_runs.rs` | C08 |

## Track D: Daimon Refactor (extract AffectPolicy, wire to WorkflowEngine)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| D01 | Extract AffectPolicy trait to roko-core foundation | `roko-core/src/foundation.rs` | — |
| D02 | Implement DaimonPolicy wrapping DaimonState | `roko-daimon/src/policy.rs` (new), `roko-daimon/src/lib.rs`, `roko-daimon/Cargo.toml` | D01 |
| D03 | Wire AffectPolicy into WorkflowEngine + EffectDriver | `roko-runtime/src/workflow_engine.rs`, `roko-runtime/src/effect_driver.rs` | D01 E06 |
| D04 | Wire DaimonPolicy into CLI run path | `roko-cli/src/run.rs`, `roko-cli/Cargo.toml` | D02 D03 W02 |

## Track G: Gateway Consolidation (unify 5 provider abstractions)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| G01 | Service contract: unified request/response types | `roko-core/src/foundation.rs` | — |
| G02 | Durable gateway event writer + projection | `roko-agent/src/gateway_events.rs` (new), `roko-agent/src/lib.rs` | G01 |
| G03 | ProviderCallCell: move provider execution into cell | `roko-agent/src/model_call_service.rs` | G01 S01 |
| G04 | HTTP gateway adapter: wire routes to ModelCallService | `roko-serve/src/routes/gateway.rs` | G03 |
| G05 | CLI runner adapter: ensure EffectDriver uses ModelCallService | `roko-cli/src/run.rs` | G03 W02 |
| G06 | Domain caller migration (research, dreams, neuro) | `roko-dreams/src/runner.rs`, `roko-neuro/src/episode_completion.rs`, `roko-std/src/tool/builtin/web_search.rs` | G03 |
| G07 | Cache cell + budget cell in ModelCallService | `roko-agent/src/model_call_service.rs` | G03 |
| G08 | Thinking cap + convergence detection cells | `roko-agent/src/model_call_service.rs` | G03 |
| G09 | Wire force_backend to CascadeRouter learning | `roko-agent/src/model_call_service.rs`, `roko-learn/src/cascade_router.rs` | G03 S03 |

## Track K: Knowledge Feedback Loop (neuro → routing → prompts)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| K01 | Add knowledge-aware routing method to CascadeRouter | `roko-learn/src/cascade_router.rs` | S03 |
| K02 | Wire knowledge query into ModelCallService routing | `roko-agent/src/model_call_service.rs` | K01 G03 |
| K03 | Inject knowledge into prompt assembly | `roko-compose/src/prompt_assembly_service.rs` | S07 |
| K04 | Record knowledge usage in episode metadata | `roko-learn/src/feedback_service.rs` | S10 K01 |
| K05 | Knowledge confidence update loop | `roko-neuro/src/lib.rs` | K04 |

## Track X: Security Hardening

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| X01 | Fix contract fail-open to fail-closed | `roko-agent/src/safety/mod.rs` | — |
| X02 | Consolidate stream JSON parsers | `roko-agent/src/streaming.rs`, `roko-agent/src/lib.rs` | — |

## Track L: Layering Firewall (enforce crate dependency rules)

| Batch | Title | Write Scope | Deps |
|-------|-------|-------------|------|
| L01 | Add layer metadata to all Cargo.toml files | All `crates/*/Cargo.toml` | F01 |
| L02 | Create layer-check binary | `roko-cli/src/layer_check.rs` (new), `roko-cli/src/lib.rs` | L01 |
| L03 | Configure cargo-deny | `deny.toml` (new) | — |
| L04 | Add layer-check to CI workflow | `.github/workflows/ci.yml` | L02 |

## Summary

| Track | Batches | Focus |
|-------|---------|-------|
| F (Foundation) | 6 | Fix crate cycle, unify trait duplication |
| S (Services) | 13 | Make foundation services production-ready |
| E (Engine) | 8 | Make WorkflowEngine production-ready |
| W (Wiring) | 8 | Connect to live CLI/ACP/serve paths |
| O (Observability) | 6 | Real-time visibility + CLI output |
| R (Retirement) | 5 | Feature-gate orchestrate.rs |
| C (CLI/Demo) | 12 | Demo features + formatted output |
| T (Tests) | 5 | Integration tests |
| D (Daimon) | 4 | Refactor affect engine for WorkflowEngine |
| G (Gateway) | 9 | Consolidate provider abstractions |
| K (Knowledge) | 5 | Wire neuro → routing → prompts |
| X (Security) | 2 | Contract hardening + parser consolidation |
| L (Layering) | 4 | Enforce crate dependency rules |
| **Total** | **87** | |

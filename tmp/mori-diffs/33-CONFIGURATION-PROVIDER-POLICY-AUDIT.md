# 33 - Configuration And Provider Policy Audit

Date: 2026-04-27

Purpose: this file documents the configuration, credential, provider-selection, and safety-policy architecture gaps that prevent Roko from reliably proving all providers end to end. It complements [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md), [31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md](31-REPOSITORY-WIDE-ARCHITECTURE-SCAN.md), and [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).

If an agent is assigned "make all providers work from config" or "fix global config and env keys", this file is the implementation handoff.

## Executive Verdict

Roko has many pieces of a good config system, but they are not authoritative. There is a rich `RokoConfig` schema, a layered CLI config loader, provider profiles, secret stores, secret scrubbers, and provider adapters. The problem is that runtime entrypoints can still read env vars directly, synthesize providers ad hoc, pass secrets through env maps, construct agents outside the dispatch path, and choose unsafe permissions locally.

This means provider failures are hard to reason about. A live provider can work in one path and fail in another because each path resolves keys, model profiles, command defaults, safety flags, and timeouts differently.

Initial self-grade after this pass: `9.83 / 10`.

Reason: this pass includes repository-wide scan counts, concrete source evidence, a target config/policy design, and implementation checklists. It is not a `10` because a complete proof would include a generated config-source graph and live provider proof artifacts.

## Method

Commands used during this pass:

```bash
python3 - <<'PY'
from pathlib import Path
import re
root=Path('/Users/will/dev/nunchi/roko/roko')
files=sorted(root.glob('crates/**/*.rs'))
patterns={
  'env': re.compile(r'std::env::(?:var|var_os|set_var|remove_var)|env::(?:var|var_os|set_var|remove_var)'),
  'dotenv': re.compile(r'dotenv|\.env'),
  'provider_key': re.compile(r'ANTHROPIC|OPENAI|MOONSHOT|ZAI|Z_AI|PERPLEXITY|CLAUDE|CODEX|GEMINI|GOOGLE_API|OPENROUTER|OLLAMA'),
  'roko_env': re.compile(r'ROKO_[A-Z0-9_]+'),
  'config_file': re.compile(r'roko\.toml|config\.toml|RokoConfig|Config'),
  'unsafe': re.compile(r'dangerously_skip_permissions|dangerously-bypass|dangerously-skip|bypassPermissions|skip_permissions'),
  'provider_create': re.compile(r'create_agent_for_model|AgentOptions|ProviderProfile|ProviderConfig|model_key|model_slug|provider='),
}
for path in files:
    text=path.read_text(errors='ignore')
    counts={k: len(p.findall(text)) for k,p in patterns.items()}
    if any(counts.values()):
        print(path.relative_to(root), counts)
PY
```

```bash
rg -n "ANTHROPIC|OPENAI|MOONSHOT|ZAI|Z_AI|PERPLEXITY|CLAUDE|CODEX|GEMINI|GOOGLE_API|OPENROUTER|OLLAMA|ROKO_[A-Z0-9_]+|dangerously_skip_permissions|dangerously-bypass|dangerously-skip|std::env::var|env::var|\\.env|roko\\.toml|create_agent_for_model|AgentOptions|ProviderProfile|ProviderConfig" crates -g '*.rs'
```

The scan intentionally overcounts tests and comments, but the hotspots correctly identify where config and provider policy are owned outside one runtime context.

## Current Scan Counts

| Crate | Env Reads | Dotenv Refs | Provider Key Refs | `ROKO_*` Refs | Config Refs | Unsafe Refs | Provider Create Refs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `roko-cli` | 73 | 134 | 103 | 113 | 1486 | 72 | 305 |
| `roko-core` | 15 | 19 | 24 | 98 | 666 | 1 | 79 |
| `roko-agent` | 11 | 20 | 35 | 17 | 340 | 16 | 256 |
| `roko-serve` | 13 | 31 | 10 | 11 | 268 | 1 | 108 |
| `roko-learn` | 2 | 0 | 6 | 0 | 106 | 0 | 203 |
| `roko-chain` | 1 | 0 | 0 | 2 | 228 | 0 | 0 |
| `roko-orchestrator` | 1 | 9 | 0 | 0 | 125 | 0 | 0 |
| `roko-acp` | 0 | 1 | 1 | 0 | 106 | 1 | 11 |
| `roko-runtime` | 1 | 2 | 0 | 2 | 111 | 0 | 0 |
| `roko-dreams` | 0 | 1 | 0 | 0 | 99 | 1 | 6 |
| `roko-gate` | 3 | 53 | 0 | 0 | 17 | 0 | 0 |
| `roko-compose` | 1 | 5 | 0 | 0 | 42 | 0 | 6 |
| `roko-std` | 2 | 0 | 8 | 18 | 0 | 0 | 0 |
| `roko-agent-server` | 1 | 0 | 0 | 2 | 19 | 0 | 2 |
| `roko-neuro` | 1 | 0 | 1 | 0 | 9 | 0 | 3 |

Interpretation:

- `roko-cli` is still the dominant config owner and provider-policy owner.
- `roko-core` owns schema, but also reads env and synthesizes effective providers.
- `roko-agent` owns provider adapters, but also reads env and carries unsafe flags.
- `roko-serve` has enough config/provider logic to behave differently from CLI.
- `roko-std` and `roko-neuro` read provider keys directly for some capabilities.

## Hot Files

| File | Why It Matters |
| --- | --- |
| `crates/roko-cli/src/config.rs` | Defines a separate CLI config model, layered loader, global config path, env overrides, and merge behavior. |
| `crates/roko-core/src/config/schema.rs` | Defines `RokoConfig`, but also checks env vars and synthesizes providers/models. |
| `crates/roko-agent/src/provider/mod.rs` | Provider factory is useful, but it also handles mock env, safety thread-locals, fallback `ExecAgent`, and default provider synthesis. |
| `crates/roko-cli/src/orchestrate.rs` | Legacy runtime still reads config/env, chooses providers, and sets unsafe policy. |
| `crates/roko-cli/src/dispatch_v2.rs` | CLI dispatch builds provider requests and unsafe CLI arguments. |
| `crates/roko-cli/src/runner/agent_stream.rs` | Runner stream path carries env and unsafe flags into CLI subprocesses. |
| `crates/roko-serve/src/dispatch.rs` | Server dispatch creates agents and sets `dangerously_skip_permissions`. |
| `crates/roko-serve/src/routes/providers.rs` | HTTP route creates agents directly for provider test calls. |
| `crates/roko-dreams/src/runner.rs` | Dream runner creates agents directly instead of using runtime dispatch. |
| `crates/roko-neuro/src/episode_completion.rs` | Reads `ANTHROPIC_API_KEY` directly inside a domain crate. |
| `crates/roko-std/src/tool/builtin/web_search.rs` | Built-in web search reads `PERPLEXITY_API_KEY` directly. |
| `crates/roko-core/src/secrets/resolve.rs` | Good secret resolver exists, but it is not the mandatory provider credential path. |
| `crates/roko-core/src/config/provider.rs` | `ProviderConfig::resolve_api_key` reads direct env instead of using `SecretResolver`. |
| `crates/roko-cli/src/config_cmd.rs` | Writes `~/.roko/.env` secrets, but runtime provider resolution does not uniformly go through the secret abstraction. |

## Target Design

Configuration and provider policy should resolve once into a `RuntimeContext`. After that point, runtime components consume typed services and never inspect global environment or raw config files directly.

| Component | Responsibility | Must Not Do |
| --- | --- | --- |
| `ConfigLoader` | Load global config, project config, `ROKO_CONFIG`, and field env overrides with provenance | Construct agents or read provider keys |
| `SecretService` | Resolve credentials from env, `.env`, `.roko/secrets.toml`, and future vault sources | Return secrets to UI payloads or write them into durable events |
| `ProviderRegistry` | Resolve provider/model profiles, slugs, limits, capabilities, and proof status | Spawn providers directly |
| `RuntimePolicy` | Resolve safety, approval, sandbox, network, shell, and path policy once | Let provider adapters decide dangerous defaults |
| `RuntimeEnvironment` | Build sanitized env maps for subprocesses | Pass whole process env implicitly |
| `RuntimeContext` | Bundle config, secrets, provider registry, policy, environment, store, and event sink | Mutate global env or hidden thread-local state |
| `Dispatcher` | Execute provider calls using `RuntimeContext` services | Re-resolve config or credentials per entrypoint |
| CLI/HTTP/TUI adapters | Parse input and render output | Read provider keys, config files, or unsafe defaults directly |

The key rule: provider setup should have one authoritative path: `ConfigLoader -> SecretService -> ProviderRegistry -> RuntimePolicy -> Dispatcher`.

## P0 Findings

### P0-01 There Are Two Config Models, And Neither Is The Runtime Contract

Problem:

`roko-core` defines `RokoConfig`, while `roko-cli` defines a separate `Config`, `ConfigLayer`, layered loader, source tracking, global config path, and env override behavior. The server also has its own config load/reload paths. Runtime code does not receive one canonical resolved config object from one owner.

Evidence:

```text
crates/roko-core/src/config/schema.rs: RokoConfig with providers, models, gates, routing, budget, serve, tools, chain, agents.
crates/roko-cli/src/config.rs: Config plus ConfigLayer, load_layered, resolve_paths, global_config_path.
crates/roko-cli/src/config.rs:2727: precedence is ROKO__* env vars -> ROKO_CONFIG -> project -> global -> defaults.
crates/roko-serve/src/lib.rs: loads roko.toml and global config separately for server startup.
```

Why it matters:

When config semantics live in CLI code, server and runner paths can drift. When schema code also reads env vars, pure config parsing becomes runtime-specific. The result is hard to prove because "effective config" depends on which entrypoint called it.

Target design:

Create one `ResolvedRuntimeConfig` with field provenance. It should be produced by one `ConfigLoader` and stored in `RuntimeContext`. CLI and server should not own merge semantics.

Implementation checklist:

- [ ] Move layered config resolution out of `roko-cli` into `roko-core`, `roko-runtime`, or the future application-service crate from [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).
- [ ] Decide whether legacy `crates/roko-cli/src/config.rs::Config` is still needed or should be a compatibility adapter into `RokoConfig`.
- [ ] Make `ResolvedRuntimeConfig` include `global_path`, `project_path`, `env_override_path`, field source map, and validation warnings.
- [ ] Make CLI, server, runner, TUI, PRD generation, dreams, and proof harnesses all receive the same `ResolvedRuntimeConfig`.
- [ ] Move global path logic such as `~/.config/roko/config.toml` and `ROKO_CONFIG` out of CLI-only code.
- [ ] Add a grep gate: `rg "load_layered|global_config_path|resolve_paths" crates/roko-cli/src crates/roko-serve/src` is allowed only in adapters after migration.
- [ ] Add proof that `roko config show`, `roko plan run`, server `/api/config`, and provider proof report the same config source map.

### P0-02 Provider Credentials Bypass The Secret Abstraction

Problem:

`roko-core::secrets::SecretResolver` exists, but provider config still resolves API keys directly from env vars. Some domain and tool code reads provider keys directly as well.

Evidence:

```text
crates/roko-core/src/secrets/resolve.rs: SecretResolver supports env -> file -> vault -> prompt chain.
crates/roko-core/src/config/provider.rs: ProviderConfig::resolve_api_key uses std::env::var.
crates/roko-core/src/config/schema.rs: effective_providers checks ANTHROPIC_API_KEY directly.
crates/roko-neuro/src/episode_completion.rs: reads ANTHROPIC_API_KEY directly.
crates/roko-std/src/tool/builtin/web_search.rs: reads PERPLEXITY_API_KEY directly.
crates/roko-serve/src/routes/templates.rs: passes through ANTHROPIC_API_KEY directly.
crates/roko-serve/src/routes/deployments.rs: passes through ANTHROPIC_API_KEY directly.
```

Why it matters:

Direct env reads make credentials depend on process state, not runtime state. They also bypass source provenance, redaction policy, secret availability status, and provider matrix classification.

Target design:

Make `SecretService` the only credential path. Provider configs should refer to a `SecretRef`, not directly call env. Tools and domain crates should request capabilities, not provider keys.

Implementation checklist:

- [ ] Replace `ProviderConfig::resolve_api_key` with `ProviderConfig::api_key_ref` plus `SecretService::resolve_provider_secret`.
- [ ] Support current env names such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `MOONSHOT_API_KEY`, `ZAI_API_KEY`, `PERPLEXITY_API_KEY`, `GEMINI_API_KEY`, and `OPENROUTER_API_KEY` as secret aliases.
- [ ] Support `ROKO_SECRET_LLM_<PROVIDER>` as the canonical secret namespace.
- [ ] Load `~/.roko/.env` and project `.env` only through `SecretService`.
- [ ] Replace direct key reads in neuro and std web search with capability services.
- [ ] Make provider proof report secret status as `configured`, `missing_credentials`, `auth_failed`, or `redacted`.
- [ ] Add a grep gate: `rg "ANTHROPIC_API_KEY|OPENAI_API_KEY|MOONSHOT_API_KEY|ZAI_API_KEY|PERPLEXITY_API_KEY|std::env::var" crates/roko-neuro/src crates/roko-std/src crates/roko-serve/src/routes crates/roko-cli/src/runner` has an allowlist limited to config/secret adapters and tests.

### P0-03 Unsafe Runtime Policy Is Still A Local Flag

Problem:

Unsafe permission behavior is carried through multiple config structs and provider options. Some paths default to dangerous behavior locally.

Evidence:

```text
roko-cli scan count: 72 unsafe refs.
roko-agent scan count: 16 unsafe refs.
crates/roko-cli/src/serve_runtime.rs: builds runner config with dangerously_skip_permissions.
crates/roko-cli/src/runner/types.rs: default construction includes dangerously_skip_permissions in runner config.
crates/roko-cli/src/dispatch_v2.rs: pushes --dangerously-skip-permissions and --dangerously-bypass-approvals-and-sandbox.
crates/roko-agent/src/provider/claude_cli.rs: applies options.dangerously_skip_permissions to Claude CLI agent.
crates/roko-serve/src/dispatch.rs: creates AgentOptions with dangerously_skip_permissions.
```

Why it matters:

Safety policy must be explainable and durable. If each caller decides whether to bypass permissions, two entrypoints can run the same task with different risk behavior.

Target design:

`RuntimePolicy` should be resolved once and passed through `RuntimeContext`. Provider adapters receive a policy-derived execution mode, not raw booleans. Every provider call emits a durable `safety.policy.selected` event.

Implementation checklist:

- [ ] Define `RuntimePolicy` with `approval_mode`, `sandbox_mode`, `network_policy`, `path_policy`, `shell_policy`, and `dangerous_bypass_allowed`.
- [ ] Define `RuntimePolicySource` showing whether each policy came from CLI flag, config, env, project profile, or default.
- [ ] Replace raw `dangerously_skip_permissions` fields in runtime-facing structs with a policy reference or derived `ProviderExecutionPolicy`.
- [ ] Make dangerous bypass default to `false` in all entrypoints.
- [ ] Add durable events for policy selected, policy denied, provider execution policy applied, and unsafe bypass used.
- [ ] Add proof for denied shell, denied path, denied network, and allowed explicit bypass with evidence.
- [ ] Add a grep gate: `rg "dangerously_skip_permissions|dangerously-bypass|dangerously-skip" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src` is limited to policy adapters and provider argument renderers.

### P0-04 Provider Construction Is Not Fully Behind Dispatch

Problem:

`create_agent_for_model` is useful, but it is not enough as the runtime seam. Several entrypoints still construct providers or agents directly instead of using the higher-level dispatcher and runtime event path.

Evidence:

```text
crates/roko-serve/src/routes/providers.rs: creates an agent directly for provider tests.
crates/roko-serve/src/dispatch.rs: creates agents directly for server dispatch.
crates/roko-dreams/src/runner.rs: creates agents directly for dream review.
crates/roko-cli/src/vision_loop/evaluator.rs: creates agents directly for vision evaluation.
crates/roko-cli/src/agent_spawn.rs: wraps create_agent_for_model directly.
crates/roko-cli/src/dispatch_v2.rs: creates agents and CLI provider configs below CLI.
crates/roko-cli/src/runner/agent_stream.rs: still carries a CLI provider stream path.
```

Why it matters:

The provider matrix can only prove one path. Direct agent creation can bypass prompt diagnostics, runtime events, policy selection, provider health updates, retry classification, and proof reporting.

Target design:

Runtime code should call `Dispatcher` or `ModelCallService`. Direct `create_agent_for_model` should be internal to provider/dispatch adapters and tests.

Implementation checklist:

- [ ] Move server provider test endpoint onto the same dispatcher used by runner.
- [ ] Move dream runner model calls onto `ModelCallService`.
- [ ] Move neuro completion and vision evaluation onto `ModelCallService` or specialized services built on it.
- [ ] Keep direct `create_agent_for_model` calls inside `roko-agent`, dispatch adapters, and tests only.
- [ ] Emit provider lifecycle events for every model call regardless of caller.
- [ ] Add grep gate: `rg "create_agent_for_model|AgentOptions" crates/roko-cli/src crates/roko-serve/src crates/roko-dreams/src crates/roko-neuro/src crates/roko-std/src` excludes dispatch/provider adapters and tests.
- [ ] Add proof that provider test endpoint, plan runner, dream review, and CLI one-shot all record the same provider lifecycle event schema.

## P1 Findings

### P1-01 Provider Availability Status Is Not A First-Class Runtime Object

Problem:

Provider proof wants statuses such as `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, and `unsupported`. Those statuses are not yet a single runtime object that config resolution, provider adapters, HTTP, TUI, and proof scripts all share.

Evidence:

```text
crates/roko-agent/src/retry.rs: classifies auth failures and retry behavior.
crates/roko-agent/src/provider/openai_compat.rs: maps provider-specific HTTP failures.
crates/roko-agent/src/gemini/adapter.rs: classifies auth failures.
crates/roko-serve/src/routes/providers.rs: exposes has_api_key and provider test responses.
tests/proof/mori-diffs/prove-runtime-end-to-end.sh: expected to classify provider outcomes.
```

Why it matters:

Without one status vocabulary, "provider works" can mean different things in CLI proof, HTTP providers page, and runtime retry logic.

Target design:

Create `ProviderAvailability` and `ProviderProofStatus` as durable provider registry concepts. Provider adapters map raw errors into this vocabulary once.

Implementation checklist:

- [ ] Define `ProviderProofStatus` with exactly `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, and `failed`.
- [ ] Define `ProviderAvailability` with provider id, model key, slug, credential source, status, checked_at, latency, error_class, and redacted evidence path.
- [ ] Make dispatcher/provider adapters return provider status details on failure.
- [ ] Make HTTP providers endpoint and proof scripts read the same status projection.
- [ ] Persist provider availability under runtime projections, not only stdout.
- [ ] Add proof that each provider status appears correctly when credentials are absent, invalid, rate-limited, unsupported, or successful.

### P1-02 Redaction Exists In Multiple Places But Is Not One Policy

Problem:

Roko has multiple secret scrubbers and config maskers. This is good defense in depth, but there is no one redaction policy that all durable events, HTTP responses, logs, proof artifacts, and prompts must use.

Evidence:

```text
crates/roko-core/src/obs/scrub.rs: observability scrubber.
crates/roko-agent/src/safety/scrub.rs: agent safety scrubber.
crates/roko-serve/src/routes/config.rs: masks secret fields in config responses.
crates/roko-serve/src/state.rs: owns a LogScrubber for server state.
crates/roko-core/src/tool/trace.rs: scrubs tool traces before storage.
```

Why it matters:

Provider proof must store enough raw evidence to debug failures without leaking secrets. Multiple scrubbers make it hard to prove every output channel is protected.

Target design:

Define `RedactionPolicy` and `RedactionService` as runtime services. Existing scrubbers can remain implementations, but durable writes and HTTP responses should call the runtime service.

Implementation checklist:

- [ ] Define a shared `RedactionPolicy` with patterns, field names, header names, and max retained bytes.
- [ ] Define `RedactionService::scrub_text`, `scrub_json`, `scrub_headers`, and `scrub_event`.
- [ ] Make proof artifacts call `RedactionService` before writing stdout/stderr/body evidence.
- [ ] Make HTTP config, provider, logs, and events endpoints use the same service.
- [ ] Make prompt diagnostics redact any included env/config snippets.
- [ ] Add a canary proof that a fake API key is absent from events, proof artifacts, HTTP responses, and logs.

### P1-03 Global Config And Global Env Are CLI-Centric

Problem:

The documented global config path and `~/.roko/.env` creation live primarily in CLI code. Server and library consumers should not have to depend on CLI semantics to find config and secrets.

Evidence:

```text
crates/roko-cli/src/config.rs: global_config_path resolves ~/.config/roko/config.toml or XDG_CONFIG_HOME.
crates/roko-cli/src/config_cmd.rs: creates ~/.roko/.env with a commented template.
crates/roko-cli/src/config_cmd.rs: cmd_set_secret writes ~/.roko/.env.
crates/roko-serve/src/lib.rs: separately searches config paths during startup.
```

Why it matters:

The user expects keys added to global config or global env files to work for CLI, UI, server, proof scripts, dreams, and runner. That cannot be guaranteed while global config semantics are adapter-owned.

Target design:

Move global config path resolution and global env loading into `ConfigLoader` and `SecretService`. CLI commands can manage those files, but runtime discovery should not be CLI-specific.

Implementation checklist:

- [ ] Move `global_config_path` and `resolve_paths` to the runtime config package.
- [ ] Define canonical global config path and legacy fallback paths.
- [ ] Define canonical global secret env path and project secret env path.
- [ ] Make server startup and proof scripts use `ConfigLoader`.
- [ ] Make `roko config path` print the same paths `RuntimeContext` actually used.
- [ ] Add proof that a provider key in global secret storage is used by CLI plan run and HTTP provider test without exporting it in the shell.

## P2 Findings

### P2-01 Env Var Overrides Need A Registry And Docs

Problem:

Many `ROKO_*` variables exist, but there is not one machine-readable registry with owner, type, default, scope, redaction behavior, and deprecation status.

Evidence:

```text
crates/roko-core/src/config/schema.rs: ROKO_PROVIDER, ROKO_MODEL_SLUG, ROKO_MODEL, ROKO_BACKEND, ROKO_EFFORT, ROKO_CONTEXT_LIMIT_K, ROKO_MAX_AGENTS, ROKO_BUDGET_USD, ROKO_PARALLEL, ROKO_EXPRESS, ROKO_SKIP_TESTS, ROKO_CLIPPY.
crates/roko-cli/src/auth.rs: ROKO_API_KEY.
crates/roko-cli/src/tui/config_meta.rs: UI metadata for several ROKO_* overrides.
crates/roko-agent/src/provider/mod.rs: ROKO_DISPATCHER and ROKO_MOCK_STATE_PATH for mocks.
crates/roko-agent/src/process/mcp.rs: ROKO_MCP_CONFIG.
```

Why it matters:

Without a registry, env overrides become hidden APIs. Hidden APIs are hard to deprecate and hard to prove.

Target design:

Add an `EnvOverrideRegistry` that config loader, doctor, TUI, and docs all consume.

Implementation checklist:

- [ ] Define `EnvOverrideSpec` with name, type, config field, scope, source, default, redaction, and deprecation status.
- [ ] Move `ROKO_*` override metadata out of TUI-specific files.
- [ ] Make `roko doctor`, `roko config show`, and TUI config view use the registry.
- [ ] Add CI that fails on unregistered `ROKO_*` references in production code.
- [ ] Add generated docs for global config and env overrides.

### P2-02 Compatibility Defaults Need Expiry Dates

Problem:

Several paths synthesize defaults to keep older configs working, such as Anthropic provider synthesis when `ANTHROPIC_API_KEY` is set and CLI command fallback when `claude` or `codex` appears. Compatibility is useful, but indefinite compatibility paths become shadow config systems.

Evidence:

```text
crates/roko-core/src/config/schema.rs: effective_providers synthesizes anthropic and claude_cli providers.
crates/roko-agent/src/provider/mod.rs: known protocol CLI command synthesis for missing explicit provider config.
crates/roko-agent/src/provider/mod.rs: fallback to ExecAgent when no provider is found.
```

Why it matters:

Fallbacks make first run easier but hide missing config. They also complicate proof because a provider can be "working" through legacy inference rather than explicit provider registry configuration.

Target design:

Compatibility defaults should emit warnings, durable events, and migration suggestions. Each should have an owner and removal condition.

Implementation checklist:

- [ ] Add a `CompatibilityDecision` event for synthesized provider, synthesized model, legacy command fallback, and ExecAgent fallback.
- [ ] Add config validation warning when provider/model definitions are inferred.
- [ ] Add `roko config migrate` output that writes explicit provider/model entries.
- [ ] Make provider proof report whether success used explicit config or compatibility inference.
- [ ] Add a removal/retirement checklist for each compatibility fallback.

## Implementation Order

Implement in this order to avoid replacing one set of ad hoc paths with another.

1. [ ] Define `ResolvedRuntimeConfig`, `ConfigLoader`, and field provenance outside CLI-only code.
2. [ ] Define `SecretService` and migrate provider credential lookup away from direct env reads.
3. [ ] Define `RuntimePolicy` and replace raw dangerous bypass booleans in runtime-facing APIs.
4. [ ] Define `ProviderAvailability` and make proof, HTTP, and TUI consume the same status projection.
5. [ ] Migrate direct provider construction to dispatcher or `ModelCallService`.
6. [ ] Define `RedactionService` and route durable writes plus HTTP responses through it.
7. [ ] Add `EnvOverrideRegistry` and generated config/env docs.
8. [ ] Add compatibility decision events and migration warnings.
9. [ ] Add grep gates and CI checks for direct env reads, direct provider construction, unsafe flag sprawl, and unregistered `ROKO_*` vars.

## Grep Gates

These commands should eventually pass with zero output or an explicit allowlist.

```bash
rg "std::env::var|std::env::var_os|env::var|env::var_os" \
  crates/roko-cli/src \
  crates/roko-serve/src \
  crates/roko-agent/src \
  crates/roko-neuro/src \
  crates/roko-std/src \
  -g '*.rs'
```

```bash
rg "ANTHROPIC_API_KEY|OPENAI_API_KEY|MOONSHOT_API_KEY|ZAI_API_KEY|PERPLEXITY_API_KEY|GEMINI_API_KEY|OPENROUTER_API_KEY" crates -g '*.rs'
```

```bash
rg "dangerously_skip_permissions|dangerously-bypass|dangerously-skip|skip_permissions" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src -g '*.rs'
```

```bash
rg "create_agent_for_model|AgentOptions" \
  crates/roko-cli/src \
  crates/roko-serve/src \
  crates/roko-dreams/src \
  crates/roko-neuro/src \
  crates/roko-std/src \
  -g '*.rs'
```

```bash
rg "ROKO_[A-Z0-9_]+" crates -g '*.rs'
```

## Proof Requirements

Config/provider-policy work is complete only when these are true:

- [ ] One command prints resolved config provenance for CLI and server startup.
- [ ] A provider key in global secret storage works without exporting the key in the shell.
- [ ] A provider key in project secret storage overrides global secret storage when configured to do so.
- [ ] Missing credential status is deterministic and does not try a provider call.
- [ ] Invalid credential status becomes `auth_failed` with redacted evidence.
- [ ] Rate limit status becomes `rate_limited` with redacted evidence.
- [ ] Unsupported provider status becomes `unsupported` with no provider call attempted.
- [ ] Provider success emits provider lifecycle events, usage, model key, slug, source config, and credential source.
- [ ] Unsafe bypass emits a policy event and is impossible by default.
- [ ] Redaction canary is absent from logs, events, proof artifacts, and HTTP responses.
- [ ] CLI plan run, HTTP provider test, dream review, and one-shot dispatch use the same provider registry and policy path.

## Agent Handoff Checklist

Use this checklist when assigning the config/provider work to an agent with no other context.

- [ ] Read this file and [32-DEPENDENCY-LAYERING-AUDIT.md](32-DEPENDENCY-LAYERING-AUDIT.md).
- [ ] Run the scan commands from the Method section and save the before counts.
- [ ] Pick one P0 finding.
- [ ] Implement the target service or migrate one call path without adding another bypass.
- [ ] Add or update a grep gate for the bypass being removed.
- [ ] Run the grep gate before and after the change.
- [ ] Run the smallest relevant cargo check.
- [ ] Update this file and [29-CURRENT-RUNTIME-GAP-LEDGER.md](29-CURRENT-RUNTIME-GAP-LEDGER.md) with evidence.
- [ ] Do not mark complete until proof artifacts show active runtime behavior.

## What Not To Do

- [ ] Do not add another env var read at the call site where a provider key is needed.
- [ ] Do not add provider-specific branches to runner, server routes, or dreams.
- [ ] Do not store raw provider stderr/stdout without redaction.
- [ ] Do not let compatibility defaults silently decide production behavior.
- [ ] Do not use direct `create_agent_for_model` outside dispatch/provider adapters unless the path is test-only.
- [ ] Do not call config work complete because `roko config show` works; provider proof must use the same resolved runtime context.

## 2026-04-27 Deepening Pass: Source-Verified Drift Cases

This addendum tightens the audit with specific source-level failure modes found while tracing config, env loading, provider proof, HTTP config, and dispatch. The earlier sections remain valid; this section explains where agents should start when implementing the config/provider work.

Additional commands used:

```bash
rg -n "global_config|merge_global|ROKO_CONFIG|load_config|resolve_paths|\\.roko/.env|ProviderDispatchResolver|ProviderConfig::resolve_api_key|create_agent_for_model|mask_secret_fields|provider test" crates/roko-cli/src crates/roko-core/src crates/roko-agent/src crates/roko-serve/src -g '*.rs'
sed -n '2560,2625p' crates/roko-cli/src/main.rs
sed -n '2560,2765p' crates/roko-cli/src/config.rs
sed -n '140,380p' crates/roko-core/src/config/schema.rs
sed -n '1,120p' crates/roko-core/src/config/mod.rs
sed -n '220,520p' crates/roko-serve/src/routes/providers.rs
sed -n '1,280p' crates/roko-serve/src/routes/config.rs
sed -n '440,840p' crates/roko-cli/src/dispatch_v2.rs
```

### D1 - Project `.roko/.env` Is Loaded Relative To Startup CWD, Not Resolved Workdir

Status: open.

Evidence:

- `crates/roko-cli/src/main.rs` loads `~/.roko/.env` first.
- The same function then loads `.roko/.env` from the process current directory before CLI parsing.
- A comment says the CLI has not parsed yet, so `workdir == cwd`.
- Commands that later use `--repo`, `--workdir`, `--config`, or discovered parent `roko.toml` can therefore resolve config from one workspace while secrets are loaded from another.

Why this matters:

Provider proof can falsely fail when the key is in the target workspace's `.roko/.env`, but the process was launched from a different directory. It can also falsely pass by using secrets from the wrong cwd.

Implementation checklist:

- [ ] Move env-file loading out of CLI startup and into `ConfigLoader` or `SecretService`.
- [ ] Compute project secret paths from the resolved project/workdir, not startup cwd.
- [ ] Preserve precedence: process env > resolved project `.roko/.env` > global `~/.roko/.env`, unless a stricter policy is configured.
- [ ] Emit `ConfigSourceEvent` or proof metadata showing which env files were consulted.
- [ ] Add a temp-workspace proof that launches `roko --repo /tmp/target config providers test --all` from another cwd and still uses `/tmp/target/.roko/.env`.
- [ ] Add a negative proof that a cwd `.roko/.env` is not used when `--repo` points elsewhere.

### D2 - Global Config Merge Is Manual And Call-Site Dependent

Status: open.

Evidence:

- `crates/roko-core/src/config/mod.rs::load_config(workdir)` reads only `workdir/roko.toml`, interpolates env vars, and resolves file secrets.
- `crates/roko-cli/src/config.rs::merge_global_providers` separately merges global providers/models into a `RokoConfig`.
- Grep shows call sites that remember to call `merge_global_providers`, including `run.rs`, `serve_runtime.rs`, and a path in `main.rs`.
- Other paths call `roko_core::config::load_config` directly, including daemon, PRD, worker/cloud, and some TUI/config paths.

Why this matters:

A provider defined in `~/.roko/config.toml` can work in one command and be invisible in another. This is the exact kind of loose end that prevents reproducible "all providers work" proof.

Implementation checklist:

- [ ] Replace `load_config(workdir)` and `merge_global_providers(config)` pairs with one `RuntimeConfigLoader::load(workdir, options)`.
- [ ] Make global provider/model merge default for runtime commands unless an explicit isolated mode is requested.
- [ ] Include merge provenance for each provider and model: `global`, `project`, `env_override`, `compat_synthesized`, or `builtin_default`.
- [ ] Update daemon, PRD, worker/cloud, serve, TUI, one-shot run, plan run, config providers, dreams, and proof harnesses to use the same loader.
- [ ] Add a grep gate that disallows `merge_global_providers` outside the loader and tests.
- [ ] Add proof that a provider defined only in global config is visible to `roko plan run`, `roko run`, `roko serve` `/api/providers`, `roko config providers test`, and the provider proof script.

### D3 - There Are At Least Three Credential Systems

Status: open.

Evidence:

- `ProviderConfig::resolve_api_key` reads the env var named by `api_key_env`.
- `roko config secrets set` writes raw entries into `~/.roko/.env`.
- `roko-core/src/secrets/env.rs` defines `ROKO_SECRET_<CATEGORY>_<PROVIDER>` resolution.
- `roko-core/src/secrets/file.rs` defines a restricted-permission TOML `FileStore`.
- `roko-cli/src/credentials.rs` stores server login credentials in `~/.roko/credentials.json`.

Why this matters:

These systems solve different problems, but provider runtime calls are still bound to the simplest path: direct env lookup. That means secret source, rotation, proof status, and redaction are not one contract.

Implementation checklist:

- [ ] Define `SecretRef` for provider credentials, with backwards-compatible support for `api_key_env`.
- [ ] Define `ProviderCredential` with `provider_id`, `kind`, `source`, `present`, `redacted_fingerprint`, and `expires_at`.
- [ ] Make `SecretService` read process env, resolved project env, global env, `ROKO_SECRET_*`, restricted file store, and future vault stores through one chain.
- [ ] Change provider adapters to receive a resolved credential handle or absence status; adapters should not call `std::env`.
- [ ] Keep `~/.roko/credentials.json` explicitly scoped to server auth/login, not LLM provider keys.
- [ ] Add a migration command that can import `~/.roko/.env` provider keys into the secret store without logging values.
- [ ] Add proof that the same key can be supplied by env var, global `.env`, project `.env`, and `ROKO_SECRET_LLM_PROVIDER`, with identical provider status output.

### D4 - Provider Tests Are Useful But Not The Same As Runtime Proof

Status: open.

Evidence:

- CLI `config providers test` sends handcrafted minimal provider requests in `crates/roko-cli/src/commands/config_cmd.rs`.
- HTTP `POST /api/providers/{id}/test` creates an agent with `create_agent_for_model` and calls `agent.run(...)` directly.
- Runner proof should go through dispatch, prompt assembly, runtime events, operation/projection persistence, learning feedback, and gate evidence.
- Current provider test responses show success/error, latency, tokens, and model, but do not persist a provider proof status projection with the exact required status vocabulary.

Why this matters:

Connectivity tests prove the key and endpoint can answer a tiny request. They do not prove the provider works in the actual orchestrated runtime. A provider can pass `config providers test` and still fail under plan execution, prompt assembly, streaming, safety policy, tool use, or event projection.

Implementation checklist:

- [ ] Keep provider connectivity tests, but rename their proof status to `connectivity_ok`, not `proved`.
- [ ] Add `ProviderProofService` that runs through the same dispatch facade as the runner.
- [ ] Persist `ProviderProofResult` with exactly these terminal statuses: `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, `failed`.
- [ ] Include `provider_id`, `model_key`, `model_slug`, `config_source`, `credential_source`, `runtime_path`, `event_log_path`, `artifact_path`, and redacted evidence.
- [ ] Make CLI provider proof, HTTP provider proof, and proof scripts call `ProviderProofService`.
- [ ] Add a proof case where `config providers test` passes but `ProviderProofService` fails a runtime requirement; this should be visible as `failed`, not hidden by connectivity success.

### D5 - HTTP Config Redaction Is Narrower Than Runtime Secret Risk

Status: open.

Evidence:

- `crates/roko-serve/src/routes/config.rs::mask_secret_fields` masks `serve.auth.api_key`, `server.auth_token`, and `deploy.railway_api_token`.
- Provider config can include `extra_headers`.
- `RokoConfig::interpolate_env_vars` can interpolate provider `base_url`, `api_key_env`, `command`, and `extra_headers`.
- `resolve_file_secrets` replaces `extra_headers` keys ending in `_file` with the file contents under the non-`_file` header name.

Why this matters:

Provider headers can contain secrets. Once interpolated or file-resolved, a header value may be a raw credential. `/api/config`, config proof bundles, logs, and prompt diagnostics need one redaction service, not a narrow list of known server fields.

Implementation checklist:

- [ ] Extend config redaction to providers and model/provider proof payloads.
- [ ] Treat all fields named or containing `key`, `token`, `secret`, `authorization`, `x-api-key`, `cookie`, and provider-specific auth headers as sensitive.
- [ ] Redact provider `extra_headers` values by default unless explicitly marked public.
- [ ] Redact interpolated config output and resolved file-secret output.
- [ ] Emit a redaction canary proof: place a fake secret in provider `extra_headers`, call `/api/config`, export proof, read event logs, and assert the canary does not appear.
- [ ] Move this behavior behind the shared `RedactionService` from P1-02.

### D6 - Compatibility Synthesis Is Good UX But Bad Proof Unless Labeled

Status: open.

Evidence:

- `RokoConfig::effective_providers` synthesizes `claude_cli` from `[agent]` command when no provider table exists.
- It synthesizes `anthropic` when `ANTHROPIC_API_KEY` or `[agent].env` suggests one.
- `create_agent_for_model` synthesizes provider/model defaults for known protocol CLI commands when explicit config is missing.
- It falls back to `ExecAgent` if no provider is found.

Why this matters:

First-run heuristics are useful. But provider proof must distinguish explicit provider support from heuristic fallback. Otherwise "Claude works" might mean explicit `claude_cli`, synthesized `claude_cli`, direct `anthropic`, or an `ExecAgent` shelling out to a command.

Implementation checklist:

- [ ] Emit `CompatibilityDecision` for synthesized provider, synthesized model, known protocol command fallback, and ExecAgent fallback.
- [ ] Add `compatibility_source` to provider/model resolution outputs.
- [ ] Make proof fail closed when a provider expected to be explicit is satisfied by compatibility fallback.
- [ ] Make first-run flows call `roko config migrate` or offer to write explicit providers/models after successful heuristic use.
- [ ] Add a grep/proof gate that no production provider proof is marked `proved` when dispatch target is `ExecAgent` fallback.

### D7 - `ProviderDispatchResolver` Is The Right Shape But Not The Whole Gateway

Status: partial.

Evidence:

- `ProviderDispatchResolver` resolves model key to provider config and runtime type.
- `AgentDispatcherV2` can create provider-backed agents and bridge result events into `AgentRuntimeEvent`.
- `classify_runtime` can distinguish CLI, provider-backed bridge, and unsupported providers.
- Target docs in `tmp/unified/08-GATEWAY.md` describe a stronger inference gateway: centralized secrets, cache, cost tracking, loop detection, output budgeting, thinking caps, provider call, cache store, stats, queues, and batch API.

Why this matters:

`ProviderDispatchResolver` is a necessary seam, but Mori-like provider execution needs a gateway/service boundary above raw provider calls. Otherwise cost, caching, retry/fallback, safety, provider health, prompt diagnostics, and proof remain scattered. See [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md) for the dedicated source-verified gateway migration checklist.

Implementation checklist:

- [ ] Keep `ProviderDispatchResolver` as the provider/model resolver.
- [ ] Add `InferenceGateway` or `ModelCallService` above it as the only runtime-facing model-call API.
- [ ] Move provider proof, runner dispatch, server provider test, dream review, research calls, neuro completion, and built-in web search onto that gateway.
- [ ] Make gateway requests include policy, budget, redaction, credential handle, prompt diagnostics id, operation id, and caller surface.
- [ ] Make gateway responses include normalized runtime events, usage, cost, provider health update, retry/fallback metadata, and proof evidence refs.
- [ ] Add HTTP projection endpoints for gateway/provider stats instead of route-local provider test state.

## Concrete Implementation Batches

Batch A - Loader and Provenance:

- [ ] Create `RuntimeConfigLoader`.
- [ ] Move global path, project discovery, `ROKO_CONFIG`, and env-file resolution under it.
- [ ] Return `ResolvedRuntimeConfig` with provenance and warnings.
- [ ] Replace direct `load_config` plus manual global merge in CLI/serve/runtime call sites.
- [ ] Add proofs for global config, project config, `--repo`, and `ROKO_CONFIG` precedence.

Batch B - Secret Service:

- [ ] Create `SecretService` and `SecretRef`.
- [ ] Bridge `api_key_env` into secret refs without breaking existing configs.
- [ ] Wire provider adapters through resolved credentials.
- [ ] Classify missing credentials before any network/provider call.
- [ ] Add proof for global/project/env/file-store secret sources.

Batch C - Provider Proof Service:

- [ ] Define `ProviderProofResult`.
- [ ] Implement status classification: `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, `failed`.
- [ ] Make CLI, HTTP, and shell proof scripts call the same service.
- [ ] Persist proof results under a queryable projection.
- [ ] Add redacted evidence artifacts for each provider and status.

Batch D - Redaction and Config API:

- [ ] Add shared `RedactionService`.
- [ ] Redact provider `extra_headers`, interpolated values, file-secret values, logs, events, and proof bundles.
- [ ] Add canary tests/proofs for every public/queryable output surface.
- [ ] Make `/api/config` and config export use the shared service.

Batch E - Gateway Boundary:

- [ ] Introduce `ModelCallService` or `InferenceGateway`.
- [ ] Move runner, serve provider test, one-shot, dreams, neuro, research, and tools onto it incrementally.
- [ ] Emit `GatewayRequestStarted`, `ProviderCallStarted`, `ProviderCallFinished`, `ProviderCallFailed`, `ProviderFallbackSelected`, and `GatewayRequestFinished` events.
- [ ] Add projections for provider stats, model stats, proof status, and credential status.

Batch F - Compatibility Retirement:

- [ ] Emit compatibility decisions for inferred providers/models and ExecAgent fallback.
- [ ] Make migration command write explicit provider/model entries.
- [ ] Add proof that explicit providers are used for all provider matrix cases.
- [ ] Fail provider proof when an expected provider only works through unlabeled fallback.

## Updated Acceptance Criteria

- [ ] One runtime loader is used by CLI, runner, serve, TUI, daemon, dreams, PRD, worker, research, neuro, tools, and proof scripts.
- [ ] Provider credentials are resolved by `SecretService`, not direct env reads at call sites.
- [ ] Project env files are selected by resolved workdir/repo, not startup cwd.
- [ ] Provider proof statuses are durable, queryable, and use the exact shared vocabulary.
- [ ] Provider connectivity tests are clearly separated from runtime proof.
- [ ] `/api/config`, logs, events, and proof bundles cannot leak provider header secrets.
- [ ] Compatibility provider/model synthesis is explicitly labeled and cannot silently satisfy proof.
- [ ] The provider/model runtime path is `RuntimeConfigLoader -> SecretService -> ProviderRegistry -> RuntimePolicy -> ModelCallService/InferenceGateway -> ProviderDispatchResolver -> provider adapter`.

Updated self-grade after deepening pass: `9.88 / 10`.

Reason: this addendum resolves the largest remaining ambiguity in the earlier audit by identifying exact source-level drift cases and converting each into implementation batches and proof requirements. It is not a `10` because the config-source graph and provider matrix proof artifacts still need to be generated by implementation work.

## 2026-04-27 Second Deepening Pass - Runtime Context, Secret Chain, And Provider Proof

The previous pass correctly identified the drift. This pass converts the config/provider work into a stricter service design that aligns with [34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md](34-OBSERVABILITY-PROJECTION-QUERY-AUDIT.md) and [41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md](41-INFERENCE-GATEWAY-MODEL-CALL-SERVICE-AUDIT.md). The key architectural correction is that config resolution must produce a durable, observable `RuntimeContextBuild` record before any provider call happens.

The target path is:

```text
CLI/HTTP/TUI adapter
  -> RuntimeBuilder
  -> RuntimeConfigLoader
  -> SecretService
  -> ProviderRegistry
  -> RuntimePolicyResolver
  -> RuntimeContext
  -> ModelCallService / InferenceGateway
  -> ProviderDispatchResolver
  -> ProviderAdapter
```

No runtime producer should independently read provider env vars, merge global config, choose dangerous permissions, synthesize provider defaults, or decide that a provider is "proved."

Updated self-grade after second deepening pass: `9.92 / 10`.

Reason: the doc now has implementation-ready service contracts, event/projection requirements, a provider-status taxonomy, source evidence, and migration batches that tie configuration directly to runtime proof. It remains below `10` until the config-source graph and provider proof bundles are generated by code.

### Additional Source Evidence From This Pass

These references were checked on 2026-04-27:

```text
crates/roko-cli/src/main.rs:2591 loads global ~/.roko/.env.
crates/roko-cli/src/main.rs:2601 loads local .roko/.env from startup cwd.
crates/roko-cli/src/config.rs:2576 defines global_config_path in CLI.
crates/roko-cli/src/config.rs:2614 defines merge_global_providers in CLI.
crates/roko-cli/src/config.rs:2668 defines resolve_paths in CLI.
crates/roko-cli/src/config.rs:2734 documents layered precedence inside CLI config.
crates/roko-cli/src/run.rs:472 loads roko_core config directly.
crates/roko-cli/src/run.rs:475 manually calls merge_global_providers.
crates/roko-cli/src/run.rs:731 reads ANTHROPIC_API_KEY directly.
crates/roko-cli/src/run.rs:748 resolves ANTHROPIC_API_KEY directly.
crates/roko-cli/src/agent_serve.rs:374 reads ANTHROPIC_API_KEY to override provider choice.
crates/roko-cli/src/agent_serve.rs:556 reads ROKO_CONFIG directly.
crates/roko-core/src/config/schema.rs:204 synthesizes effective providers.
crates/roko-core/src/config/schema.rs:208 checks ANTHROPIC_API_KEY during provider synthesis.
crates/roko-core/src/config/schema.rs:252 checks ANTHROPIC_API_KEY during anthropic synthesis.
crates/roko-core/src/config/schema.rs:431 interpolates env vars in provider config.
crates/roko-core/src/config/schema.rs:459 resolves *_file secrets into provider extra_headers.
crates/roko-core/src/config/provider.rs:76 resolves API keys by reading api_key_env directly.
crates/roko-core/src/secrets/env.rs defines ROKO_SECRET_<CATEGORY>_<PROVIDER> env naming.
crates/roko-core/src/secrets/resolve.rs defines SecretResolver/SecretProvider chain.
crates/roko-neuro/src/episode_completion.rs:25 reads ANTHROPIC_API_KEY directly.
crates/roko-std/src/tool/builtin/web_search.rs:261 reads PERPLEXITY_API_KEY directly.
crates/roko-cli/src/commands/config_cmd.rs:315 starts CLI provider tests.
crates/roko-cli/src/commands/config_cmd.rs:348 starts provider test all.
crates/roko-cli/src/commands/config_cmd.rs:865 checks provider api_key_env with std::env.
crates/roko-cli/src/commands/config_cmd.rs:1118 calls provider.resolve_api_key for OpenAI-compatible tests.
crates/roko-cli/src/commands/config_cmd.rs:1277 calls provider.resolve_api_key for Anthropic tests.
crates/roko-cli/src/commands/config_cmd.rs:1405 calls provider.resolve_api_key for Gemini tests.
crates/roko-serve/src/routes/providers.rs:67 reports has_api_key from provider_config.resolve_api_key.
crates/roko-serve/src/routes/providers.rs:301 creates an agent directly for provider tests.
crates/roko-serve/src/dispatch.rs:1807 creates an agent directly for server dispatch.
crates/roko-serve/src/dispatch.rs:1823 sets dangerously_skip_permissions true.
crates/roko-cli/src/dispatch_v2.rs:218 pushes --dangerously-skip-permissions.
crates/roko-cli/src/dispatch_v2.rs:256 pushes --dangerously-bypass-approvals-and-sandbox.
crates/roko-cli/src/dispatch_v2.rs:454 defines ProviderDispatchResolver.
crates/roko-cli/src/dispatch_v2.rs:533 defines AgentDispatcherV2.
crates/roko-cli/src/dispatch_v2.rs:578 calls create_agent_for_model.
crates/roko-cli/src/dispatch_v2.rs:763 classifies provider runtime type.
crates/roko-serve/src/routes/config.rs:229 masks only a narrow set of config secret fields.
crates/roko-core/src/config/provider.rs:55 allows provider extra_headers.
crates/roko-cli/src/commands/config_cmd.rs:1158 forwards provider extra_headers in provider tests.
```

### Runtime Context Build Record

Configuration is not complete when it returns a `RokoConfig`. It is complete when the runtime has a build record that can be queried, redacted, replayed, and attached to provider proof.

```rust
pub struct RuntimeContextBuild {
    pub build_id: String,
    pub workdir: PathBuf,
    pub started_at_ms: i64,
    pub config: ResolvedRuntimeConfig,
    pub secrets: SecretResolutionReport,
    pub providers: ProviderResolutionReport,
    pub policy: RuntimePolicyReport,
    pub compatibility: Vec<CompatibilityDecision>,
    pub warnings: Vec<RuntimeConfigWarning>,
    pub redaction: RedactionSummary,
}
```

Implementation checklist:

- [ ] Emit `config.context_build_started` before loading files or secrets.
- [ ] Emit `config.source_loaded` for global config, project config, `ROKO_CONFIG`, env override, project env file, global env file, and defaults.
- [ ] Emit `config.field_resolved` or a compact `config.resolved` event with field source hashes, not raw secret values.
- [ ] Emit `secret.resolved`, `secret.missing`, `secret.redacted`, and `secret.denied` events with redacted fingerprints.
- [ ] Emit `provider.resolved` for every provider/model pair with source and compatibility metadata.
- [ ] Emit `runtime_policy.selected` with approval, sandbox, network, shell, path, and dangerous-bypass decisions.
- [ ] Emit `config.context_build_finished` with `build_id`, warning count, provider count, model count, and redaction status.
- [ ] Store the build record as an artifact under `.roko/runtime/context/<build_id>.json`.
- [ ] Make provider proof bundles include `runtime_context_build_id`.
- [ ] Make `/api/config/effective` and `roko config show --effective` return the same redacted build record.

Done criteria:

- [ ] `roko plan run`, `roko run`, `roko serve`, `roko config providers test`, provider proof scripts, dreams, neuro, vision, and web search all have a `runtime_context_build_id`.
- [ ] A provider failure can be traced to exactly one config source and one credential source.
- [ ] No runtime proof is accepted without a context build record.

### Resolved Runtime Config Contract

`ResolvedRuntimeConfig` must be the runtime contract. `RokoConfig` can remain the file schema; CLI `Config` can remain a compatibility or UX shape. Neither should be the direct runtime authority after build time.

```rust
pub struct ResolvedRuntimeConfig {
    pub schema_version: u16,
    pub project_config_path: Option<PathBuf>,
    pub global_config_path: Option<PathBuf>,
    pub env_override_path: Option<PathBuf>,
    pub project_env_path: Option<PathBuf>,
    pub global_env_path: Option<PathBuf>,
    pub effective: RokoConfig,
    pub field_sources: FieldSourceMap,
    pub warnings: Vec<RuntimeConfigWarning>,
}
```

```rust
pub enum ConfigSourceKind {
    CliFlag,
    EnvOverride,
    EnvFile,
    ProjectConfig,
    GlobalConfig,
    CompatibilitySynthesis,
    BuiltinDefault,
}
```

Implementation checklist:

- [ ] Move path resolution for global config, project config, `ROKO_CONFIG`, and env files out of `crates/roko-cli/src/config.rs`.
- [ ] Keep CLI commands for editing config files, but call `RuntimeConfigLoader` for runtime resolution.
- [ ] Make `roko_core::config::load_config` either private to the loader or clearly named `load_project_file_only`.
- [ ] Replace manual `merge_global_providers` call sites with loader output.
- [ ] Make every resolved provider/model field carry source provenance.
- [ ] Make field source map redaction-safe by storing field paths and source kinds, not values.
- [ ] Add `isolated = true` loader option for tests that intentionally ignore global config.
- [ ] Add `strict = true` loader option for proof runs that fail on compatibility synthesis.
- [ ] Add compatibility migration warnings when legacy `[agent] command` or env-only Anthropic synthesis is used.

Done criteria:

- [ ] `rg -n "merge_global_providers|global_config_path|resolve_paths|ROKO_CONFIG" crates/roko-cli/src crates/roko-serve/src -g '*.rs'` is zero outside CLI config management commands, loader adapters, and tests.
- [ ] A global-only provider is visible to CLI plan run, one-shot run, server provider list, HTTP provider proof, and provider proof CLI.
- [ ] A project-only provider overrides global provider fields with clear source metadata.
- [ ] A `ROKO_CONFIG` override excludes project/global config according to the documented policy and records that exclusion in the build report.

### Secret Service Contract

The `SecretService` must unify process env, resolved env files, `ROKO_SECRET_*`, restricted file store, and future vault sources. Provider configs should not expose raw API key strings or call `std::env`.

```rust
pub struct SecretRef {
    pub namespace: Namespace,
    pub aliases: Vec<String>,
    pub required_for: Vec<String>,
}
```

```rust
pub struct ResolvedSecret {
    pub namespace: Namespace,
    pub status: SecretStatus,
    pub source: Option<SecretSourceKind>,
    pub redacted_fingerprint: Option<String>,
    pub expires_at_ms: Option<i64>,
}
```

```rust
pub enum SecretStatus {
    Present,
    Missing,
    DeniedByPolicy,
    InvalidFormat,
    RedactionFailed,
}
```

Implementation checklist:

- [ ] Add provider credential aliases for `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `MOONSHOT_API_KEY`, `ZAI_API_KEY`, `PERPLEXITY_API_KEY`, `GEMINI_API_KEY`, and `OPENROUTER_API_KEY`.
- [ ] Make canonical provider namespaces `llm.anthropic`, `llm.openai`, `llm.moonshot`, `llm.zai`, `llm.perplexity`, `llm.gemini`, `llm.openrouter`, `cli.claude`, and `cli.codex` where applicable.
- [ ] Make `ProviderConfig` hold `api_key_ref` or `credential_ref`; keep `api_key_env` as a legacy alias field.
- [ ] Replace `ProviderConfig::resolve_api_key` with `SecretService::resolve`.
- [ ] Replace direct key reads in `run.rs`, `agent_serve.rs`, `episode_completion.rs`, and `web_search.rs`.
- [ ] Replace provider list `has_api_key` with credential status from `SecretService`.
- [ ] Replace config provider test direct env checks with `SecretService`.
- [ ] Prevent raw secret values from entering events, logs, TUI state, HTTP responses, proof bundles, prompt diagnostics, or subprocess env dumps.
- [ ] Add secret source precedence tests for process env, project env file, global env file, `ROKO_SECRET_*`, and file store.

Done criteria:

- [ ] `rg -n "std::env::var\\(|std::env::var_os\\(|resolve_api_key\\(|ANTHROPIC_API_KEY|OPENAI_API_KEY|MOONSHOT_API_KEY|ZAI_API_KEY|PERPLEXITY_API_KEY" crates/roko-cli/src crates/roko-serve/src crates/roko-neuro/src crates/roko-std/src crates/roko-agent/src -g '*.rs'` only reports config/secret adapters, CLI config management commands, and tests.
- [ ] Provider proof can classify missing credentials without attempting a network call.
- [ ] Provider proof can show which secret source was used without leaking the value.

### Provider Registry And Compatibility Decisions

Provider/model resolution should be explicit and queryable. Compatibility synthesis can remain for first-run UX, but proof must distinguish it from explicit config.

```rust
pub struct ProviderResolutionReport {
    pub providers: Vec<ResolvedProvider>,
    pub models: Vec<ResolvedModelProfile>,
    pub compatibility: Vec<CompatibilityDecision>,
}
```

```rust
pub struct CompatibilityDecision {
    pub decision_id: String,
    pub kind: CompatibilityDecisionKind,
    pub reason: String,
    pub input_source: String,
    pub produced_provider: Option<String>,
    pub produced_model: Option<String>,
    pub proof_allowed: bool,
}
```

```rust
pub enum CompatibilityDecisionKind {
    SynthesizedClaudeCliProvider,
    SynthesizedAnthropicProviderFromEnv,
    SynthesizedModelFromKnownSlug,
    KnownProtocolCommandFallback,
    ExecAgentFallback,
}
```

Implementation checklist:

- [ ] Move `effective_providers` synthesis into `ProviderRegistry` so it can report compatibility decisions.
- [ ] Preserve first-run behavior but set `proof_allowed = false` for unlabeled or unsafe fallback.
- [ ] Make proof strict mode fail if an expected provider is satisfied by `ExecAgentFallback`.
- [ ] Make proof strict mode fail if an expected API provider is satisfied by CLI fallback unless the scenario explicitly asks for CLI.
- [ ] Emit `provider.compatibility_decision` events with decision ids.
- [ ] Add `/api/providers` fields for `source`, `compatibility_source`, `proof_allowed`, and `credential_status`.
- [ ] Add `roko config migrate providers` that writes explicit providers/models from accepted compatibility decisions.

Done criteria:

- [ ] Provider matrix proof reports explicit provider/model source for every provider.
- [ ] A synthesized provider can be used for UX but cannot silently satisfy `proved`.
- [ ] `create_agent_for_model` fallback to `ExecAgent` is never marked `proved`.

### Runtime Policy Contract

Dangerous permission behavior must be a policy decision, not a per-call boolean.

```rust
pub struct RuntimePolicy {
    pub approval_mode: ApprovalMode,
    pub sandbox_mode: SandboxMode,
    pub network_policy: NetworkPolicy,
    pub shell_policy: ShellPolicy,
    pub path_policy: PathPolicy,
    pub dangerous_bypass_allowed: bool,
    pub source: RuntimePolicySourceMap,
}
```

```rust
pub struct ProviderExecutionPolicy {
    pub provider_id: String,
    pub allow_cli_bypass_flags: bool,
    pub allow_network: bool,
    pub allowed_paths: Vec<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
}
```

Implementation checklist:

- [ ] Replace runtime-facing `dangerously_skip_permissions` booleans with `ProviderExecutionPolicy`.
- [ ] Keep provider adapter argument rendering as the only place that turns policy into CLI flags.
- [ ] Make `dangerously_skip_permissions: true` in server dispatch impossible unless `RuntimePolicy` explicitly allows it.
- [ ] Emit `runtime_policy.selected` before provider calls.
- [ ] Emit `runtime_policy.applied_to_provider` with provider id, model key, and redacted policy summary.
- [ ] Emit `runtime_policy.denied` when a provider call or tool is blocked.
- [ ] Add proof cases for explicit allowed bypass, denied bypass, denied network, and denied path.

Done criteria:

- [ ] `rg -n "dangerously_skip_permissions|dangerously-bypass|dangerously-skip" crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src -g '*.rs'` is limited to policy structs, provider adapter rendering, migration adapters, and tests.
- [ ] No server route hardcodes dangerous bypass.
- [ ] Provider proof bundle includes the selected runtime policy id.

### Provider Proof Service Contract

Provider connectivity checks and runtime proof need separate labels. Connectivity is useful, but it is not Mori-level proof.

```rust
pub struct ProviderProofRequest {
    pub provider_id: String,
    pub model_key: Option<String>,
    pub runtime_context_build_id: String,
    pub scenario: ProviderProofScenario,
    pub strict: bool,
}
```

```rust
pub struct ProviderProofResult {
    pub provider_id: String,
    pub model_key: String,
    pub model_slug: String,
    pub status: ProviderProofStatus,
    pub connectivity_status: Option<ProviderConnectivityStatus>,
    pub credential_status: SecretStatus,
    pub config_source: ConfigSourceKind,
    pub compatibility_decision_id: Option<String>,
    pub runtime_path: String,
    pub evidence_event_ids: Vec<String>,
    pub projection_cursor: Option<u64>,
    pub artifact_refs: Vec<String>,
}
```

```rust
pub enum ProviderProofStatus {
    Proved,
    MissingCredentials,
    AuthFailed,
    RateLimited,
    Unsupported,
    Failed,
}
```

Implementation checklist:

- [ ] Keep `config providers test` as connectivity by default.
- [ ] Add `config providers prove` or `roko proof provider-matrix` for runtime proof.
- [ ] Make HTTP `POST /api/providers/{id}/test` return `connectivity_status`, not runtime `proved`.
- [ ] Add HTTP `POST /api/providers/{id}/prove` that calls `ProviderProofService`.
- [ ] Make `ProviderProofService` call `ModelCallService::probe_provider` and require runtime events.
- [ ] Persist proof results to `provider_state` and `proof_state` projections.
- [ ] Add explicit provider scenarios for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Classify auth errors before generic failure when HTTP status or provider stderr makes that possible.
- [ ] Classify rate limits before generic failure when HTTP status or provider stderr makes that possible.
- [ ] Classify unsupported provider/model without attempting unsupported runtime calls.

Done criteria:

- [ ] CLI provider matrix proof and HTTP provider proof call the same service.
- [ ] Proof output uses only `proved`, `missing_credentials`, `auth_failed`, `rate_limited`, `unsupported`, or `failed`.
- [ ] A provider is marked `proved` only if prompt assembly, policy selection, model call, runtime events, and projection query all succeed.
- [ ] A provider with a valid connectivity test but failed runtime dispatch is reported as `failed`, not `proved`.

### Redaction Service Contract

Config redaction must cover more than server auth fields. Provider headers, interpolated values, file-secret values, prompt diagnostics, event payloads, proof artifacts, and logs all need the same redaction policy.

```rust
pub trait RedactionService: Send + Sync {
    fn redact_value(&self, path: &str, value: serde_json::Value) -> RedactedValue;
    fn redact_text(&self, surface: RedactionSurface, text: &str) -> RedactedText;
    fn scan_canary(&self, surface: RedactionSurface, text: &str) -> CanaryScanResult;
}
```

Implementation checklist:

- [ ] Treat field names containing `key`, `token`, `secret`, `authorization`, `cookie`, `x-api-key`, `api-key`, `bearer`, and `credential` as sensitive by default.
- [ ] Treat provider `extra_headers` values as sensitive by default.
- [ ] Redact resolved `*_file` secret contents.
- [ ] Redact interpolated config values before HTTP responses or proof artifacts.
- [ ] Redact provider stderr/stdout before durable event append.
- [ ] Redact prompt diagnostics before durable event append.
- [ ] Add a redaction canary to config show, `/api/config`, provider proof, runtime events, projection responses, stream frames, and proof bundles.

Done criteria:

- [ ] `routes/config.rs::mask_secret_fields` is replaced by shared redaction service or becomes a thin adapter.
- [ ] The canary string is absent from `.roko/runtime/events.jsonl`, `.roko/proof`, HTTP responses, TUI projection frames, and logs.
- [ ] Provider `extra_headers` never appear unredacted in config or proof output.

## Second-Pass Implementation Batches

Batch G - Runtime Builder And Context Build Record:

- [ ] Create `RuntimeBuilder` with inputs `workdir`, CLI options, env snapshot handle, clock, redaction service, and event store.
- [ ] Build `RuntimeContextBuild` before constructing dispatcher/model-call services.
- [ ] Attach `runtime_context_build_id` to every provider call, runner operation, HTTP operation, TUI session, and proof bundle.
- [ ] Add `config.context_build_*` runtime events.
- [ ] Add `/api/config/context-builds/latest` and CLI `roko config context --latest`.

Batch H - Loader Migration:

- [ ] Move CLI config path functions into shared loader code.
- [ ] Make `roko_core::config::load_config` project-file-only or migrate callers to `RuntimeConfigLoader`.
- [ ] Replace manual merge call sites in `run.rs`, `main.rs`, `serve_runtime.rs`, daemon, PRD, worker/cloud, TUI, and proof scripts.
- [ ] Add strict and isolated loader modes.
- [ ] Update docs and CLI help for global config and project env precedence.

Batch I - Secret Migration:

- [ ] Add `SecretRef`/`ResolvedSecret` types.
- [ ] Map legacy `api_key_env` to aliases.
- [ ] Wire provider adapter creation through resolved credentials.
- [ ] Replace direct env reads in CLI run, agent serve, neuro completion, web search, config provider tests, and server providers route.
- [ ] Add redacted secret source events and projection rows.

Batch J - Provider Proof Migration:

- [ ] Split connectivity tests from runtime proof in CLI and HTTP naming.
- [ ] Implement `ProviderProofService`.
- [ ] Implement provider proof scenarios for Anthropic, OpenAI, Moonshot, Z.AI, Perplexity, Claude CLI, and Codex CLI.
- [ ] Require `ModelCallService::probe_provider`.
- [ ] Persist provider proof into `provider_state` and `proof_state`.
- [ ] Add proof artifact examples for every terminal status.

Batch K - Policy And Redaction Migration:

- [ ] Replace runtime-facing dangerous bypass booleans with policy-derived execution policy.
- [ ] Remove server hardcoded `dangerously_skip_permissions: true`.
- [ ] Replace route-local config masking with shared `RedactionService`.
- [ ] Add canary proof across config, events, projections, streams, logs, and proof bundles.
- [ ] Make provider proof fail if policy/redaction evidence is missing.

## Second-Pass Grep Gates

```bash
rg -n "merge_global_providers|global_config_path|resolve_paths|ROKO_CONFIG" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-core/src -g '*.rs'
```

Expected end state:

- [ ] Only the runtime config loader, CLI config management commands, and tests own config path resolution.

```bash
rg -n "resolve_api_key\\(|std::env::var\\(|std::env::var_os\\(|ANTHROPIC_API_KEY|OPENAI_API_KEY|MOONSHOT_API_KEY|ZAI_API_KEY|PERPLEXITY_API_KEY|GEMINI_API_KEY|OPENROUTER_API_KEY" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src crates/roko-core/src crates/roko-neuro/src crates/roko-std/src -g '*.rs'
```

Expected end state:

- [ ] Only secret/config adapters and tests can read provider secret env vars.

```bash
rg -n "create_agent_for_model\\(" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-dreams/src crates/roko-neuro/src crates/roko-std/src -g '*.rs'
```

Expected end state:

- [ ] Production call sites use `ModelCallService` or dispatch facade; provider factory use is internal to provider/dispatch adapters and tests.

```bash
rg -n "dangerously_skip_permissions|dangerously-bypass|dangerously-skip" \
  crates/roko-cli/src crates/roko-serve/src crates/roko-agent/src -g '*.rs'
```

Expected end state:

- [ ] Dangerous bypass is rendered only from `RuntimePolicy` in provider adapter argument code.

```bash
rg -n "mask_secret_fields|extra_headers|bearer_auth|x-api-key|authorization_file|api_key_env" \
  crates/roko-serve/src/routes crates/roko-cli/src/commands crates/roko-core/src/config crates/roko-agent/src -g '*.rs'
```

Expected end state:

- [ ] Config/HTTP/provider/proof surfaces use shared redaction and secret services.

## Second-Pass Definition Of Complete

- [ ] One `RuntimeBuilder` builds a redacted, queryable context build record for every runtime entrypoint.
- [ ] One `RuntimeConfigLoader` owns global/project/env override precedence and field provenance.
- [ ] One `SecretService` owns provider credentials and returns redacted credential status.
- [ ] One `ProviderRegistry` owns provider/model resolution and compatibility decisions.
- [ ] One `RuntimePolicyResolver` owns dangerous bypass, sandbox, approval, network, shell, and path policy.
- [ ] One `ProviderProofService` owns the provider proof status taxonomy.
- [ ] One `ModelCallService` path is used for runtime provider proof.
- [ ] Config, provider, policy, and secret events are appended to the runtime event store.
- [ ] Config/proof/query endpoints expose source metadata without leaking secrets.
- [ ] All second-pass grep gates pass or have documented allowlists with retirement dates.

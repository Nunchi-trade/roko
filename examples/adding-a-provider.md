# Adding a New Provider

Use this path when the provider already speaks an OpenAI-compatible chat completions API. In that case, you do not need Rust changes. You only need config.

If the provider needs a different protocol, a custom auth flow, or request/response translation that `openai_compat` cannot cover, this is not the right path.

## What "zero-code" means here

- No changes under `crates/`
- No new `ProviderKind`
- No new adapter registration
- Only `roko.toml` changes plus an API key in your environment

## Before you start

- Pick a provider ID. Example: `my_provider`
- Pick one or more model keys. Example: `my-model`
- Confirm the provider exposes an OpenAI-compatible `/chat/completions` style API
- Confirm which API key env var you want to use. Example: `MY_PROVIDER_API_KEY`

Examples in this repo:

- `examples/roko-glm.toml`
- `examples/roko-kimi.toml`
- `examples/roko-openrouter.toml`
- `examples/roko-ollama.toml`

## Step 1: Add the provider

Add a new `[providers.*]` entry to `roko.toml`:

```toml
[providers.my_provider]
kind = "openai_compat"
base_url = "https://api.my-provider.com/v1"
api_key_env = "MY_PROVIDER_API_KEY"
timeout_ms = 180000
```

Required fields for the zero-code path:

- `kind = "openai_compat"`
- `base_url`

Usually needed:

- `api_key_env` for hosted providers

Optional tuning fields:

- `timeout_ms`
- `ttft_timeout_ms`
- `connect_timeout_ms`
- `extra_headers`
- `max_concurrent`

Example with custom headers:

```toml
[providers.my_provider]
kind = "openai_compat"
base_url = "https://api.my-provider.com/v1"
api_key_env = "MY_PROVIDER_API_KEY"
extra_headers = { "HTTP-Referer" = "https://github.com/nunchi/roko", "X-Title" = "roko-agent" }
```

## Step 2: Add one or more models

Point model entries at the provider you just added:

```toml
[models.my-model]
provider = "my_provider"
slug = "my-model-v1"
context_window = 128000
max_output = 16384
supports_tools = true
supports_thinking = true
tool_format = "openai_json"
cost_input_per_m = 1.00
cost_output_per_m = 5.00
```

Minimum useful fields:

- `provider`
- `slug`

Strongly recommended fields:

- `context_window`
- `supports_tools`
- `tool_format = "openai_json"`

Add capability flags only if the provider/model really supports them:

- `supports_thinking`
- `supports_vision`
- `supports_web_search`
- `supports_mcp_tools`
- `supports_partial`
- `supports_grounding`
- `supports_code_execution`
- `supports_caching`

Add cost fields if you want router and reporting data to be accurate:

- `cost_input_per_m`
- `cost_output_per_m`
- `cost_cache_read_per_m`
- `cost_cache_write_per_m`
- `cost_per_request`

## Step 3: Select the model

Either make it the default:

```toml
[agent]
default_model = "my-model"
fallback_model = "my-model"
```

Or leave your defaults alone and test with an override:

```bash
roko run --model my-model "Say hello in one sentence."
```

If you use tiered routing, point the tiers at the new model as needed:

```toml
[agent.tier_models]
mechanical = "my-model"
focused = "my-model"
integrative = "my-model"
architectural = "my-model"
```

## Step 4: Set the API key

Export the env var named in `api_key_env`:

```bash
export MY_PROVIDER_API_KEY="sk-..."
```

For local-only providers such as Ollama, you can omit `api_key_env` if the endpoint does not require auth.

## Step 5: Verify the config is loaded

Check the effective config:

```bash
roko config show
```

Smoke-test the provider by running a prompt against the model key:

```bash
roko run --model my-model "Reply with the single word: ok"
```

If you are using the HTTP server, you can also inspect the loaded registry:

```bash
roko serve --port 9090
curl http://127.0.0.1:9090/api/providers
curl http://127.0.0.1:9090/api/models
```

## Step 6: Reload without restarting everything

If you changed `roko.toml` while the daemon is running:

```bash
roko daemon reload
```

If you are using the HTTP API server:

```bash
curl -X POST http://127.0.0.1:9090/api/config/reload
```

## Common mistakes

- Adding `[models.*]` without the matching `[providers.*]` entry
- Setting `provider = "..."` to the API slug instead of the provider table key
- Using a provider base URL that does not expose an OpenAI-compatible chat endpoint
- Forgetting to export the env var named by `api_key_env`
- Setting capability flags optimistically instead of matching the real API
- Omitting `tool_format = "openai_json"` for normal OpenAI-compatible tool calling

## When zero-code is not enough

You need code changes instead of this guide when:

- the provider is not OpenAI-compatible
- the provider needs a new protocol family
- the provider needs custom request shaping beyond config fields and the existing adapter behavior
- the provider needs custom response parsing

In that case, add a new provider through the custom protocol path rather than forcing it into `openai_compat`.

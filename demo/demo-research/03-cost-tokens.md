# Cost & token instrumentation

**Default answer: roko already does this.** `AgentEfficiencyEvent` in
`.roko/learn/efficiency.jsonl` is richer than what LiteLLM, Langfuse, or
the native API objects expose. See `08-reuse-map.md`. This file covers
(a) the existing schema, (b) when LiteLLM is still useful (inside
adapter shims for competing frameworks), and (c) the few accounting
gotchas worth getting right.

## Roko's existing record (the ground truth)

`crates/roko-learn/src/efficiency.rs::AgentEfficiencyEvent` captures
**per-turn** what most external observability stacks capture per *call*.
Fields already populated:

| Field | What |
|---|---|
| `backend`, `model` | Which provider/model handled the turn |
| `input_tokens`, `output_tokens` | Standard pair |
| `reasoning_tokens` | Extended thinking output (Claude 3.7+/Opus 4.x) |
| `cache_read_tokens`, `cache_write_tokens` | Anthropic prompt cache |
| `cost_usd`, `cost_usd_without_cache` | Both modes — see "cache accounting" |
| `prompt_sections`, `system_prompt_tokens`, `total_prompt_tokens` | Per-section attribution |
| `tools_available`, `tools_used`, `tool_calls` | Utilization vs. capacity |
| `wall_time_ms`, `time_to_first_token_ms`, `was_warm_start` | Latency triangle |
| `gate_passed`, `outcome`, `gate_errors` | Correctness signal |

Helpers: `cache_hit_rate()`, `tool_utilization()`, `cache_savings_usd()`,
`total_tokens()`.

Persisted append-only to `.roko/learn/efficiency.jsonl`. Already tailed by
the TUI file watcher.

The price table is in `crates/roko-learn/src/cost_table.rs` (per-model
input/output/cache_read/cache_write rates). The query layer is
`crates/roko-learn/src/costs_db.rs::CostsDb` with `query_by_model()`,
`query_by_role()`, `query_by_plan()`, `query_by_complexity()`,
`query_by_time_range()`, `summarize()`.

**For a benchmark:** filter efficiency events by `plan_id =
"bench-{name}-{backend}"` and you have everything you need.

## Layers of instrumentation (legacy guide)

Below: the three-layer model from before, with notes on which layers roko
already covers.

## Layer 0: just trust the API response

(roko's existing dispatcher already does this — fields above are populated
from native API responses.)


Every modern LLM API returns a `usage` object. For a single-vendor demo, this
is enough.

| Provider | Field | Notes |
|---|---|---|
| Anthropic | `response.usage.input_tokens` | Plus `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens` |
| OpenAI | `response.usage.prompt_tokens` | Plus `completion_tokens`, `cached_tokens` (in `prompt_tokens_details`) |
| Google | `response.usage_metadata.prompt_token_count` | Plus `candidates_token_count`, `cached_content_token_count` |

To turn tokens into USD, you need a price table. Anthropic publishes one
([anthropic.com/pricing](https://www.anthropic.com/pricing)), OpenAI does
too. Build a small lookup keyed by `(model, token_kind)`:

```python
# illustrative
PRICES = {
    ("claude-sonnet-4-6", "input"):       3.0  / 1_000_000,  # $3 / MTok
    ("claude-sonnet-4-6", "output"):      15.0 / 1_000_000,
    ("claude-sonnet-4-6", "cache_write"):  3.75 / 1_000_000,
    ("claude-sonnet-4-6", "cache_read"):   0.30 / 1_000_000,
    ("gpt-4o",            "input"):        2.5  / 1_000_000,
    ("gpt-4o",            "output"):       10.0 / 1_000_000,
}
```

**Pros.** Zero infra. Always exact (it's what you got billed for).

**Cons.** You maintain the price table. Per-vendor field names differ. No
aggregation across runs.

## Layer 1: LiteLLM gateway (only inside adapter shims)

For roko's own runs, **skip LiteLLM**. The dispatcher already speaks 8
provider protocols natively and writes the same schema as LiteLLM would.

For **competing frameworks** (LangGraph, CrewAI, AutoGen, OpenAI Agents
SDK) that we wrap as roko backends (see `04-eval-harnesses.md`), LiteLLM
is useful *inside* the Python adapter to give us a clean cost number
back. We don't run a global proxy; each adapter uses LiteLLM as a
client-side library:

```python
# demo/demo-research/adapters/langgraph.py
from langchain_anthropic import ChatAnthropic
# point at LiteLLM proxy *or* call Anthropic directly and read .usage
model = ChatAnthropic(
    base_url="http://localhost:4000",  # if running a proxy
    model="claude-sonnet-4-6",
)
# OR no proxy, call native and read response.usage_metadata
```

The adapter returns `{ usage: { input, output, cache_read, cache_write } }`
to the Rust backend, which writes the standard `AgentEfficiencyEvent`.

### When LiteLLM-as-proxy *is* worth running

If multiple competing-framework adapters all need to share token/cost
accounting, a single LiteLLM proxy in front of them avoids re-implementing
the same logic per adapter. You'd run it on a free port (not :6677, which
is roko-serve), and only the adapters point at it. Roko itself bypasses
the proxy entirely.

[BerriAI/litellm](https://github.com/BerriAI/litellm) features useful
*for adapters*:

- Single OpenAI-format interface for 100+ providers
- `cost_per_token`, `completion_cost`, `token_counter` helpers
- Cache-token accounting (Anthropic prompt cache, OpenAI cached tokens,
  Gemini cached content) — saves you maintaining a price table
- Per-key spend attribution if you want to label adapter calls

### Setup shape (proxy mode, optional)

```bash
pip install 'litellm[proxy]'
# config.yaml
model_list:
  - model_name: claude-sonnet-4-6
    litellm_params:
      model: anthropic/claude-sonnet-4-6
      api_key: os.environ/ANTHROPIC_API_KEY
  - model_name: gpt-4o
    litellm_params:
      model: openai/gpt-4o
      api_key: os.environ/OPENAI_API_KEY

litellm_settings:
  success_callback: ["langfuse"]
  failure_callback: ["langfuse"]

litellm --config config.yaml --port 4000
```

Then point every framework at `http://localhost:4000`:

```python
# Anthropic SDK
client = anthropic.Anthropic(base_url="http://localhost:4000", api_key="sk-...")

# OpenAI SDK / Agents / LangChain / CrewAI / AutoGen
client = OpenAI(base_url="http://localhost:4000/v1", api_key="sk-...")
```

(LiteLLM accepts any API key — the real keys live in the proxy config.
Optionally generate per-framework virtual keys to attribute spend.)

### Virtual keys → per-framework spend attribution

```bash
curl -X POST http://localhost:4000/key/generate \
  -H "Authorization: Bearer sk-master" \
  -d '{"models": ["claude-sonnet-4-6"], "metadata": {"framework": "roko"}}'
```

Now your spend logs come pre-labeled by framework. The dashboard becomes:

```
SELECT framework, SUM(spend) FROM spend_logs WHERE date = CURRENT_DATE
GROUP BY framework ORDER BY 2 DESC;
```

### What LiteLLM tracks per call

Every call writes a row with:
- `model`, `request_id`, `start_time`, `end_time`
- `input_tokens`, `output_tokens`, `cached_tokens`, `cache_creation_tokens`
- `cost` (USD)
- `metadata` (your framework label)
- full request + response bodies (optional, off by default for size)

For roko's runs, **`AgentEfficiencyEvent` already captures all of this and
more.** LiteLLM's spend log is only the source of truth for adapter calls
(competing frameworks).

### Integrations

- **Langfuse** — automatic; see `04-eval-harnesses.md`.
- **OpenTelemetry** — emits OTel traces; pipe to Phoenix or Tempo.
- **Prometheus** — `/metrics` endpoint; pipe to Grafana for live cost
  dashboards (see `05-realtime-visualization.md`).

## Layer 2: tokenizer-based pre-flight estimation

Before you call the API, estimate cost with the tokenizer. Useful for
budget enforcement (refuse to dispatch if estimated > budget).

```python
import tiktoken                       # OpenAI
enc = tiktoken.encoding_for_model("gpt-4o")
n = len(enc.encode(prompt))

import anthropic                       # Anthropic
client = anthropic.Anthropic()
n = client.beta.messages.count_tokens(model="claude-sonnet-4-6", messages=[...])
```

LiteLLM also exposes `litellm.token_counter(model=..., messages=...)` as a
unified interface.

**Pros.** Pre-flight, no API call.
**Cons.** Approximate. Doesn't include tool definitions, system prompts
appended by frameworks, or caching savings.

## Cache accounting (the easy thing to get wrong)

Anthropic prompt caching changes per-call cost by 5-10x. With caching:

- **Cache write**: 1.25x normal input cost (one-time per cached prefix)
- **Cache read**: 0.1x normal input cost (every subsequent call hitting it)
- **Cache hit window**: 5 minutes by default

If your demo runs 50 SWE-bench tasks and each one re-sends the same 30K-token
system prompt, **with caching** the total system-prompt cost is roughly
`30K * 1.25x` (one write) `+ 49 * 30K * 0.1x` (49 reads) — about 6% of the
no-cache cost. This is a big deal.

A demo that doesn't account for caching will systematically:

- Underestimate roko's wins (roko's persistent prompts cache well)
- Overestimate frameworks that don't use Anthropic-native caching

**roko already separates these.** `AgentEfficiencyEvent` has dedicated
`cache_read_tokens` and `cache_write_tokens`, plus both `cost_usd` and
`cost_usd_without_cache`. The TUI bench tab can render either or both
side-by-side without any new accounting.

For competing-framework adapters: LiteLLM separates `cached_tokens` from
`input_tokens` in its spend logs, so the adapter can extract them from
its API response and pass them to roko in the standard schema.

## Cache disable for fair comparison

Some demos call for caching *off* across frameworks, so you measure
"steady-state" cost without prompt-cache wins. Disable it:

```python
# Anthropic — don't pass cache_control on any block
# OpenAI — caching is implicit and can't be disabled, but rare on long prompts
```

With caching off, expect SWE-bench Lite at ~$60-150 for roko (vs $20-40 with
caching). Document which mode the demo numbers were taken in.

## Streaming + cost

If the demo wants live token-by-token display (`05-realtime-visualization.md`),
streaming changes a few things:

- Cost is only known at the *final* `message_stop` / `usage` event.
- Until then, you have to estimate from output character count or
  per-event token deltas.
- LiteLLM emits a final usage event in the stream; subscribe to that for
  exact cost.

Live cost meter pseudocode:

```python
running_input = 0
running_output = 0
async for chunk in stream:
    if chunk.type == "message_delta" and chunk.usage:
        running_output = chunk.usage.output_tokens
    update_meter(running_input, running_output)
```

## Per-task cost vs per-success cost

The single most informative metric for the roko pitch is **USD per
successful task**, not total spend.

```
USD per success = total spend / count(pass@1)
```

A framework that costs $0.20/task at 50% success ($0.40/success) is *worse*
than one that costs $0.30/task at 90% success ($0.33/success), even though
the per-task number is higher. This is the chart that should be on the
investor slide.

`CostsDb::summarize()` already returns `success_rate` and `total_cost_usd`
on the same struct, so `cost_usd / success_rate` gives you USD/success
directly without joining tables.

## What to log per task

**Already covered.** `AgentEfficiencyEvent` (per turn) and `Episode` (per
turn with gate verdicts) and `TaskMetric` (per task aggregate) collectively
contain every field the original schema below was meant to capture. They
persist automatically to `.roko/learn/efficiency.jsonl`,
`.roko/memory/episodes.jsonl`, and `.roko/memory/task-metrics.jsonl`.

The fields you'd want for a flat `runs.csv` are derivable with one query
against `CostsDb` plus an aggregation by `task_id`. No extra schema
needed.

For external export (Inspect AI submission, Langfuse, Pareto plot script),
flatten to:

```json
{
  "task_id": "swe-bench-mini-001",
  "backend": "roko",                  // — efficiency.backend
  "model": "claude-sonnet-4-6",
  "started_at": "2026-04-26T18:30:00Z",
  "ended_at": "2026-04-26T18:34:12Z",
  "wall_seconds": 252,                // — efficiency.wall_time_ms / 1000
  "tokens_input": 18432,              // sum of efficiency.input_tokens
  "tokens_output": 2104,
  "tokens_cache_read": 14000,
  "tokens_cache_create": 0,
  "tokens_reasoning": 512,            // roko-only signal
  "usd_cost": 0.0892,                 // sum of efficiency.cost_usd
  "usd_cost_without_cache": 0.412,    // for reporting both modes
  "outcome": "pass",                  // — episode.outcome
  "score": 1.0,                       // — episode.gate_verdicts last passed
  "tool_calls": 7,                    // — sum of efficiency.tool_calls.len()
  "gate_failures": 1,                 // — count of gate_verdicts where !passed
  "gate_failures_recovered": 1,
  "iteration_count": 2,               // — efficiency.iteration max
  "ttft_ms": 420,                     // — efficiency.time_to_first_token_ms
  "rerun_index": 0
}
```

The flattener is `crates/roko-cli/src/bin/bench_export.rs` — ~200 lines
that reads the JSONL, joins by task_id, writes CSV/Parquet. Single
command:

```bash
cargo run -p roko-cli --bin bench_export -- \
    --plan-prefix bench-swe-mini- \
    --output reports/2026-04-26/runs.csv
```

This is the input to every external chart tool.

# Frameworks to compare against

**Architectural move: each competitor framework becomes a roko backend.**
This means we don't build a separate harness per framework — roko's
existing dispatcher invokes each one, and roko's existing
`AgentEfficiencyEvent` pipeline records cost/tokens/latency uniformly.

See `08-reuse-map.md` and `04-eval-harnesses.md` for the full pattern.
Each "backend" is ~100 lines of Rust (a `roko-agent` backend impl that
shells out) plus ~50 lines of Python (the framework adapter that runs
the actual framework and returns standardized JSON).

Five baselines covering the realistic competition. Each entry: install,
minimal "agent loop" shape, how it emits token/cost data, and the specific
weakness roko's pitch should highlight.

The grid we want to fill:

| Framework | Install | Loop primitive | Cost data | Reliability |
|---|---|---|---|---|
| Anthropic SDK direct | `pip install anthropic` | manual `messages.create` loop | `usage` field on response | None — ad hoc |
| OpenAI Agents SDK | `pip install openai-agents` | `Agent` + `Runner.run` | `usage` field | Built-in retries |
| Anthropic Claude Agent SDK | `pip install claude-agent-sdk` | `Agent` + `query` | `usage` field | Built-in retries |
| LangGraph | `pip install langgraph langchain-anthropic` | `StateGraph` + nodes | LangSmith trace OR LiteLLM | Conditional edges |
| CrewAI | `pip install crewai` | `Crew` + `Task` + `Agent` roles | LiteLLM (built-in) | Some retries |
| AutoGen / AG2 | `pip install autogen` | `AssistantAgent` + `UserProxyAgent` group chat | LiteLLM | Group voting |

Pin the same model behind every framework — see `03-cost-tokens.md` for the
LiteLLM gateway pattern that makes this clean.

## Anthropic SDK direct (the floor)

The "no orchestration" baseline. Just a tool-calling loop.

```python
# illustrative shape, not implementation
import anthropic
client = anthropic.Anthropic()

def run_task(prompt, tools):
    messages = [{"role": "user", "content": prompt}]
    while True:
        resp = client.messages.create(
            model="claude-sonnet-4-6",
            max_tokens=4096,
            tools=tools,
            messages=messages,
        )
        # resp.usage.input_tokens, resp.usage.output_tokens,
        # resp.usage.cache_creation_input_tokens, resp.usage.cache_read_input_tokens
        if resp.stop_reason == "end_turn":
            return resp, total_usage
        # else handle tool_use, append tool_result, loop
```

**What it gets you.** The cheapest possible per-call cost (no scaffolding
overhead). Forces you to wire tool execution, retries, and termination
yourself.

**What roko should beat it on.** Reliability (no gates → silent regressions),
plan complexity (no DAG), persistence (no resume on crash), learning (no
adaptive routing).

**What it will beat roko on.** Raw cost on trivial tasks where roko's
overhead (system prompt build, gate runs, episode logging) is overhead.
That's expected — show the crossover point.

## OpenAI SDK direct

Same shape as the Anthropic direct, against OpenAI models. Use only if the
demo specifically wants to show roko works across providers. Otherwise pick
one direct-SDK baseline and stick to it.

```python
from openai import OpenAI
client = OpenAI()
resp = client.chat.completions.create(
    model="gpt-4o", messages=[...], tools=[...],
)
# resp.usage.prompt_tokens, resp.usage.completion_tokens
```

## OpenAI Agents SDK

[openai-agents](https://github.com/openai/openai-agents-python). First-party
agent loop from OpenAI.

```python
from agents import Agent, Runner

agent = Agent(
    name="coder",
    instructions="You fix bugs in Python repos.",
    tools=[bash_tool, edit_tool],
    model="gpt-4o",
)

result = await Runner.run(agent, "Fix issue #1234 in repo X.")
# result.usage  # OpenAI usage object
```

**What it gets you.** Decent built-in tool loop, handoffs between agents,
guardrails, sessions, tracing.

**What roko should beat it on.** Cross-vendor (this is OpenAI-only),
deterministic gates, Rust-native execution, MCP (it does support MCP but
less centrally).

## Anthropic Claude Agent SDK

[claude-agent-sdk-python](https://github.com/anthropics/claude-agent-sdk-python).
First-party agent loop from Anthropic; reuses Claude Code's harness.

```python
from claude_agent_sdk import query

async for message in query(prompt="Fix issue #1234"):
    print(message)
```

**What it gets you.** Same harness as Claude Code (file edits, bash, MCP,
hooks, plan mode). Strong default for a coding agent.

**What roko should beat it on.** Multi-task DAGs, gates with adaptive
thresholds, learning loops, persistence/resume, multi-agent orchestration.
The Claude SDK is a *single* agent with a tool loop; roko is a *fleet* with
plans and gates.

This is probably the most important baseline for your story because it's
what users would otherwise reach for.

## LangChain / LangGraph

LangChain is the legacy chain-runner; LangGraph is the production-leaning
state-machine subset. Use **LangGraph**, not bare LangChain.

```python
from langgraph.graph import StateGraph
from langchain_anthropic import ChatAnthropic

graph = StateGraph(state_schema=MyState)
graph.add_node("plan", plan_node)
graph.add_node("execute", execute_node)
graph.add_conditional_edges("execute", should_replan)
app = graph.compile()
result = app.invoke({"task": "..."})
```

**What it gets you.** Explicit state machine, branching, loops, persistence
via checkpointer, integration with LangSmith for tracing.

**What roko should beat it on.** Token efficiency (LangGraph adds prompt
overhead per node hop), Rust execution (Python-only), gates not built in,
plan generation isn't a primary concept.

Published 2026 cost benchmarks land LangGraph at ~$0.08/task on a 10-step
research pipeline — the cheapest of the open frameworks.

## CrewAI

[crewai](https://github.com/crewAIInc/crewAI). Role-based multi-agent.

```python
from crewai import Agent, Task, Crew

researcher = Agent(role="Researcher", goal="Find sources", ...)
writer = Agent(role="Writer", goal="Draft report", ...)
crew = Crew(agents=[researcher, writer], tasks=[t1, t2])
result = crew.kickoff()
```

**What it gets you.** Easiest to set up, role-based mental model. Built-in
LiteLLM integration for cost tracking out of the box.

**What roko should beat it on.** Reliability (role prompting alone is
brittle), cost on complex tasks (~$0.15/task vs LangGraph's $0.08),
verifiability (no gates).

## AutoGen / AG2

[ag2](https://github.com/ag2ai/ag2) (formerly AutoGen). Multi-agent group
chat with voting / debate.

```python
from autogen import AssistantAgent, UserProxyAgent

assistant = AssistantAgent(name="coder", llm_config={...})
user = UserProxyAgent(name="user", code_execution_config={...})
user.initiate_chat(assistant, message="Fix the bug")
```

**What it gets you.** Multi-agent debate, group decision-making, code
execution.

**What roko should beat it on.** Cost — AutoGen consistently uses 5-6x more
tokens than LangGraph due to inter-agent messaging. ~$0.45-0.50/task on the
same 10-step research pipeline. Great showcase for "roko's planner picks the
single best path instead of debating."

## (Optional) LlamaIndex Agents

If the demo cares about retrieval-heavy tasks. Skip otherwise — it's not
roko's strength either.

## (Optional) Cursor / Devin / Cline

Closed-source competitors. Don't include in a programmatic comparison
(no API for fair runs), but do include their published numbers in a
"context" slide.

## Required: pin the same model

Every framework above must hit the same model. If LangGraph runs against
Sonnet 4.6 and AutoGen against GPT-4o, you're benchmarking models, not
orchestration.

Each adapter receives `{ model: "claude-sonnet-4-6", ... }` from the roko
backend invocation and uses it. Adapters that go through LiteLLM
(`base_url=http://localhost:4000`) get unified cache-token tracking;
adapters that hit Anthropic SDK directly read `response.usage` themselves.
Either is fine as long as the schema returned to roko is the same.

For roko's own runs (the `roko` backend), no gateway — the dispatcher
already speaks Anthropic's protocol natively.

## Optional: pin the *tools* too

Some frameworks ship default tools (web search, calculator). For a fair
fight, define a minimal common toolkit:

- `bash(command: str) -> str`
- `read_file(path: str) -> str`
- `write_file(path: str, content: str) -> None`
- `apply_patch(diff: str) -> None`

Wrap these once in Python and adapt to each framework's tool spec
(LangGraph `@tool`, OpenAI Agents `function_tool`, CrewAI `BaseTool`, etc).

## Skill ceiling per framework

For an honest comparison, allocate equal effort to each framework's prompt
and tool design. A senior engineer can squeeze 20-40% more pass@1 out of
LangGraph than the default config will give. If you only optimize roko, the
comparison is dishonest.

Minimum viable parity:

- Same system prompt template (with framework-specific reformatting allowed)
- Same model
- Same tool surface
- Same retry budget (e.g. max 8 tool turns)
- Same task input format

## Configuration matrix

For the final demo, the comparison grid we'd run looks like:

```
            HumanEval    BFCL v4    SWE-bench-mini    τ²-retail    roko-bench
roko        ✓            ✓          ✓                 ✓            ✓
anthropic   ✓            ✓          ✓                 ✓            ✓
oai-agents  ✓            ✓          ✓                 ✓            ✓
langgraph   ✓            ✓          ✓                 ✓            ✓
crewai      ✓            ✓          ✓                 ✓            ✓
autogen     ✓            ✓          ✓                 ✓            ✓
```

= 6 frameworks × 5 benchmarks = 30 cells. Each cell stores: pass@1, pass^4,
total tokens, USD, wall time, gate-failure-rate (roko cells only).

That's the headline table. `05-realtime-visualization.md` covers how to
*display* it during a live demo.

## How a competitor becomes a roko backend

Each framework gets two files:

```
crates/roko-agent/src/backends/<name>.rs    # ~100 lines Rust
demo/demo-research/adapters/<name>.py        # ~50 lines Python
```

The Rust side implements the existing `Backend` trait
(`crates/roko-agent/src/backends/mod.rs`):

```rust
// illustrative — matches the pattern of existing backends
pub struct LangGraphBackend { python_path: PathBuf, adapter_path: PathBuf }

#[async_trait]
impl Backend for LangGraphBackend {
    async fn dispatch(&self, req: DispatchRequest) -> Result<DispatchResponse> {
        let json_in = serde_json::to_string(&req.to_adapter_format())?;
        let output = tokio::process::Command::new(&self.python_path)
            .arg(&self.adapter_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?
            .write_stdin(&json_in)
            .await?
            .wait_with_output()
            .await?;
        let resp: AdapterResponse = serde_json::from_slice(&output.stdout)?;
        Ok(DispatchResponse::from_adapter(resp))  // emits AgentEfficiencyEvent
    }
}
```

`DispatchResponse::from_adapter()` writes the standard
`AgentEfficiencyEvent` to `.roko/learn/efficiency.jsonl` with
`backend = "langgraph"`. From this point, every existing roko view
(F1 Dashboard, F10 Learning, the new F11 Bench) sees this run alongside
roko's own runs.

Backend registration goes through the existing `CascadeRouter` — for
benchmarks we override with `force_backend = "langgraph"`, so routing is
deterministic for comparison runs but adaptive for normal use.

The Python adapter is whatever shape the framework needs — a small
script that reads stdin, runs the framework, returns a JSON dict on
stdout. Sample shape in `04-eval-harnesses.md` § "Backend pattern".

## What this reuse buys us

| Capability | If we built standalone | With backend pattern |
|---|---|---|
| Cost tracking | New schema, new persistence | `efficiency.jsonl` (existing) |
| Trace storage | New schema, new viewer | `episodes.jsonl` (existing) |
| Live dashboard | New web app | TUI bench tab (existing widgets) |
| Aggregation | New SQL | `CostsDb` (existing) |
| Multi-rep / pass^k | New runner | `run_plan(reps=N)` (existing) |
| Gates / scoring | New adapter | gate pipeline (existing) |
| Per-task budgets | New | `tasks.toml` field (existing) |
| Dispatch retries | New | `force_backend` + cascade logic (existing) |

The competitor adapters are the only net-new code.

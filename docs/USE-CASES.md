# Use cases and positioning

> **Abstract:** Concrete use cases for Roko, organized by maturity tier and user persona.
> Each use case maps to real CLI commands, names competitive alternatives, and estimates
> cost per task. Written for engineers evaluating whether to adopt Roko.

> **Implementation**: Mixed (Available Now through Specified)

---

## Who this is for

Four personas appear throughout this document. Each cares about different things:

| Persona | Role | Primary concern |
|---------|------|-----------------|
| **Solo dev** | Individual contributor, side projects or small startups | Leverage: doing 10x the work with the same hours |
| **Team lead** | Engineering manager, 4-12 person team | Throughput: clearing backlogs, unblocking reviews, cutting costs |
| **Platform eng** | Infrastructure or developer experience team | Integration: plugging agents into CI, webhooks, internal tools |
| **Researcher** | Academic or R&D engineer exploring agent architectures | Extensibility: custom gates, new learning algorithms, novel routing |

---

## How to read the maturity tiers

Each use case is tagged with a maturity tier from [`STATUS.md`](STATUS.md):

| Tier | What it means | Can you use it today? |
|------|---------------|----------------------|
| **Available now** | Shipping. CLI-accessible. Tested in the self-hosting loop. | Yes |
| **Near-term** | Code exists (Built tier) or is partially wired. Weeks to months from shipping. | Partially |
| **Vision** | Specified in PRD docs. No code, or scaffold stubs only. | No |

---

## Tier 1: Available now

These work today. You can run the commands below after `cargo build --workspace`.

### 1.1 Autonomous plan execution

**What it is.** Describe a feature as a PRD. Roko decomposes it into a task DAG, dispatches
agents in parallel, runs each output through an 11-gate verification pipeline, persists state
for crash recovery, and learns from outcomes.

**Who needs it.** Team leads and solo devs sitting on backlogs of well-understood work:
migrations, refactors, boilerplate, test coverage. The work is clear but tedious.

**How Roko solves it.**

```bash
# Capture the idea
roko prd idea "Migrate all API handlers from actix-web to axum"

# Draft a structured PRD (agent-driven)
roko prd draft new "actix-to-axum-migration"

# Generate implementation plan with task DAG
roko prd plan actix-to-axum-migration

# Execute — agents work in parallel, gates verify, state persists
roko plan run plans/

# Resume after interruption
roko plan run plans/ --resume-plan
```

The DAG executor parallelizes independent tasks. The cascade router picks the cheapest model
that passes gates for each task type. If a gate fails, the agent retries with error context
injected into the next prompt.

**Estimated cost.** $0.08-$2.10 per completed task, depending on complexity and model
selection. Mechanical tasks (rename, move, add derives) land near $0.08. Architecture
decisions that require Opus-class models reach $2.10.

**Competitive landscape.**

| Alternative | Limitation |
|-------------|-----------|
| Cursor | One task at a time. No DAG execution, no gate pipeline. |
| Devin | $500/month. Closed platform. No self-hosting. |
| OpenAI Codex | Sandboxed execution. No multi-task orchestration. No learning. |
| SWE-Agent | Single agent, no parallel execution. Benchmark-focused, not production-focused. |
| Claude Code | Single-agent CLI. No plan decomposition, no crash recovery, no adaptive routing. |

Roko is the only option that combines DAG-parallel execution, multi-gate verification, crash
recovery via state snapshots, and adaptive model routing in a single tool.

---

### 1.2 Single-shot task execution

**What it is.** Give Roko a prompt. It assembles project context, dispatches an agent, runs
the output through gates, and exits with code 0 (pass) or 1 (fail).

**Who needs it.** Solo devs who want an agent that verifies its own work. Platform engineers
scripting agent tasks in CI.

**How Roko solves it.**

```bash
# Fix a specific bug
roko run "fix the compile error in roko-gate/src/pipeline.rs"

# Add a feature
roko run "add a health check endpoint returning JSON with uptime and version"

# Pipe context in
git diff HEAD~1 | roko run "review this diff for security issues"
```

The 6-layer `SystemPromptBuilder` assembles project context (repo map, relevant code, role
instructions, prior learnings) within a configurable token budget. The gate pipeline
(compile, test, clippy, diff) runs after every agent turn.

**Estimated cost.** $0.10-$1.50 per task. The cascade router starts with the cheapest
viable model and escalates only on failure.

**Competitive landscape.**

| Alternative | Limitation |
|-------------|-----------|
| Cursor / Copilot | No gate verification. No exit codes for scripting. |
| Claude Code | No adaptive model routing. No gate pipeline. |
| Aider | No multi-model routing. Limited verification (lint only). |

---

### 1.3 Research synthesis

**What it is.** Deep-dive research on any topic. Produces a structured report with citations,
stored in `.roko/research/`. Can enhance existing PRDs and plans with research-backed context.

**Who needs it.** Anyone making technical decisions. Researchers surveying a field. Solo devs
evaluating libraries before committing to an implementation path.

**How Roko solves it.**

```bash
# Standalone research
roko research topic "Rust async patterns for high-throughput servers"

# Deep research (async, uses Perplexity deep research)
roko research topic "Zero-knowledge proof systems for agent attestation" --deep

# Enhance a PRD with research
roko research enhance-prd oauth2-auth

# Optimize a plan with research
roko research enhance-plan actix-migration

# Direct web search
roko research search "axum middleware patterns" --recency month
```

**Estimated cost.** $0.05-$0.50 per research query, depending on depth. Deep research
queries cost more but produce comprehensive reports with academic citations.

**Competitive landscape.** Most coding agents treat research as out of scope. Roko's research
agent stands on its own, independent of code execution. Perplexity provides raw search; Roko
adds structured synthesis, PRD integration, and persistent storage.

---

### 1.4 PRD lifecycle management

**What it is.** A structured pipeline from raw ideas to published product requirements
to executable plans. Each stage is a CLI command.

**Who needs it.** Team leads managing feature backlogs. Solo devs who prefer thinking at
the requirements level and letting agents handle decomposition.

**How Roko solves it.**

```bash
# Capture ideas as they come
roko prd idea "WebSocket support for real-time updates"
roko prd idea "Rate limiting middleware"

# List and triage
roko prd list

# Draft a structured PRD (agent refines your idea)
roko prd draft new "websocket-realtime"

# Research and enhance
roko research enhance-prd websocket-realtime

# Promote to published
roko prd draft promote websocket-realtime

# Generate implementation plan
roko prd plan websocket-realtime

# Check coverage
roko prd status

# Find duplicates and gaps
roko prd consolidate
```

PRDs live in `.roko/prd/` with a clear lifecycle: `ideas/` -> `drafts/` -> `published/`.
Each stage adds structure. The `prd plan` command generates a `tasks.toml` with dependency
ordering, complexity estimates, role assignments, and test hints.

**Competitive landscape.** No other agent tool provides a requirements-to-execution pipeline.
Cursor, Devin, and Codex start at the task level. Roko starts at the requirements level.

---

### 1.5 Learning and cost optimization

**What it is.** Every agent turn updates 10+ learning subsystems: episode logger, cost
tracker, playbook rules, skill library, cascade router, experiments, and efficiency events.
The system gets cheaper and better with use.

**Who needs it.** Team leads paying LLM API bills. Anyone running agents at volume who wants
visibility into cost-per-task and levers to reduce it.

**How Roko solves it.**

```bash
# Check current status and learning metrics
roko status --cfactor

# View routing decisions
roko model route claude-sonnet-4-6 --explain --complexity focused

# List provider health
roko provider health

# Run A/B experiments on models
roko experiment model create \
  --id sonnet-vs-flash \
  --role implementer \
  --variant sonnet:claude-sonnet-4-6:claude_cli \
  --variant flash:gemini-2-5-flash:gemini \
  --min-trials 30

# Check experiment results
roko experiment model show sonnet-vs-flash

# Analyze execution data for insights
roko research analyze

# View the dashboard
roko dashboard --page efficiency
```

The cascade router uses a three-stage selection process:
1. **Static rules** filter models by capability (context window, tool support)
2. **Confidence routing** picks the cheapest model above a confidence threshold
3. **UCB bandit** explores when data is sparse, exploits when confident

Gate-adaptive thresholds (EMA per rung) learn expected pass rates and flag anomalies.
Playbook rules accumulate behavioral heuristics from past runs. All of this persists in
`.roko/learn/` and carries across sessions.

**Estimated savings.** Teams paying $1,000+/month for AI coding can cut costs 60-80% by
routing mechanical tasks to cheaper models while reserving expensive models for architecture
decisions.

**Competitive landscape.** No other agent tool provides adaptive model routing, A/B
experimentation, or cost tracking. Cursor uses a fixed model. Devin is a black box. Codex
does not expose routing decisions.

---

### 1.6 Configuration and multi-provider setup

**What it is.** Layered configuration: global defaults, project overrides, secret
management, and multi-provider support with circuit breakers.

**Who needs it.** Platform engineers standardizing agent configuration across teams. Solo devs
switching between providers.

**How Roko solves it.**

```bash
# Interactive setup wizard
roko config init

# Set provider-specific config
roko config set agent.model claude-opus-4-6 --project
roko config set-secret GEMINI_API_KEY sk-abc123...

# Validate configuration
roko config validate

# Check provider connectivity
roko provider list
roko provider test gemini

# Migrate legacy config
roko config migrate --dry-run
```

Configuration merges three layers: global (`~/.config/roko/config.toml`), project
(`roko.toml`), and environment variables. Each provider has independent circuit breaker
state, latency tracking, and health monitoring.

**Competitive landscape.** Most tools support one provider. Roko supports five backends
(Claude CLI, Anthropic API, OpenAI-compatible, Cursor ACP, Ollama) with automatic failover.

---

## Tier 2: Near-term

Code exists for these. Some are partially wired. Weeks to months from shipping.

### 2.1 Event-driven autonomous workflows

**What it is.** A daemon that watches for events (GitHub webhooks, cron schedules, file
changes) and triggers agent workflows without human initiation.

**Who needs it.** Platform engineers who want agents responding to GitHub issues, reviewing
PRs, and fixing CI failures automatically. Team leads who want overnight batch processing.

**How Roko would solve it.**

```bash
# Start the daemon
roko daemon start

# Configure subscriptions in roko.toml:
# [[subscriptions]]
# name = "auto-triage-issues"
# trigger = "github:issues:opened"
# template = "issue-triage"

# Monitor
roko daemon status
roko daemon logs -f

# Install as system service
roko daemon install
```

**Current status.** The daemon command structure exists. Subscription configuration is
specified. The HTTP webhook receiver and cron scheduler need implementation.

**Competitive landscape.**

| Alternative | Limitation |
|-------------|-----------|
| GitHub Copilot for PRs | Review only. No execution, no learning. |
| Devin | Closed platform. No self-hosting, no customization. |
| Custom GitHub Actions | No learning, no routing, no gate pipeline. One-off scripts. |

---

### 2.2 Interactive TUI dashboard

**What it is.** A terminal UI (ratatui-based) for monitoring plan execution, agent status,
gate results, learning metrics, and cost tracking in real time.

**Who needs it.** Solo devs and team leads who want visibility into what agents are doing
without reading log files.

**How Roko would solve it.**

```bash
# Launch the TUI
roko dashboard

# Tab navigation: F1-F7
# Dashboard | Plans | Agents | Git | Logs | Config | Inspect
```

**Current status.** The `roko dashboard` command exists and renders text-mode output. The
TUI scaffold (tab structure, page routing) is in place. The ratatui wiring is on the
critical path — it's the #1 priority in [`STATUS.md`](STATUS.md).

Text mode works now:

```bash
roko dashboard --text
roko dashboard --page efficiency
roko dashboard --list-pages
```

**Competitive landscape.** Cursor has a visual IDE. Devin has a web dashboard. No CLI-based
agent tool provides a terminal dashboard with real-time plan execution, gate results, and
cost tracking.

---

### 2.3 Knowledge store (Neuro)

**What it is.** A tiered knowledge system: 6 knowledge types (Insight, Heuristic, Warning,
CausalLink, StrategyFragment, AntiKnowledge) across 4 validation tiers (Transient, Working,
Consolidated, Persistent). Knowledge decays based on its validation tier.

**Who needs it.** Researchers building agents that accumulate domain knowledge. Team leads
who want institutional knowledge to persist across developer turnover.

**How Roko would solve it.**

```bash
# Query the knowledge store
roko neuro query "authentication patterns"

# Check knowledge store stats
roko neuro stats

# Garbage collect low-confidence entries
roko neuro gc
```

**Current status.** The `roko-neuro` crate is Built — structs exist, HDC encoding works,
but the store is not yet wired to the orchestrator's context injection pipeline. CLI commands
exist but operate on the standalone store, not the orchestration loop.

**Competitive landscape.** No other agent tool provides typed, tiered, decaying knowledge
management. LangChain has vector stores (external); everything else has no built-in memory.

---

### 2.4 Conductor (anomaly detection and intervention)

**What it is.** A cybernetic regulator: 10 watchers monitor agent behavior in real time.
Graduated interventions (Continue, Restart, Fail) respond to stuck agents, runaway costs,
and anomalous patterns. Circuit breakers prevent cascading failures.

**Who needs it.** Platform engineers running agents in production where provider outages
and stuck agents need automatic handling.

**Current status.** The `roko-conductor` crate is Built with 10 watchers and circuit breaker
logic. Not yet called from the orchestrator. Wiring it is priority #5 in the roadmap.

**Competitive landscape.** No other agent tool provides built-in anomaly detection,
stuck-agent detection, or graduated intervention. Most tools let agents run until timeout.

---

### 2.5 Code intelligence

**What it is.** Tree-sitter parsing, symbol graph with PageRank importance scoring, and
10,240-bit HDC fingerprints for sub-millisecond code similarity search. Three language
providers: Rust, TypeScript, Go.

**Who needs it.** Researchers working on code understanding. Platform engineers building
code search into internal tools.

**Current status.** The `roko-index` and `roko-lang-*` crates are Built with 30 tests.
No MCP server, no search API, no persistent storage. The gap is exposure, not capability.

**Competitive landscape.** Sourcegraph provides code search as a service. GitHub code search
is cloud-only. Roko's code intelligence is local-first, HDC-accelerated, and designed to
feed into agent prompts — not just human search.

---

## Tier 3: Vision

Specified in PRD documents. No shipping code. These represent where Roko is heading, not
what it does today.

### 3.1 Automatic plan generation from published PRDs

**What it is.** When a PRD is promoted to published status, Roko automatically generates an
implementation plan and begins execution. Removes the last manual step in the self-hosting
loop.

**Who needs it.** Anyone who wants full end-to-end autonomy. Write the PRD, walk away, come
back to verified code.

**Current gap.** The `prd draft promote` command exists with an `--auto-execute` flag, but
the automatic trigger from PRD publish to `prd plan` is not implemented. This is priority #2
on the critical path.

---

### 3.2 Failure feedback loop (re-planning)

**What it is.** When gates fail on a task, the failure context feeds back into the plan
generator. The system re-decomposes the failing task into smaller subtasks, picks different
models, or adjusts the approach.

**Who needs it.** Team leads running large plans where some tasks are harder than estimated.
Solo devs who want the system to adapt when things go wrong.

**Current gap.** Gate failures trigger retry with error context injection (this works). What
doesn't exist is re-planning: decomposing a failing task into subtasks or adjusting the
plan structure based on failure patterns. This is priority #3 on the critical path.

---

### 3.3 Offline consolidation (Dreams)

**What it is.** When idle, agents enter a dream cycle: NREM replay (prioritized episode
review), REM imagination (counterfactual reasoning about what could have gone differently),
and integration staging (promoting validated knowledge to higher tiers).

**Who needs it.** Researchers exploring agent metacognition. Long-running deployments
where raw experience should compound into validated knowledge.

**Current status.** The `roko-dreams` crate has runner and cycle facade stubs. Core
algorithms are unimplemented. CLI commands exist:

```bash
roko dream run       # trigger a consolidation cycle
roko dream report    # view latest dream report
roko dream schedule  # check next scheduled cycle
```

---

### 3.4 Affect-driven compute allocation (Daimon)

**What it is.** PAD vectors (Pleasure, Arousal, Dominance) from cognitive science modulate
which model the agent uses, how much context it assembles, and whether it explores or
exploits. When things go well, the agent uses cheaper models. When things go badly, it
escalates.

**Who needs it.** Researchers studying agent decision-making. Cost-conscious teams who want
resource allocation that adapts to task difficulty.

**Current status.** The `roko-daimon` crate is wired into live prompt assembly and
model routing. PAD vectors and 6 behavioral states now influence both affect guidance in
the system prompt and tier-biased model selection; the remaining gaps are somatic landscape
and VCG-style context bidding.

---

### 3.5 HTTP API and server mode

**What it is.** REST API with Server-Sent Events for real-time streaming. Agent-as-a-Service
for any HTTP client.

**Who needs it.** Platform engineers building internal tools with agent capabilities.
Teams embedding agents into web applications.

**Current status.** The `roko serve` command exists. Route scaffolds are in place. No
implemented handlers.

```bash
roko serve --port 3000
```

---

### 3.6 Multi-agent coordination

**What it is.** Stigmergy-based coordination: agents communicate through typed, decaying,
scoped pheromones instead of direct messaging. Morphogenetic specialization via
reaction-diffusion lets agents differentiate into roles based on environmental signals.

**Who needs it.** Researchers working on multi-agent systems. Teams running agents at volume
who need coordination without a central controller.

**Current status.** Specified in PRD section 13. No code exists.

---

### 3.7 Agent skill marketplace and brain export

**What it is.** Export an agent's learned state (routing weights, playbook rules, skill
patterns, gate thresholds) as a portable artifact. Import it on a fresh instance. Share
accumulated experience across teams.

**Who needs it.** Team leads onboarding new developers. Organizations where knowledge should
compound across people, not stay trapped in individual configurations.

**Current status.** Specified. The data structures exist in `.roko/learn/`. The export/import
commands are not yet implemented.

---

## Cost comparison

The economics of a 100-task migration backlog:

| Approach | Unit cost | Total | Time |
|----------|-----------|-------|------|
| Manual development | ~2 hours x $100/hr per task | ~$20,000 | 4-8 weeks |
| Roko (mechanical tasks, cheap models) | ~$0.15/task avg, ~1.86 iterations | ~$28 | Hours |
| Roko (mixed complexity) | ~$0.80/task avg, ~1.86 iterations | ~$149 | Hours |
| Roko (hard tasks, expensive models) | ~$2.10/task avg, ~1.86 iterations | ~$391 | Hours |
| Devin | $500/month flat | $500+ | Days |
| Cursor (manual, one at a time) | ~$20/month + developer time | ~$5,000+ | Weeks |

The cascade router's bandit algorithm learns which model tier passes gates for each task
type, automatically migrating cheap tasks to cheap models. A 100-task run starts expensive
and gets cheaper as the router accumulates data.

---

## Competitive positioning matrix

How Roko compares to alternatives across the use cases above:

| Capability | **Roko** | **Cursor** | **Devin** | **Codex** | **SWE-Agent** | **Claude Code** |
|-----------|---------|----------|---------|---------|-------------|---------------|
| Plan decomposition (PRD to DAG) | Yes | No | Limited | No | No | No |
| Parallel task execution | Yes | No | Yes | No | No | No |
| Gate verification pipeline | 11 gates, adaptive | No | Unknown | Sandboxed tests | SWE-bench | No |
| Crash recovery and resume | Yes | No | Yes | No | No | No |
| Adaptive model routing | 3-stage cascade | Fixed | Fixed | Fixed | Fixed | Fixed |
| Cost tracking per task | Yes | No | No | No | No | No |
| A/B model experiments | Yes | No | No | No | No | No |
| Learning from outcomes | 10+ subsystems | No | Unknown | No | No | No |
| Research synthesis | Yes | No | No | No | No | No |
| PRD lifecycle | Full pipeline | No | No | No | No | No |
| Self-hosting | Yes | No | No | No | No | No |
| Open source | Yes | No | No | No | Yes | No |
| Runs locally | Yes | Yes | No | No | Yes | Yes |
| Multiple LLM providers | 5 backends | 1 | 1 | 1 | Configurable | 1 |

---

## Niche opportunities

### Rust ecosystem

Roko is written in Rust. Its gate pipeline natively supports Cargo. Its language providers
understand ownership, trait bounds, and lifetimes. The Rust community values verification
and correctness — Roko's 11-gate pipeline resonates.

**Entry angle.** "The AI coding agent that runs `cargo clippy` and `cargo test` on every
output, automatically."

**Persona.** Solo dev or team lead working in Rust.

### Cost-sensitive teams

Most agent tools optimize for capability, not cost. Roko's cascade router explicitly
optimizes the cost-quality tradeoff. The A/B experiment system provides empirical data
on which models deliver the best cost-per-successful-task.

**Entry angle.** "Stop paying Opus prices for tasks that Haiku can handle."

**Persona.** Team lead managing LLM API budgets.

### Compliance-heavy organizations

Roko's safety layer (role auth, pre/post checks, capability tokens) and gate pipeline
produce an auditable record of every verification check. Every agent decision is logged
with full context in `.roko/episodes.jsonl`.

**Entry angle.** "Every AI code change verified by 11 independent checks, with a full
audit trail."

**Persona.** Platform engineer in a regulated industry.

### Open-source maintainers

Large OSS projects have massive issue backlogs. The PRD pipeline (issue -> idea -> draft ->
plan -> execute -> verify -> PR) maps directly to the maintainer workflow.

**Entry angle.** "Turn your GitHub issues into verified PRs, automatically."

**Persona.** Solo dev or team lead maintaining a popular open-source project.

### Agent infrastructure teams

Teams building their own agent systems need routing, learning, verification, and
observability as composable building blocks. Roko's crates are independently usable:

- `roko-agent` — agent pool, 5 LLM backends, event streaming
- `roko-gate` — verification pipeline with adaptive thresholds
- `roko-learn` — episodes, bandits, playbooks, experiments
- `roko-compose` — prompt assembly with token budgets

**Entry angle.** "Agent building blocks — routing, gates, learning — as reusable Rust
crates."

**Persona.** Platform engineer or researcher building custom agent systems.

---

## The self-hosting proof

Roko's strongest positioning argument is concrete: it uses itself to develop itself. The
workflow runs end-to-end today:

```
roko prd idea -> roko prd draft -> roko research enhance-prd
  -> roko prd plan -> roko plan run -> gate -> persist -> resume
```

Each improvement to Roko improves the agent that builds Roko. This compound improvement
loop requires a PRD-to-plan-to-execution-to-verification pipeline to close — and no other
tool has one.

Three gaps remain before full autonomy:
1. Automatic plan generation when a PRD is published (manual step today)
2. Failure feedback loop for re-planning (retry works, re-planning doesn't)
3. Interactive TUI for monitoring (text-mode fallback works)

After these, the only human input is the initial PRD.

---

## Quick reference: commands by persona

### Solo dev

```bash
roko init                                # set up a project
roko run "add auth middleware"           # single task
roko research topic "JWT vs session"     # research before deciding
roko status                              # check what happened
```

### Team lead

```bash
roko prd idea "migrate to axum"          # capture work
roko prd draft new "axum-migration"      # structure it
roko prd plan axum-migration             # decompose into tasks
roko plan run plans/                     # execute the plan
roko prd status                          # coverage report
roko experiment model show sonnet-vs-flash  # check model experiments
```

### Platform engineer

```bash
roko config init                         # set up providers
roko provider list                       # check connectivity
roko provider health                     # circuit breaker status
roko config validate                     # validate config
roko daemon start                        # event-driven workflows (near-term)
roko serve --port 3000                   # HTTP API (near-term)
```

### Researcher

```bash
roko research topic "HDC for code search" --deep   # deep research
roko neuro query "embedding patterns"               # knowledge store query
roko neuro stats                                    # knowledge metrics
roko model route claude-sonnet-4-6 --explain        # routing internals
roko dream run                                      # trigger consolidation (vision)
```

---

*Updated 2026-04-13. Status calibrated against [`STATUS.md`](STATUS.md). CLI commands
verified against [`CLI-REFERENCE.md`](CLI-REFERENCE.md).*

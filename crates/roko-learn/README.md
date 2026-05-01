# roko-learn

Learning subsystems -- episodic memory, pattern discovery, model routing, and feedback loops.

## What it does

Consumes the signal stream produced by the orchestrator and agents, persists durable records
of what worked, and surfaces reusable knowledge back to the composer and router. Includes
bandit-based model selection, cost-quality Pareto optimization, anomaly detection, and
adaptive routing with budget enforcement.

## Key types and modules

- `episode_logger` -- append-only JSONL record of agent turns (`.roko/episodes.jsonl`)
- `playbook` / `playbook_rules` -- reusable patterns extracted from episodes
- `skill_library` -- structured skills agents can invoke
- `cascade_router` -- cost/quality model cascade with tier routing
- `model_router` / `routing_extras` -- lookahead and calibration around cascade decisions
- `bandits` / `bandit_research` -- Thompson sampling for model/prompt selection
- `prompt_experiment` / `model_experiment` -- A/B experiment infrastructure
- `pattern_discovery` -- mining episodes for recurring shapes
- `provider_health` -- per-provider circuit breaker for LLM routing
- `latency` -- rolling latency EMAs and percentiles for routing feedback
- `anomaly` -- runaway loop, cost spike, and quality degradation detection
- `pareto` -- cost-quality Pareto frontier computation
- `budget` -- budget tracking and enforcement guardrails
- `context_pack_cache` -- cached prompts keyed by task fingerprint
- `efficiency` / `aggregate` -- per-turn efficiency JSONL telemetry
- `forensic_replay` -- debugging failed tasks (GATE-07)
- `causal` -- Granger causality and PC algorithm for causal DAG discovery (TA-08)
- `bayesian_confidence` -- Beta-Binomial confidence updating (AS-07)
- `adas` -- autocatalytic optimization (LEARN-08)
- `conductor` -- learned intervention policy for conductor retries/aborts

## Usage

```rust
use roko_learn::episode_logger::EpisodeLogger;
use roko_learn::cascade_router::CascadeRouter;

let logger = EpisodeLogger::open(".roko/episodes.jsonl")?;
logger.log_turn(&turn)?;

let router = CascadeRouter::load(".roko/learn/cascade-router.json")?;
let model = router.select(&task_context)?;
```

## Architecture

Sits between the orchestrator (which emits events) and the composer/router (which consumes
learned knowledge). The `event_subscriber` module fans runtime events into the appropriate
learning subsystems. Persisted state lives under `.roko/learn/`. The cascade router and
prompt experiments feed directly back into the agent dispatcher's model selection.

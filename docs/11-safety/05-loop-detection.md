# Loop Detection and Secret Zeroization

> **Layer**: L1 Framework (loop guard), L3 Harness (circuit breaker), Cross-cut (Safety & Provenance)
>
> **Crate**: `roko-agent` (safety/rate_limit.rs), `roko-conductor` (circuit breaker), `roko-agent` (safety/scrub.rs)
>
> **Synapse traits**: `Policy` (observe Engram streams, detect loops), `Gate` (verify termination conditions)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md)

---

## Loop Detection

### The Problem

An autonomous agent can enter infinite or near-infinite loops through several mechanisms:

1. **Tool call loops**: The agent calls a tool, the result triggers another tool call, which triggers another, ad infinitum. Example: a bash command fails, the agent retries with a slightly modified command, which also fails, triggering another retry.

2. **Reasoning loops**: The agent enters a circular reasoning pattern where it considers option A, rejects it for option B, rejects B for C, then rejects C for A. The LLM sees the same context repeatedly and produces the same cycling pattern.

3. **Escalation loops**: The agent encounters a permission error, attempts to work around it, encounters another error, and spirals into increasingly creative (and dangerous) workaround attempts.

4. **Resource exhaustion**: A loop that individually seems harmless but cumulatively exhausts token budget, API rate limits, or disk space.

### Rate Limiting as Loop Defense

The primary loop defense in the current Roko codebase is the sliding-window rate limiter in `roko-agent/src/safety/rate_limit.rs`. This limits the number of tool calls per (role, tool) pair within a configurable time window.

**Default configuration**: 60 calls per 60-second window.

The rate limiter detects loops indirectly: a legitimate agent making rapid tool calls in sequence (e.g., reading 50 files in parallel) approaches the limit but rarely exceeds it. An agent stuck in a retry loop, however, will rapidly exhaust the budget for that tool.

```rust
pub struct RateLimiter {
    policy: RateLimitPolicy,
    state: Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>,
}

pub struct RateLimitPolicy {
    pub max_calls_per_window: usize,    // Default: 60
    pub window_duration: Duration,       // Default: 60s
}
```

The implementation is thread-safe (`Send + Sync`) with a single-lock critical section. The `check_and_record()` method atomically checks the cap and records the timestamp under one mutex acquisition, preventing TOCTOU races.

### Circuit Breaker Pattern

The `roko-conductor` crate implements a circuit breaker pattern that monitors agent health and intervenes when anomalies are detected:

**States**:
- **Closed** (normal operation): All tool calls proceed normally
- **Half-open** (testing recovery): Limited tool calls allowed to test if the issue has resolved
- **Open** (circuit broken): Tool calls are rejected; the agent is paused

**Triggers for opening the circuit**:
- Gate failure rate exceeds threshold (configurable, default 50% over last 10 tasks)
- Error rate on tool calls exceeds threshold (configurable, default 30% over last 20 calls)
- Agent has been running for longer than the maximum session duration (configurable)
- Drawdown metric exceeds safety threshold (for chain-domain agents)

The circuit breaker operates at L3 Harness, wrapping the agent execution loop in `orchestrate.rs`. When the circuit opens, the orchestrator pauses the current task and records the intervention as an Engram with `Kind::InterventionReceived`.

### Conductor Diagnosis Engine

The `DiagnosisEngine` in `roko-conductor` performs root cause analysis when the circuit breaker triggers:

1. **Tail signal analysis**: Examines the last N signals (configurable, default 200) for patterns
2. **Ghost turn detection**: Identifies turns where the agent produced no meaningful output (empty responses, repeated tool call failures)
3. **Efficiency degradation**: Compares recent efficiency metrics against historical baselines
4. **Phase stuck detection**: Identifies when the agent has been in the same execution phase for too long

The diagnosis produces a `ConductorDecision`:
- `Continue`: Normal operation, no intervention needed
- `Pause`: Pause execution, wait for human review
- `Retry`: Retry the current task with a fresh agent instance
- `Skip`: Mark the current task as failed and move to the next
- `Abort`: Abort the entire plan execution

---

## Secret Zeroization

### The Problem

Sensitive data (API keys, private keys, credentials) that enters an agent's memory space can persist even after the variable holding it is dropped. In languages with garbage collection, the data sits in memory until the GC reclaims it — potentially minutes or hours. Even in Rust, where memory is freed deterministically at scope exit, the freed memory may contain the secret until the OS reallocates the page.

This matters for agents because:
- Core dumps capture the entire memory space
- Swap files may persist data to disk
- In shared hosting environments, memory pages may be accessible to other processes (in rare misconfigurations)
- In TEE environments, the TEE protects against external access but not against the agent itself logging secrets

### Zeroize-on-Drop

The solution is zeroize-on-drop: when a sensitive value goes out of scope, its memory is overwritten with zeros before being freed. The `zeroize` crate provides this for Rust:

```rust
use zeroize::Zeroizing;

// TaintedString (design target) uses Zeroizing<String>:
pub struct TaintedString {
    value: zeroize::Zeroizing<String>,
    labels: std::collections::HashSet<TaintLabel>,
}
```

When a `Zeroizing<String>` is dropped, the `Drop` implementation overwrites every byte of the string's buffer with zeros. The compiler is prevented from optimizing this away by using volatile writes (the `zeroize` crate handles this).

### Current Implementation: ScrubPolicy

The current Roko implementation uses regex-based secret scrubbing (see [00-defense-in-depth.md](00-defense-in-depth.md) §Guard 6: Secret Scrubbing) rather than zeroize-on-drop. The scrubber runs on tool output before it enters the LLM context:

```rust
pub fn scrub_secrets(content: &str, policy: &ScrubPolicy) -> String {
    let mut result = content.to_string();
    if !policy.disable_defaults {
        for pattern in default_patterns() {
            result = apply_pattern(&result, pattern);
        }
    }
    for raw in &policy.extra_patterns {
        let Ok(re) = Regex::new(raw) else { continue; };
        let extra = Pattern { re, replace_group: None };
        result = apply_pattern(&result, &extra);
    }
    result
}
```

The scrubber is pure — it allocates a new `String` and never mutates shared state. The original content string remains in memory until the caller drops it, but the scrubbed version is what enters the LLM context.

**Limitation**: The scrubber does not provide zeroize-on-drop for the original content. If a tool result contains a private key, the key exists in memory (in the original `ToolResult::Ok { content }` string) until that result is dropped. The `Zeroizing<String>` wrapper in `TaintedString` (Tier 2) addresses this.

---

## Ghost Turn Detection

A ghost turn is an agent turn that produces no meaningful output — an empty response, a repeated tool call failure, or a response that is functionally identical to the previous turn. Ghost turns indicate the agent is stuck in a loop or has lost coherence.

The conductor's ghost turn detection works by comparing consecutive turns:

1. **Empty turn**: Agent response has zero content or only whitespace
2. **Repeat turn**: Agent response is identical (or near-identical via edit distance) to the previous turn
3. **Failure-only turn**: Agent made tool calls that all failed with the same error class
4. **No-progress turn**: Agent's efficiency metric (tokens produced per token consumed) drops below a minimum threshold

Ghost turns are tracked per task. When ghost turns exceed a configurable threshold (default: 3 consecutive ghost turns), the conductor triggers a circuit breaker transition from Closed to Half-Open, and from Half-Open to Open if recovery fails.

### Liveness Properties

Ghost turn detection enforces the liveness property from temporal logic (see [11-temporal-logic.md](11-temporal-logic.md)):

```
G(task_started → F(task_completed ∨ task_failed))
    "Every started task eventually completes or fails"
```

Without ghost turn detection, an agent stuck in a loop violates this property — the task is started but never completes. The conductor's intervention ensures liveness by forcing a Skip or Abort decision when the agent cannot make progress.

### Integration with Efficiency Events

The efficiency event stream (`.roko/learn/efficiency.jsonl`) provides the raw data for ghost turn detection. Each agent turn produces an efficiency event:

```json
{"task_id":"T-42","turn":7,"tokens_in":1250,"tokens_out":340,"tool_calls":2,"tool_failures":0,"duration_ms":8200}
```

The DiagnosisEngine aggregates these events to detect efficiency degradation over time, complementing the per-turn ghost detection with trend-based anomaly detection.

---

## Adaptive Gate Thresholds

The adaptive gate threshold system in `roko-learn` provides a feedback loop that tightens safety requirements when failure rates increase:

- Gate pass rates are tracked per rung using an exponential moving average (EMA)
- When the EMA drops below a configurable threshold, the gate tightens (requires higher confidence for passage)
- When the EMA rises above the threshold, the gate loosens (allows lower confidence)
- Thresholds are persisted to `.roko/learn/gate-thresholds.json` and loaded on restart

This creates a self-regulating safety system: an agent that repeatedly fails gates gets harder gates, not easier ones. An agent that consistently passes gates earns slightly looser thresholds, enabling faster iteration.

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Nygard (2018) | Release It! — circuit breaker pattern for distributed systems |
| Vaucher et al. (2018) | Zeroization patterns for sensitive data in memory |
| Barthe et al. (2014) | Verified security of cryptographic implementations (constant-time, no-leakage) |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — SafetyLayer composition including rate limiter and scrubber
- [09-adaptive-risk.md](09-adaptive-risk.md) — Bayesian adaptive guardrails
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — Integration gap between safety and CLI

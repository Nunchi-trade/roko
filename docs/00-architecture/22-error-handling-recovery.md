# Error handling and recovery patterns

> Cross-cutting -- All Layers
> Status: **Specification** -- recovery patterns for every subsystem
> Canonical source: various crates (see per-subsystem references)

---

## Purpose

Every subsystem in Roko can fail. This document catalogs the failure modes, specifies the recovery strategy for each, and defines the error propagation rules that determine whether a failure is retried, escalated, or absorbed.

---

## 1. Error classification

All errors in Roko fall into four categories:

| Category | Retry? | Escalate? | Examples |
|---|---|---|---|
| **Transient** | Yes (with backoff) | After N retries | Network timeout, rate limit, temporary API error |
| **Deterministic** | No (same input = same failure) | Yes | Compile error, invalid config, schema mismatch |
| **Resource** | Yes (after resource freed) | After timeout | Disk full, memory pressure, too many open files |
| **Catastrophic** | No | Immediate | Data corruption, missing critical files, auth revocation |

```rust
/// Error classification used by the recovery engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Transient,
    Deterministic,
    Resource,
    Catastrophic,
}

impl ErrorClass {
    pub fn should_retry(&self) -> bool {
        matches!(self, Self::Transient | Self::Resource)
    }

    pub fn should_escalate(&self) -> bool {
        matches!(self, Self::Deterministic | Self::Catastrophic)
    }
}
```

---

## 2. Retry policy

### 2.1 Exponential backoff with jitter

Transient errors use truncated exponential backoff with jitter:

```
delay = min(base_ms * 2^attempt + random(0..jitter_ms), max_delay_ms)
```

| Parameter | Default | Range |
|---|---|---|
| `base_ms` | 500 | 100 - 5,000 |
| `max_delay_ms` | 30,000 | 5,000 - 120,000 |
| `max_retries` | 3 | 0 - 10 |
| `jitter_ms` | 200 | 0 - 1,000 |

```rust
pub struct RetryPolicy {
    pub base_ms: u64,
    pub max_delay_ms: u64,
    pub max_retries: u32,
    pub jitter_ms: u64,
}

impl RetryPolicy {
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let exp_delay = self.base_ms.saturating_mul(2u64.saturating_pow(attempt));
        let jitter = rand::thread_rng().gen_range(0..=self.jitter_ms);
        let total = exp_delay.saturating_add(jitter).min(self.max_delay_ms);
        Duration::from_millis(total)
    }
}
```

### 2.2 Rate limit handling

When a provider returns HTTP 429 (rate limit), the retry uses the `Retry-After` header if present, otherwise falls back to exponential backoff.

```
fn handle_rate_limit(response: &Response, policy: &RetryPolicy, attempt: u32) -> Duration {
    if let Some(retry_after) = response.header("Retry-After") {
        Duration::from_secs(retry_after.parse().unwrap_or(60))
    } else {
        policy.delay_for(attempt)
    }
}
```

---

## 3. Per-subsystem failure modes

### 3.1 Agent dispatch (roko-agent)

| Failure mode | Class | Recovery |
|---|---|---|
| LLM API timeout | Transient | Retry with backoff; after 3 failures, route to different provider |
| LLM API 429 (rate limit) | Transient | Use Retry-After header; circuit breaker if persistent |
| LLM API 500 (server error) | Transient | Retry with backoff; circuit breaker after threshold |
| LLM API 401 (auth) | Catastrophic | Halt task; notify user; do not retry |
| Agent process crash | Transient | Restart agent; replay last turn from episode log |
| Agent exceeds max_turns | Deterministic | Stop agent; mark task as failed; escalate to replanning |
| MCP server unavailable | Transient | Retry connection; fall back to non-MCP tools |
| Tool execution timeout | Transient | Kill tool process; retry once; skip tool on second failure |

Circuit breaker thresholds: `conductor.circuit_breaker_threshold` (default 5) failures within `circuit_breaker_reset_secs` (default 300s) opens the circuit. Half-open after reset period; one success closes it.

### 3.2 Gate pipeline (roko-gate)

| Failure mode | Class | Recovery |
|---|---|---|
| Compile failure | Deterministic | Feed error to agent; retry task (new agent turn) |
| Test failure | Deterministic | Feed test output to agent; retry task |
| Clippy failure | Deterministic | Feed warnings to agent; retry task |
| Gate process timeout | Transient | Kill process; retry gate once; skip gate on second timeout |
| Gate binary not found | Catastrophic | Halt task; log error; skip gate for this run |
| Diff gate: no changes | Deterministic | Pass (no-op task); log warning |

Gate retries are bounded by `gates.max_iterations` (default 5). After max iterations, the task is marked as failed.

### 3.3 Orchestration (roko-orchestrator)

| Failure mode | Class | Recovery |
|---|---|---|
| Plan file parse error | Deterministic | Reject plan; show parse error to user |
| Task dependency cycle | Deterministic | Reject plan; show cycle to user |
| Executor state corruption | Resource | Delete state file; restart from plan (lose progress) |
| Session budget exceeded | Resource | Block new tasks; complete running tasks; save state |
| Agent spawn failure | Transient | Retry spawn; after 3 failures, skip task |
| State file write failure | Resource | Retry write; buffer in memory; warn user |

State persistence uses atomic writes (write to temp file, then rename) to prevent corruption:

```rust
fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
```

### 3.4 Substrate / file system (roko-fs)

| Failure mode | Class | Recovery |
|---|---|---|
| JSONL write failure (disk full) | Resource | Buffer in memory (up to 1,000 signals); retry on next prune |
| JSONL read failure (corrupt line) | Deterministic | Skip corrupt line; log warning; continue reading |
| File lock contention | Transient | Retry with backoff; timeout after 10s |
| Missing .roko directory | Resource | Create directory; initialize empty files |
| Signal too large (> 1MB) | Deterministic | Reject signal; log error |

JSONL corruption recovery: the reader skips lines that fail JSON deserialization. Each line is independent, so one corrupt line does not invalidate the file. The skipped count is logged.

### 3.5 Learning subsystem (roko-learn)

| Failure mode | Class | Recovery |
|---|---|---|
| Episode log write failure | Resource | Buffer in memory; retry on next tick |
| Playbook extraction failure | Deterministic | Skip extraction; use existing playbook |
| Bandit state corruption | Deterministic | Reset arm to prior distribution; log warning |
| Experiment store write failure | Resource | Buffer update; retry on next tick |
| Cascade router state corruption | Deterministic | Delete state file; reinitialize from defaults |

Learning subsystem failures are non-blocking. The system continues to function without learning -- it just does not improve. All learning state files can be deleted and regenerated.

### 3.6 Prompt composition (roko-compose)

| Failure mode | Class | Recovery |
|---|---|---|
| Token budget exceeded | Deterministic | Drop lowest-priority sections until within budget |
| Section content missing | Deterministic | Skip section; log warning |
| Template parse error | Catastrophic | Fall back to minimal template (role + task only) |
| Encoding failure | Deterministic | Fall back to raw text (no template formatting) |

Composition failures should never prevent an agent from running. The minimal fallback prompt is just the role description and task description -- enough for the LLM to attempt the task.

### 3.7 Configuration (roko-core/config)

| Failure mode | Class | Recovery |
|---|---|---|
| Missing roko.toml | Resource | Use RokoConfig::default() |
| TOML parse error | Deterministic | Show error with line/column; refuse to start |
| Schema version mismatch | Deterministic | Run migration chain; fail if no path exists |
| Validation warning | -- | Print warning; continue |
| Validation error | Deterministic | Refuse to start; show error |

---

## 4. Error propagation rules

### 4.1 Propagation hierarchy

```
Agent error
    |
    v
Gate pipeline catches -> retry or fail task
    |
    v
Orchestrator catches -> retry task or fail plan
    |
    v
CLI catches -> show error to user, save state
```

Each layer catches errors from the layer below and decides: retry, escalate, or absorb.

### 4.2 Absorption rules

Some errors are absorbed (logged but not propagated):

| Error | Absorbed by | Rationale |
|---|---|---|
| Episode log write failure | Learning subsystem | Learning is optional; task can succeed without logging |
| Skill extraction failure | Learning subsystem | Skills improve future tasks; current task is unaffected |
| Dashboard render failure | TUI | Display errors should not affect task execution |
| Metric emission failure | Conductor | Metrics are observability; core function is unaffected |

### 4.3 Escalation rules

Some errors escalate immediately:

| Error | Escalated to | Rationale |
|---|---|---|
| Auth failure (401) | User | Cannot be fixed by retry |
| Budget exceeded | Orchestrator | Policy decision, not a technical failure |
| State corruption | User | Risk of data loss requires human decision |
| Config parse error | User | Cannot start without valid config |

---

## 5. Circuit breaker pattern

The circuit breaker prevents cascading failures when a provider is degraded:

```rust
pub struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    threshold: u32,           // default: 5
    reset_timeout: Duration,  // default: 300s
    last_failure: Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // All requests rejected
    HalfOpen, // One test request allowed
}

impl CircuitBreaker {
    pub fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                if self.last_failure.elapsed() >= self.reset_timeout {
                    self.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true, // allow one test request
        }
    }

    pub fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitState::Closed;
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Instant::now();
        if self.failure_count >= self.threshold {
            self.state = CircuitState::Open;
        }
    }
}
```

---

## 6. Graceful degradation ladder

When resources are constrained, the system degrades through a defined sequence:

```
Level 0: Normal operation
    All features active. Full model selection. All learning loops.

Level 1: Budget pressure (warn_threshold reached)
    Route to cheaper models. Disable experiment exploration. Log warning.

Level 2: Budget critical (block_threshold reached)
    Block new tasks. Complete running tasks. Save state.

Level 3: Provider degraded (circuit breaker open)
    Route to alternative providers. Fall back to local models if available.

Level 4: All providers degraded
    Queue tasks. Retry periodically. Notify user.

Level 5: Disk pressure
    Reduce logging verbosity. Prune aggressively. Warn user.

Level 6: Unrecoverable
    Save state. Print diagnostic. Exit with non-zero code.
```

---

## 7. State recovery after crash

If the process crashes, the next run recovers via:

1. **Executor state**: load from `.roko/state/executor.json` (atomic-write protected).
2. **Episode log**: read from `.roko/episodes.jsonl` (append-only, skip corrupt lines).
3. **Signal log**: read from `.roko/signals.jsonl` (append-only, skip corrupt lines).
4. **Learning state**: load from `.roko/learn/*.json` files. Missing files initialize to defaults.
5. **Agent state**: not recoverable. Running agents are lost on crash. The orchestrator re-dispatches tasks that were in-progress.

The `--resume` flag to `roko plan run` loads the executor state and skips completed tasks:

```bash
roko plan run plans/ --resume .roko/state/executor.json
```

---

## 8. Configuration parameters

| Parameter | Default | Range | Description |
|---|---|---|---|
| `retry_base_ms` | 500 | 100 - 5,000 | Base delay for exponential backoff |
| `retry_max_delay_ms` | 30,000 | 5,000 - 120,000 | Maximum retry delay |
| `retry_max_retries` | 3 | 0 - 10 | Maximum retry attempts for transient errors |
| `retry_jitter_ms` | 200 | 0 - 1,000 | Random jitter added to retry delay |
| `circuit_breaker_threshold` | 5 | 1 - 50 | Failures before circuit opens |
| `circuit_breaker_reset_secs` | 300 | 30 - 3,600 | Seconds before half-open |
| `state_write_timeout_ms` | 5,000 | 1,000 - 30,000 | Timeout for state file writes |
| `jsonl_max_corrupt_lines` | 100 | 1 - 10,000 | Max corrupt lines before rejecting file |
| `signal_max_size_bytes` | 1,048,576 | 1,024 - 10,485,760 | Max size of a single signal |
| `memory_buffer_max` | 1,000 | 100 - 100,000 | Max signals buffered in memory during I/O failure |

---

## 9. Test criteria

1. Transient error retries with exponential backoff (verify delay sequence: 500ms, 1s, 2s).
2. Circuit breaker opens after `threshold` failures and rejects subsequent requests.
3. Circuit breaker transitions to half-open after `reset_timeout`.
4. Successful request in half-open state closes the circuit.
5. Atomic write prevents state corruption (kill process mid-write, verify temp file exists but main file is untouched).
6. JSONL reader skips corrupt lines and reports count.
7. Budget exceeded blocks new tasks but does not kill running agents.
8. Auth failure (401) escalates immediately without retry.
9. Missing .roko directory is created on first access.
10. Full crash recovery: kill process, restart with `--resume`, verify completed tasks are skipped.
11. Degradation ladder: simulate budget pressure and verify model tier downgrade.

---

## Cross-references

- [../07-conductor/](../07-conductor/INDEX.md) -- Circuit breaker and health monitoring
- [../05-learning/09-provider-health-circuit-breaker.md](../05-learning/09-provider-health-circuit-breaker.md) -- Provider health registry
- [../05-learning/14-stability-mechanisms.md](../05-learning/14-stability-mechanisms.md) -- Feedback loop stability
- [20-configuration-schema.md](20-configuration-schema.md) -- Config parameters referenced here
- `crates/roko-core/src/config/schema.rs` -- ConductorConfig with circuit breaker settings
- `crates/roko-agent/src/dispatcher/mod.rs` -- Agent dispatch with retry logic
- `crates/roko-cli/src/orchestrate.rs` -- Orchestrator error handling

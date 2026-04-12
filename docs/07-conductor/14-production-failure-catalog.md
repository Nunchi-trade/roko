# Production Failure Catalog

> Every issue cataloged here was hit in production during batch runs
> between March and April 2026. Each entry traces the full chain:
> symptom, root cause, and the Conductor response that detects or
> prevents recurrence.

---

## Summary

21 production failures across 6 categories. Each failure maps to one
or more Conductor mechanisms (watchers, circuit breaker, diagnosis
engine, anomaly detector, or process supervisor) that either already
detects it or would detect it once fully wired.

### Category Overview

| Category | Issues | Conductor Coverage |
|----------|--------|-------------------|
| State Corruption | #1-4 | Event-sourced state, circuit breaker |
| Data Pipeline | #5, #13-15 | Diagnosis engine, typed pipelines |
| Process Management | #6-9 | ProcessSupervisor, ghost-turn watcher |
| Resource Management | #10-12 | Cost watcher, context pressure watcher, anomaly detector |
| Merge & Coordination | #16-18 | Review loop watcher, spec drift watcher |
| Observability | #19-21 | Efficiency events, structured signals |

---

## Category: State Corruption

These failures share a pattern: mutable state updated non-atomically
by multiple callers, producing states that violate invariants.

### Issue #1: in_flight/completed Overlap

**Symptom**: Tasks appeared in both `in_flight` and `completed_tasks`
sets simultaneously. Downstream logic that assumed mutual exclusivity
would double-count work or skip gating.

**Root Cause**: `snapshot()` returned state with a task in both sets.
The transition from in_flight to completed was not atomic — the task
was added to completed before being removed from in_flight, and a
concurrent snapshot captured the intermediate state.

**Conductor Response**:
- **Circuit breaker**: The corrupted state could cause a task to be
  gated twice or not at all. If the double-gate fails (because the
  task is already completed), the failure increments the plan's
  failure count. At MAX_PLAN_FAILURES=2, the circuit breaker opens.
- **Structural prevention**: Event-sourced state. A `TaskCompleted`
  event is a single atomic fact. The projection derives in_flight
  and completed as disjoint sets. Partial state is structurally
  impossible.

**Design Principles Violated**: #1 (Single source of truth), #2
(Event-sourced state).

---

### Issue #2: Orphaned Plans

**Symptom**: Tasks existed in task state but had no corresponding
`plan_phase` entry. The orchestrator tried to advance these tasks but
had no phase context, causing panics or silent drops.

**Root Cause**: Plan creation involved multiple writes — inserting
tasks, then inserting the plan_phase. A crash between writes left
tasks belonging to no plan.

**Conductor Response**:
- **Iteration loop watcher**: Orphaned tasks that the orchestrator
  attempts to advance will fail repeatedly. The iteration loop
  watcher detects this after MAX_ITERATIONS=3 and fires a Critical
  severity signal, which the intervention policy maps to Fail.
- **Structural prevention**: Event-sourced state. A `PlanCreated`
  event contains both plan metadata and initial task set. The
  projection builds both atomically from one event.

**Design Principles Violated**: #1 (Single source of truth), #2
(Event-sourced state), #5 (Fail loud).

---

### Issue #3: Branch Divergence

**Symptom**: Worktree branches diverged from the batch branch.
Merging back produced conflicts or quarantine loops — the plan would
fail, be retried, fail again with the same conflict, and loop.

**Root Cause**: Plan branches were long-lived. The batch branch
advanced as other plans merged. The longer a plan ran, the further
its branch diverged. Rebasing frequently failed (see Issue #16).

**Conductor Response**:
- **Review loop watcher**: Repeated merge-conflict failures that
  result in review rejections trigger the watcher at
  MAX_REVIEW_CYCLES=3.
- **Circuit breaker**: Repeated plan failures from merge conflicts
  open the breaker at MAX_PLAN_FAILURES=2.
- **Structural prevention**: Ephemeral branch model. Branches are
  created from current HEAD immediately before work, merged or
  discarded after gate pass. No branch lives long enough to diverge.

**Design Principles Violated**: #3 (Ephemeral everything), #10
(Monotonic progress).

---

### Issue #4: CONTEXT.md Concurrent Appends

**Symptom**: Multiple agents writing to shared CONTEXT.md
simultaneously. Content was interleaved, truncated, or lost. Agents
reading the file saw corrupted context, degrading output quality.

**Root Cause**: CONTEXT.md was a plain file in the shared worktree.
No locking, no coordination. File system does not guarantee atomic
appends for concurrent writers.

**Conductor Response**:
- **Spec drift watcher**: Corrupted context causes agent output to
  drift from the task specification. When cosine similarity drops
  below MAX_DRIFT=0.25, the watcher fires.
- **Quality degradation (anomaly detector)**: Context corruption
  degrades quality scores. When recent average drops >0.15 below
  earlier average AND recent average is below 0.5, the anomaly
  detector fires.
- **Structural prevention**: Event-sourced context. Each agent
  receives context via `context/in/` (read-only). Outputs go to
  `context/out/`. The shared mutable file is eliminated.

**Design Principles Violated**: #1 (Single source of truth), #7
(Process isolation), #9 (Immutable artifacts).

---

## Category: Data Pipeline

These failures share a root cause: LLM-generated artifacts consumed
without validation.

### Issue #5: Counter Bug (TOML Fences)

**Symptom**: `task_weighted_progress` reported ~2.5% when actual
completion was much higher. ETA stuck at 8 hours. 388 of 544 task
files affected.

**Root Cause**: Enrichment LLM (Haiku) wrapped TOML in markdown code
fences. TOML parser returned `Err`, which was silently swallowed.
Empty checklists produced wrong progress fractions.

**Conductor Response**:
- **Diagnosis engine**: The 34-pattern engine includes `TomlParsing`
  as an error category. When TOML parse failures are detected in gate
  output, the engine suggests `RetryWithFix` intervention.
- **Structural prevention**: Schema validation at generation time.
  Parse generated TOML immediately, reject/retry on failure.

**Design Principles Violated**: #4 (Typed pipelines), #5 (Fail loud).

---

### Issue #13: Enrichment TOML Fences

**Symptom**: Identical to Issue #5 from the pipeline perspective.
388/544 enrichment-generated TOML files wrapped in markdown fences.

**Root Cause**: LLMs trained on chat data wrap structured output in
code fences even when told not to. The pipeline piped output to file
without validation.

**Conductor Response**: Same as Issue #5 — diagnosis engine detects
the pattern, schema validation prevents recurrence.

**Design Principles Violated**: #4 (Typed pipelines).

---

### Issue #14: Verify Script Stale References

**Symptom**: Verify scripts referenced packages, modules, functions
that did not exist. Scripts failed with "not found" during gate
phase, failing correct implementations.

**Root Cause**: Enrichment LLM hallucinated plausible package names
and function signatures. Scripts were not validated against the
codebase at generation time.

**Conductor Response**:
- **Compile-fail-repeat watcher**: Stale references produce compile
  errors. After MAX_COMPILE_FAILS=3 identical failures, the watcher
  fires with Warning severity → restart.
- **Diagnosis engine**: Matches `E0432` (unresolved import) and
  `E0433` (unresolved path) patterns, suggesting `ImportNotFound`
  category and `RetryWithFix` intervention.
- **Structural prevention**: Dry-run validation at enrichment time.
  Verify all referenced symbols exist before accepting the script.

**Design Principles Violated**: #4 (Typed pipelines), #11 (Anticipate).

---

### Issue #15: Review Verdict Parsing

**Symptom**: Review verdicts parsed incorrectly. Plans that should
have passed were failed, and vice versa. Review cycle looped
unnecessarily.

**Root Cause**: Reviewers output TOML in markdown. The regex
fallback parser was fragile and confused by similar patterns in
commentary.

**Conductor Response**:
- **Review loop watcher**: Incorrect parsing causes repeated review
  rejections for plans that should have passed. At MAX_REVIEW_CYCLES=3,
  the watcher fires.
- **Structural prevention**: Typed review pipeline. `ReviewReport`
  struct with schema-validated JSON. Parsing is deserialization, not
  regex.

**Design Principles Violated**: #4 (Typed pipelines), #9 (Immutable
artifacts).

---

## Category: Process Management

These failures stem from treating agent processes as fire-and-forget.

### Issue #6: Spawn Races

**Symptom**: Agents exited with near-zero output. Retry fired
instantly. Exit event from attempt N confused with attempt N+1 —
killing the new attempt or double-counting the failure.

**Root Cause**: No attempt tracking. Exit events did not identify
which spawn attempt they belonged to. Retries fired without backoff.

**Conductor Response**:
- **Ghost-turn watcher**: Near-zero output agents are detected at
  MAX_GHOST_TURNS=3. Ghost detection catches the symptom even if
  the spawn race itself is not detected.
- **ProcessSupervisor**: Monotonically increasing attempt IDs.
  Exit events carry attempt IDs. Stale events are ignored
  structurally.

**Design Principles Violated**: #7 (Process isolation), #5 (Fail
loud).

---

### Issue #7: Orphaned Cargo Processes

**Symptom**: Timeout killed the shell script but not the cargo
process tree. Orphaned cargo processes accumulated, starving CPU
and memory.

**Root Cause**: `kill(pid)` does not kill descendants unless they
are in the same process group.

**Conductor Response**:
- **Cost overrun watcher**: Orphaned processes burning CPU
  indirectly increase turn costs. The cost watcher fires at
  $10 limit.
- **Context pressure watcher**: Resource starvation from orphans
  degrades system performance, indirectly increasing context
  pressure.
- **ProcessSupervisor**: `kill_all_descendants(pid)` with
  bottom-up kill ordering. Process group management via `setsid`.
  Periodic orphan reaper sweep.

**Design Principles Violated**: #7 (Process isolation).

---

### Issue #8: Claude CLI Cold Start

**Symptom**: Every agent turn took 2-5s startup overhead. Over
hundreds of turns, 10-30 minutes of pure waste.

**Root Cause**: CLI spawns a new subprocess per turn. No persistent
connection or subprocess reuse.

**Conductor Response**:
- **Time overrun watcher**: Cumulative cold start overhead
  contributes to phase time exceeding the 80% threshold.
- **Efficiency events**: Per-turn `time_to_first_token_ms` and
  `wall_time_ms` fields capture cold start overhead for analysis.
- **Structural prevention**: Agent connection pooling. Warm
  connections amortize startup cost.

**Design Principles Violated**: #8 (Measure everything).

---

### Issue #9: Agent Ghost Turns

**Symptom**: Agent appeared active but produced no useful output —
repeating itself, asking clarifying questions to nobody, or
describing what it would do without doing it. Burned significant
token budget.

**Root Cause**: LLM agents enter degenerate loops when context is
confusing, instructions ambiguous, or errors unhandled.

**Conductor Response**:
- **Ghost-turn watcher**: Primary detection. At MAX_GHOST_TURNS=3,
  fires Warning severity → restart with fresh context.
- **Stuck-pattern watcher**: Detects repetitive output patterns at
  MAX_STUCK_PATTERNS=4.
- **Anomaly detector (prompt loop)**: If the same prompt hash
  appears 5 times in a 20-prompt window, the session is aborted.
- **Cost spike (anomaly detector)**: Ghost turns consuming expensive
  API calls trigger z-score > 3.0 detection.

**Design Principles Violated**: #11 (Anticipate), #8 (Measure
everything).

---

## Category: Resource Management

These failures arise from treating resources as unlimited.

### Issue #10: Disk Pressure

**Symptom**: Build failures with cryptic errors. Only 7.3 GB free
on a 1.8 TB drive. Cargo target directories and worktree copies had
accumulated.

**Root Cause**: No proactive disk monitoring. Multiple worktrees
each with their own target directory. GC only ran when explicitly
triggered.

**Conductor Response**:
- **Health monitor**: `SystemSnapshot` can be extended with disk
  pressure checks.
- **Anomaly detector (budget exhaustion)**: Budget tracking catches
  cost-related resource exhaustion; disk exhaustion requires an
  analogous disk budget.
- **Structural prevention**: DiskBudget. Estimate disk footprint
  before starting a plan. Refuse if budget exceeds available space.

**Design Principles Violated**: #6 (Resource budgets), #11
(Anticipate).

---

### Issue #11: Gate Serialization Bottleneck

**Symptom**: Plans completed implementation quickly but waited in
queue for gate verification. Serialized gate processing, one at a
time.

**Root Cause**: Double semaphore (`cargo_gate` + `verify_chain`,
both with 1 permit) serialized all compilation. After worktree
isolation, separate target directories made serialization
unnecessary.

**Conductor Response**:
- **Time overrun watcher**: Gate queue waiting contributes to phase
  time exceeding the 80% threshold, making the bottleneck visible.
- **Efficiency events**: `wall_time_ms` vs `duration_ms` gap
  reveals queue wait time in event data.
- **Structural prevention**: Build dependency graph for scheduling.
  Parallelize gates with independent build graphs.

**Design Principles Violated**: #6 (Resource budgets), #8 (Measure
everything).

---

### Issue #12: Memory Pressure from Large Prompts

**Symptom**: Agent output quality degraded as prompt size increased.
Agents ignored relevant context buried in large prompts or fixated
on irrelevant context. Token costs increased proportionally.

**Root Cause**: "Include everything" prompt strategy. Prompts
exceeding 100K tokens. LLM attention is not uniform — middle content
gets less attention.

**Conductor Response**:
- **Context window pressure watcher**: Fires at 80% of model's
  context window. This is the primary defense against oversized
  prompts.
- **Spec drift watcher**: Large prompts cause agents to drift from
  spec. Drift detection catches the quality degradation symptom.
- **Quality degradation (anomaly detector)**: Quality scores drop
  when prompts are too large, triggering degradation detection.
- **Structural prevention**: Adaptive context dropping. Score each
  section by relevance, include in priority order until budget
  reached.

**Design Principles Violated**: #8 (Measure everything), #11
(Anticipate).

---

## Category: Merge & Coordination

These failures arise from concurrent plans interacting through
shared branches and files.

### Issue #16: Rebase Failures

**Symptom**: "batch rebase failed" permanently killed plans. Work
was lost with no recovery.

**Root Cause**: Long-lived branches needed rebasing onto advancing
batch branch. Rebase failure was treated as permanent rather than
recoverable.

**Conductor Response**:
- **Iteration loop watcher**: Rebase-fail-retry loops detected at
  MAX_ITERATIONS=3 → Critical severity → Fail.
- **Circuit breaker**: Repeated rebase failures increment plan
  failure count. Breaker opens at MAX_PLAN_FAILURES=2.
- **Structural prevention**: Ephemeral branches. Never rebase.
  Branches are born from current HEAD and merged or discarded.

**Design Principles Violated**: #3 (Ephemeral everything), #10
(Monotonic progress).

---

### Issue #17: Merge Conflicts at Gate

**Symptom**: Two plans that both passed gates individually would
conflict when merged. Second plan fails, retries, fails again —
loop.

**Root Cause**: Plans scheduled without considering file overlap.
Both succeed in isolation but conflict when combined.

**Conductor Response**:
- **Compile-fail-repeat watcher**: Merge conflicts produce compile
  errors when the merged result does not build. Watcher fires at
  MAX_COMPILE_FAILS=3.
- **Review loop watcher**: Merge conflicts that manifest as review
  rejections trigger the watcher.
- **Circuit breaker**: The retry-conflict-retry loop produces
  plan failures that open the breaker.
- **Structural prevention**: Dependency graph with pre-merge
  conflict detection. Dry-run merge before attempting real merge.

**Design Principles Violated**: #11 (Anticipate), #6 (Resource
budgets).

---

### Issue #18: Worktree Symlinks to Shared State

**Symptom**: Race conditions when multiple agents accessed shared
state through symlinks. Changes by one agent affected another's
view.

**Root Cause**: Worktrees had symlinks to shared mutable files
(CONTEXT.md, plan state). Writes from any worktree mutated the same
file.

**Conductor Response**:
- **Spec drift watcher**: Corrupted shared state causes output
  drift from specification.
- **Stuck-pattern watcher**: Agents receiving corrupted context
  may produce repetitive confused output.
- **Structural prevention**: Full worktree isolation. No shared
  mutable state. Orchestrator crosses worktree boundaries through
  explicit collect/inject, never shared file handles.

**Design Principles Violated**: #7 (Process isolation), #1 (Single
source of truth).

---

## Category: Observability

These failures share a theme: insufficient information to diagnose
problems quickly.

### Issue #19: Buried Failures in Logs

**Symptom**: Critical errors hidden in the middle of 50,000-line
log files. TUI showed aggregated status but not individual failure
details. Required manual grep to find failures.

**Root Cause**: Unstructured logging. All events to same stream
with no severity routing or queryability.

**Conductor Response**:
- **Efficiency events**: Structured `AgentEfficiencyEvent` with 20+
  fields replaces unstructured logging for agent performance data.
- **Conductor signals**: Every conductor intervention produces a
  typed `Signal` with severity, watcher name, plan ID, and count.
  These are queryable, not buried in logs.
- **Structural prevention**: EventBus with queryable event stream.
  Query for "all errors in the last hour" without scanning full log.

**Design Principles Violated**: #5 (Fail loud), #8 (Measure
everything).

---

### Issue #20: No Signal on WHY Plans Fail

**Symptom**: TUI showed "Failed" with no root cause. Operator had
to dig through logs, worktree state, and git history.

**Root Cause**: Failure path recorded status change but not reason.
Error messages logged but not attached to plan state.

**Conductor Response**:
- **Diagnosis engine**: Classifies errors into 20 categories with
  suggested interventions. Provides the "why" that was missing.
- **Conductor intervention signals**: Include watcher name, severity,
  plan ID, and descriptive message. Signal content explains why the
  intervention fired.
- **Structural prevention**: Enriched error digests. Every failure
  produces a structured `FailureReport` attached to plan state.

**Design Principles Violated**: #5 (Fail loud), #8 (Measure
everything).

---

### Issue #21: ETA Completely Wrong

**Symptom**: ETA showed 8+ hours when actual remaining was ~2 hours.
Progress bar at ~2.5% when actual completion was ~40%.

**Root Cause**: Directly caused by Issue #5. Weighted progress
depended on checklist counts from broken TOML files. With 388/544
failing to parse, the fraction was wrong.

**Conductor Response**:
- **Anomaly detector**: Internal inconsistency (40% gates passed
  but 2.5% progress shown) is detectable as a quality anomaly.
- **Efficiency events**: `gate_passed` field provides a reliable
  progress signal independent of checklist parsing.
- **Structural prevention**: Progress from gate outcomes, not
  checklist parsing. Gates passed / gates total is a single reliable
  metric.

**Design Principles Violated**: #8 (Measure everything), #5 (Fail
loud), #11 (Anticipate).

---

## Cross-Reference Tables

### Issue → Conductor Mechanism

| # | Issue | Primary Mechanism | Secondary Mechanism |
|---|-------|------------------|-------------------|
| 1 | in_flight/completed overlap | Circuit breaker | Event-sourced state |
| 2 | Orphaned plans | Iteration loop watcher | Event-sourced state |
| 3 | Branch divergence | Review loop watcher | Circuit breaker |
| 4 | CONTEXT.md concurrent appends | Spec drift watcher | Quality anomaly detector |
| 5 | Counter bug (TOML fences) | Diagnosis engine | Schema validation |
| 6 | Spawn races | Ghost-turn watcher | ProcessSupervisor |
| 7 | Orphaned cargo processes | ProcessSupervisor | Cost watcher |
| 8 | Claude CLI cold start | Time overrun watcher | Efficiency events |
| 9 | Agent ghost turns | Ghost-turn watcher | Prompt loop detector |
| 10 | Disk pressure | Health monitor | Budget anomaly |
| 11 | Gate serialization | Time overrun watcher | Efficiency events |
| 12 | Large prompt pressure | Context pressure watcher | Spec drift watcher |
| 13 | Enrichment TOML fences | Diagnosis engine | Schema validation |
| 14 | Verify script stale refs | Compile-fail-repeat watcher | Diagnosis engine |
| 15 | Review verdict parsing | Review loop watcher | Typed review pipeline |
| 16 | Rebase failures | Iteration loop watcher | Circuit breaker |
| 17 | Merge conflicts at gate | Compile-fail-repeat watcher | Circuit breaker |
| 18 | Worktree symlinks | Spec drift watcher | Stuck-pattern watcher |
| 19 | Buried failures | Efficiency events | Conductor signals |
| 20 | No failure signal | Diagnosis engine | Conductor signals |
| 21 | ETA wrong | Anomaly detector | Efficiency events |

### Issue → Design Principle

| Principle | Prevents Issues |
|-----------|----------------|
| #1 Single source of truth | #1, #2, #4, #18 |
| #2 Event-sourced state | #1, #2, #4 |
| #3 Ephemeral everything | #3, #16 |
| #4 Typed pipelines | #5, #13, #14, #15 |
| #5 Fail loud, recover fast | #1, #2, #5, #6, #19, #20, #21 |
| #6 Resource budgets | #10, #11, #17 |
| #7 Process isolation | #4, #6, #7, #9, #18 |
| #8 Measure everything | #8, #9, #11, #12, #19, #20, #21 |
| #9 Immutable artifacts | #4, #15 |
| #10 Monotonic progress | #3, #16 |
| #11 Anticipate, don't react | #9, #10, #12, #14, #17, #21 |

### Issue → Refactoring Phase

| Phase | Issues Addressed |
|-------|-----------------|
| Phase 0 (Instrument) | #19, #21 |
| Phase 1 (Quick Wins) | #5, #10, #12, #13, #14, #20 |
| Phase 2 (Decompose) | #3, #16 |
| Phase 3 (Foundation) | #1, #2, #4, #5, #6, #7, #10, #18, #19 |
| Phase 4 (Core) | #8, #11, #12, #15, #17 |
| Phase 5 (Cybernetic) | #9, #14, #20, #21 |

---

## Cross-References

- [01-watcher-ensemble.md](01-watcher-ensemble.md) — Watcher
  mechanisms referenced throughout this catalog
- [02-circuit-breaker.md](02-circuit-breaker.md) — Circuit breaker
  responses to repeated failures
- [04-diagnosis-engine.md](04-diagnosis-engine.md) — Error
  classification for data pipeline failures
- [06-health-monitors.md](06-health-monitors.md) — System health
  checks for resource failures
- [11-anomaly-detection-learning.md](11-anomaly-detection-learning.md)
  — Anomaly detection for quality and cost failures
- [13-process-supervision-wiring.md](13-process-supervision-wiring.md)
  — Process management failure responses

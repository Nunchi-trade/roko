# Integrations -- Documented Wiring Not Connected

These are cross-system integrations described in doc 24-cross-section-integration-map.md that are not yet wired in the codebase. Each entry references the doc section numbers (From -> To).

## Tier 1 -- High Priority

### INT-01: Failure -> Replanning (M3)
- [x] Wire gate failures to trigger re-planning

**Spec** (doc 24, M3): When a task fails its gate pipeline, the failure should feed back into the plan generator for automatic re-planning.
**Current code**: Gate failures are recorded but do not trigger re-planning. This is CLAUDE.md priority item 11.
**Accept when**:
- [x] A gate failure on a task triggers plan generator with failure context
  - `maybe_emit_gate_failure_plan_revision()` at orchestrate.rs:5075 triggers replanning on gate failure; `attempt_replan()` at line 10266 dispatches with failure summary
- [x] Re-planned task appears in the executor queue
  - `apply_replan_result()` at orchestrate.rs:5249 injects regenerated tasks into live executor state
- [x] End-to-end test: fail a gate, verify re-plan happens
  - `gate_failure_plan_revision_dedupes_and_caps_replans` test at orchestrate.rs:18095; synthetic replan fixture at `try_synthetic_replan_fixture()`
**Priority**: P1

### INT-02: Skills -> Prompts (M4)
- [x] Wire learned skills into prompt composition

**Spec** (doc 24, M4): Learned skills/playbooks should enrich system prompts.
**Current code**: SystemPromptBuilder has `relevant_skills` and `relevant_playbooks` fields (layer 6). Partially wired -- verify the full path from skill storage to prompt assembly.
**Accept when**:
- [x] Skills from `.roko/learn/` are loaded and injected into layer 6
  - `load_or_create_skill_library()` at orchestrate.rs:4330 loads from `.roko/learn/skills.json`; `relevant_skills` passed to `PromptBuildOptions` at line 16516
- [x] Prompts include relevant skill content
  - SystemPromptBuilder layer 6 at system_prompt_builder.rs:568+ renders skills section with budgeting
**Priority**: P1

### INT-03: Neuro -> Composition full (M5)
- [x] Wire full neuro knowledge enrichment into composition

**Spec** (doc 24, M5): Neuro's durable knowledge store should fully enrich prompt composition.
**Current code**: Partially wired. Neuro provides knowledge but not full enrichment path.
**Accept when**:
- [x] Neuro knowledge retrieved during composition
  - `KnowledgeStore::init()` at orchestrate.rs:4337; knowledge queried per-task for context enrichment
- [x] Anti-knowledge (what NOT to do) included via layer 7
  - `query_anti_knowledge_patterns()` at orchestrate.rs:2569 queries `KnowledgeKind::AntiKnowledge`; result fed to `extra_anti_patterns` in prompt build (line 13271+13288)
**Priority**: P1

### INT-04: Cost -> Routing (M6)
- [x] Wire cost tracking into model routing decisions

**Spec** (doc 24, M6): Cost data should influence which model tier is selected.
**Current code**: CascadeRouter exists and persists to `.roko/learn/cascade-router.json`. Verify cost feedback loop is complete.
**Accept when**:
- [x] Cost per model tracked and fed back to CascadeRouter
  - `observe_multi_objective()` at orchestrate.rs:9917 feeds normalized_cost into CascadeRouter per task completion
- [x] Router adjusts tier selection based on cost history
  - CascadeRouter uses cost in multi-objective observation; LinUCB context includes cost signals; conductor `routing_bias.prefer_cheaper` forces cheaper tier selection (orchestrate.rs:12755)
**Priority**: P1

### INT-05: Daimon -> Orchestration (M1)
- [x] Wire daimon behavior primitives into orchestration

**Spec** (doc 24, M1): Daimon provides behavior primitives that orchestration should use.
**Current code**: roko-daimon crate exists but is Phase 2+. Not wired.
**Accept when**:
- [x] Daimon primitives influence task selection or agent behavior
  - `DaimonState` at orchestrate.rs:2979; `should_replan_after_task_failure()` at line 16917 checks daimon behavioral state; `AffectEvent::TaskOutcome` at line 6941 drives daimon appraisal; model selection biased by daimon (pre_daimon_model at line 13021)
**Priority**: P2 (blocked on daimon development)

### INT-06: Daimon -> Composition (M2)
- [x] Wire daimon into prompt composition

**Spec** (doc 24, M2): Daimon behavior context should enrich prompts.
**Current code**: roko-daimon is Phase 2+. Not wired.
**Accept when**:
- [x] Daimon context included in system prompt
  - `build_daimon_context_section()` at orchestrate.rs:13387 builds daimon section from `task_affect_state` + `behavioral_state`; injected as prompt section
**Priority**: P2 (blocked on daimon development)

## Tier 2

### INT-07: Dreams -> Neuro (M7)
- [x] Wire dreams consolidation into neuro knowledge store

**Spec** (doc 24, M7): Offline dream consolidation should feed distilled knowledge into neuro.
**Current code**: roko-dreams is Phase 2+. Not wired.
**Priority**: P2

### INT-08: Code Intel -> Composition (M8)
- [x] Wire code intelligence into prompt composition

**Spec** (doc 24, M8): Code intelligence (from roko-mcp-code) should enrich prompts with codebase context.
**Current code**: roko-mcp-code exists but does not feed into SystemPromptBuilder.
**Accept when**:
- [x] Code intel results injected into domain context layer (layer 3)
  - `code_context_for_task()` at orchestrate.rs:16574 queries `roko_index::WorkspaceIndex`; results passed as `code_context` in `PromptBuildOptions` (line 16518) and injected into prompt composition
**Priority**: P1

### INT-09: Conductor -> Routing direct (M9)
- [x] Wire conductor health into routing decisions

**Spec** (doc 24, M9): Conductor health signals should directly influence routing.
**Current code**: Partially wired via health checks but not direct routing influence.
**Priority**: P2

### INT-10: Experiments -> Static config (M10)
- [x] Wire experiment results back into default config

**Spec** (doc 24, M10): Winning experiment variants should be promoted to static config.
**Current code**: ExperimentStore exists but doesn't write back to config.
**Priority**: P2

### INT-11: Orchestration -> Daimon (M11)
- [x] Wire orchestration events into daimon

**Spec** (doc 24, M11): Orchestration outcomes should inform daimon behavior.
**Current code**: Wired. Task-level events (GateResult, TaskOutcome, Blocked, TimePressure, QueueWait) were already flowing into DaimonState::appraise(). Plan-level completion events now also feed into the daimon via AffectEvent::TaskOutcome and record_somatic_outcome in the run_all completion path.
**Priority**: P2

### INT-12: AntiKnowledge -> Composition (M15)
- [x] Wire anti-knowledge into composition

**Spec** (doc 24, M15): Anti-knowledge (what not to do) should be in prompts.
**Current code**: SystemPromptBuilder has `anti_patterns` (layer 7). Partially wired.
**Accept when**:
- [x] Anti-knowledge from neuro automatically populates layer 7
  - `query_anti_knowledge_patterns()` at orchestrate.rs:2569 queries `KnowledgeStore` for `KnowledgeKind::AntiKnowledge`; results combined with safety constraints and passed as `extra_anti_patterns` to `build_system_prompt_with_context_validated()` at line 13288
**Priority**: P1

## Tier 3-4

### INT-13: Pheromones -> Orchestration (M12)
- [x] Wire pheromone signals into orchestration decisions

**Current code**: Wired. Conductor health monitor results (Degraded/Critical) now deposit Anomaly/Threat pheromones into the pheromone field. These are picked up by `active_pheromone_chunks()` and injected into system prompts via layer 3c, closing the conductor health -> orchestration feedback loop.
**Priority**: P2

### INT-14: Safety -> Composition (M13)
- [x] Wire safety context into prompt composition

**Current code**: Wired. SafetyLayer::constraints_as_anti_patterns() extracts human-readable constraints from the active bash deny-patterns, git protections, network restrictions, governance rules, contract invariants, and path policies. These are injected as anti-patterns (layer 7) alongside neuro anti-knowledge in the validated prompt build path.
**Priority**: P2

### INT-15: Neuro -> Gate Thresholds (M14)
- [x] Wire neuro knowledge into adaptive gate thresholds

**Current code**: Wired. AdaptiveThresholds::apply_neuro_hints() accepts known failure/stable rung lists and adjusts CUSUM sensitivity and EMA baselines. The orchestrator calls apply_neuro_gate_hints() at plan-run start, querying the neuro KnowledgeStore for gate-related failure/stability patterns and feeding them to the thresholds.
**Priority**: P3

### INT-16: Code Intel -> Verification (M16)
- [x] Wire code intel into gate verification

**Current code**: Wired. RungExecutionInputs gains a code_intel_hints field populated from roko-index search results scoped to the current task description. The symbol gate (rung 3) injects these hints as tags on the signal for focused verification.
**Priority**: P3

### INT-17: Tech Analysis -> Heartbeat (M17)
- [x] Wire technical analysis into heartbeat monitoring

**Current code**: Wired. `BidderContext` gains `build_pass_rate`, `test_pass_rate`, `complexity_trend`, and `regression_detected` fields fed from `CodingOracle` observations. `OraclePredictionsBidder` now generates a second "technical analysis" candidate when build/test pass rates drop below 0.8 or a regression is detected. Token budget scales with complexity trend.
**Priority**: P3

### INT-18: Dreams -> Daimon (M18)
- [x] Wire dream insights into daimon behavior

**Current code**: Wired. AffectEvent::DreamOutcome variant added to roko-daimon, appraised in DaimonState::appraise(). Orchestrator's maybe_auto_dream() converts DreamCycleReport metrics (knowledge entries, playbooks, regressions, hypotheses, episodes processed) into affect deltas. Positive dream outcomes boost pleasure/dominance/confidence; regressions raise arousal and lower confidence.
**Priority**: P3

### INT-19: Coordination -> Dreams (M19)
- [x] Wire coordination signals into dream consolidation

**Current code**: Wired. DreamTrigger::CoordinationPattern variant added to roko-dreams. Conductor's PatternDetector stores compound patterns after evaluate() via last_compound_patterns field; orchestrator drains them via take_compound_patterns() and calls maybe_coordination_dream() at Delta heartbeat frequency. Critical-severity patterns (resource_exhaustion, quality_degradation, progress_stall) trigger immediate dream consolidation.
**Priority**: P3

### INT-20: Lifecycle -> Neuro restore (M20)
- [x] Wire lifecycle restore events into neuro

**Current code**: Wired. The orchestrator's handle_runtime_event now processes AgentLifecycleTransition events, recording significant transitions (hibernation, degradation, metamorphosis, restores) as KnowledgeEntry items in the neuro store via record_lifecycle_knowledge(). Degradation/deletion transitions are recorded as AntiKnowledge; operational transitions as Procedural knowledge.
**Priority**: P3

## Cross-Cut Arbitration

### INT-21: Cross-cut arbitration protocol
- [x] Implement arbitration for Daimon/Neuro/Dreams conflicts

**Spec** (doc 13): When cross-cuts (Daimon, Neuro, Dreams) produce conflicting guidance, an arbitration protocol resolves the conflict.
**Current code**: Wired. `roko_core::arbitration` module implements `Arbitrator` with `SubsystemGuidance` entries from Daimon, Neuro, and Dreams. Conflicts are detected when 2+ subsystems provide different recommendations for the same domain. Resolution uses weighted priority (Neuro 0.50, Daimon 0.30, Dreams 0.20) multiplied by per-guidance confidence. Configurable via `ArbitrationConfig`. 8 unit tests verify conflict detection, weight-based resolution, and configuration override.
**Priority**: P2

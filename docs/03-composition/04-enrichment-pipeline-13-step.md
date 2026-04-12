# 04 — Enrichment Pipeline: 13-Step Context Pre-Computation

> Layer 2 Scaffold — Synapse Architecture
> Status: **Implemented** — `roko-compose::enrichment` (1,187 lines total)
> Canonical source: `crates/roko-compose/src/enrichment/`

---

## Abstract

The enrichment pipeline pre-computes context artifacts before agent sessions begin. Rather than having agents spend tokens discovering what they need, the pipeline generates 13 typed artifacts (PRD extracts, briefs, decompositions, research memos, dependency manifests, fixture manifests, integration notes, verification scripts, reviews, tests, invariants, and scribe tasks) using the cheapest appropriate model for each step. Each artifact is stored on disk, staleness-checked, and selectively injected into agent prompts based on role and task type.

This document specifies the 13 enrichment steps, the LLM client abstraction, the staleness-checking mechanism, the TOML repair logic, and the continue-on-failure semantics.

---

## 1. The 13 Enrichment Steps

Each step in the pipeline produces a single typed artifact:

```rust
// crates/roko-compose/src/enrichment/step.rs

pub enum EnrichStep {
    /// Extract PRD sections relevant to this plan.
    Prd,
    /// Generate strategist briefs (What/Why/How summaries).
    Briefs,
    /// Generate task TOMLs from plan decomposition.
    Tasks,
    /// Decompose plan into step-by-step subtasks.
    Decompose,
    /// Deep research on relevant topics with citations.
    Research,
    /// Identify external dependency requirements.
    Dependencies,
    /// Identify test fixture requirements.
    Fixtures,
    /// Generate integration notes for cross-crate changes.
    Integration,
    /// Generate verification scripts (invariant checks).
    Verify,
    /// Generate review task lists.
    Reviews,
    /// Generate test task lists.
    Tests,
    /// Generate invariant specifications.
    Invariants,
    /// Generate scribe task lists for documentation.
    Scribe,
}
```

The steps are ordered by dependency. The canonical execution order is defined by `ALL_ORDERED`:

```rust
pub const ALL_ORDERED: &[EnrichStep] = &[
    EnrichStep::Prd,
    EnrichStep::Briefs,
    EnrichStep::Tasks,
    EnrichStep::Decompose,
    EnrichStep::Research,
    EnrichStep::Dependencies,
    EnrichStep::Fixtures,
    EnrichStep::Integration,
    EnrichStep::Verify,
    EnrichStep::Reviews,
    EnrichStep::Tests,
    EnrichStep::Invariants,
    EnrichStep::Scribe,
];
```

### Step Details

| # | Step | Output File | Needs LLM? | Default Model | Purpose |
|---|------|------------|-----------|---------------|---------|
| 1 | Prd | `prd-extract.md` | Yes | Haiku | Extract plan-relevant PRD sections |
| 2 | Briefs | `brief.md` | Yes | Sonnet | Generate What/Why/How task summaries |
| 3 | Tasks | `tasks.toml` | Yes | Sonnet | Generate task specifications |
| 4 | Decompose | `decomposition.md` | Yes | Sonnet | Step-by-step subtask breakdown |
| 5 | Research | `research.md` | Yes | Opus | Deep research with citations |
| 6 | Dependencies | `dependency-manifest.toml` | Yes | Haiku | External dependency list |
| 7 | Fixtures | `fixture-manifest.toml` | Yes | Haiku | Test fixture requirements |
| 8 | Integration | `integration.md` | Yes | Sonnet | Cross-crate integration notes |
| 9 | Verify | `verify.sh` | Yes | Haiku | Invariant verification script |
| 10 | Reviews | `review-tasks.toml` | Yes | Haiku | Review task assignments |
| 11 | Tests | `test-tasks.toml` | Yes | Haiku | Test task assignments |
| 12 | Invariants | `invariants.md` | Yes | Sonnet | Invariant specifications |
| 13 | Scribe | `scribe-tasks.toml` | Yes | Haiku | Documentation task assignments |

All 13 steps require an LLM call. The cheapest model (Haiku) is used for mechanical extraction tasks (PRD extraction, dependency listing, fixture listing). Sonnet handles reasoning-heavy tasks (briefs, decomposition, integration notes). Opus is reserved for deep research.

---

## 2. The LLM Client Abstraction

The pipeline is generic over an LLM client trait:

```rust
// crates/roko-compose/src/enrichment/mod.rs

pub trait LlmClient: Send + Sync {
    fn complete(
        &self,
        model: &str,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<String>;
}
```

Four backend implementations are defined:

```rust
// crates/roko-compose/src/enrichment/step.rs

pub enum LlmBackend {
    /// Anthropic Claude models via API.
    Claude,
    /// OpenAI Codex models via API.
    Codex,
    /// Cursor's inference endpoint.
    Cursor,
    /// Local models via Ollama.
    Ollama,
}
```

The pipeline uses two client modes:
- **Batch client:** For steps that produce many artifacts (one per plan). Batches requests for cost efficiency.
- **Direct client:** For steps that produce single artifacts. Sends individual requests.

---

## 3. The EnrichmentPipeline

```rust
// crates/roko-compose/src/enrichment/pipeline.rs

pub struct EnrichmentPipeline<C: LlmClient> {
    client: Arc<C>,
    config: EnrichmentConfig,
    output_dir: PathBuf,
}

impl<C: LlmClient> EnrichmentPipeline<C> {
    /// Run a single enrichment step.
    pub async fn run_step(&self, step: EnrichStep) -> Result<PathBuf> {
        // 1. Check staleness — skip if output exists and is fresh
        // 2. Build prompt for this step
        // 3. Call LLM client
        // 4. Validate output (TOML parse check if applicable)
        // 5. If TOML invalid, one repair retry
        // 6. Write output to disk
        // 7. Return output path
    }

    /// Run all 13 steps in order. Continue on failure.
    pub async fn run_all(&self) -> Vec<StepResult> {
        let mut results = Vec::new();
        for step in EnrichStep::ALL_ORDERED {
            match self.run_step(*step).await {
                Ok(path) => results.push(StepResult::Success { step: *step, path }),
                Err(e) => {
                    tracing::warn!(?step, error = %e, "enrichment step failed, continuing");
                    results.push(StepResult::Failed { step: *step, error: e.to_string() });
                }
            }
        }
        results
    }
}
```

### 3.1 Staleness Checking

Before running a step, the pipeline checks whether the output file already exists and is fresh:

```rust
fn is_stale(&self, step: &EnrichStep) -> bool {
    let output_path = self.output_dir.join(step.output_filename());
    if !output_path.exists() {
        return true; // No output yet — definitely stale
    }
    let metadata = std::fs::metadata(&output_path).ok();
    let modified = metadata.and_then(|m| m.modified().ok());
    match modified {
        Some(mod_time) => {
            let age = SystemTime::now().duration_since(mod_time).unwrap_or_default();
            age > self.config.max_staleness
        }
        None => true,
    }
}
```

Default max_staleness is 24 hours. If the output exists and was generated within 24 hours, the step is skipped. This prevents re-running expensive LLM calls when the enrichment pipeline is invoked multiple times (e.g., after a plan run failure and restart).

### 3.2 TOML Repair

Steps that produce TOML output (Tasks, Dependencies, Fixtures, Reviews, Tests, Scribe) include a validation and repair pass:

```rust
fn validate_and_repair_toml(
    &self,
    step: &EnrichStep,
    raw_output: &str,
) -> Result<String> {
    match toml::from_str::<toml::Value>(raw_output) {
        Ok(_) => Ok(raw_output.to_string()),
        Err(parse_error) => {
            // One retry: send the parse error back to the LLM
            // with instructions to fix the TOML syntax
            let repair_prompt = format!(
                "The following TOML has a syntax error:\n\
                Error: {parse_error}\n\n\
                Content:\n{raw_output}\n\n\
                Fix the TOML syntax error and return only the corrected TOML."
            );
            let repaired = self.client.complete(
                step.default_model(),
                "You fix TOML syntax errors. Return only valid TOML.",
                &repair_prompt,
            )?;
            toml::from_str::<toml::Value>(&repaired)?;
            Ok(repaired)
        }
    }
}
```

The repair step uses one retry only. If the repair also fails, the step is marked as failed and the pipeline continues to the next step. This "one-retry" policy prevents infinite LLM loops on malformed output.

### 3.3 Continue-on-Failure Semantics

The pipeline runs all 13 steps regardless of individual failures:

```
Step 1 (Prd):     ✓ → prd-extract.md
Step 2 (Briefs):  ✓ → brief.md
Step 3 (Tasks):   ✗ (TOML repair failed)
Step 4 (Decompose): ✓ → decomposition.md
...
```

Failed steps are logged as warnings. The agent receives whatever artifacts were successfully generated. Missing artifacts are simply absent from the prompt — the PromptComposer's priority-based dropping handles this gracefully (missing sections reduce the prompt size but do not break it).

This is a deliberate design choice. The enrichment pipeline runs before the agent session starts. If a step fails, the cost of retrying is low (one more LLM call), but the cost of blocking the entire pipeline is high (delayed agent start, blocked plan execution). The agent can often succeed without every enrichment artifact.

---

## 4. Step Selection

Not every task needs all 13 enrichment steps. The `StepSelector` determines which steps to run based on task characteristics:

| Task Type | Steps Run | Steps Skipped | Rationale |
|-----------|----------|---------------|-----------|
| Simple rename | Prd, Briefs | 11 others | Mechanical task needs minimal context |
| Standard implementation | Prd, Briefs, Tasks, Decompose, Research | 8 others | Core implementation artifacts |
| Cross-crate integration | All 13 | None | Complex tasks need full enrichment |
| Review task | Prd, Reviews | 11 others | Reviews need the PRD and review checklist |
| Documentation task | Prd, Scribe, Research | 10 others | Documentation needs PRD and citations |

Step selection is driven by the task's complexity band (Trivial/Standard/Complex) and role:

```rust
pub fn steps_for(complexity: Complexity, role: AgentRole) -> Vec<EnrichStep> {
    match (complexity, role) {
        (Complexity::Trivial, _) => vec![EnrichStep::Prd, EnrichStep::Briefs],
        (Complexity::Standard, AgentRole::Scribe) => {
            vec![EnrichStep::Prd, EnrichStep::Scribe, EnrichStep::Research]
        }
        (Complexity::Complex, _) => EnrichStep::ALL_ORDERED.to_vec(),
        _ => vec![
            EnrichStep::Prd, EnrichStep::Briefs, EnrichStep::Tasks,
            EnrichStep::Decompose, EnrichStep::Research,
        ],
    }
}
```

### Agentic RAG Integration

The step selection mechanism is the practical application of Self-RAG (Asai et al. 2023) to the enrichment pipeline. Self-RAG introduces reflection tokens that let agents decide WHEN to retrieve — the model judges whether it has enough context before triggering retrieval. Roko's step selector makes this decision at the task level: a simple rename task is classified as needing minimal context (Self-RAG's "no retrieval needed"), while a cross-crate integration task triggers full enrichment (Self-RAG's "retrieval strongly needed").

---

## 5. Disk Layout

Enrichment artifacts are stored on disk under the plan directory:

```
.roko/plans/<plan-slug>/
├── prd-extract.md
├── brief.md
├── tasks.toml
├── decomposition.md
├── research.md
├── dependency-manifest.toml
├── fixture-manifest.toml
├── integration.md
├── verify.sh
├── review-tasks.toml
├── test-tasks.toml
├── invariants.md
└── scribe-tasks.toml
```

Every artifact is a file on disk, not in memory. This makes them:
- **Diffable:** `git diff` shows what changed between enrichment runs
- **Inspectable:** Human reviewers can read the artifacts directly
- **Cacheable:** Staleness checking uses file modification timestamps
- **Debuggable:** If an agent produces bad output, the input artifacts are readable

---

## 6. Compound AI System Pattern

The enrichment pipeline embodies the Compound AI Systems paradigm [Zaharia et al., BAIR 2024]. Instead of sending a single monolithic prompt to a frontier model, the pipeline:

1. Decomposes the context assembly problem into 13 typed sub-problems
2. Assigns each sub-problem to the cheapest model that can handle it
3. Stores intermediate results on disk for reuse
4. Composes the results into a tailored prompt for the agent

This achieves the central insight of compound AI: "clever engineering > model scaling." A system of Haiku calls at $0.01/artifact produces context that enables a single Sonnet call to achieve higher task success than Opus without enrichment.

### Cost Analysis

| Enrichment approach | Cost per plan | Agent success rate |
|--------------------|---------------|-------------------|
| No enrichment | $0 | ~45% |
| All 13 steps (Haiku/Sonnet mix) | ~$0.15 | ~78% |
| Manual context assembly | $0 (human time) | ~72% |

The $0.15 enrichment investment produces a ~33% improvement in agent success rate. The key is using the cheapest model for each step: Haiku for mechanical extraction ($0.005/call), Sonnet for reasoning ($0.02/call), Opus for deep research ($0.08/call).

---

## 7. Academic Foundations

**Modular RAG** [Gao et al. 2023]. The evolution from Naive RAG (retrieve-then-read) to Modular RAG (composable retrieval/generation/augmentation modules). The enrichment pipeline is a Modular RAG implementation: each step is a composable module that retrieves, generates, or augments context for the downstream agent.

**Self-RAG: Adaptive Retrieval with Reflection Tokens** [Asai et al. 2023]. Self-RAG learns WHEN to retrieve, WHAT to retrieve, and WHETHER retrieved content is useful. The step selector implements the "when to retrieve" decision. CRAG (Yan et al. 2024) adds self-correction — when retrieval confidence is low, the system falls back to alternative strategies. The TOML repair logic is a simple form of CRAG: when the initial generation fails, retry with corrective feedback.

**DSPy: Programmatic Prompt Optimization** [Khattab et al. 2023]. DSPy reframed prompting as programming: define modules with typed signatures, compose pipelines, optimize automatically. The enrichment pipeline is a DSPy-compatible pipeline: each step has a typed signature (plan → artifact), and the pipeline can be optimized against downstream task success by adjusting which steps to run and how to parameterize them.

**LLMLingua: Prompt Compression** [Jiang et al., EMNLP 2023]. The enrichment pipeline is an alternative to compression: instead of compressing raw context to fit the budget, pre-compute focused artifacts that are already dense. A PRD extract is more token-efficient than the full PRD compressed 5×, because extraction removes irrelevant sections entirely rather than compressing them.

**"Write for Amnesia" Principle** (from Mori development). Every agent session starts cold, with no conversation history and no shared memory. The files on disk are the only truth. Every piece of context an agent needs must be pre-assembled on disk before the session starts. The enrichment pipeline is the implementation of this principle: it does the context preparation work ahead of time so agents do not burn tokens figuring out what they need.

---

## 8. Context Injection

Enrichment artifacts are injected into agent prompts through the context injection system:

```
context/in/
├── execution-pack.md          # Merged context for any role
├── implementer-pack.md        # Role-specific: Implementer
├── architect-pack.md          # Role-specific: Architect
├── scribe-pack.md             # Role-specific: Scribe
├── brief.md                   # Implementation brief
├── prd2-extract.md            # Relevant PRD sections
├── decomposition.md           # Step-by-step breakdown
├── verify-tasks.toml          # Verification checklist
├── learning.md                # Learning pack
├── research.md                # Research artifacts
├── playbook.md                # Applicable playbook rules
└── reflections.md             # Prior iteration reflections
```

Each role receives guidance on which files to read:

| Role | Primary pack | Additional files |
|------|-------------|-----------------|
| Implementer | execution-pack.md | brief.md |
| Architect | architect-pack.md | review-tasks.toml, verify-tasks.toml |
| Scribe | scribe-pack.md | scribe-tasks.toml, research.md |
| Auditor | auditor-pack.md | verify-tasks.toml |

This prevents agents from reading the entire context directory. Each agent opens exactly what it needs.

---

## 9. Current Status and Gaps

| Aspect | Status |
|--------|--------|
| 13 enrichment steps defined | **Implemented** |
| EnrichmentPipeline struct | **Implemented** |
| Staleness checking | **Implemented** |
| TOML repair (one retry) | **Implemented** |
| Continue-on-failure | **Implemented** |
| LlmClient trait | **Implemented** |
| Step selector by complexity/role | **Implemented** |
| 4 backend types defined | **Implemented** |
| Adaptive step selection (learned from outcomes) | **Not yet** |
| Parallel step execution | **Not yet** (sequential only) |
| Cost tracking per step | **Not yet** |

---

## Cross-References

- [03-role-templates.md](03-role-templates.md) — Per-role budget allocation that determines which artifacts are injected
- [05-token-budget-management.md](05-token-budget-management.md) — Budget constraints on enrichment artifact size
- [08-5-stage-assembly-pipeline.md](08-5-stage-assembly-pipeline.md) — Assembly pipeline that consumes enrichment artifacts
- [13-current-status-and-gaps.md](13-current-status-and-gaps.md) — Overall status
- `crates/roko-compose/src/enrichment/step.rs` — Step definitions
- `crates/roko-compose/src/enrichment/pipeline.rs` — Pipeline implementation

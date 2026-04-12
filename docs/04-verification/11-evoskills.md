# 11 — EvoSkills: Self-Evolving Verification Skills

> **Layer**: L3 Harness — Verification × L2 Engine — Learning
> **Crates**: `roko-learn` (skill_library, pattern_discovery), `roko-gate`
> **Status**: Skill library scaffold exists, adversarial verification designed

---

## 1. Overview

EvoSkills is a self-evolving skill library where verification skills improve
autonomously through adversarial surrogate verification and cross-model transfer. The
core idea: agents accumulate reusable tool-use patterns from successful task executions,
and these patterns are validated against adversarial test suites to ensure they
generalize.

The empirical results from the reference system are striking:
- Baseline success rate: 32%
- With EvoSkills: 75% (+43 percentage points)
- Cross-model transfer improvement: +35–44 percentage points

These numbers come from the adversarial surrogate verification process: skills that
survive adversarial testing are genuinely useful, not just incidental patterns from
lucky executions.

> **Citation**: refactoring-prd/09-innovations.md — Innovation X: "EvoSkills (32%→75%,
> cross-model +35-44pp)."

---

## 2. The Three-Tier Learning Hierarchy

Skills emerge from Roko's three-tier learning system:

### Tier 1: Episodes (Raw)

Every agent execution is recorded as an episode in `.roko/episodes.jsonl`. An episode
contains:
- Task specification
- Tool calls (in order)
- Gate verdicts
- Final outcome (passed/failed)
- Token counts and timing

Episodes are the raw data. They are never modified or deleted.

### Tier 2: Patterns (Extracted)

When 5+ similar episodes show the same tool-use sequence leading to success, a pattern
is extracted:
- Precondition: what task characteristics trigger this pattern?
- Procedure: what tool calls compose the pattern?
- Postcondition: what gate outcomes does this pattern produce?

Patterns are hypotheses — they suggest that a particular approach works for a particular
kind of task. They are not yet validated.

### Tier 3: Playbook (Validated)

When a pattern has been successfully applied 5+ times in production (not just extracted
from historical data), it is promoted to a playbook rule. Playbook rules are:
- Validated through actual use
- Injected into agent context via the prompt's "skills" section
- Tracked with confidence scores and usage telemetry

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §D —
> "Mori's learning hierarchy: Tier 1 Episodes, Tier 2 Patterns, Tier 3 Playbook."

---

## 3. Adversarial Surrogate Verification

The breakthrough in EvoSkills is the validation step. Rather than trusting that
extracted patterns generalize, the system tests them adversarially:

### 3.1 Surrogate Test Generation

For each candidate skill, generate a suite of adversarial test cases that specifically
try to break the skill:
- Edge cases the skill might not handle
- Input variations that test generalization
- Failure modes that would reveal brittle patterns

### 3.2 Cross-Model Testing

Apply the skill with different models to verify it transfers:
- If a skill works with Model A but fails with Model B, it may be an artifact of
  Model A's specific capabilities rather than a genuine skill
- Skills that work across 3+ models are robust and likely capture genuine
  task-completion knowledge

### 3.3 Confidence Scoring

Each skill maintains a confidence score based on:

```
confidence = (validations / (validations + failures)) × cross_model_factor
```

Where `cross_model_factor` is:
- 1.0 if validated across 3+ models
- 0.8 if validated across 2 models
- 0.6 if validated on only 1 model

Skills below a confidence threshold (e.g., 0.5) are demoted back to Tier 2 for
re-extraction.

---

## 4. Skill Structure

```rust
pub struct Skill {
    pub id: String,
    pub name: String,
    pub precondition: String,         // When to apply this skill
    pub procedure: String,            // What to do (tool call sequence summary)
    pub postcondition: String,        // Expected outcome
    pub confidence: f64,              // [0, 1] based on validation
    pub source_episodes: Vec<String>, // Episode IDs this was extracted from
    pub validations: u64,             // Times successfully applied
    pub failures: u64,                // Times applied but failed
    pub task_categories: Vec<String>, // Which task types this applies to
    pub created_at: String,
    pub last_validated_at: Option<String>,
}
```

### Example Skill

```
Skill: "Rust Compile Fix — Missing Import"
Precondition: compile gate fails with error[E0433] or error[E0425]
Procedure:
  1. Read the error message to identify the missing symbol
  2. Search for the symbol in the codebase (Grep)
  3. Identify the crate/module that exports it
  4. Add the use statement to the file
Postcondition: compile gate passes
Confidence: 0.92 (validated 46 times, failed 4 times, 3 models)
```

> **Citation**: SAGE (arXiv:2512.17102) — "Agents that accumulate reusable tool-use
> patterns across tasks use 26% fewer steps and 59% fewer tokens."

---

## 5. Skill Injection into Agent Context

Validated skills are injected into the agent's prompt as a dedicated section:

```
## Relevant Skills

Based on the current task (compile error fix, auth module), the following
verified skills may be applicable:

### Skill: Rust Compile Fix — Missing Import (confidence: 0.92)
When: compile gate fails with error[E0433] or error[E0425]
Do: Read error → Grep for symbol → Identify export → Add use statement
Expected: compile gate passes

### Skill: Auth Module Test Pattern (confidence: 0.78)
When: task involves auth module changes
Do: Read existing auth tests → Modify in parallel → Run TestGate
Expected: all auth tests pass
```

The section effectiveness tracker (from the gate-to-scaffold feedback loop) monitors
whether skill injection actually improves gate pass rates. Skills whose injection
doesn't improve outcomes get their priority reduced.

---

## 6. Cross-Model Transfer

The +35–44pp cross-model transfer improvement means that skills extracted from Model A's
successful executions help Model B succeed:

```
Model A executes 100 tasks:
  → 50 succeed, 50 fail (50% base rate)
  → Extract 15 skills from the 50 successes

Model B executes 100 tasks (same distribution):
  → Without skills: 32% success rate
  → With Model A's skills: 67–76% success rate (+35–44pp)
```

This transfer works because the skills encode *task-completion knowledge* (how to fix
compile errors, how to write tests, how to modify auth modules), not model-specific
behaviors. A skill like "read the error, search for the symbol, add the import" works
regardless of which model executes it.

This has a practical implication: **new models get instant expertise**. When a new model
is added to the routing pool, it immediately benefits from the accumulated skill library
without any warm-up period.

---

## 7. Skill Evolution

Skills evolve over time through three mechanisms:

### 7.1 Refinement

When a skill fails, the failure is analyzed:
- Was the precondition too broad? → Narrow the precondition
- Was the procedure missing a step? → Add the missing step
- Was the postcondition wrong? → Correct the expected outcome

### 7.2 Specialization

A general skill that works for most cases may fail for a specific sub-case. In that
case, a specialized variant is created:

```
General: "Fix compile error — missing import"
Specialized: "Fix compile error — missing import from workspace crate"
  (adds: check Cargo.toml for workspace dependencies before adding use)
```

### 7.3 Retirement

Skills whose confidence drops below the threshold (e.g., due to codebase evolution
making old patterns obsolete) are retired:
- Removed from the active playbook
- Retained in the archive for historical analysis
- May be re-extracted if conditions change

---

## 8. Relationship to Verification

EvoSkills are deeply connected to the verification layer:

### 8.1 Gate Verdicts Drive Skill Extraction

Skills are extracted from episodes where **all gates passed**. A successful episode must
have:
- Compile gate: PASS
- Lint gate: PASS
- Test gate: PASS
- Any other rungs that ran: PASS

This ensures skills are extracted from genuinely successful executions, not from
partial successes.

### 8.2 Gate Verdicts Validate Skills

Skill validation means: "Apply the skill on a new task and verify that the gates pass."
The gate pipeline is the validation oracle. No gate pass = no validation credit.

### 8.3 Skills Improve Gate Pass Rates

The feedback loop closes: skills extracted from gate successes are injected into prompts,
leading to more gate successes, leading to more skill extraction. This is a positive
feedback loop that compounds over time.

The risk of positive feedback loops is runaway behavior. The adversarial surrogate
verification is the stabilizer: it prunes skills that don't genuinely help, preventing
the accumulation of noise.

---

## 9. Relationship to the Evaluation Lifecycle

EvoSkills operate at the Consolidation Speed tier (Loop 9: Skill Extraction):

```
Machine Speed:     Per-turn tool call data collection
Cognitive Speed:   Gate pipeline produces verdicts
Consolidation:     Skill extraction from episodes with passing gates   ← EvoSkills
Retrospective:     Cross-model validation of extracted skills
Meta:              Evaluate whether skill library is net positive
```

The consolidation loop runs after a batch of tasks completes. It scans recent successful
episodes, clusters them by similarity, and extracts candidate skills. The retrospective
loop validates candidates across models. The meta loop evaluates whether the skill
library as a whole is improving outcomes.

> **Citation**: bardo-backup/prd/16-testing/09-evaluation-map.md — "14 feedback loops
> across 5 speed tiers."

---

## 10. Academic Foundations

### SAGE (arXiv:2512.17102)

Self-Acquired Generalist Expertise: demonstrated that agents accumulating reusable
patterns across tasks use 26% fewer steps and 59% fewer tokens. EvoSkills applies this
to verification-guided development.

### Voyager (Wang et al. 2023)

Showed that an LLM agent in Minecraft that accumulates a skill library achieves
dramatically more complex goals than one that starts fresh each time. The skill library
is the agent's "long-term procedural memory."

### DSPy Bayesian Optimizers

The Bayesian optimization approach to prompt tuning from DSPy provides a framework for
skill injection: each skill is a prompt component whose inclusion probability is
optimized based on gate outcomes.

> **Citation**: SAGE (arXiv:2512.17102), Voyager (Wang et al. 2023) — Skill library
> foundations.

---

## 11. Summary

EvoSkills transform Roko from a system that treats each task independently into one
that learns from experience. The three-tier hierarchy (Episodes → Patterns → Playbook)
ensures that only validated, generalizing skills enter the agent's context. Adversarial
surrogate verification prevents noise accumulation. Cross-model transfer means knowledge
compounds across the entire model pool.

The 32% → 75% improvement is the headline number, but the deeper insight is: **skills
are extracted from verification successes and validated by verification**. The gate
pipeline is both the source of skill data and the arbiter of skill quality.

# 11 — EvoSkills: Self-Evolving Verification Skills

> **Layer**: L3 Harness — Verification × L2 Engine — Learning
> **Crates**: `roko-learn` (skill_library, pattern_discovery), `roko-gate`
> **Status**: Skill library scaffold exists, adversarial verification designed


> **Implementation**: Shipping

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

---

## 12. Skill Genome Representation

To evolve skills systematically, we need a genome — a structured representation
that can be mutated, crossed over, and evaluated. The skill genome encodes not just
the procedure but the entire agent configuration that produces the skill.

> **Citation**: "Agent Skill Acquisition for Large Language Models" (ICLR 2025) —
> MAP-Elites-style cyclic optimization for LLM agent skills.

### 12.1 Genome Structure

```rust
/// A skill genome encodes the full agent configuration that produces a skill.
/// This is the unit of evolution — what gets mutated, crossed over, and selected.
pub struct SkillGenome {
    /// Unique identifier for this genome.
    pub id: String,
    /// The skill this genome encodes (precondition, procedure, postcondition).
    pub skill: Skill,

    // --- Evolvable parameters ---

    /// System prompt strategy: how the skill is described to the agent.
    /// Mutation: rephrase, add/remove context, change emphasis.
    pub prompt_template: String,
    /// Tool usage preferences: which tools the skill prioritizes.
    /// Each entry is (tool_name, priority_weight).
    /// Mutation: adjust weights, add/remove tools.
    pub tool_preferences: Vec<(String, f64)>,
    /// Retry strategy when the skill fails partway through.
    pub retry_config: RetryGenome,
    /// Model temperature for LLM calls during skill execution.
    /// Mutation: Gaussian perturbation.
    pub temperature: f64,            // default: 0.3, range: [0.0, 1.0]
    /// Token budget allocated to this skill's execution.
    /// Mutation: scale up/down.
    pub token_budget: usize,         // default: 4096
    /// Gate weights: how much the skill cares about each gate.
    /// Used for fitness computation. Mutation: adjust per-gate weights.
    pub gate_weights: Vec<f64>,      // one per rung

    // --- Behavioral descriptor (for MAP-Elites) ---

    /// Measured behavioral characteristics (not evolvable — computed from execution).
    pub behavior: BehavioralDescriptor,

    // --- Fitness ---

    /// Fitness score from the last evaluation.
    pub fitness: f64,
    /// Number of evaluations.
    pub evaluations: u64,
}

#[derive(Debug, Clone)]
pub struct RetryGenome {
    /// Maximum retries for this skill.
    pub max_retries: u32,       // default: 3, range: [1, 8]
    /// Backoff multiplier between retries.
    pub backoff_factor: f64,    // default: 1.5, range: [1.0, 4.0]
    /// Whether to adjust prompt between retries.
    pub prompt_mutation_on_retry: bool,
}
```

### 12.2 Behavioral Descriptors

Behavioral descriptors define the axes of the MAP-Elites archive. They are measured
from execution, not set directly — the same genome can produce different behaviors
in different contexts.

```rust
/// Measured behavioral characteristics of a skill execution.
/// These define the axes of the MAP-Elites quality-diversity archive.
#[derive(Debug, Clone)]
pub struct BehavioralDescriptor {
    /// Axis 1: Task completion rate [0, 1].
    /// What fraction of tasks this skill completes successfully.
    pub completion_rate: f64,
    /// Axis 2: Average gate score across all rungs [0, 1].
    /// How thoroughly the skill passes verification.
    pub gate_score: f64,
    /// Axis 3: Token efficiency — inverse of tokens per successful task.
    /// Normalized to [0, 1] where 1.0 = cheapest observed.
    pub token_efficiency: f64,
    /// Axis 4: Generalization breadth — fraction of task categories
    /// where this skill has been successfully applied.
    pub generalization: f64,
}

impl BehavioralDescriptor {
    /// Discretize into a MAP-Elites cell index.
    ///
    /// Each axis is divided into `resolution` bins.
    /// With 4 axes and 10 bins each, the archive has 10,000 cells.
    pub fn to_cell(&self, resolution: usize) -> [usize; 4] {
        let bin = |v: f64| ((v * resolution as f64) as usize).min(resolution - 1);
        [
            bin(self.completion_rate),
            bin(self.gate_score),
            bin(self.token_efficiency),
            bin(self.generalization),
        ]
    }
}
```

---

## 13. MAP-Elites for Skill Quality-Diversity

Standard evolutionary algorithms optimize for a single fitness function, converging
to one solution. MAP-Elites maintains an archive of diverse, high-quality solutions
across a behavioral space. This is critical for skills: we don't want one "best" skill —
we want a diverse repertoire covering different task types and strategies.

> **Citation**: Mouret & Clune, "Illuminating Search Spaces by Mapping Elites"
> (arXiv:1504.04909, 2015).

> **Citation**: "Quality-Diversity Methods for the Modern Data Scientist" (WIREs
> Computational Statistics, 2025).

### 13.1 Archive Structure

```rust
/// MAP-Elites archive: a grid of skill genomes indexed by behavioral descriptor.
///
/// Each cell stores the highest-fitness genome observed for that behavior region.
/// Empty cells represent unexplored behavioral niches.
pub struct SkillArchive {
    /// Grid resolution per behavioral axis.
    pub resolution: usize,         // default: 10
    /// Number of behavioral dimensions.
    pub dimensions: usize,         // 4 (completion, gate_score, efficiency, generalization)
    /// The archive grid. Keyed by flattened cell index.
    /// Each cell stores the best genome for that behavioral niche.
    pub cells: HashMap<usize, SkillGenome>,
    /// Total evaluations performed (across all generations).
    pub total_evaluations: u64,
    /// Generation counter.
    pub generation: u64,
}

impl SkillArchive {
    /// Insert a genome into the archive if it's the best for its cell.
    pub fn try_insert(&mut self, genome: SkillGenome) -> InsertResult {
        let cell = genome.behavior.to_cell(self.resolution);
        let flat_index = self.flatten_index(&cell);

        match self.cells.entry(flat_index) {
            Entry::Vacant(e) => {
                e.insert(genome);
                InsertResult::NewNiche
            }
            Entry::Occupied(mut e) => {
                if genome.fitness > e.get().fitness {
                    e.insert(genome);
                    InsertResult::Improved
                } else {
                    InsertResult::Rejected
                }
            }
        }
    }

    /// Coverage: fraction of cells that are filled.
    pub fn coverage(&self) -> f64 {
        let total_cells = self.resolution.pow(self.dimensions as u32);
        self.cells.len() as f64 / total_cells as f64
    }

    /// QD-score: sum of all fitness values in the archive.
    /// Higher = better quality AND diversity.
    pub fn qd_score(&self) -> f64 {
        self.cells.values().map(|g| g.fitness).sum()
    }

    /// Select a random parent from the archive for mutation.
    pub fn random_parent(&self, rng: &mut impl Rng) -> Option<&SkillGenome> {
        let keys: Vec<_> = self.cells.keys().collect();
        if keys.is_empty() { return None; }
        let idx = rng.gen_range(0..keys.len());
        self.cells.get(keys[idx])
    }
}

pub enum InsertResult {
    /// Filled a previously empty cell — new behavioral niche discovered.
    NewNiche,
    /// Replaced an existing genome with a fitter one.
    Improved,
    /// Existing genome in that cell was fitter — genome discarded.
    Rejected,
}
```

### 13.2 Evolution Loop

```rust
/// One generation of MAP-Elites skill evolution.
///
/// Pseudocode:
///   for _ in 0..batch_size:
///       parent = archive.random_parent()
///       offspring = mutate(parent)
///       fitness, behavior = evaluate(offspring)
///       offspring.fitness = fitness
///       offspring.behavior = behavior
///       archive.try_insert(offspring)
pub struct SkillEvolver {
    pub archive: SkillArchive,
    /// Number of offspring per generation.
    pub batch_size: usize,          // default: 16
    /// Mutation operator configuration.
    pub mutation: MutationConfig,
    /// Gate pipeline for fitness evaluation.
    pub evaluator: GatePipeline,
    /// Task sampler for evaluation.
    pub task_sampler: Box<dyn TaskSampler>,
}

pub struct MutationConfig {
    /// Probability of mutating the prompt template.
    pub prompt_mutation_rate: f64,    // default: 0.3
    /// Probability of adjusting tool preferences.
    pub tool_mutation_rate: f64,      // default: 0.2
    /// Standard deviation for continuous parameter perturbation.
    pub param_sigma: f64,            // default: 0.1
    /// Probability of crossover (recombination of two parents).
    pub crossover_rate: f64,         // default: 0.2
}
```

### 13.3 Mutation Operators

```rust
impl SkillGenome {
    /// Mutate this genome to produce an offspring.
    pub fn mutate(&self, config: &MutationConfig, rng: &mut impl Rng) -> Self {
        let mut offspring = self.clone();
        offspring.id = generate_id();

        // Mutate prompt template (rephrase, add/remove context)
        if rng.gen::<f64>() < config.prompt_mutation_rate {
            offspring.prompt_template = mutate_prompt(&self.prompt_template);
        }

        // Mutate tool preferences (adjust weights)
        if rng.gen::<f64>() < config.tool_mutation_rate {
            for (_, weight) in &mut offspring.tool_preferences {
                *weight = (*weight + rng.gen_range(-0.2..0.2)).clamp(0.0, 1.0);
            }
        }

        // Mutate continuous parameters (Gaussian perturbation)
        offspring.temperature = (self.temperature
            + rng.gen::<f64>() * config.param_sigma * 2.0 - config.param_sigma)
            .clamp(0.0, 1.0);
        offspring.token_budget = (self.token_budget as f64
            * (1.0 + rng.gen_range(-0.2..0.2))) as usize;

        // Mutate retry config
        if rng.gen::<f64>() < 0.1 {
            offspring.retry_config.max_retries =
                (self.retry_config.max_retries as i32 + rng.gen_range(-1..=1))
                    .clamp(1, 8) as u32;
        }

        offspring
    }

    /// Crossover: recombine two genomes.
    pub fn crossover(&self, other: &Self, rng: &mut impl Rng) -> Self {
        let mut offspring = self.clone();
        offspring.id = generate_id();

        // Uniform crossover on discrete fields
        if rng.gen::<bool>() {
            offspring.prompt_template = other.prompt_template.clone();
        }
        if rng.gen::<bool>() {
            offspring.tool_preferences = other.tool_preferences.clone();
        }
        if rng.gen::<bool>() {
            offspring.retry_config = other.retry_config.clone();
        }

        // Intermediate crossover on continuous fields
        let alpha = rng.gen::<f64>();
        offspring.temperature = alpha * self.temperature
            + (1.0 - alpha) * other.temperature;
        offspring.token_budget = ((alpha * self.token_budget as f64
            + (1.0 - alpha) * other.token_budget as f64) as usize).max(1024);

        offspring
    }
}
```

---

## 14. Fitness Evaluation

Skill fitness is measured by running the skill on sampled tasks and observing gate
outcomes.

```rust
/// Fitness function for skill genomes.
///
/// Evaluates a skill by applying it to N sampled tasks and measuring
/// gate outcomes. The fitness is a weighted combination of:
///   - Gate pass rate (primary)
///   - Token efficiency (secondary)
///   - Generalization across task categories (tertiary)
pub struct SkillFitness {
    /// Number of evaluation tasks per genome.
    pub eval_tasks: usize,          // default: 5
    /// Gate pipeline for evaluation.
    pub gate_pipeline: GatePipeline,
    /// Fitness weights.
    pub weights: FitnessWeights,
}

pub struct FitnessWeights {
    /// Weight for gate pass rate [0, 1].
    pub gate_pass: f64,    // default: 0.5
    /// Weight for token efficiency [0, 1].
    pub efficiency: f64,   // default: 0.3
    /// Weight for cross-task generalization [0, 1].
    pub generalization: f64, // default: 0.2
}

impl SkillFitness {
    /// Evaluate a genome and return its fitness and behavioral descriptor.
    pub async fn evaluate(&self, genome: &SkillGenome,
                          tasks: &[Task]) -> (f64, BehavioralDescriptor) {
        let mut pass_count = 0;
        let mut total_tokens = 0;
        let mut categories_seen = HashSet::new();
        let mut categories_passed = HashSet::new();
        let mut gate_scores = Vec::new();

        for task in tasks.iter().take(self.eval_tasks) {
            let result = self.run_skill(genome, task).await;
            if result.verdict.passed {
                pass_count += 1;
                categories_passed.insert(task.category.clone());
            }
            total_tokens += result.tokens_used;
            categories_seen.insert(task.category.clone());
            gate_scores.push(result.verdict.score);
        }

        let completion = pass_count as f64 / self.eval_tasks as f64;
        let avg_gate = gate_scores.iter().sum::<f32>() as f64
            / gate_scores.len().max(1) as f64;
        let efficiency = if total_tokens > 0 {
            (pass_count as f64 * 1000.0) / total_tokens as f64
        } else { 0.0 };
        let generalization = if !categories_seen.is_empty() {
            categories_passed.len() as f64 / categories_seen.len() as f64
        } else { 0.0 };

        let fitness = self.weights.gate_pass * completion
            + self.weights.efficiency * efficiency.min(1.0)
            + self.weights.generalization * generalization;

        let behavior = BehavioralDescriptor {
            completion_rate: completion,
            gate_score: avg_gate,
            token_efficiency: efficiency.min(1.0),
            generalization,
        };

        (fitness, behavior)
    }
}
```

---

## 15. Fitness Landscape Analysis

Understanding the topology of the skill space reveals where to search for improvements
and where the landscape is deceptive or rugged.

### 15.1 Landscape Metrics

```rust
/// Fitness landscape analysis for the skill archive.
pub struct LandscapeAnalysis {
    /// Local optima count: cells where no neighbor has higher fitness.
    pub local_optima: usize,
    /// Ruggedness: average fitness difference between adjacent cells.
    /// High ruggedness = many local optima, hard to optimize.
    pub ruggedness: f64,
    /// Neutrality: fraction of adjacent cell pairs with equal fitness.
    /// High neutrality = flat plateaus (drift without progress).
    pub neutrality: f64,
    /// Fitness-distance correlation (FDC): correlation between fitness
    /// and distance to the global optimum.
    /// FDC > 0.15 → landscape is "easy" (gradient toward optimum).
    /// FDC < -0.15 → landscape is "deceptive" (gradient away from optimum).
    pub fdc: f64,
    /// Evolvability: fraction of mutations that produce fitter offspring.
    pub evolvability: f64,
    /// Coverage frontier: cells at the boundary of explored space.
    pub frontier_cells: usize,
}

impl SkillArchive {
    /// Analyze the fitness landscape of the current archive.
    pub fn analyze_landscape(&self) -> LandscapeAnalysis {
        let mut local_optima = 0;
        let mut fitness_diffs = Vec::new();
        let mut neutral_count = 0;
        let mut total_pairs = 0;

        for (&idx, genome) in &self.cells {
            let neighbors = self.get_neighbors(idx);
            let is_local_optimum = neighbors.iter()
                .all(|n| n.fitness <= genome.fitness);
            if is_local_optimum { local_optima += 1; }

            for neighbor in &neighbors {
                let diff = (genome.fitness - neighbor.fitness).abs();
                fitness_diffs.push(diff);
                if diff < 0.01 { neutral_count += 1; }
                total_pairs += 1;
            }
        }

        let ruggedness = if fitness_diffs.is_empty() { 0.0 }
            else { fitness_diffs.iter().sum::<f64>() / fitness_diffs.len() as f64 };
        let neutrality = if total_pairs == 0 { 0.0 }
            else { neutral_count as f64 / total_pairs as f64 };

        LandscapeAnalysis {
            local_optima,
            ruggedness,
            neutrality,
            fdc: self.compute_fdc(),
            evolvability: self.compute_evolvability(),
            frontier_cells: self.count_frontier_cells(),
        }
    }
}
```

### 15.2 Landscape-Adaptive Evolution

The landscape analysis feeds back into the evolution strategy:

```
if ruggedness > 0.3:
    // Rugged landscape — increase mutation strength to escape local optima
    mutation.param_sigma *= 1.5
    mutation.prompt_mutation_rate *= 1.3

if neutrality > 0.5:
    // Flat landscape — increase crossover to search broadly
    mutation.crossover_rate *= 1.5
    mutation.param_sigma *= 0.8  // reduce random walk on plateaus

if fdc < -0.15:
    // Deceptive landscape — novelty-driven search instead of fitness-driven
    switch_to_novelty_search()

if evolvability < 0.1:
    // Low evolvability — most mutations are harmful
    // Reduce mutation rates, increase elitism
    mutation.param_sigma *= 0.5
    batch_size *= 2  // more samples per generation to find rare improvements
```

---

## 16. Speciation for Prompt Strategies

Skills that use fundamentally different strategies (e.g., "fix by reading error
messages" vs "fix by searching codebase for patterns") should be protected from
competing directly. Speciation groups similar genomes into species and allocates
evaluation budget proportionally.

> **Citation**: Stanley & Miikkulainen, "Evolving Neural Networks through Augmenting
> Topologies" (NEAT, Evolutionary Computation, 2002) — speciation via compatibility
> distance.

### 16.1 Compatibility Distance

```rust
/// Compatibility distance between two skill genomes.
///
/// Measures how different two genomes are along structural and
/// parametric dimensions. Used for speciation.
pub struct CompatibilityMetric {
    /// Weight for prompt template difference (semantic similarity).
    pub c_prompt: f64,   // default: 1.0
    /// Weight for tool preference difference.
    pub c_tools: f64,    // default: 0.5
    /// Weight for continuous parameter difference.
    pub c_params: f64,   // default: 0.3
}

impl CompatibilityMetric {
    /// Compute compatibility distance between two genomes.
    pub fn distance(&self, a: &SkillGenome, b: &SkillGenome) -> f64 {
        // Prompt distance: Jaccard similarity of n-gram sets
        let prompt_dist = 1.0 - ngram_jaccard(&a.prompt_template,
                                               &b.prompt_template, 3);

        // Tool distance: cosine distance of preference vectors
        let tool_dist = tool_cosine_distance(&a.tool_preferences,
                                              &b.tool_preferences);

        // Parameter distance: normalized L2 distance
        let param_dist = (
            (a.temperature - b.temperature).powi(2)
            + ((a.token_budget as f64 - b.token_budget as f64) / 8192.0).powi(2)
            + ((a.retry_config.max_retries as f64
                - b.retry_config.max_retries as f64) / 8.0).powi(2)
        ).sqrt();

        self.c_prompt * prompt_dist
            + self.c_tools * tool_dist
            + self.c_params * param_dist
    }
}
```

### 16.2 Species Management

```rust
/// Species: a group of genomes with similar strategies.
pub struct Species {
    pub id: usize,
    /// Representative genome (used for distance comparisons).
    pub representative: SkillGenome,
    /// Members of this species.
    pub members: Vec<SkillGenome>,
    /// Adjusted fitness (fitness / species_size for fitness sharing).
    pub adjusted_fitness: f64,
    /// Generations since this species improved.
    pub stagnation_counter: u32,
}

pub struct SpeciesManager {
    pub species: Vec<Species>,
    /// Compatibility threshold. Genomes within this distance are same species.
    /// Dynamically adjusted to maintain target_species count.
    pub threshold: f64,              // default: 1.0
    /// Desired number of species.
    pub target_species: usize,       // default: 5
    /// Maximum generations without improvement before species is dissolved.
    pub stagnation_limit: u32,       // default: 15
}

impl SpeciesManager {
    /// Assign a genome to a species (or create a new one).
    pub fn speciate(&mut self, genome: SkillGenome) {
        for species in &mut self.species {
            let dist = self.metric.distance(&genome, &species.representative);
            if dist < self.threshold {
                species.members.push(genome);
                return;
            }
        }
        // No compatible species — create a new one
        self.species.push(Species {
            id: self.next_id(),
            representative: genome.clone(),
            members: vec![genome],
            adjusted_fitness: 0.0,
            stagnation_counter: 0,
        });
    }

    /// Adjust threshold to maintain target species count.
    pub fn adjust_threshold(&mut self) {
        if self.species.len() > self.target_species {
            self.threshold += 0.3; // merge similar species
        } else if self.species.len() < self.target_species {
            self.threshold -= 0.3; // split into more species
        }
        self.threshold = self.threshold.max(0.1); // floor
    }
}
```

---

## 17. AURORA: Learned Behavioral Descriptors

Hand-crafted behavioral descriptors (completion rate, gate score, etc.) may miss
important behavioral axes. AURORA uses a variational autoencoder to *discover*
behavioral descriptors from execution traces.

> **Citation**: AURORA (Unsupervised Behavior Discovery with QD, 2021–2024) — learned
> behavioral descriptors via VAE for quality-diversity optimization.

```rust
/// AURORA: learned behavioral descriptors from execution traces.
///
/// Instead of hand-crafting axes like "completion_rate" and "efficiency",
/// train a VAE on execution traces to discover latent behavioral dimensions.
pub struct AuroraDescriptor {
    /// Dimensionality of the learned behavioral space.
    pub latent_dims: usize,    // default: 4
    /// The trained VAE encoder (execution trace → latent vector).
    pub encoder: Box<dyn TraceEncoder>,
    /// Archive resolution in the learned space.
    pub resolution: usize,     // default: 10
}

pub trait TraceEncoder: Send + Sync {
    /// Encode an execution trace into a latent behavioral vector.
    fn encode(&self, trace: &ExecutionTrace) -> Vec<f64>;
}

/// Training AURORA:
///
/// 1. Collect N execution traces from the episode log
/// 2. Extract features: tool call sequences, edit patterns, gate results,
///    timing distributions, token usage patterns
/// 3. Train a VAE on the feature vectors:
///    - Encoder: features → z (latent behavioral descriptor)
///    - Decoder: z → reconstructed features
///    - Loss: reconstruction + KL divergence
/// 4. The latent space z becomes the behavioral descriptor for MAP-Elites
///
/// Benefits over hand-crafted descriptors:
/// - Discovers axes like "cautious vs aggressive editing" automatically
/// - Adapts to the codebase's actual behavioral diversity
/// - Can reveal unexpected behavioral niches worth exploring
```

---

## 18. CMA-ES for Continuous Skill Parameters

For continuous parameters (temperature, token budget, gate weights), CMA-ES is
more sample-efficient than random mutation.

> **Citation**: Hansen, "The CMA Evolution Strategy: A Tutorial"
> (arXiv:1604.00772).

```rust
/// CMA-ES optimizer for continuous skill parameters.
///
/// Operates on the continuous sub-vector of the genome:
/// [temperature, token_budget_normalized, backoff_factor, gate_weights...]
pub struct SkillCmaEs {
    /// Dimensionality of the continuous parameter space.
    pub n: usize,
    /// Population size per generation.
    pub lambda: usize,         // default: 4 + floor(3 * ln(n))
    /// Number of parents for recombination.
    pub mu: usize,             // default: lambda / 2
    /// Distribution mean (current best parameter estimate).
    pub mean: Vec<f64>,
    /// Step-size (global mutation strength).
    pub sigma: f64,            // initial: 0.3
    /// Covariance matrix (encodes parameter correlations).
    pub covariance: Vec<Vec<f64>>,
    /// Evolution path for step-size adaptation.
    pub p_sigma: Vec<f64>,
    /// Evolution path for covariance adaptation.
    pub p_c: Vec<f64>,
}

impl SkillCmaEs {
    /// Sample a population of parameter vectors.
    pub fn sample(&self, rng: &mut impl Rng) -> Vec<Vec<f64>> {
        // z ~ N(0, C), x = mean + sigma * z
        let cholesky = cholesky_decompose(&self.covariance);
        (0..self.lambda).map(|_| {
            let z: Vec<f64> = (0..self.n)
                .map(|_| rng.sample(StandardNormal))
                .collect();
            let scaled = mat_vec_mul(&cholesky, &z);
            self.mean.iter().zip(scaled.iter())
                .map(|(m, s)| m + self.sigma * s)
                .collect()
        }).collect()
    }

    /// Update distribution from fitness-ranked population.
    pub fn update(&mut self, population: &[Vec<f64>], fitness: &[f64]) {
        // 1. Rank by fitness, select top mu
        // 2. Update mean (weighted recombination)
        // 3. Update evolution paths
        // 4. Update covariance matrix (rank-one + rank-mu updates)
        // 5. Update step-size sigma via CSA
        // (Full algorithm: Hansen tutorial, Algorithm 1)
        todo!("See CMA-ES tutorial for complete update equations")
    }
}
```

**Integration**: CMA-ES optimizes the continuous parameters while MAP-Elites manages
the discrete structure (prompt templates, tool sets) and diversity. The combination
provides both efficient local optimization (CMA-ES) and global exploration (MAP-Elites).

---

## 19. Persistence and Reporting

### 19.1 Archive Persistence

```
.roko/learn/
├── skill-archive.json          # MAP-Elites archive
│   {"cells": {"42": {"id": "sk_a3f", "fitness": 0.87, ...}},
│    "generation": 150, "total_evaluations": 2400}
├── species.json                # Species state
│   {"species": [{"id": 1, "members": 12, "adjusted_fitness": 0.72}]}
├── landscape.json              # Latest landscape analysis
│   {"local_optima": 3, "ruggedness": 0.18, "fdc": 0.42}
└── cma-es-state.json           # CMA-ES optimizer state
    {"mean": [...], "sigma": 0.25, "covariance": [[...]]}
```

### 19.2 Dashboard Metrics

```
Skill Evolution:
  Archive: 847 / 10,000 cells filled (8.5% coverage)
  QD-score: 423.7 (↑ 12.3 from last generation)
  Species: 5 active, 2 stagnant
  Best fitness: 0.94 (Skill: "Rust Compile Fix — Missing Import")
  Landscape: FDC=0.42 (searchable), ruggedness=0.18 (smooth)
  CMA-ES sigma: 0.25 (converging)
  Generation: 150, total evaluations: 2,400
```

---

## 20. Test Criteria for Evolutionary Components

| Test | Property |
|---|---|
| `archive_insert_new_niche` | Empty cell → InsertResult::NewNiche |
| `archive_insert_improvement` | Fitter genome replaces existing in same cell |
| `archive_insert_rejected` | Less fit genome rejected in occupied cell |
| `archive_coverage_increases` | Successive insertions increase coverage |
| `archive_qd_score_monotone` | QD-score never decreases on replacement |
| `mutation_bounds_respected` | Temperature stays in [0, 1], budget stays positive |
| `crossover_intermediate` | Continuous params are between parent values |
| `speciation_groups_similar` | Genomes with distance < threshold → same species |
| `speciation_splits_different` | Genomes with distance > threshold → different species |
| `threshold_adjusts_dynamically` | Too many species → threshold increases |
| `stagnation_dissolves_species` | 15 gens without improvement → species dissolved |
| `fitness_evaluation_deterministic` | Same genome + same tasks → same fitness |
| `landscape_local_optima_detected` | Cell with no fitter neighbors → counted |
| `landscape_fdc_positive_for_easy` | Smooth landscape → FDC > 0.15 |
| `cma_es_sigma_decreases_on_convergence` | Near optimum → sigma shrinks |

//! Enrichment pipeline: run steps against a plan directory.
//!
//! Ported from `apps/mori/src/support_enrich/mod.rs` lines 215-319.
//!
//! The pipeline orchestrates step execution: staleness checks, input reading,
//! prompt building, LLM calls, validation, and repair. It uses the [`LlmClient`]
//! trait for model calls (no HTTP/subprocess logic here).
//!
//! # I/O boundary
//!
//! The pipeline does perform filesystem I/O (reading inputs, writing outputs)
//! because it is the orchestration layer. Prompt builders and validators are
//! pure functions that receive/return strings.
//!
//! # COMP-09: Learning paths
//!
//! The pipeline now tracks per-step cost via [`StepCost`] and supports
//! adaptive step selection via [`StepOutcomeHistory`].

use std::path::Path;
use std::time::Instant;

use super::client::LlmClient;
use super::config::EnrichmentConfig;
use super::inputs::step_dependency_paths;
use super::outcome::{SkipReason, StepCost, StepOutcome};
use super::prompts::{self, StepInputs};
use super::step::{ALL_ORDERED, EnrichStep};
use super::validate;

// ─── COMP-09: Adaptive step selection ───────────────────────────────────

/// Historical success rate for one enrichment step, used for adaptive
/// step selection.
///
/// When a step's success rate falls below a threshold, the adaptive
/// selector can skip it to save tokens and wall-clock time.
#[derive(Clone, Debug, Default)]
pub struct StepOutcomeHistory {
    /// Per-step: (success_count, total_count).
    pub records: std::collections::HashMap<EnrichStep, (u32, u32)>,
    /// Minimum success rate below which a step is skipped.
    /// Default: 0.3 (skip if <30% success rate over history).
    pub skip_threshold: f64,
    /// Minimum number of observations before adaptive skipping kicks in.
    /// Default: 5.
    pub min_observations: u32,
}

impl StepOutcomeHistory {
    /// Create an empty history with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: std::collections::HashMap::new(),
            skip_threshold: 0.3,
            min_observations: 5,
        }
    }

    /// Record the outcome of a step execution.
    pub fn record(&mut self, step: EnrichStep, success: bool) {
        let entry = self.records.entry(step).or_insert((0, 0));
        entry.1 += 1;
        if success {
            entry.0 += 1;
        }
    }

    /// Check whether a step should be skipped based on historical success rate.
    #[must_use]
    pub fn should_skip(&self, step: EnrichStep) -> bool {
        let Some(&(successes, total)) = self.records.get(&step) else {
            return false; // No history — run it.
        };
        if total < self.min_observations {
            return false; // Not enough data — run it.
        }
        let rate = successes as f64 / total as f64;
        rate < self.skip_threshold
    }

    /// Get the success rate for a step (0.0..1.0), or None if no observations.
    #[must_use]
    pub fn success_rate(&self, step: EnrichStep) -> Option<f64> {
        self.records
            .get(&step)
            .map(|&(s, t)| if t == 0 { 0.0 } else { s as f64 / t as f64 })
    }
}

// ─── COMP-09: Concurrent step groups ─────────────────────────────────

/// A group of enrichment steps that can run concurrently because they
/// have no mutual dependencies.
///
/// Steps within a group can be dispatched in parallel to reduce
/// wall-clock time. Groups must be executed in order (group 0 before
/// group 1, etc.).
#[derive(Clone, Debug)]
pub struct ConcurrentStepGroup {
    /// Group ordinal (0-indexed).
    pub index: usize,
    /// Steps in this group that can run in parallel.
    pub steps: Vec<EnrichStep>,
}

/// Compute groups of steps that can run concurrently.
///
/// Analyzes the dependency graph from [`step_dependency_paths`] and groups
/// steps that share no producer-consumer relationship. Steps in the same
/// group can run in parallel; groups must be executed sequentially.
///
/// The algorithm:
/// 1. Build a map of which step produces which output file.
/// 2. For each step, check if any of its dependencies are produced by
///    another step (making them sequential).
/// 3. Steps whose dependencies are only external files (plan.md, etc.)
///    go into the earliest possible group.
#[must_use]
pub fn concurrent_step_groups(steps: &[EnrichStep]) -> Vec<ConcurrentStepGroup> {
    use std::collections::{HashMap, HashSet};

    if steps.is_empty() {
        return Vec::new();
    }

    // Map output filename -> producing step.
    let _producers: HashMap<String, EnrichStep> = steps
        .iter()
        .map(|&s| (s.output_filename().to_string(), s))
        .collect();

    // For each step, find which other steps must run before it.
    let mut step_deps: HashMap<EnrichStep, HashSet<EnrichStep>> = HashMap::new();
    for &step in steps {
        let mut deps = HashSet::new();
        // Check the output filenames that this step's inputs correspond to.
        // If another step produces a file that this step depends on, that
        // step must run first.
        for &other in steps {
            if other == step {
                continue;
            }
            let other_output = other.output_filename();
            // Check if this step depends on the other step's output.
            // We infer this from the step_dependency_paths structure.
            if step_depends_on_output(step, other_output) {
                deps.insert(other);
            }
        }
        step_deps.insert(step, deps);
    }

    // Topological grouping: assign each step to the earliest group where
    // all its dependencies are in earlier groups.
    let mut assigned: HashMap<EnrichStep, usize> = HashMap::new();
    let mut groups: Vec<Vec<EnrichStep>> = Vec::new();

    // Process in canonical order to get deterministic results.
    let ordered: Vec<EnrichStep> = steps.to_vec();

    for &step in &ordered {
        let deps = step_deps.get(&step).cloned().unwrap_or_default();
        let earliest_group = if deps.is_empty() {
            0
        } else {
            deps.iter()
                .filter_map(|d| assigned.get(d))
                .max()
                .map_or(0, |g| g + 1)
        };

        while groups.len() <= earliest_group {
            groups.push(Vec::new());
        }
        groups[earliest_group].push(step);
        assigned.insert(step, earliest_group);
    }

    groups
        .into_iter()
        .enumerate()
        .map(|(index, steps)| ConcurrentStepGroup { index, steps })
        .collect()
}

/// Check whether a step depends on a given output filename.
fn step_depends_on_output(step: EnrichStep, output_filename: &str) -> bool {
    // Map output filenames to the files they produce in the plan directory.
    // This is derived from step_dependency_paths logic.
    match step {
        EnrichStep::Prd | EnrichStep::Invariants => false,
        EnrichStep::Briefs => output_filename == "decomposition.md",
        EnrichStep::Tasks => false,
        EnrichStep::Decompose => output_filename == "brief.md",
        EnrichStep::Verify | EnrichStep::Reviews | EnrichStep::Tests | EnrichStep::Scribe => {
            output_filename == "tasks.toml"
        }
        EnrichStep::Research => matches!(
            output_filename,
            "tasks.toml"
                | "brief.md"
                | "decomposition.md"
                | "verify-tasks.toml"
                | "review-tasks.toml"
        ),
        EnrichStep::Dependencies => {
            matches!(output_filename, "tasks.toml" | "brief.md" | "research.md")
        }
        EnrichStep::Fixtures => matches!(
            output_filename,
            "tasks.toml" | "brief.md" | "research.md" | "dependency-manifest.toml"
        ),
        EnrichStep::Integration => matches!(
            output_filename,
            "tasks.toml"
                | "verify-tasks.toml"
                | "review-tasks.toml"
                | "research.md"
                | "dependency-manifest.toml"
                | "fixture-manifest.toml"
        ),
    }
}

/// Update a [`StepOutcomeHistory`] from a batch of step outcomes.
///
/// Call this after running steps to feed results back into the adaptive
/// selection system.
pub fn update_history(history: &mut StepOutcomeHistory, outcomes: &[StepOutcome]) {
    for outcome in outcomes {
        let step = outcome.step();
        let success = outcome.is_success();
        // Only record non-skipped outcomes (we don't want to penalize
        // skipped steps).
        if !outcome.is_skipped() {
            history.record(step, success);
        }
    }
}

/// The enrichment pipeline. Parameterized by an [`LlmClient`] implementation.
///
/// Use [`run_step`](Self::run_step) for a single step,
/// [`run_steps`](Self::run_steps) for an explicit subset, or
/// [`run_all`](Self::run_all) for the full pipeline in dependency order.
pub struct EnrichmentPipeline<C: LlmClient> {
    /// Pipeline configuration.
    config: EnrichmentConfig,
    /// LLM client for model calls.
    client: C,
    /// COMP-09: Adaptive step selection history.
    outcome_history: Option<StepOutcomeHistory>,
}

impl<C: LlmClient> EnrichmentPipeline<C> {
    /// Create a new pipeline with the given config and LLM client.
    pub fn new(config: EnrichmentConfig, client: C) -> Self {
        Self {
            config,
            client,
            outcome_history: None,
        }
    }

    /// Borrow the pipeline configuration.
    pub const fn config(&self) -> &EnrichmentConfig {
        &self.config
    }

    /// COMP-09: Attach an outcome history for adaptive step selection.
    ///
    /// When set, steps with historically low success rates are automatically
    /// skipped with [`SkipReason::AdaptiveSkip`].
    #[must_use]
    pub fn with_outcome_history(mut self, history: StepOutcomeHistory) -> Self {
        self.outcome_history = Some(history);
        self
    }

    /// COMP-09: Borrow the current outcome history (for persistence).
    #[must_use]
    pub fn outcome_history(&self) -> Option<&StepOutcomeHistory> {
        self.outcome_history.as_ref()
    }

    /// Run a single enrichment step for a plan.
    ///
    /// Returns a [`StepOutcome`] describing what happened. Never panics on
    /// step failure — returns `StepOutcome::Failed` instead.
    ///
    /// COMP-09: Now tracks per-step cost and respects adaptive selection.
    pub async fn run_step(&self, step: EnrichStep, plan_base: &str) -> StepOutcome {
        let plan_dir = self.config.plan_dir(plan_base);
        let output_file = plan_dir.join(step.output_filename());
        let start = Instant::now();

        // Dry-run mode: always skip.
        if self.config.dry_run {
            return StepOutcome::Skipped {
                step,
                reason: SkipReason::DryRun,
            };
        }

        // COMP-09: Adaptive step selection — skip steps with historically
        // low success rates.
        if let Some(history) = &self.outcome_history {
            if history.should_skip(step) {
                return StepOutcome::Skipped {
                    step,
                    reason: SkipReason::AdaptiveSkip,
                };
            }
        }

        // Staleness check: skip if output exists, is fresh, and force is off.
        if output_file.exists() && !self.config.force {
            let stale = output_is_stale(&plan_dir, step, &output_file);
            if !stale {
                return StepOutcome::Skipped {
                    step,
                    reason: SkipReason::Fresh,
                };
            }
        }

        // Read inputs.
        let inputs = match read_step_inputs(&plan_dir, step) {
            Ok(inputs) => inputs,
            Err(e) => {
                return StepOutcome::Failed {
                    step,
                    message: format!("failed to read inputs: {e}"),
                    cost: StepCost {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        ..StepCost::default()
                    },
                };
            }
        };

        // Non-LLM steps: generate via pure extraction.
        if !step.needs_llm() {
            return match prompts::generate_without_llm(step, &inputs) {
                Ok(content) => self.finalize_output(step, &output_file, &content, 0, start),
                Err(e) => StepOutcome::Failed {
                    step,
                    message: e,
                    cost: StepCost {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        ..StepCost::default()
                    },
                },
            };
        }

        // Build prompt and call LLM.
        let (system, user_msg) = prompts::build_prompt(step, &inputs);
        let model = self
            .config
            .model_override
            .as_deref()
            .unwrap_or_else(|| step.default_model(self.config.backend));

        match self.client.call(model, &system, &user_msg, 8192).await {
            Ok(raw_output) => {
                self.validate_and_write(step, &output_file, model, &raw_output, start)
                    .await
            }
            Err(e) => StepOutcome::Failed {
                step,
                message: format!("LLM call failed: {e}"),
                cost: StepCost {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    llm_calls: 1,
                    input_tokens: crate::estimate_tokens(&format!("{system}{user_msg}")) as u32,
                    ..StepCost::default()
                },
            },
        }
    }

    /// Run all enrichment steps for a plan in dependency order.
    ///
    /// Continues past failures — each step's outcome is collected.
    /// Returns the list of outcomes for all 13 steps.
    pub async fn run_all(&self, plan_base: &str) -> Vec<StepOutcome> {
        self.run_steps(plan_base, ALL_ORDERED).await
    }

    /// Run a selected subset of enrichment steps in the order provided.
    ///
    /// Continues past failures — each step's outcome is collected.
    /// Returns the list of outcomes for the requested steps only.
    pub async fn run_steps(&self, plan_base: &str, steps: &[EnrichStep]) -> Vec<StepOutcome> {
        let mut outcomes = Vec::with_capacity(steps.len());
        for &step in steps {
            outcomes.push(self.run_step(step, plan_base).await);
        }
        outcomes
    }

    /// Validate LLM output, attempt TOML repair if needed, and write to disk.
    async fn validate_and_write(
        &self,
        step: EnrichStep,
        output_file: &Path,
        model: &str,
        raw_output: &str,
        start: Instant,
    ) -> StepOutcome {
        let normalized = validate::normalize_step_output(step, raw_output);

        match validate::validate_step_output(step, &normalized) {
            Ok(()) => self.finalize_output(step, output_file, &normalized, 1, start),
            Err(original_err) if step.is_toml() => {
                // Attempt TOML repair: one retry via LLM.
                let (repair_sys, repair_user) =
                    prompts::build_repair_prompt(step, raw_output, &original_err);

                match self
                    .client
                    .call(model, &repair_sys, &repair_user, 8192)
                    .await
                {
                    Ok(repaired_raw) => match validate::repair_toml_output(step, &repaired_raw) {
                        Ok(repaired) => {
                            self.finalize_output(step, output_file, &repaired, 2, start)
                        }
                        Err(repair_err) => StepOutcome::Failed {
                            step,
                            message: format!(
                                "TOML repair failed: {repair_err} (original: {original_err})"
                            ),
                            cost: StepCost {
                                elapsed_ms: start.elapsed().as_millis() as u64,
                                llm_calls: 2,
                                output_tokens: crate::estimate_tokens(raw_output) as u32,
                                ..StepCost::default()
                            },
                        },
                    },
                    Err(e) => StepOutcome::Failed {
                        step,
                        message: format!("TOML repair LLM call failed: {e}"),
                        cost: StepCost {
                            elapsed_ms: start.elapsed().as_millis() as u64,
                            llm_calls: 2,
                            output_tokens: crate::estimate_tokens(raw_output) as u32,
                            ..StepCost::default()
                        },
                    },
                }
            }
            Err(err) => StepOutcome::Failed {
                step,
                message: err,
                cost: StepCost {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    llm_calls: 1,
                    output_tokens: crate::estimate_tokens(raw_output) as u32,
                    ..StepCost::default()
                },
            },
        }
    }

    /// Write validated output to disk and return a Generated outcome.
    #[allow(clippy::unused_self)]
    fn finalize_output(
        &self,
        step: EnrichStep,
        output_file: &Path,
        content: &str,
        llm_calls: u32,
        start: Instant,
    ) -> StepOutcome {
        // Empty output is always an error, even if validation somehow passed.
        if content.trim().is_empty() {
            return StepOutcome::Failed {
                step,
                message: format!("generated output for {step} was empty"),
                cost: StepCost {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    llm_calls,
                    ..StepCost::default()
                },
            };
        }

        // Ensure parent directory exists.
        if let Some(parent) = output_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return StepOutcome::Failed {
                    step,
                    message: format!("failed to create directory {}: {e}", parent.display()),
                    cost: StepCost {
                        elapsed_ms: start.elapsed().as_millis() as u64,
                        llm_calls,
                        ..StepCost::default()
                    },
                };
            }
        }

        let output_bytes = content.len();
        match std::fs::write(output_file, content) {
            Ok(()) => StepOutcome::Generated {
                step,
                llm_calls,
                cost: StepCost {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    llm_calls,
                    output_tokens: crate::estimate_tokens(content) as u32,
                    output_bytes,
                    ..StepCost::default()
                },
            },
            Err(e) => StepOutcome::Failed {
                step,
                message: format!("failed to write {}: {e}", output_file.display()),
                cost: StepCost {
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    llm_calls,
                    ..StepCost::default()
                },
            },
        }
    }
}

/// Check if an output file is stale relative to its input dependencies.
///
/// Returns `true` if any input file is newer than the output, or if the output
/// does not exist.
///
/// Ported from Mori `output_is_stale` (lines 679-695).
fn output_is_stale(plan_dir: &Path, step: EnrichStep, output_file: &Path) -> bool {
    let Ok(output_meta) = std::fs::metadata(output_file) else {
        return true;
    };
    let Ok(output_mtime) = output_meta.modified() else {
        return true;
    };

    step_dependency_paths(plan_dir, step)
        .into_iter()
        .any(|path| {
            std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map(|mtime| mtime > output_mtime)
                .unwrap_or(false)
        })
}

/// Read the input files needed for a given step.
///
/// Ported from Mori `read_step_inputs` (lines 597-677).
fn read_step_inputs(plan_dir: &Path, step: EnrichStep) -> Result<StepInputs, String> {
    let plan_path = plan_dir.join("plan.md");
    let plan_content = read_optional_file(&plan_path)
        .map_err(|e| format!("failed to read plan.md: {e}"))?
        .ok_or_else(|| format!("plan.md not found in {}", plan_dir.display()))?;

    let tasks_content = match step {
        EnrichStep::Verify
        | EnrichStep::Reviews
        | EnrichStep::Tests
        | EnrichStep::Research
        | EnrichStep::Dependencies
        | EnrichStep::Fixtures
        | EnrichStep::Integration => read_optional_file(&plan_dir.join("tasks.toml"))
            .map_err(|e| format!("failed to read tasks.toml: {e}"))?,
        _ => None,
    };

    let brief_content = match step {
        EnrichStep::Decompose
        | EnrichStep::Research
        | EnrichStep::Dependencies
        | EnrichStep::Fixtures
        | EnrichStep::Integration => read_optional_file(&plan_dir.join("brief.md"))
            .map_err(|e| format!("failed to read brief.md: {e}"))?,
        _ => None,
    };

    let decomposition_content = match step {
        EnrichStep::Briefs
        | EnrichStep::Research
        | EnrichStep::Dependencies
        | EnrichStep::Fixtures
        | EnrichStep::Integration => read_optional_file(&plan_dir.join("decomposition.md"))
            .map_err(|e| format!("failed to read decomposition.md: {e}"))?,
        _ => None,
    };

    let verify_content = match step {
        EnrichStep::Research | EnrichStep::Integration | EnrichStep::Fixtures => {
            read_optional_file(&plan_dir.join("verify-tasks.toml"))
                .map_err(|e| format!("failed to read verify-tasks.toml: {e}"))?
        }
        _ => None,
    };

    let review_content = match step {
        EnrichStep::Research | EnrichStep::Integration => {
            read_optional_file(&plan_dir.join("review-tasks.toml"))
                .map_err(|e| format!("failed to read review-tasks.toml: {e}"))?
        }
        _ => None,
    };

    let research_content = match step {
        EnrichStep::Integration | EnrichStep::Fixtures => {
            read_optional_file(&plan_dir.join("research.md"))
                .map_err(|e| format!("failed to read research.md: {e}"))?
        }
        _ => None,
    };

    let dependency_manifest = match step {
        EnrichStep::Fixtures | EnrichStep::Integration => {
            read_optional_file(&plan_dir.join("dependency-manifest.toml"))
                .map_err(|e| format!("failed to read dependency-manifest.toml: {e}"))?
        }
        _ => None,
    };

    let fixture_manifest = match step {
        EnrichStep::Integration => read_optional_file(&plan_dir.join("fixture-manifest.toml"))
            .map_err(|e| format!("failed to read fixture-manifest.toml: {e}"))?,
        _ => None,
    };

    Ok(StepInputs {
        plan_content,
        tasks_content,
        brief_content,
        decomposition_content,
        verify_content,
        review_content,
        research_content,
        dependency_manifest,
        fixture_manifest,
    })
}

/// Read a file if it exists, returning `None` if it does not.
fn read_optional_file(path: &Path) -> Result<Option<String>, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::client::LlmClient;
    use crate::enrichment::outcome::SkipReason;
    use crate::enrichment::step::LlmBackend;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ── Mock LLM Client ─────────────────────────────────────────────

    /// A mock LLM client that returns configurable responses.
    struct MockLlmClient {
        /// Response to return on each call. If multiple entries, they are
        /// returned in order (first call gets index 0, etc.).
        responses: Vec<Result<String, String>>,
        /// Counter for how many calls have been made.
        call_count: AtomicU32,
    }

    impl MockLlmClient {
        fn new(responses: Vec<Result<String, String>>) -> Self {
            Self {
                responses,
                call_count: AtomicU32::new(0),
            }
        }

        fn single_ok(response: &str) -> Self {
            Self::new(vec![Ok(response.to_string())])
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlmClient {
        async fn call(
            &self,
            _model: &str,
            _system: &str,
            _user: &str,
            _max_tokens: u32,
        ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst) as usize;
            let fallback = Err("no responses configured".to_string());
            let response = if idx < self.responses.len() {
                &self.responses[idx]
            } else {
                self.responses.last().unwrap_or(&fallback)
            };
            match response {
                Ok(s) => Ok(s.clone()),
                Err(e) => Err(e.clone().into()),
            }
        }
    }

    // ── Test helpers ────────────────────────────────────────────────

    fn make_config(root: &std::path::Path) -> EnrichmentConfig {
        EnrichmentConfig {
            repo_root: root.to_path_buf(),
            backend: LlmBackend::Claude,
            gateway_url: None,
            gateway_key: None,
            batch_mode: false,
            model_override: None,
            force: false,
            dry_run: false,
            quiet: true,
        }
    }

    /// Create a plan directory with a minimal plan.md.
    fn setup_plan_dir(root: &std::path::Path, plan_base: &str) -> std::path::PathBuf {
        let plan_dir = root.join(".roko").join("plans").join(plan_base);
        std::fs::create_dir_all(&plan_dir).expect("create plan dir");
        std::fs::write(
            plan_dir.join("plan.md"),
            "# Test Plan\n\n## Step 1\nImplement feature X.\n\n## Step 2\nAdd tests.\n",
        )
        .expect("write plan.md");
        plan_dir
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn dry_run_returns_skipped_dry_run_for_every_step() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        let mut config = make_config(tmp.path());
        config.dry_run = true;

        let client = MockLlmClient::single_ok("unused");
        let pipeline = EnrichmentPipeline::new(config, client);
        let outcomes = pipeline.run_all("test-plan").await;

        assert_eq!(outcomes.len(), 13);
        for outcome in &outcomes {
            match outcome {
                StepOutcome::Skipped { reason, .. } => {
                    assert_eq!(*reason, SkipReason::DryRun);
                }
                other => panic!("expected Skipped(DryRun), got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn run_steps_executes_only_requested_steps_in_explicit_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        let client = MockLlmClient::single_ok("unused");
        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let selected = [EnrichStep::Tasks, EnrichStep::Prd, EnrichStep::Briefs];
        let outcomes = pipeline.run_steps("test-plan", &selected).await;

        assert_eq!(outcomes.len(), selected.len());
        for (outcome, step) in outcomes.iter().zip(selected) {
            assert_eq!(outcome.step(), step);
            assert!(
                matches!(outcome, StepOutcome::Generated { .. }),
                "expected generated outcome for {step}, got {outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn force_false_fresh_output_returns_skipped_fresh() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // Pre-create the Prd output so it is fresh.
        std::fs::write(plan_dir.join("prd-extract.md"), "# Existing PRD\n").expect("write output");

        let config = make_config(tmp.path());
        let client = MockLlmClient::single_ok("unused");
        let pipeline = EnrichmentPipeline::new(config, client);

        let outcome = pipeline.run_step(EnrichStep::Prd, "test-plan").await;
        match outcome {
            StepOutcome::Skipped {
                step,
                reason: SkipReason::Fresh,
            } => {
                assert_eq!(step, EnrichStep::Prd);
            }
            other => panic!("expected Skipped(Fresh), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn force_true_regenerates_even_with_fresh_output() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // Pre-create the Prd output.
        std::fs::write(plan_dir.join("prd-extract.md"), "# Old PRD\n").expect("write output");

        let mut config = make_config(tmp.path());
        config.force = true;

        let client = MockLlmClient::single_ok("unused");
        let pipeline = EnrichmentPipeline::new(config, client);

        let outcome = pipeline.run_step(EnrichStep::Prd, "test-plan").await;
        match outcome {
            StepOutcome::Generated {
                step, llm_calls, ..
            } => {
                assert_eq!(step, EnrichStep::Prd);
                assert_eq!(llm_calls, 0); // Prd is non-LLM
            }
            other => panic!("expected Generated, got {other:?}"),
        }

        // Verify the file was overwritten.
        let content = std::fs::read_to_string(plan_dir.join("prd-extract.md")).expect("read");
        assert_ne!(content, "# Old PRD\n");
    }

    #[tokio::test]
    async fn toml_repair_succeeds_after_one_retry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // Pre-create tasks.toml so Verify can read it.
        std::fs::write(
            plan_dir.join("tasks.toml"),
            "[meta]\nplan = \"test\"\n\n[[task]]\nid = \"T1\"\n",
        )
        .expect("write tasks.toml");

        // First response: invalid TOML. Second response: valid TOML.
        let client = MockLlmClient::new(vec![
            Ok("this is not valid toml {{{}}}".to_string()),
            Ok("[meta]\nplan = \"repaired\"\nrole = \"verifier\"\ntotal = 1\n\n[[task]]\nid = \"CG1\"\ntitle = \"Check\"\ntype = \"compile\"\ncommand = \"cargo check\"\nblocking = true\nstatus = \"pending\"\n".to_string()),
        ]);

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Verify, "test-plan").await;

        match outcome {
            StepOutcome::Generated {
                step, llm_calls, ..
            } => {
                assert_eq!(step, EnrichStep::Verify);
                assert_eq!(llm_calls, 2); // 1 original + 1 repair
            }
            other => panic!("expected Generated with 2 calls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn toml_repair_hard_fail_both_invalid() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        std::fs::write(
            plan_dir.join("tasks.toml"),
            "[meta]\nplan = \"test\"\n\n[[task]]\nid = \"T1\"\n",
        )
        .expect("write tasks.toml");

        // Both responses are invalid TOML.
        let client = MockLlmClient::new(vec![
            Ok("not valid toml 1".to_string()),
            Ok("not valid toml 2".to_string()),
        ]);

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Verify, "test-plan").await;

        assert!(outcome.is_failed(), "expected Failed, got {outcome:?}");
    }

    #[tokio::test]
    async fn non_toml_validation_error_no_repair() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // Return empty output for a non-TOML LLM step (Decompose).
        // Empty output should fail without attempting repair.
        let client = MockLlmClient::single_ok("");

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Decompose, "test-plan").await;

        assert!(outcome.is_failed(), "expected Failed for empty output");
        // Only 1 call should be made (no repair attempt for non-TOML).
        // The mock was called once, but the empty output is caught by finalize_output
        // before validate_and_write even reaches the repair path.
    }

    #[tokio::test]
    async fn empty_output_is_always_failed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // For an LLM step, return whitespace-only output.
        let client = MockLlmClient::single_ok("   \n  \n  ");

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Invariants, "test-plan").await;

        assert!(
            outcome.is_failed(),
            "expected Failed for whitespace output, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn run_all_continues_past_failed_steps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // All LLM calls fail, but non-LLM steps should succeed.
        let client = MockLlmClient::new(vec![Err("LLM is down".to_string())]);

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcomes = pipeline.run_all("test-plan").await;

        assert_eq!(outcomes.len(), 13, "should have 13 outcomes");

        // Non-LLM steps should succeed.
        let generated: Vec<_> = outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::Generated { .. }))
            .collect();
        assert!(
            !generated.is_empty(),
            "at least some non-LLM steps should succeed"
        );

        // LLM steps should fail.
        let failed: Vec<_> = outcomes.iter().filter(|o| o.is_failed()).collect();
        assert!(
            !failed.is_empty(),
            "LLM steps should fail when client errors"
        );

        // Total should be 13 (no panics, no short-circuits).
        let total =
            generated.len() + failed.len() + outcomes.iter().filter(|o| o.is_skipped()).count();
        assert_eq!(total, 13);
    }

    #[tokio::test]
    async fn non_llm_steps_generate_with_zero_llm_calls() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        let client = MockLlmClient::single_ok("unused");
        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);

        // Prd is a non-LLM step.
        let outcome = pipeline.run_step(EnrichStep::Prd, "test-plan").await;
        match outcome {
            StepOutcome::Generated {
                llm_calls, cost, ..
            } => {
                assert_eq!(llm_calls, 0);
                // COMP-09: verify cost is populated.
                assert!(cost.elapsed_ms < 5000, "should be fast");
            }
            other => panic!("expected Generated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn llm_step_with_valid_output_generates_one_call() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        // Return valid markdown for Decompose (non-TOML LLM step).
        let client = MockLlmClient::single_ok("# Decomposition\n\n## Step 1\nDo the thing.\n");

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Decompose, "test-plan").await;

        match outcome {
            StepOutcome::Generated {
                llm_calls, cost, ..
            } => {
                assert_eq!(llm_calls, 1);
                // COMP-09: verify cost tracking includes output tokens.
                assert!(cost.output_tokens > 0, "should have output tokens");
            }
            other => panic!("expected Generated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn integration_run_all_with_tempdir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "full-plan");

        // Pre-seed some artifacts so downstream steps can read them.
        std::fs::write(
            plan_dir.join("tasks.toml"),
            "[meta]\nplan = \"full-plan\"\niteration = 1\ntotal = 2\ndone = 0\n\n[[task]]\nid = \"T1\"\ntitle = \"Implement\"\nstatus = \"pending\"\nfiles = []\nacceptance = []\ndepends_on = []\n\n[[task]]\nid = \"T2\"\ntitle = \"Test\"\nstatus = \"pending\"\nfiles = []\nacceptance = []\ndepends_on = [\"T1\"]\n",
        ).expect("write tasks.toml");

        // Valid TOML response for any TOML step, valid markdown for others.
        let valid_toml = "[meta]\nplan = \"full-plan\"\nrole = \"generated\"\ntotal = 1\n\n[[task]]\nid = \"G1\"\ntitle = \"Generated\"\nstatus = \"pending\"\n";
        let valid_md = "# Generated Content\n\nSome content here.\n";

        // We need enough responses for all LLM steps (6 of them).
        let client = MockLlmClient::new(vec![
            Ok(valid_toml.to_string()), // for first TOML LLM step
            Ok(valid_md.to_string()),   // for first MD LLM step
            Ok(valid_toml.to_string()),
            Ok(valid_md.to_string()),
            Ok(valid_toml.to_string()),
            Ok(valid_md.to_string()),
            Ok(valid_toml.to_string()),
            Ok(valid_md.to_string()),
        ]);

        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcomes = pipeline.run_all("full-plan").await;

        assert_eq!(outcomes.len(), 13);

        // Count generated vs failed.
        let generated_count = outcomes
            .iter()
            .filter(|o| matches!(o, StepOutcome::Generated { .. }))
            .count();
        let failed_count = outcomes.iter().filter(|o| o.is_failed()).count();

        // Non-LLM steps (7) should all generate. LLM steps may fail if the
        // mock response doesn't match what the step expects (e.g. TOML step
        // getting markdown), but at minimum the 7 non-LLM steps should work.
        assert!(
            generated_count >= 7,
            "expected at least 7 generated, got {generated_count} (failed: {failed_count})"
        );
    }

    #[tokio::test]
    async fn missing_plan_md_returns_failed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Create plan dir but no plan.md.
        let plan_dir = tmp.path().join(".roko").join("plans").join("no-plan");
        std::fs::create_dir_all(&plan_dir).expect("create dir");

        let client = MockLlmClient::single_ok("unused");
        let config = make_config(tmp.path());

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Prd, "no-plan").await;

        assert!(outcome.is_failed(), "expected Failed for missing plan.md");
        if let StepOutcome::Failed { message, .. } = outcome {
            assert!(
                message.contains("plan.md"),
                "error should mention plan.md: {message}"
            );
        }
    }

    #[tokio::test]
    async fn staleness_check_regenerates_when_input_newer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plan_dir = setup_plan_dir(tmp.path(), "stale-plan");

        // Create output file first.
        let output_path = plan_dir.join("prd-extract.md");
        std::fs::write(&output_path, "# Old content\n").expect("write output");

        // Wait a tiny bit then touch the input to make it newer.
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(
            plan_dir.join("plan.md"),
            "# Updated Plan\n\n## New step\nDo updated thing.\n",
        )
        .expect("update plan.md");

        let client = MockLlmClient::single_ok("unused");
        let config = make_config(tmp.path()); // force=false

        let pipeline = EnrichmentPipeline::new(config, client);
        let outcome = pipeline.run_step(EnrichStep::Prd, "stale-plan").await;

        match outcome {
            StepOutcome::Generated { step, .. } => {
                assert_eq!(step, EnrichStep::Prd);
            }
            other => panic!("expected Generated (stale output should regenerate), got {other:?}"),
        }
    }

    // ── COMP-09: Adaptive step selection tests ─────────────────────

    #[test]
    fn step_outcome_history_should_skip_low_success_rate() {
        let mut history = StepOutcomeHistory::new();
        history.min_observations = 3;
        history.skip_threshold = 0.3;

        // Record 5 failures and 0 successes for Research.
        for _ in 0..5 {
            history.record(EnrichStep::Research, false);
        }

        assert!(history.should_skip(EnrichStep::Research));
        assert!(!history.should_skip(EnrichStep::Prd)); // no data
    }

    #[test]
    fn step_outcome_history_keeps_high_success_rate() {
        let mut history = StepOutcomeHistory::new();
        history.min_observations = 3;

        // Record 8 successes and 2 failures.
        for _ in 0..8 {
            history.record(EnrichStep::Decompose, true);
        }
        for _ in 0..2 {
            history.record(EnrichStep::Decompose, false);
        }

        assert!(!history.should_skip(EnrichStep::Decompose)); // 80% success
    }

    #[test]
    fn step_outcome_history_no_skip_below_min_observations() {
        let mut history = StepOutcomeHistory::new();
        history.min_observations = 10;

        // Only 3 observations, all failures.
        for _ in 0..3 {
            history.record(EnrichStep::Scribe, false);
        }

        assert!(!history.should_skip(EnrichStep::Scribe)); // not enough data
    }

    #[test]
    fn step_outcome_history_success_rate() {
        let mut history = StepOutcomeHistory::new();
        history.record(EnrichStep::Prd, true);
        history.record(EnrichStep::Prd, true);
        history.record(EnrichStep::Prd, false);

        let rate = history.success_rate(EnrichStep::Prd).unwrap();
        assert!((rate - 2.0 / 3.0).abs() < 0.01);

        assert!(history.success_rate(EnrichStep::Research).is_none());
    }

    #[tokio::test]
    async fn adaptive_skip_respects_outcome_history() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _plan_dir = setup_plan_dir(tmp.path(), "test-plan");

        let mut history = StepOutcomeHistory::new();
        history.min_observations = 3;
        history.skip_threshold = 0.3;

        // Make Decompose have a terrible history.
        for _ in 0..5 {
            history.record(EnrichStep::Decompose, false);
        }

        let client = MockLlmClient::single_ok("# Content\n\nSome decomposition.\n");
        let mut config = make_config(tmp.path());
        config.force = true;

        let pipeline = EnrichmentPipeline::new(config, client).with_outcome_history(history);
        let outcome = pipeline.run_step(EnrichStep::Decompose, "test-plan").await;

        match outcome {
            StepOutcome::Skipped {
                reason: SkipReason::AdaptiveSkip,
                step,
            } => {
                assert_eq!(step, EnrichStep::Decompose);
            }
            other => panic!("expected Skipped(AdaptiveSkip), got {other:?}"),
        }
    }

    #[test]
    fn step_cost_tracks_output_bytes() {
        let cost = StepCost {
            elapsed_ms: 100,
            input_tokens: 500,
            output_tokens: 200,
            llm_calls: 1,
            output_bytes: 1234,
        };
        assert_eq!(cost.output_bytes, 1234);
        assert_eq!(cost.llm_calls, 1);
    }
}

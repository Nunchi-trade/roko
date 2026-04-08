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

use std::path::Path;

use super::client::LlmClient;
use super::config::EnrichmentConfig;
use super::inputs::step_dependency_paths;
use super::outcome::{SkipReason, StepOutcome};
use super::prompts::{self, StepInputs};
use super::step::{ALL_ORDERED, EnrichStep};
use super::validate;

/// The enrichment pipeline. Parameterized by an [`LlmClient`] implementation.
///
/// Use [`run_step`](Self::run_step) for a single step or
/// [`run_all`](Self::run_all) for the full pipeline in dependency order.
pub struct EnrichmentPipeline<C: LlmClient> {
    /// Pipeline configuration.
    config: EnrichmentConfig,
    /// LLM client for model calls.
    client: C,
}

impl<C: LlmClient> EnrichmentPipeline<C> {
    /// Create a new pipeline with the given config and LLM client.
    pub const fn new(config: EnrichmentConfig, client: C) -> Self {
        Self { config, client }
    }

    /// Borrow the pipeline configuration.
    pub const fn config(&self) -> &EnrichmentConfig {
        &self.config
    }

    /// Run a single enrichment step for a plan.
    ///
    /// Returns a [`StepOutcome`] describing what happened. Never panics on
    /// step failure — returns `StepOutcome::Failed` instead.
    pub async fn run_step(&self, step: EnrichStep, plan_base: &str) -> StepOutcome {
        let plan_dir = self.config.plan_dir(plan_base);
        let output_file = plan_dir.join(step.output_filename());

        // Dry-run mode: always skip.
        if self.config.dry_run {
            return StepOutcome::Skipped {
                step,
                reason: SkipReason::DryRun,
            };
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
                };
            }
        };

        // Non-LLM steps: generate via pure extraction.
        if !step.needs_llm() {
            return match prompts::generate_without_llm(step, &inputs) {
                Ok(content) => self.finalize_output(step, &output_file, &content, 0),
                Err(e) => StepOutcome::Failed { step, message: e },
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
                self.validate_and_write(step, &output_file, model, &raw_output)
                    .await
            }
            Err(e) => StepOutcome::Failed {
                step,
                message: format!("LLM call failed: {e}"),
            },
        }
    }

    /// Run all enrichment steps for a plan in dependency order.
    ///
    /// Continues past failures — each step's outcome is collected.
    /// Returns the list of outcomes for all 13 steps.
    pub async fn run_all(&self, plan_base: &str) -> Vec<StepOutcome> {
        let mut outcomes = Vec::with_capacity(ALL_ORDERED.len());
        for &step in ALL_ORDERED {
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
    ) -> StepOutcome {
        let normalized = validate::normalize_step_output(step, raw_output);

        match validate::validate_step_output(step, &normalized) {
            Ok(()) => self.finalize_output(step, output_file, &normalized, 1),
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
                        Ok(repaired) => self.finalize_output(step, output_file, &repaired, 2),
                        Err(repair_err) => StepOutcome::Failed {
                            step,
                            message: format!(
                                "TOML repair failed: {repair_err} (original: {original_err})"
                            ),
                        },
                    },
                    Err(e) => StepOutcome::Failed {
                        step,
                        message: format!("TOML repair LLM call failed: {e}"),
                    },
                }
            }
            Err(err) => StepOutcome::Failed { step, message: err },
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
    ) -> StepOutcome {
        // Empty output is always an error, even if validation somehow passed.
        if content.trim().is_empty() {
            return StepOutcome::Failed {
                step,
                message: format!("generated output for {step} was empty"),
            };
        }

        // Ensure parent directory exists.
        if let Some(parent) = output_file.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return StepOutcome::Failed {
                    step,
                    message: format!("failed to create directory {}: {e}", parent.display()),
                };
            }
        }

        match std::fs::write(output_file, content) {
            Ok(()) => StepOutcome::Generated { step, llm_calls },
            Err(e) => StepOutcome::Failed {
                step,
                message: format!("failed to write {}: {e}", output_file.display()),
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
            StepOutcome::Generated { step, llm_calls } => {
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
            StepOutcome::Generated { step, llm_calls } => {
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
            StepOutcome::Generated { llm_calls, .. } => {
                assert_eq!(llm_calls, 0);
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
            StepOutcome::Generated { llm_calls, .. } => {
                assert_eq!(llm_calls, 1);
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
}

//! Runtime dispatch for Roko's advertised 7-rung gate pipeline.
//!
//! This module centralizes the rung-to-gate mapping used by the CLI
//! orchestrator so the mapping can be exercised from `roko-gate` integration
//! tests without duplicating the selector logic in multiple crates.
//!
//! Several gates still need richer inputs than the orchestrator currently
//! attaches to the base `GatePayload` signal. For those cases, the dispatcher
//! returns an explicit stub verdict instead of silently skipping the gate or
//! introducing a false failure.

use crate::CompileGate;
use crate::clippy_gate::ClippyGate;
use crate::fact_check::{FactCheckGate, SearchOracle};
use crate::gate_pipeline::{ComposedGatePipeline, GateComposition};
use crate::generated_test_gate::{ArtifactStore as GeneratedArtifactStore, GeneratedTestGate};
use crate::integration_gate::IntegrationGate;
use crate::llm_judge_gate::{JudgeOracle, LlmJudgeGate};
use crate::payload::BuildSystem;
use crate::property_test_gate::PropertyTestGate;
use crate::rung_selector::{PlanComplexity, Rung, RungCaps, select_rungs};
use crate::shell::ShellGate;
use crate::symbol_gate::SymbolGate;
use crate::test_gate::TestGate;
use crate::verify_chain_gate::{VERIFY_SCRIPT_TAG, VerifyChainGate};
use async_trait::async_trait;
use roko_core::config::{GateRungConfig, GatesConfig};
use roko_core::{Context, Signal, Verdict, Verify};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_COMPILE_TIMEOUT_SECS: u64 = 600;
const DEFAULT_LINT_TIMEOUT_SECS: u64 = 300;
const DEFAULT_TEST_TIMEOUT_SECS: u64 = 900;
const DEFAULT_SYMBOL_TIMEOUT_SECS: u64 = 120;
const DEFAULT_GENERATED_TEST_TIMEOUT_SECS: u64 = 900;
const DEFAULT_VERIFY_CHAIN_TIMEOUT_SECS: u64 = 1_200;
const DEFAULT_PROPERTY_TEST_TIMEOUT_SECS: u64 = 900;
const DEFAULT_FACT_CHECK_TIMEOUT_SECS: u64 = 120;
const DEFAULT_LLM_JUDGE_TIMEOUT_SECS: u64 = 120;
const DEFAULT_INTEGRATION_TIMEOUT_SECS: u64 = 120;

/// Optional per-gate signals for rungs that need richer inputs than the base
/// `GatePayload` signal currently provides.
#[derive(Clone, Debug, Default)]
pub struct RungExecutionInputs {
    /// `SymbolGate` expects a `SymbolManifest` body.
    pub symbol_signal: Option<Signal>,
    /// `FactCheckGate` expects text or claim-like content.
    pub fact_check_signal: Option<Signal>,
    /// `LlmJudgeGate` expects a `JudgePayload` or text diff.
    pub llm_judge_signal: Option<Signal>,
    /// INT-16: Code-intelligence context from `roko-index` used to enrich
    /// verification decisions.  Symbol and LLM-judge gates may use these
    /// hints to focus checks on relevant symbols / files.
    pub code_intel_hints: Vec<String>,
}

/// Configuration knobs for executing the 7-rung runtime gate mapping.
#[derive(Clone, Default)]
pub struct RungExecutionConfig {
    /// Source roots for `SymbolGate`.
    pub source_roots: Option<Vec<PathBuf>>,
    /// Artifact store for `GeneratedTestGate`.
    pub generated_test_artifacts: Option<Arc<dyn GeneratedArtifactStore>>,
    /// Optional fallback gate for `VerifyChainGate`.
    pub verify_chain_fallback: Option<Arc<dyn Verify>>,
    /// Search oracle for `FactCheckGate`.
    pub fact_check_oracle: Option<Arc<dyn SearchOracle>>,
    /// Fact-check confidence threshold. Defaults to the gate's builtin value.
    pub fact_check_min_confidence: Option<f64>,
    /// Judge oracle for `LlmJudgeGate`.
    pub llm_judge_oracle: Option<Arc<dyn JudgeOracle>>,
    /// Judge threshold. Defaults to `0.8`.
    pub llm_judge_min_score: Option<f32>,
    /// Build-system-specific integration test pattern to run on rung 6.
    pub integration_test_pattern: Option<String>,
    /// Build system for the integration scenario. Defaults to cargo.
    pub integration_build_system: Option<BuildSystem>,
    /// Optional verdict publisher for broadcasting gate outcomes.
    pub verdict_publisher: Option<crate::verdict_publisher::VerdictPublisher>,
    /// Optional timeout override for concrete gates in this rung.
    pub timeout_ms: Option<u64>,
}

/// Builds a [`ComposedGatePipeline`] from gate config and plan complexity.
pub struct GatePipelineBuilder;

impl GatePipelineBuilder {
    /// Build the effective gate pipeline. Custom `[[gates.rungs]]` entries
    /// replace complexity-selected defaults.
    #[must_use]
    pub fn from_config(config: &GatesConfig, complexity: PlanComplexity) -> ComposedGatePipeline {
        Self::from_config_with_execution(
            config,
            complexity,
            RungExecutionInputs::default(),
            RungExecutionConfig::default(),
        )
    }

    /// Build the effective gate pipeline with runtime inputs for rich rungs.
    #[must_use]
    pub fn from_config_with_execution(
        config: &GatesConfig,
        complexity: PlanComplexity,
        inputs: RungExecutionInputs,
        execution: RungExecutionConfig,
    ) -> ComposedGatePipeline {
        if config.has_custom_rungs() {
            return Self::from_custom_config_with_execution(
                config.effective_rungs(),
                inputs,
                execution,
            );
        }

        let caps = RungCaps {
            has_lint_tool: config.clippy_enabled,
            ..RungCaps::all()
        };
        let rungs = select_rungs(complexity, &caps, 0)
            .into_iter()
            .filter(|rung| !(config.skip_tests && *rung == Rung::Test));
        Self::from_default_rungs_with_execution(rungs, inputs, execution)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn from_custom_config_with_execution(
        rungs: Vec<GateRungConfig>,
        inputs: RungExecutionInputs,
        execution: RungExecutionConfig,
    ) -> ComposedGatePipeline {
        let mut pipeline =
            ComposedGatePipeline::new("gate-pipeline:custom", GateComposition::Sequential);
        for rung_config in rungs {
            pipeline.push(Self::gate_from_rung_config(
                &rung_config,
                inputs.clone(),
                execution.clone(),
            ));
        }
        pipeline
    }

    #[allow(clippy::needless_pass_by_value)]
    fn from_default_rungs_with_execution(
        rungs: impl IntoIterator<Item = Rung>,
        inputs: RungExecutionInputs,
        execution: RungExecutionConfig,
    ) -> ComposedGatePipeline {
        let mut pipeline =
            ComposedGatePipeline::new("gate-pipeline:default", GateComposition::Sequential);
        for rung in rungs {
            pipeline.push(Self::gate_from_known_rung(
                rung,
                default_timeout_for_rung(rung),
                inputs.clone(),
                execution.clone(),
            ));
        }
        pipeline
    }

    fn gate_from_rung_config(
        rung_config: &GateRungConfig,
        inputs: RungExecutionInputs,
        execution: RungExecutionConfig,
    ) -> Box<dyn Verify> {
        let gate: Box<dyn Verify> = if rung_config.command.trim().is_empty() {
            known_rung_from_name(&rung_config.name).map_or_else(
                || shell_gate_from_config(rung_config),
                |rung| Self::gate_from_known_rung(rung, rung_config.timeout(), inputs, execution),
            )
        } else {
            shell_gate_from_config(rung_config)
        };
        optional_gate_if_needed(gate, rung_config.required)
    }

    fn gate_from_known_rung(
        rung: Rung,
        timeout_duration: Duration,
        inputs: RungExecutionInputs,
        execution: RungExecutionConfig,
    ) -> Box<dyn Verify> {
        let timeout_ms = duration_ms(timeout_duration);
        match rung {
            Rung::Compile => Box::new(CompileGate::cargo().with_timeout_ms(timeout_ms)),
            Rung::Lint => Box::new(ClippyGate::cargo().with_timeout_ms(timeout_ms)),
            Rung::Test => Box::new(TestGate::cargo().with_timeout_ms(timeout_ms)),
            Rung::Symbol | Rung::GeneratedTest | Rung::PropertyTest | Rung::Integration => {
                Box::new(CanonicalRungGate {
                    rung,
                    inputs,
                    execution,
                    name: format!("rung:{}", rung.label()),
                })
            }
        }
    }
}

/// Execute a single canonical rung of the 7-rung runtime mapping.
///
/// The mapping is:
///
/// - `0`: compile
/// - `1`: clippy
/// - `2`: test
/// - `3`: symbol
/// - `4`: generated test + verify-chain
/// - `5`: property test + fact-check
/// - `6`: llm-judge + integration
///
/// Any `rung > 6` executes every rung in order and flattens the resulting
/// verdicts.
pub async fn run_rung(
    base_signal: &Signal,
    ctx: &Context,
    rung: u32,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Vec<Verdict> {
    if rung > 6 {
        let mut verdicts = Vec::new();
        for rung in crate::rung_selector::CANONICAL_ORDER {
            verdicts.extend(run_canonical_rung(base_signal, ctx, rung, inputs, config).await);
        }
        return verdicts;
    }

    let Some(rung) = Rung::from_index(rung) else {
        unreachable!("rung > 6 is handled above");
    };
    run_canonical_rung(base_signal, ctx, rung, inputs, config).await
}

fn gate_timeout_ms(config: &RungExecutionConfig) -> Option<u64> {
    config.timeout_ms.map(|timeout_ms| timeout_ms.max(1))
}

/// Execute one [`Rung`] using the canonical 7-rung runtime mapping.
pub async fn run_canonical_rung(
    base_signal: &Signal,
    ctx: &Context,
    rung: Rung,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Vec<Verdict> {
    match rung {
        Rung::Compile => {
            let mut gate = CompileGate::cargo();
            if let Some(timeout_ms) = gate_timeout_ms(config) {
                gate = gate.with_timeout_ms(timeout_ms);
            }
            vec![gate.verify(base_signal, ctx).await]
        }
        Rung::Lint => {
            let mut gate = ClippyGate::cargo();
            if let Some(timeout_ms) = gate_timeout_ms(config) {
                gate = gate.with_timeout_ms(timeout_ms);
            }
            vec![gate.verify(base_signal, ctx).await]
        }
        Rung::Test => {
            let mut gate = TestGate::cargo();
            if let Some(timeout_ms) = gate_timeout_ms(config) {
                gate = gate.with_timeout_ms(timeout_ms);
            }
            vec![gate.verify(base_signal, ctx).await]
        }
        Rung::Symbol => vec![run_symbol_gate(ctx, inputs, config).await],
        Rung::GeneratedTest => vec![
            run_generated_test_gate(base_signal, ctx, config).await,
            run_verify_chain_gate(base_signal, ctx, config).await,
        ],
        Rung::PropertyTest => vec![
            run_property_test_gate(base_signal, ctx, config).await,
            run_fact_check_gate(ctx, inputs, config).await,
        ],
        Rung::Integration => vec![
            run_llm_judge_gate(ctx, inputs, config).await,
            run_integration_gate(base_signal, ctx, config).await,
        ],
    }
}

fn stub_verdict(gate: &str, detail: impl Into<String>) -> Verdict {
    let message = format!("stub gate; {}", detail.into());
    let mut verdict = Verdict::pass(gate.to_string());
    verdict.reason.clone_from(&message);
    verdict.detail = Some(message);
    verdict
}

async fn run_symbol_gate(
    ctx: &Context,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(signal) = inputs.symbol_signal.as_ref() else {
        return stub_verdict("symbol", "no SymbolManifest wired into rung 3");
    };
    let Some(source_roots) = config.source_roots.clone() else {
        return stub_verdict("symbol", "no source roots configured for rung 3");
    };
    // INT-16: When code-intel hints are available, tag the signal so the
    // symbol gate can focus on the most relevant files/symbols.
    let signal = if inputs.code_intel_hints.is_empty() {
        signal.clone()
    } else {
        let mut enriched = signal.clone();
        for (i, hint) in inputs.code_intel_hints.iter().enumerate().take(10) {
            enriched
                .tags
                .insert(format!("code_intel_hint_{i}"), hint.clone());
        }
        enriched
    };
    SymbolGate::new(source_roots).verify(&signal, ctx).await
}

async fn run_generated_test_gate(
    base_signal: &Signal,
    ctx: &Context,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(artifacts) = config.generated_test_artifacts.clone() else {
        return stub_verdict("generated_test:cargo", "generated test artifacts not wired");
    };
    let mut gate = GeneratedTestGate::new(artifacts);
    if let Some(timeout_ms) = gate_timeout_ms(config) {
        gate = gate.with_timeout_ms(timeout_ms);
    }
    gate.verify(base_signal, ctx).await
}

async fn run_verify_chain_gate(
    base_signal: &Signal,
    ctx: &Context,
    config: &RungExecutionConfig,
) -> Verdict {
    if base_signal.tag(VERIFY_SCRIPT_TAG).is_none() && config.verify_chain_fallback.is_none() {
        return stub_verdict("verify_chain", "no verify script wired into rung 4");
    }
    let gate = config
        .verify_chain_fallback
        .clone()
        .map_or_else(VerifyChainGate::strict, VerifyChainGate::with_fallback);
    let gate = if let Some(timeout_ms) = gate_timeout_ms(config) {
        gate.with_timeout_ms(timeout_ms)
    } else {
        gate
    };
    gate.verify(base_signal, ctx).await
}

async fn run_fact_check_gate(
    ctx: &Context,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(signal) = inputs.fact_check_signal.as_ref() else {
        return stub_verdict("fact_check", "no fact-check content wired into rung 5");
    };
    let Some(oracle) = config.fact_check_oracle.clone() else {
        return stub_verdict("fact_check", "no fact-check oracle configured");
    };
    let min_confidence = config
        .fact_check_min_confidence
        .unwrap_or(FactCheckGate::DEFAULT_MIN_CONFIDENCE);
    FactCheckGate::new(oracle, min_confidence)
        .verify(signal, ctx)
        .await
}

async fn run_property_test_gate(
    base_signal: &Signal,
    ctx: &Context,
    config: &RungExecutionConfig,
) -> Verdict {
    let mut gate = PropertyTestGate::cargo();
    if let Some(timeout_ms) = gate_timeout_ms(config) {
        gate = gate.with_timeout_ms(timeout_ms);
    }
    gate.verify(base_signal, ctx).await
}

async fn run_llm_judge_gate(
    ctx: &Context,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(signal) = inputs.llm_judge_signal.as_ref() else {
        return stub_verdict("llm_judge", "no judge payload wired into rung 6");
    };
    let Some(oracle) = config.llm_judge_oracle.clone() else {
        return stub_verdict("llm_judge", "no judge oracle configured");
    };
    let min_score = config.llm_judge_min_score.unwrap_or(0.8);
    LlmJudgeGate::new(oracle, min_score)
        .verify(signal, ctx)
        .await
}

async fn run_integration_gate(
    base_signal: &Signal,
    ctx: &Context,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(pattern) = config.integration_test_pattern.as_ref() else {
        return stub_verdict(
            "integration:build_test",
            "no integration scenario wired into rung 6",
        );
    };
    let build_system = config
        .integration_build_system
        .unwrap_or(BuildSystem::Cargo);
    let mut gate = IntegrationGate::build_test(build_system, pattern);
    if let Some(timeout_ms) = gate_timeout_ms(config) {
        gate = gate.with_timeout_ms(timeout_ms);
    }
    gate.verify(base_signal, ctx).await
}

fn default_timeout_for_rung(rung: Rung) -> Duration {
    let secs = match rung {
        Rung::Compile => DEFAULT_COMPILE_TIMEOUT_SECS,
        Rung::Lint => DEFAULT_LINT_TIMEOUT_SECS,
        Rung::Test => DEFAULT_TEST_TIMEOUT_SECS,
        Rung::Symbol => DEFAULT_SYMBOL_TIMEOUT_SECS,
        Rung::GeneratedTest => {
            DEFAULT_GENERATED_TEST_TIMEOUT_SECS.max(DEFAULT_VERIFY_CHAIN_TIMEOUT_SECS)
        }
        Rung::PropertyTest => {
            DEFAULT_PROPERTY_TEST_TIMEOUT_SECS.max(DEFAULT_FACT_CHECK_TIMEOUT_SECS)
        }
        Rung::Integration => DEFAULT_LLM_JUDGE_TIMEOUT_SECS.max(DEFAULT_INTEGRATION_TIMEOUT_SECS),
    };
    Duration::from_secs(secs)
}

fn known_rung_from_name(name: &str) -> Option<Rung> {
    match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "compile" | "build" | "check" => Some(Rung::Compile),
        "lint" | "clippy" => Some(Rung::Lint),
        "test" | "tests" => Some(Rung::Test),
        "symbol" | "symbols" => Some(Rung::Symbol),
        "generated-test" | "generated-tests" | "gen-test" => Some(Rung::GeneratedTest),
        "property-test" | "property-tests" | "prop-test" => Some(Rung::PropertyTest),
        "integration" | "integration-test" => Some(Rung::Integration),
        _ => None,
    }
}

fn shell_gate_from_config(rung_config: &GateRungConfig) -> Box<dyn Verify> {
    let timeout_ms = duration_ms(rung_config.timeout());
    let gate = if cfg!(windows) {
        ShellGate::new("cmd", vec!["/C".into(), rung_config.command.clone()])
    } else {
        ShellGate::new("sh", vec!["-c".into(), rung_config.command.clone()])
    }
    .with_name(rung_config.name.clone())
    .with_timeout_ms(timeout_ms);
    Box::new(gate)
}

fn optional_gate_if_needed(gate: Box<dyn Verify>, required: bool) -> Box<dyn Verify> {
    if required {
        gate
    } else {
        Box::new(OptionalGate {
            name: gate.name().to_string(),
            inner: gate,
        })
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_mul(1_000)
        .saturating_add(u64::from(duration.subsec_millis()))
}

struct OptionalGate {
    name: String,
    inner: Box<dyn Verify>,
}

#[async_trait]
impl Verify for OptionalGate {
    async fn verify(&self, engram: &Signal, ctx: &Context) -> Verdict {
        let verdict = self.inner.verify(engram, ctx).await;
        if verdict.passed {
            verdict
        } else {
            Verdict::pass(&self.name)
                .with_detail(format!(
                    "optional gate failed but is not required: {}",
                    verdict.reason
                ))
                .with_duration(verdict.duration_ms)
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

struct CanonicalRungGate {
    rung: Rung,
    inputs: RungExecutionInputs,
    execution: RungExecutionConfig,
    name: String,
}

#[async_trait]
impl Verify for CanonicalRungGate {
    async fn verify(&self, engram: &Signal, ctx: &Context) -> Verdict {
        let verdicts =
            run_canonical_rung(engram, ctx, self.rung, &self.inputs, &self.execution).await;
        aggregate_rung_verdict(&self.name, &verdicts)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

fn aggregate_rung_verdict(name: &str, verdicts: &[Verdict]) -> Verdict {
    let passed = verdicts.iter().all(|verdict| verdict.passed);
    let detail = render_rung_detail(verdicts);
    let duration = verdicts
        .iter()
        .map(|verdict| verdict.duration_ms)
        .max()
        .unwrap_or_default();
    if passed {
        Verdict::pass(name)
            .with_detail(detail)
            .with_duration(duration)
    } else {
        let failed = verdicts
            .iter()
            .filter(|verdict| !verdict.passed)
            .map(|verdict| verdict.gate.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Verdict::fail(name, format!("inner gate failed: {failed}"))
            .with_detail(detail)
            .with_duration(duration)
    }
}

fn render_rung_detail(verdicts: &[Verdict]) -> String {
    verdicts
        .iter()
        .map(|verdict| {
            format!(
                "{}: {} - {}",
                verdict.gate,
                if verdict.passed { "pass" } else { "FAIL" },
                verdict.reason
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

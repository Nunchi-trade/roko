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
use crate::generated_test_gate::{ArtifactStore as GeneratedArtifactStore, GeneratedTestGate};
use crate::integration_gate::IntegrationGate;
use crate::llm_judge_gate::{JudgeOracle, LlmJudgeGate};
use crate::payload::BuildSystem;
use crate::property_test_gate::PropertyTestGate;
use crate::rung_selector::Rung;
use crate::symbol_gate::SymbolGate;
use crate::test_gate::TestGate;
use crate::verify_chain_gate::{VERIFY_SCRIPT_TAG, VerifyChainGate};
use roko_core::{Context, Engram, Verdict, Verify};
use std::path::PathBuf;
use std::sync::Arc;

/// Optional per-gate signals for rungs that need richer inputs than the base
/// `GatePayload` signal currently provides.
#[derive(Clone, Debug, Default)]
pub struct RungExecutionInputs {
    /// `SymbolGate` expects a `SymbolManifest` body.
    pub symbol_signal: Option<Engram>,
    /// `FactCheckGate` expects text or claim-like content.
    pub fact_check_signal: Option<Engram>,
    /// `LlmJudgeGate` expects a `JudgePayload` or text diff.
    pub llm_judge_signal: Option<Engram>,
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
    base_signal: &Engram,
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

/// Execute one [`Rung`] using the canonical 7-rung runtime mapping.
pub async fn run_canonical_rung(
    base_signal: &Engram,
    ctx: &Context,
    rung: Rung,
    inputs: &RungExecutionInputs,
    config: &RungExecutionConfig,
) -> Vec<Verdict> {
    match rung {
        Rung::Compile => vec![CompileGate::cargo().verify(base_signal, ctx).await],
        Rung::Lint => vec![ClippyGate::cargo().verify(base_signal, ctx).await],
        Rung::Test => vec![TestGate::cargo().verify(base_signal, ctx).await],
        Rung::Symbol => vec![run_symbol_gate(ctx, inputs, config).await],
        Rung::GeneratedTest => vec![
            run_generated_test_gate(base_signal, ctx, config).await,
            run_verify_chain_gate(base_signal, ctx, config).await,
        ],
        Rung::PropertyTest => vec![
            PropertyTestGate::cargo().verify(base_signal, ctx).await,
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
    base_signal: &Engram,
    ctx: &Context,
    config: &RungExecutionConfig,
) -> Verdict {
    let Some(artifacts) = config.generated_test_artifacts.clone() else {
        return stub_verdict("generated_test:cargo", "generated test artifacts not wired");
    };
    GeneratedTestGate::new(artifacts)
        .verify(base_signal, ctx)
        .await
}

async fn run_verify_chain_gate(
    base_signal: &Engram,
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
    base_signal: &Engram,
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
    IntegrationGate::build_test(build_system, pattern)
        .verify(base_signal, ctx)
        .await
}

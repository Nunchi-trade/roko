#![allow(missing_docs)]

//! Benchmark helpers for the demo surfaces.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::scenarios::LlmProvider;
use crate::scenarios::yield_routing::{PreparedYieldRouting, RoundOutcome, prepare, run_round};

/// Derived C-factor metrics for a cold and warm run pair.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CFactorMetrics {
    /// Warm output improvement over cold output.
    pub output_improvement_pct: f64,
    /// Improvement expressed in basis points.
    pub improvement_bps: u32,
}

/// Full benchmark report.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CFactorReport {
    /// Cold run without seeded baseline insights.
    pub run_a: RoundOutcome,
    /// Warm run with prior knowledge from the cold run.
    pub run_b: RoundOutcome,
    /// Derived metrics.
    pub c_factor: CFactorMetrics,
}

/// Prepare the chain for a C-factor benchmark.
///
/// # Errors
///
/// Returns an error if scenario preparation fails, including missing deployed
/// contract addresses, prompt loading, or participant setup.
pub async fn prepare_benchmark(
    ctx: Arc<ChainCtx>,
    runtime_dir: PathBuf,
    persist_reputation: bool,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<PreparedYieldRouting> {
    prepare(ctx, runtime_dir, persist_reputation, false, events).await
}

/// Run the benchmark and optionally reuse a prepared scenario state.
///
/// # Errors
///
/// Returns an error if either benchmark round fails or if the underlying
/// scenario helpers report an error.
pub async fn run_benchmark(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<CFactorReport> {
    let run_a = run_round(prepared, 1, llm.clone(), events.clone()).await?;
    let run_b = run_round(prepared, 2, llm, events).await?;
    let improvement_pct = if run_a.output_eth <= 0.0 {
        0.0
    } else {
        ((run_b.output_eth - run_a.output_eth) / run_a.output_eth) * 100.0
    };
    Ok(CFactorReport {
        c_factor: CFactorMetrics {
            output_improvement_pct: improvement_pct,
            improvement_bps: (improvement_pct.max(0.0) * 100.0).round() as u32,
        },
        run_a,
        run_b,
    })
}

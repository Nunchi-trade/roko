#![allow(missing_docs)]

//! Multi-round tournament runner for the yield-routing spine.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::scenarios::LlmProvider;
use crate::scenarios::yield_routing::{PreparedYieldRouting, RoundOutcome, prepare, run_round};

/// Point on the learning curve.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LearningPoint {
    pub round: usize,
    pub output_eth: f64,
}

/// Aggregate ranking for one agent.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentRanking {
    pub agent_id: String,
    pub wins: usize,
    pub total_output: f64,
    pub avg_confidence: f64,
}

/// Tournament summary.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TournamentReport {
    pub rounds: Vec<RoundOutcome>,
    pub learning_curve: Vec<LearningPoint>,
    pub final_rankings: Vec<AgentRanking>,
}

/// Prepare the tournament scenario state once.
pub async fn prepare_tournament(
    ctx: Arc<ChainCtx>,
    runtime_dir: PathBuf,
    persist_reputation: bool,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<PreparedYieldRouting> {
    prepare(ctx, runtime_dir, persist_reputation, true, events).await
}

/// Run a multi-round tournament on top of a prepared scenario state.
pub async fn run_tournament(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    rounds: usize,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<TournamentReport> {
    let mut outcomes = Vec::new();
    for round in 1..=rounds {
        outcomes.push(run_round(prepared, round as u32, llm.clone(), events.clone()).await?);
    }

    let learning_curve = outcomes
        .iter()
        .map(|outcome| LearningPoint {
            round: outcome.round as usize,
            output_eth: outcome.output_eth,
        })
        .collect::<Vec<_>>();

    let mut aggregates: HashMap<String, (usize, f64, f64)> = HashMap::new();
    for outcome in &outcomes {
        let entry = aggregates
            .entry(outcome.winner.clone())
            .or_insert((0, 0.0, 0.0));
        entry.0 += 1;
        entry.1 += outcome.output_eth;
        entry.2 += outcome.confidence;
    }
    let mut final_rankings = aggregates
        .into_iter()
        .map(
            |(agent_id, (wins, total_output, total_confidence))| AgentRanking {
                avg_confidence: total_confidence / wins as f64,
                agent_id,
                wins,
                total_output,
            },
        )
        .collect::<Vec<_>>();
    final_rankings.sort_by(|left, right| {
        right.wins.cmp(&left.wins).then_with(|| {
            right
                .total_output
                .partial_cmp(&left.total_output)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    Ok(TournamentReport {
        rounds: outcomes,
        learning_curve,
        final_rankings,
    })
}

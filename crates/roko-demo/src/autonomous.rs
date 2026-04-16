#![allow(missing_docs)]

//! Autonomous loop runner built on top of the yield-routing helpers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::scenarios::LlmProvider;
use crate::scenarios::yield_routing::{
    BidSubmission, PreparedYieldRouting, RoundOutcome, finalize_round, generate_agent_bid,
    post_round_job, prepare, save_reputation,
};

/// Summary of an autonomous run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutonomousReport {
    pub rounds: Vec<RoundOutcome>,
    pub jobs_completed: usize,
    pub timed_out: bool,
}

/// Prepare the autonomous loop state.
pub async fn prepare_autonomous(
    ctx: Arc<ChainCtx>,
    runtime_dir: PathBuf,
    persist_reputation: bool,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<PreparedYieldRouting> {
    prepare(ctx, runtime_dir, persist_reputation, true, events).await
}

/// Run an autonomous poster/agent loop for `jobs` rounds.
pub async fn run_autonomous(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    agents: usize,
    jobs: usize,
    interval_secs: u64,
    timeout_secs: u64,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<AutonomousReport> {
    let mut rounds = Vec::new();
    let timeout = Duration::from_secs(timeout_secs);
    for round in 1..=jobs {
        let future =
            run_autonomous_round(prepared, llm.clone(), round as u32, agents, events.clone());
        let outcome = tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| anyhow::anyhow!("autonomous round {round} timed out"))??;
        rounds.push(outcome);
        if round < jobs && interval_secs > 0 {
            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        }
    }
    if prepared.persist_reputation {
        save_reputation(prepared).await?;
    }
    Ok(AutonomousReport {
        jobs_completed: rounds.len(),
        rounds,
        timed_out: false,
    })
}

async fn run_autonomous_round(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    round: u32,
    agents: usize,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<RoundOutcome> {
    let posted = post_round_job(prepared, round, events.clone()).await?;
    let workers = agents.min(5);
    let bids = if llm.label() == "stub" {
        let mut sequential = Vec::new();
        for worker_index in 0..workers {
            sequential.push(
                generate_agent_bid(
                    prepared,
                    llm.clone(),
                    round,
                    worker_index,
                    &posted.spec,
                    &posted.insights_before,
                    events.clone(),
                )
                .await?,
            );
        }
        sequential
    } else {
        collect_concurrent_bids(
            prepared,
            llm.clone(),
            round,
            workers,
            &posted,
            events.clone(),
        )
        .await?
    };
    finalize_round(prepared, posted, bids, llm, events).await
}

async fn collect_concurrent_bids(
    prepared: &PreparedYieldRouting,
    llm: Arc<dyn LlmProvider>,
    round: u32,
    workers: usize,
    posted: &crate::scenarios::yield_routing::PostedJob,
    events: Arc<dyn EventEmitter>,
) -> anyhow::Result<Vec<BidSubmission>> {
    let (sender, mut receiver) = mpsc::channel::<anyhow::Result<BidSubmission>>(workers);
    for worker_index in 0..workers {
        let sender = sender.clone();
        let llm = llm.clone();
        let events = events.clone();
        let insights = posted.insights_before.clone();
        let spec = posted.spec.clone();
        let prepared = prepared.clone();
        tokio::spawn(async move {
            let result = generate_agent_bid(
                &prepared,
                llm,
                round,
                worker_index,
                &spec,
                &insights,
                events,
            )
            .await;
            let _ = sender.send(result).await;
        });
    }
    drop(sender);

    let mut bids = Vec::new();
    while let Some(result) = receiver.recv().await {
        bids.push(result?);
    }
    bids.sort_by_key(|bid| bid.worker_index);
    Ok(bids)
}

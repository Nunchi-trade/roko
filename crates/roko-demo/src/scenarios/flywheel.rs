//! Knowledge flywheel scenario.
//!
//! Posters submit insights; confirmers acknowledge them; posters earn claimable
//! DAEJI per confirmation. Validates the pheromone-curation loop.

use std::sync::Arc;

use alloy::primitives::{U256, keccak256};
use async_trait::async_trait;

use crate::bindings::InsightBoard;
use crate::chain_ctx::ChainCtx;
use crate::events::EventEmitter;
use crate::manifest::Scenario as ScenarioManifest;
use crate::scenarios::Scenario;
use crate::scenarios::llm::{LlmProvider, LlmRequest};

/// Flywheel scenario.
pub struct Flywheel;

const POSTERS: usize = 3;
const CONFIRMERS: usize = 3;
const ROUNDS: usize = 3;

#[async_trait]
impl Scenario for Flywheel {
    fn name(&self) -> &'static str {
        "flywheel"
    }

    async fn spine(
        &self,
        ctx: Arc<ChainCtx>,
        _manifest: &ScenarioManifest,
        llm: Arc<dyn LlmProvider>,
        _events: Arc<dyn EventEmitter>,
    ) -> anyhow::Result<()> {
        let board_addr = ctx.address_of("InsightBoard")?;
        for round in 0..ROUNDS {
            for p in 0..POSTERS {
                let name = format!("worker{p}");
                let provider = ctx.wallet_provider(&name)?;
                let board = InsightBoard::new(board_addr, provider);
                let content = llm
                    .fill(LlmRequest {
                        slot: "insight_content".into(),
                        context: serde_json::json!({ "round": round, "poster": p }),
                    })
                    .await?
                    .as_str()
                    .unwrap_or("default")
                    .to_string();
                let h = keccak256(content.as_bytes());
                board
                    .post(h.into(), "ipfs://demo".into())
                    .send()
                    .await?
                    .watch()
                    .await?;
            }
            // Confirmers each confirm the most recent insight posted by
            // each poster (unless that insight is their own post, which the
            // contract rejects).
            let board_reader = InsightBoard::new(board_addr, ctx.wallet_provider("validator0")?);
            let next_id = board_reader.nextInsightId().call().await?;
            let first_in_round = next_id - U256::from(POSTERS as u64);
            for c in 0..CONFIRMERS {
                let name = format!("validator{c}");
                let provider = ctx.wallet_provider(&name)?;
                let board = InsightBoard::new(board_addr, provider);
                for p in 0..POSTERS {
                    let id = first_in_round + U256::from(p as u64);
                    let decide = llm
                        .fill(LlmRequest {
                            slot: "confirm_decision".into(),
                            context: serde_json::json!({ "id": id.to_string() }),
                        })
                        .await?
                        .as_bool()
                        .unwrap_or(true);
                    if decide {
                        let _ = board.confirm(id).send().await?.watch().await;
                    }
                }
            }
        }
        Ok(())
    }
}

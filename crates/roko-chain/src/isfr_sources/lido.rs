//! Lido stETH ISFR source — derives staking APR from exchange rate drift.
#![cfg(feature = "alloy-backend")]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::Address;
use alloy::providers::DynProvider;
use alloy::sol;
use anyhow::Context;
use async_trait::async_trait;

use super::{ISFRSource, RateClass, SourceReading};

sol! {
    #[sol(rpc)]
    interface IStETH {
        function getTotalPooledEther() external view returns (uint256);
        function getTotalShares() external view returns (uint256);
    }
}

/// Ethereum merge timestamp (Sep 15, 2022 06:42:59 UTC).
const MERGE_TIMESTAMP: u64 = 1_663_224_179;

/// Seconds in a year (365.25 days).
const SECONDS_PER_YEAR: f64 = 31_557_600.0;

/// Lido stETH staking rate source.
///
/// Exchange rate = `totalPooledEther / totalShares`.
/// APR from exchange rate drift since merge:
/// `rate_bps = ((pooled/shares - 1.0) / years_since_merge) * 10_000`.
pub struct LidoStakingSource {
    name: String,
    weight: f64,
    provider: Arc<DynProvider>,
    steth: Address,
}

impl LidoStakingSource {
    /// Create a new Lido staking source targeting a specific stETH contract.
    pub fn new(name: String, weight: f64, provider: Arc<DynProvider>, steth: Address) -> Self {
        Self {
            name,
            weight,
            provider,
            steth,
        }
    }
}

#[async_trait]
impl ISFRSource for LidoStakingSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn weight(&self) -> f64 {
        self.weight
    }

    fn rate_class(&self) -> RateClass {
        RateClass::Staking
    }

    async fn fetch_rate(&self) -> anyhow::Result<SourceReading> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before UNIX epoch")?;
        let now_ms = now.as_millis() as u64;
        let now_secs = now.as_secs();

        let contract = IStETH::new(self.steth, &*self.provider);

        let total_pooled = contract
            .getTotalPooledEther()
            .call()
            .await
            .context("Lido getTotalPooledEther call failed")?;

        let total_shares = contract
            .getTotalShares()
            .call()
            .await
            .context("Lido getTotalShares call failed")?;

        // Exchange rate as f64. Both values are in wei (u256) — ratio is dimensionless.
        let pooled_f64: f64 = total_pooled.to_string().parse::<f64>().unwrap_or(0.0);
        let shares_f64: f64 = total_shares.to_string().parse::<f64>().unwrap_or(0.0);

        let rate_bps = if shares_f64 == 0.0 || now_secs <= MERGE_TIMESTAMP {
            0u64
        } else {
            let exchange_rate = pooled_f64 / shares_f64;
            let years_since_merge = (now_secs - MERGE_TIMESTAMP) as f64 / SECONDS_PER_YEAR;
            let apr = (exchange_rate - 1.0) / years_since_merge;
            (apr * 10_000.0).max(0.0) as u64
        };

        Ok(SourceReading {
            source: self.name.clone(),
            rate_bps,
            timestamp_ms: now_ms,
            is_live: true,
            metadata: serde_json::json!({
                "protocol": "lido",
                "steth": format!("{}", self.steth),
                "total_pooled_ether": total_pooled.to_string(),
                "total_shares": total_shares.to_string(),
            }),
        })
    }
}

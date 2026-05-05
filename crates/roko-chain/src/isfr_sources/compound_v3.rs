//! Compound V3 (Comet) ISFR source — reads supply rate from a Comet market.
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
    interface IComet {
        function getUtilization() external view returns (uint256);
        function getSupplyRate(uint256 utilization) external view returns (uint64);
    }
}

/// Compound V3 lending rate source.
///
/// Calls `getUtilization()` then `getSupplyRate(utilization)`.
/// The supply rate is per-second scaled by 1e18.
/// Annualise: `rate_bps = supplyRate * 31_536_000 / 1e14`.
pub struct CompoundV3Source {
    name: String,
    weight: f64,
    provider: Arc<DynProvider>,
    comet: Address,
}

impl CompoundV3Source {
    /// Create a new Compound V3 source targeting a specific Comet market.
    pub fn new(name: String, weight: f64, provider: Arc<DynProvider>, comet: Address) -> Self {
        Self {
            name,
            weight,
            provider,
            comet,
        }
    }
}

#[async_trait]
impl ISFRSource for CompoundV3Source {
    fn name(&self) -> &str {
        &self.name
    }

    fn weight(&self) -> f64 {
        self.weight
    }

    fn rate_class(&self) -> RateClass {
        RateClass::Lending
    }

    async fn fetch_rate(&self) -> anyhow::Result<SourceReading> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before UNIX epoch")?
            .as_millis() as u64;

        let contract = IComet::new(self.comet, &*self.provider);

        let utilization = contract
            .getUtilization()
            .call()
            .await
            .context("Compound V3 getUtilization call failed")?;

        let supply_rate = contract
            .getSupplyRate(utilization)
            .call()
            .await
            .context("Compound V3 getSupplyRate call failed")?;

        // supply_rate is per-second scaled by 1e18.
        // Annualise: supply_rate * 31_536_000, then convert to bps: / 1e14.
        let annualised = supply_rate as u128 * 31_536_000u128;
        let rate_bps = (annualised / 10u128.pow(14)) as u64;

        Ok(SourceReading {
            source: self.name.clone(),
            rate_bps,
            timestamp_ms: now_ms,
            is_live: true,
            metadata: serde_json::json!({
                "protocol": "compound-v3",
                "comet": format!("{}", self.comet),
                "raw_supply_rate": supply_rate.to_string(),
                "utilization": utilization.to_string(),
            }),
        })
    }
}

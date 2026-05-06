//! Ethena sUSDe ISFR source — reads yield from the sUSDe vault.
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
    interface ISUSDe {
        function vestingAmount() external view returns (uint256);
        function totalAssets() external view returns (uint256);
    }
}

/// Ethena sUSDe structured yield source.
///
/// `vestingAmount` is distributed over 8 hours (28800 s).
/// Annualised yield in bps: `(vestingAmount * 31_536_000 * 10_000) / (28800 * totalAssets)`.
pub struct EthenaSource {
    name: String,
    weight: f64,
    provider: Arc<DynProvider>,
    susde: Address,
}

impl EthenaSource {
    /// Create a new Ethena source targeting a specific sUSDe vault.
    pub fn new(name: String, weight: f64, provider: Arc<DynProvider>, susde: Address) -> Self {
        Self {
            name,
            weight,
            provider,
            susde,
        }
    }
}

#[async_trait]
impl ISFRSource for EthenaSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn weight(&self) -> f64 {
        self.weight
    }

    fn rate_class(&self) -> RateClass {
        RateClass::Structured
    }

    async fn fetch_rate(&self) -> anyhow::Result<SourceReading> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before UNIX epoch")?
            .as_millis() as u64;

        let contract = ISUSDe::new(self.susde, &*self.provider);

        let vesting_amount = contract
            .vestingAmount()
            .call()
            .await
            .context("Ethena vestingAmount call failed")?;

        let total_assets = contract
            .totalAssets()
            .call()
            .await
            .context("Ethena totalAssets call failed")?;

        // rate_bps = (vestingAmount * 31_536_000 * 10_000) / (28800 * totalAssets)
        // Use U256 arithmetic to avoid overflow.
        let rate_bps = if total_assets.is_zero() {
            0u64
        } else {
            let numerator = vesting_amount
                * alloy::primitives::U256::from(31_536_000u64)
                * alloy::primitives::U256::from(10_000u64);
            let denominator = alloy::primitives::U256::from(28_800u64) * total_assets;
            let result = numerator / denominator;
            // Safe to truncate: yield in bps fits in u64.
            result.to::<u64>()
        };

        Ok(SourceReading {
            source: self.name.clone(),
            rate_bps,
            timestamp_ms: now_ms,
            is_live: true,
            metadata: serde_json::json!({
                "protocol": "ethena",
                "susde": format!("{}", self.susde),
                "vesting_amount": vesting_amount.to_string(),
                "total_assets": total_assets.to_string(),
            }),
        })
    }
}

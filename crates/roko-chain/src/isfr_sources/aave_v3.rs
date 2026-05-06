//! Aave V3 ISFR source — reads supply rate from the Aave V3 Pool contract.
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
    interface IAavePool {
        function getReserveData(address asset) external view returns (
            uint256 configuration,
            uint128 liquidityIndex,
            uint128 currentLiquidityRate,
            uint128 variableBorrowIndex,
            uint128 currentVariableBorrowRate,
            uint128 currentStableBorrowRate,
            uint40 lastUpdateTimestamp,
            uint16 id,
            address aTokenAddress,
            address stableDebtTokenAddress,
            address variableDebtTokenAddress,
            address interestRateStrategyAddress,
            uint128 accruedToTreasury,
            uint128 unbacked,
            uint128 isolationModeTotalDebt
        );
    }
}

/// Aave V3 lending rate source.
///
/// Reads `currentLiquidityRate` from the Aave V3 Pool contract.
/// The rate is in RAY (1e27); we convert to basis points by dividing by 1e23.
pub struct AaveV3Source {
    name: String,
    weight: f64,
    provider: Arc<DynProvider>,
    pool: Address,
    asset: Address,
}

impl AaveV3Source {
    /// Create a new Aave V3 source targeting a specific pool and asset.
    pub fn new(
        name: String,
        weight: f64,
        provider: Arc<DynProvider>,
        pool: Address,
        asset: Address,
    ) -> Self {
        Self {
            name,
            weight,
            provider,
            pool,
            asset,
        }
    }
}

#[async_trait]
impl ISFRSource for AaveV3Source {
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

        let contract = IAavePool::new(self.pool, &*self.provider);
        let result = contract
            .getReserveData(self.asset)
            .call()
            .await
            .context("Aave V3 getReserveData call failed")?;

        // currentLiquidityRate is in RAY (1e27). Convert to bps: rate / 1e23.
        let liquidity_rate = result.currentLiquidityRate;
        let ray_to_bps_divisor: u128 = 10u128.pow(23);
        let rate_bps = (liquidity_rate / ray_to_bps_divisor) as u64;

        Ok(SourceReading {
            source: self.name.clone(),
            rate_bps,
            timestamp_ms: now_ms,
            is_live: true,
            metadata: serde_json::json!({
                "protocol": "aave-v3",
                "pool": format!("{}", self.pool),
                "asset": format!("{}", self.asset),
                "raw_liquidity_rate": liquidity_rate.to_string(),
            }),
        })
    }
}

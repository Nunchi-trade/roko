//! On-chain rate submission to ISFROracle contract.
//!
//! Called from ISFRKeeper after each tick when oracle submission is configured.
//! Feature-gated behind `alloy-backend`.

use tracing::{info, warn};

/// On-chain oracle configuration for submitting rates to ISFROracle.
#[derive(Clone)]
pub struct OracleSubmitConfig {
    /// ISFROracle contract address (0x-prefixed hex).
    pub oracle_address: String,
    /// HTTP RPC URL for the chain.
    pub rpc_url: String,
    /// Private key for the keeper wallet (0x-prefixed hex).
    pub wallet_key: String,
    /// Chain ID.
    pub chain_id: u64,
}

/// Attempt to submit a rate on-chain. Logs errors but never fails the caller.
///
/// This is intentionally fire-and-forget: on-chain submission is best-effort
/// and must not block the keeper's poll loop.
#[cfg(not(feature = "alloy-backend"))]
pub async fn submit_rate_on_chain(
    _config: &OracleSubmitConfig,
    _epoch_id: u64,
    _composite_bps: u64,
    _lending_bps: u64,
    _structured_bps: u64,
    _funding_bps: u64,
    _staking_bps: u64,
    _confidence_bps: u64,
) {
    warn!("on-chain rate submission requires alloy-backend feature");
}

/// Attempt to submit a rate on-chain via ISFROracle.submitRate(). Logs errors but never fails.
#[cfg(feature = "alloy-backend")]
pub async fn submit_rate_on_chain(
    config: &OracleSubmitConfig,
    epoch_id: u64,
    composite_bps: u64,
    lending_bps: u64,
    structured_bps: u64,
    funding_bps: u64,
    staking_bps: u64,
    confidence_bps: u64,
) {
    use std::sync::Arc;

    use alloy::network::EthereumWallet;
    use alloy::primitives::{Address, U256};
    use alloy::providers::{DynProvider, Provider as _, ProviderBuilder};
    use alloy::signers::local::PrivateKeySigner;
    use alloy::sol;

    sol! {
        #[sol(rpc)]
        interface IISFROracle {
            function submitRate(
                uint256 epochId,
                uint256 compositeBps,
                uint256 lendingBps,
                uint256 structuredBps,
                uint256 fundingBps,
                uint256 stakingBps,
                uint256 confidenceBps
            ) external;
        }
    }

    let result: anyhow::Result<()> = (async {
        let url = reqwest::Url::parse(&config.rpc_url)
            .map_err(|e| anyhow::anyhow!("invalid rpc url: {e}"))?;
        let trimmed = config.wallet_key.trim_start_matches("0x");
        let signer: PrivateKeySigner = trimmed
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid wallet key: {e}"))?;
        let deployer = signer.address();
        let provider: Arc<DynProvider> = Arc::new(
            ProviderBuilder::new()
                .wallet(EthereumWallet::from(signer))
                .connect_http(url)
                .erased(),
        );
        let oracle_addr: Address = config
            .oracle_address
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid oracle address: {e}"))?;

        let oracle = IISFROracle::new(oracle_addr, &*provider);
        oracle
            .submitRate(
                U256::from(epoch_id),
                U256::from(composite_bps),
                U256::from(lending_bps),
                U256::from(structured_bps),
                U256::from(funding_bps),
                U256::from(staking_bps),
                U256::from(confidence_bps),
            )
            .from(deployer)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("submitRate send: {e}"))?
            .get_receipt()
            .await
            .map_err(|e| anyhow::anyhow!("submitRate receipt: {e}"))?;

        info!(
            epoch_id,
            composite_bps, confidence_bps, "on-chain rate submitted to ISFROracle"
        );
        Ok(())
    })
    .await;

    if let Err(e) = result {
        warn!(error = %e, epoch_id, "failed to submit rate on-chain (non-fatal)");
    }
}

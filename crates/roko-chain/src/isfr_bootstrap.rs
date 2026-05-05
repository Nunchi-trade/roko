//! ISFR contract bootstrap — deployment sequence for dev chains.
//!
//! Used only when `ChainProfile::auto_deploy` is true (mirage profile).
//! Daeji and production chains use pre-deployed addresses from roko.toml.

use crate::chain_profile::ContractAddresses;
use tracing::info;

/// Deployment order (each contract depends on those above it):
/// 1. RoleRegistry  — no deps
/// 2. MockERC20     — no deps
/// 3. WorkerRegistry(roleRegistry)
/// 4. ISFROracle(workerRegistry, roleRegistry)
/// 5. ISFRBountyPool(isfrOracle, erc20, roleRegistry)
///
/// Post-deploy steps:
/// - Grant KEEPER_ROLE to deployer in RoleRegistry
/// - Register deployer in WorkerRegistry
/// - Mint 10,000 tokens to deployer via MockERC20
/// - Approve + fund ISFRBountyPool
///
/// Returns populated ContractAddresses on success.
pub async fn bootstrap_isfr(
    _rpc_url: &str,
    _deployer_key: &str,
) -> anyhow::Result<ContractAddresses> {
    info!("ISFR bootstrap: placeholder (alloy deployment not yet wired)");

    // TODO: Wire with alloy when chain integration is active.
    //
    // Pattern (alloy 1.x):
    //   let provider = ProviderBuilder::new()
    //       .with_recommended_fillers()
    //       .wallet(EthereumWallet::from(signer))
    //       .on_http(rpc_url.parse()?);
    //
    // Then deploy each contract in order using sol!() generated types.

    Ok(ContractAddresses::default())
}

/// Verify ISFR contracts are live at the configured addresses.
///
/// Calls `getCurrentRate()` on ISFROracle and `workerCount()` on WorkerRegistry
/// to confirm the contracts are responsive.
pub async fn verify_isfr_deployment(
    _rpc_url: &str,
    contracts: &ContractAddresses,
) -> anyhow::Result<()> {
    if contracts.isfr_oracle.is_none() {
        anyhow::bail!("ISFROracle address not configured");
    }
    if contracts.worker_registry.is_none() {
        anyhow::bail!("WorkerRegistry address not configured");
    }
    // TODO: Issue eth_call to getCurrentRate() / workerCount() once alloy is wired.
    info!("ISFR deployment verification: placeholder (contracts not yet called)");
    Ok(())
}

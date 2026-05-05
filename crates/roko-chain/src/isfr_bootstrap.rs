//! ISFR contract bootstrap — deployment sequence for dev chains.
//!
//! Used only when `ChainProfile::auto_deploy` is true (mirage profile).
//! Daeji and production chains use pre-deployed addresses from roko.toml.

use crate::chain_profile::ContractAddresses;
use tracing::info;

// ─── Non-alloy stub ──────────────────────────────────────────────────────────
// When `alloy-backend` is disabled, provide a stub that always returns
// ContractAddresses::default() so callers can compile without cfg gates.

/// Deployment order (each contract depends on those above it):
/// 1. RoleRegistry(admin)          — no deps
/// 2. MockERC20("DAEJI","DAEJI",18) — no deps
/// 3. WorkerRegistry(token_address)
/// 4. ISFROracle(role_registry, worker_registry)
/// 5. ISFRBountyPool(role_registry, token, reward_per_submission)
///
/// Post-deploy wiring:
/// - oracle.setBountyPool(bounty_pool)
/// - roleRegistry.grantRole(ORACLE_ROLE, oracle)   (so oracle can call bounty pool)
/// - roleRegistry.grantRole(KEEPER_ROLE, deployer)  (so deployer can submit rates)
/// - mockERC20.mint(deployer, 10_000e18)
/// - mockERC20.approve(bounty_pool, 10_000e18)
///
/// Returns populated ContractAddresses on success.
#[cfg(not(feature = "alloy-backend"))]
pub async fn bootstrap_isfr(
    _rpc_url: &str,
    _deployer_key: &str,
    _contracts_dir: &std::path::Path,
) -> anyhow::Result<ContractAddresses> {
    info!("ISFR bootstrap: alloy-backend feature not enabled, returning defaults");
    Ok(ContractAddresses::default())
}

/// Verify ISFR contracts are live at the configured addresses (stub).
#[cfg(not(feature = "alloy-backend"))]
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
    info!("ISFR deployment verification: alloy-backend not enabled, skipping on-chain checks");
    Ok(())
}

// ─── Alloy-backed implementation ─────────────────────────────────────────────

#[cfg(feature = "alloy-backend")]
mod alloy_bootstrap {
    use std::path::Path;
    use std::sync::Arc;

    use alloy::dyn_abi::DynSolValue;
    use alloy::hex;
    use alloy::network::{EthereumWallet, TransactionBuilder};
    use alloy::primitives::{Address, Bytes, U256};
    use alloy::providers::{DynProvider, Provider, ProviderBuilder};
    use alloy::rpc::types::eth::TransactionRequest;
    use alloy::signers::local::PrivateKeySigner;
    use alloy::sol;
    use anyhow::Context;
    use serde::Deserialize;
    use tracing::info;

    use crate::chain_profile::ContractAddresses;

    // ── sol! ABI bindings for post-deploy wiring ─────────────────────────

    sol! {
        #[sol(rpc)]
        interface IRoleRegistry {
            function grantRole(bytes32 role, address account) external;
        }

        #[sol(rpc)]
        interface IISFROracle {
            function setBountyPool(address pool) external;
            function getCurrentRate() external view returns (uint256);
            function KEEPER_ROLE() external view returns (bytes32);
        }

        #[sol(rpc)]
        interface IISFRBountyPool {
            function ORACLE_ROLE() external view returns (bytes32);
            function availableBalance() external view returns (uint256);
        }

        #[sol(rpc)]
        interface IERC20Mintable {
            function mint(address to, uint256 amount) external;
            function approve(address spender, uint256 amount) external returns (bool);
        }

        #[sol(rpc)]
        interface IWorkerRegistry {
            function registeredCount() external view returns (uint256);
        }
    }

    // ── Forge artifact loader ────────────────────────────────────────────

    #[derive(Deserialize)]
    struct ForgeArtifact {
        bytecode: ForgeBytecode,
    }

    #[derive(Deserialize)]
    struct ForgeBytecode {
        object: String,
    }

    fn load_bytecode(contracts_dir: &Path, name: &str) -> anyhow::Result<Bytes> {
        let path = contracts_dir
            .join("out")
            .join(format!("{name}.sol"))
            .join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read artifact {}", path.display()))?;
        let raw: ForgeArtifact = serde_json::from_str(&text)
            .with_context(|| format!("parse artifact {}", path.display()))?;
        let hex_str = raw.bytecode.object.trim_start_matches("0x");
        let bytes = hex::decode(hex_str).with_context(|| format!("decode bytecode for {name}"))?;
        anyhow::ensure!(
            !bytes.is_empty(),
            "empty bytecode for {name} (run `forge build`?)"
        );
        Ok(Bytes::from(bytes))
    }

    // ── Deploy helper ────────────────────────────────────────────────────

    async fn deploy_contract(
        provider: &Arc<DynProvider>,
        deployer: Address,
        chain_id: u64,
        name: &str,
        init_code: Bytes,
        constructor_args: &[DynSolValue],
    ) -> anyhow::Result<Address> {
        let mut code = init_code.to_vec();
        if !constructor_args.is_empty() {
            let tuple = DynSolValue::Tuple(constructor_args.to_vec());
            code.extend_from_slice(&tuple.abi_encode_params());
        }

        let tx = TransactionRequest::default()
            .with_from(deployer)
            .with_deploy_code(Bytes::from(code))
            .with_chain_id(chain_id);

        let pending = provider
            .send_transaction(tx)
            .await
            .with_context(|| format!("submit deploy of {name}"))?;
        let receipt = pending
            .get_receipt()
            .await
            .with_context(|| format!("receipt for {name}"))?;

        anyhow::ensure!(
            receipt.status(),
            "deploy of {name} reverted (tx={}, block={})",
            receipt.transaction_hash,
            receipt.block_number.unwrap_or(0),
        );

        let address = receipt
            .contract_address
            .with_context(|| format!("missing contract_address for {name}"))?;

        info!(
            contract = %name,
            address = %format!("{address:#x}"),
            block = receipt.block_number.unwrap_or(0),
            "deployed"
        );

        Ok(address)
    }

    // ── Public entry point ───────────────────────────────────────────────

    /// Deploy ISFR contracts (RoleRegistry, MockERC20, WorkerRegistry,
    /// ISFROracle, ISFRBountyPool) and wire roles/approvals.
    #[allow(clippy::too_many_lines)]
    pub async fn bootstrap_isfr(
        rpc_url: &str,
        deployer_key: &str,
        contracts_dir: &Path,
    ) -> anyhow::Result<ContractAddresses> {
        info!(
            "ISFR bootstrap: deploying contracts from {}",
            contracts_dir.display()
        );

        // Build provider with signing wallet.
        let url =
            reqwest::Url::parse(rpc_url).with_context(|| format!("invalid rpc url: {rpc_url}"))?;
        let trimmed = deployer_key.trim_start_matches("0x");
        let signer: PrivateKeySigner = trimmed.parse().context("invalid deployer private key")?;
        let deployer = signer.address();
        let provider: Arc<DynProvider> = Arc::new(
            ProviderBuilder::new()
                .wallet(EthereumWallet::from(signer))
                .connect_http(url)
                .erased(),
        );

        let chain_id = provider.get_chain_id().await.context("get chain_id")?;

        info!(deployer = %format!("{deployer:#x}"), chain_id, "ISFR bootstrap wallet ready");

        // Fund the deployer on dev chains (Anvil/mirage-rs forks don't auto-fund
        // the default key with ETH). 10_000 ETH is plenty for contract deploys.
        let balance: U256 = provider
            .get_balance(deployer)
            .await
            .unwrap_or(U256::ZERO);
        if balance < U256::from(1_000_000_000_000_000_000u128) {
            info!("deployer balance too low ({balance}), funding via anvil_setBalance");
            let fund_amount = U256::from(10_000u64) * U256::from(10u64).pow(U256::from(18));
            let _: bool = provider
                .raw_request(
                    "anvil_setBalance".into(),
                    (deployer, fund_amount),
                )
                .await
                .context("anvil_setBalance to fund deployer")?;
            info!("deployer funded with 10,000 ETH");
        }

        // ── 1. RoleRegistry(admin) ───────────────────────────────────────
        let role_registry_bytecode = load_bytecode(contracts_dir, "RoleRegistry")?;
        let role_registry_addr = deploy_contract(
            &provider,
            deployer,
            chain_id,
            "RoleRegistry",
            role_registry_bytecode,
            &[DynSolValue::Address(deployer)],
        )
        .await?;

        // ── 2. MockERC20("DAEJI", "DAEJI", 18) ─────────────────────────
        let mock_erc20_bytecode = load_bytecode(contracts_dir, "MockERC20")?;
        let token_addr = deploy_contract(
            &provider,
            deployer,
            chain_id,
            "MockERC20",
            mock_erc20_bytecode,
            &[
                DynSolValue::String("DAEJI".to_string()),
                DynSolValue::String("DAEJI".to_string()),
                DynSolValue::Uint(U256::from(18), 8),
            ],
        )
        .await?;

        // ── 3. WorkerRegistry(stakeToken) ───────────────────────────────
        let worker_registry_bytecode = load_bytecode(contracts_dir, "WorkerRegistry")?;
        let worker_registry_addr = deploy_contract(
            &provider,
            deployer,
            chain_id,
            "WorkerRegistry",
            worker_registry_bytecode,
            &[DynSolValue::Address(token_addr)],
        )
        .await?;

        // ── 4. ISFROracle(roleRegistry, workerRegistry) ─────────────────
        let oracle_bytecode = load_bytecode(contracts_dir, "ISFROracle")?;
        let oracle_addr = deploy_contract(
            &provider,
            deployer,
            chain_id,
            "ISFROracle",
            oracle_bytecode,
            &[
                DynSolValue::Address(role_registry_addr),
                DynSolValue::Address(worker_registry_addr),
            ],
        )
        .await?;

        // ── 5. ISFRBountyPool(roleRegistry, token, rewardPerSubmission) ──
        let bounty_pool_bytecode = load_bytecode(contracts_dir, "ISFRBountyPool")?;
        let reward_per_submission = U256::from(1_000_000_000_000_000_000u128); // 1 ether
        let bounty_pool_addr = deploy_contract(
            &provider,
            deployer,
            chain_id,
            "ISFRBountyPool",
            bounty_pool_bytecode,
            &[
                DynSolValue::Address(role_registry_addr),
                DynSolValue::Address(token_addr),
                DynSolValue::Uint(reward_per_submission, 256),
            ],
        )
        .await?;

        // ── Post-deploy wiring ──────────────────────────────────────────

        info!("ISFR bootstrap: wiring contracts");

        // 5a. oracle.setBountyPool(bounty_pool)
        let oracle = IISFROracle::new(oracle_addr, &*provider);
        oracle
            .setBountyPool(bounty_pool_addr)
            .from(deployer)
            .send()
            .await
            .context("oracle.setBountyPool send")?
            .get_receipt()
            .await
            .context("oracle.setBountyPool receipt")?;
        info!("oracle.setBountyPool wired");

        // 5b. Get ORACLE_ROLE from bounty pool, grant to oracle in role registry.
        let bounty_pool = IISFRBountyPool::new(bounty_pool_addr, &*provider);
        let oracle_role = bounty_pool
            .ORACLE_ROLE()
            .call()
            .await
            .context("bountyPool.ORACLE_ROLE")?;
        let role_reg = IRoleRegistry::new(role_registry_addr, &*provider);
        role_reg
            .grantRole(oracle_role, oracle_addr)
            .from(deployer)
            .send()
            .await
            .context("grantRole(ORACLE_ROLE, oracle) send")?
            .get_receipt()
            .await
            .context("grantRole(ORACLE_ROLE, oracle) receipt")?;
        info!("ORACLE_ROLE granted to oracle");

        // 5c. Get KEEPER_ROLE from oracle, grant to deployer in role registry.
        let keeper_role = oracle
            .KEEPER_ROLE()
            .call()
            .await
            .context("oracle.KEEPER_ROLE")?;
        role_reg
            .grantRole(keeper_role, deployer)
            .from(deployer)
            .send()
            .await
            .context("grantRole(KEEPER_ROLE, deployer) send")?
            .get_receipt()
            .await
            .context("grantRole(KEEPER_ROLE, deployer) receipt")?;
        info!("KEEPER_ROLE granted to deployer");

        // 5d. mockERC20.mint(deployer, 10_000e18)
        let mint_amount = U256::from(10_000u64) * U256::from(10u64).pow(U256::from(18));
        let erc20 = IERC20Mintable::new(token_addr, &*provider);
        erc20
            .mint(deployer, mint_amount)
            .from(deployer)
            .send()
            .await
            .context("mockERC20.mint send")?
            .get_receipt()
            .await
            .context("mockERC20.mint receipt")?;
        info!(amount = %mint_amount, "minted tokens to deployer");

        // 5e. mockERC20.approve(bounty_pool, 10_000e18)
        erc20
            .approve(bounty_pool_addr, mint_amount)
            .from(deployer)
            .send()
            .await
            .context("mockERC20.approve send")?
            .get_receipt()
            .await
            .context("mockERC20.approve receipt")?;
        info!("approved bounty pool for token spending");

        let addrs = ContractAddresses {
            role_registry: Some(format!("{role_registry_addr:#x}")),
            worker_registry: Some(format!("{worker_registry_addr:#x}")),
            isfr_oracle: Some(format!("{oracle_addr:#x}")),
            bounty_pool: Some(format!("{bounty_pool_addr:#x}")),
            bounty_token: Some(format!("{token_addr:#x}")),
            agent_registry: None,
            job_market: None,
        };

        info!(?addrs, "ISFR bootstrap complete");
        Ok(addrs)
    }

    // ── Verification ─────────────────────────────────────────────────────

    /// Verify ISFR contracts are live at the configured addresses by calling
    /// view functions on the oracle and worker registry.
    pub async fn verify_isfr_deployment(
        rpc_url: &str,
        contracts: &ContractAddresses,
    ) -> anyhow::Result<()> {
        let oracle_str = contracts
            .isfr_oracle
            .as_deref()
            .context("ISFROracle address not configured")?;
        let worker_str = contracts
            .worker_registry
            .as_deref()
            .context("WorkerRegistry address not configured")?;

        let url =
            reqwest::Url::parse(rpc_url).with_context(|| format!("invalid rpc url: {rpc_url}"))?;
        let provider: Arc<DynProvider> =
            Arc::new(ProviderBuilder::new().connect_http(url).erased());

        // Check oracle has code.
        let oracle_addr: Address = oracle_str
            .parse()
            .with_context(|| format!("invalid oracle address: {oracle_str}"))?;
        let code = provider
            .get_code_at(oracle_addr)
            .await
            .context("get_code_at oracle")?;
        anyhow::ensure!(
            !code.is_empty(),
            "no code at ISFROracle address {oracle_str}"
        );

        // Call getCurrentRate() as a smoke test.
        let oracle = IISFROracle::new(oracle_addr, &*provider);
        let current_rate = oracle
            .getCurrentRate()
            .call()
            .await
            .context("ISFROracle.getCurrentRate()")?;
        info!(current_rate_bps = %current_rate, "ISFROracle verified");

        // Check worker registry has code.
        let worker_addr: Address = worker_str
            .parse()
            .with_context(|| format!("invalid worker registry address: {worker_str}"))?;
        let code = provider
            .get_code_at(worker_addr)
            .await
            .context("get_code_at worker registry")?;
        anyhow::ensure!(
            !code.is_empty(),
            "no code at WorkerRegistry address {worker_str}"
        );

        // Call registeredCount() as a smoke test.
        let worker_reg = IWorkerRegistry::new(worker_addr, &*provider);
        let worker_count = worker_reg
            .registeredCount()
            .call()
            .await
            .context("WorkerRegistry.registeredCount()")?;
        info!(worker_count = %worker_count, "WorkerRegistry verified");

        info!("ISFR deployment verification passed");
        Ok(())
    }
}

#[cfg(feature = "alloy-backend")]
pub use alloy_bootstrap::{bootstrap_isfr, verify_isfr_deployment};

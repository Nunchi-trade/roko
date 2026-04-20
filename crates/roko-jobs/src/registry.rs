//! Pre-registered job definitions. Ports the `JOB_REGISTRY` dict from
//! `cli/jobs/registry.py`.

use std::collections::HashMap;
use std::sync::OnceLock;

use roko_custody::CustodyPolicy;

use crate::types::{JobCategory, JobDefinition, TriggerType};

// Selector constants mirror the Python registry.
const FUNDING_KEEPER_SETTLE_SELECTOR: &str = "0xce6d8c44";
const DPNL_KEEPER_SETTLE_SELECTOR: &str = "0x5bff25c6";
const LIQ_FLAGGER_FLAG_POSITION_SELECTOR: &str = "0x680f25aa";
const LIQ_FLAGGER_FLAG_ACCOUNT_SELECTOR: &str = "0xfc429602";
const LIQ_EXECUTOR_LIQUIDATE_POSITION_SELECTOR: &str = "0xc74d1f9d";
const LIQ_EXECUTOR_LIQUIDATE_ACCOUNT_SELECTOR: &str = "0x168cbba5";
const LIQ_EXECUTOR_LIQUIDATE_POSITIONS_SELECTOR: &str = "0xf3510b94";
const ORACLE_UPDATE_PRICE_FEEDS_SELECTOR: &str = "0x9045975b";

fn custody(dests: &[&str], selectors: &[&str], rate: u32, value_cap: f64) -> CustodyPolicy {
    CustodyPolicy {
        destinations: dests.iter().map(|s| (*s).to_string()).collect(),
        selectors: selectors.iter().map(|s| (*s).to_string()).collect(),
        value_cap_eth: value_cap,
        rate_limit_per_block: rate,
    }
}

fn build_registry() -> HashMap<String, JobDefinition> {
    let mut m = HashMap::new();

    m.insert(
        "oracle_updater".into(),
        JobDefinition {
            job_id: "oracle_updater".into(),
            name: "Oracle Updater".into(),
            description: "Push fresh oracle prices on each new block.".into(),
            category: JobCategory::Keeper,
            trigger: TriggerType::NewBlock,
            trigger_config: HashMap::new(),
            required_role: None,
            requires_tee: false,
            min_stake_eth: 10.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &[ORACLE_UPDATE_PRICE_FEEDS_SELECTOR], 2, 0.0),
            strategy_interface: "roko_jobs::KeeperStrategy".into(),
            context_template: "KeeperContext".into(),
            engine_type: "keeper".into(),
            default_strategy: Some("cli.jobs.keepers.oracle:OracleKeeperStrategy".into()),
        },
    );
    m.insert(
        "funding_keeper".into(),
        JobDefinition {
            job_id: "funding_keeper".into(),
            name: "Funding Keeper".into(),
            description: "Sample mark-vs-index premium, compute TWAP, and settle funding rates.".into(),
            category: JobCategory::Keeper,
            trigger: TriggerType::NewBlock,
            trigger_config: HashMap::new(),
            required_role: Some("AUTHORIZED".into()),
            requires_tee: false,
            min_stake_eth: 10.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &[FUNDING_KEEPER_SETTLE_SELECTOR], 1, 0.0),
            strategy_interface: "roko_jobs::KeeperStrategy".into(),
            context_template: "KeeperContext".into(),
            engine_type: "keeper".into(),
            default_strategy: Some("cli.jobs.keepers.funding:FundingKeeperStrategy".into()),
        },
    );
    m.insert(
        "dpnl_keeper".into(),
        JobDefinition {
            job_id: "dpnl_keeper".into(),
            name: "DPNL Keeper".into(),
            description: "Settle dPNL windows from the oracle funding rate.".into(),
            category: JobCategory::Keeper,
            trigger: TriggerType::NewBlock,
            trigger_config: HashMap::new(),
            required_role: Some("AUTHORIZED".into()),
            requires_tee: false,
            min_stake_eth: 10.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &[DPNL_KEEPER_SETTLE_SELECTOR], 1, 0.0),
            strategy_interface: "roko_jobs::KeeperStrategy".into(),
            context_template: "KeeperContext".into(),
            engine_type: "keeper".into(),
            default_strategy: Some("cli.jobs.keepers.dpnl:DPNLKeeperStrategy".into()),
        },
    );
    m.insert(
        "liq_flagger".into(),
        JobDefinition {
            job_id: "liq_flagger".into(),
            name: "Liquidation Flagger".into(),
            description: "Flag under-collateralised positions for liquidation.".into(),
            category: JobCategory::Keeper,
            trigger: TriggerType::OracleUpdate,
            trigger_config: HashMap::new(),
            required_role: None,
            requires_tee: false,
            min_stake_eth: 10.0,
            stake_token: "HYPE".into(),
            custody: custody(
                &[],
                &[
                    LIQ_FLAGGER_FLAG_POSITION_SELECTOR,
                    LIQ_FLAGGER_FLAG_ACCOUNT_SELECTOR,
                ],
                5,
                0.0,
            ),
            strategy_interface: "roko_jobs::KeeperStrategy".into(),
            context_template: "KeeperContext".into(),
            engine_type: "keeper".into(),
            default_strategy: Some("cli.jobs.keepers.liquidation:LiqFlaggerStrategy".into()),
        },
    );
    m.insert(
        "liq_executor".into(),
        JobDefinition {
            job_id: "liq_executor".into(),
            name: "Liquidation Executor".into(),
            description: "Execute liquidations on flagged positions.".into(),
            category: JobCategory::Operator,
            trigger: TriggerType::Event,
            trigger_config: HashMap::from([(
                "event_name".to_string(),
                serde_json::Value::String("PositionFlagged".into()),
            )]),
            required_role: Some("OPERATOR_ROLE".into()),
            requires_tee: false,
            min_stake_eth: 50.0,
            stake_token: "HYPE".into(),
            custody: custody(
                &[],
                &[
                    LIQ_EXECUTOR_LIQUIDATE_POSITION_SELECTOR,
                    LIQ_EXECUTOR_LIQUIDATE_ACCOUNT_SELECTOR,
                    LIQ_EXECUTOR_LIQUIDATE_POSITIONS_SELECTOR,
                ],
                10,
                0.0,
            ),
            strategy_interface: "roko_jobs::KeeperStrategy".into(),
            context_template: "KeeperContext".into(),
            engine_type: "keeper".into(),
            default_strategy: Some("cli.jobs.keepers.liquidation:LiqExecutorStrategy".into()),
        },
    );
    m.insert(
        "market_maker".into(),
        JobDefinition {
            job_id: "market_maker".into(),
            name: "Market Maker".into(),
            description: "Provide two-sided liquidity via TEE-cleared cooperative rounds.".into(),
            category: JobCategory::Cooperative,
            trigger: TriggerType::ClearingRound,
            trigger_config: HashMap::new(),
            required_role: None,
            requires_tee: true,
            min_stake_eth: 100.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &["0x00000000"], 1, 0.0),
            strategy_interface: "roko_jobs::CooperativeStrategy".into(),
            context_template: "StrategyContext".into(),
            engine_type: "cooperative".into(),
            default_strategy: None,
        },
    );
    m.insert(
        "abm_agent".into(),
        JobDefinition {
            job_id: "abm_agent".into(),
            name: "ABM Agent".into(),
            description: "Automated bin management for concentrated liquidity via TEE.".into(),
            category: JobCategory::Cooperative,
            trigger: TriggerType::OracleUpdate,
            trigger_config: HashMap::from([(
                "deviation_threshold_bps".to_string(),
                serde_json::Value::from(50),
            )]),
            required_role: None,
            requires_tee: true,
            min_stake_eth: 100.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &["0x00000000"], 1, 0.0),
            strategy_interface: "roko_jobs::CooperativeStrategy".into(),
            context_template: "StrategyContext".into(),
            engine_type: "cooperative".into(),
            default_strategy: None,
        },
    );
    m.insert(
        "glv_manager".into(),
        JobDefinition {
            job_id: "glv_manager".into(),
            name: "GLV Capital Manager".into(),
            description: "Manage vault capital allocation, deposits, withdrawals, and harvesting.".into(),
            category: JobCategory::Managed,
            trigger: TriggerType::Timer,
            trigger_config: HashMap::from([
                ("interval_s".to_string(), serde_json::Value::from(60)),
                (
                    "also_on_event".to_string(),
                    serde_json::Value::String("WithdrawRequested".into()),
                ),
            ]),
            required_role: Some("MANAGER_ROLE".into()),
            requires_tee: false,
            min_stake_eth: 50.0,
            stake_token: "HYPE".into(),
            custody: custody(&[], &["0x00000000"], 2, 10.0),
            strategy_interface: "roko_jobs::ManagedStrategy".into(),
            context_template: "ManagedContext".into(),
            engine_type: "managed".into(),
            default_strategy: None,
        },
    );

    m
}

static REGISTRY: OnceLock<HashMap<String, JobDefinition>> = OnceLock::new();

pub fn job_registry() -> &'static HashMap<String, JobDefinition> {
    REGISTRY.get_or_init(build_registry)
}

pub fn get_job(job_id: &str) -> Result<&'static JobDefinition, String> {
    job_registry()
        .get(job_id)
        .ok_or_else(|| format!("unknown job {job_id:?}"))
}

pub fn list_jobs() -> Vec<&'static JobDefinition> {
    let mut v: Vec<&JobDefinition> = job_registry().values().collect();
    v.sort_by(|a, b| a.job_id.cmp(&b.job_id));
    v
}

pub fn list_jobs_by_category(category: JobCategory) -> Vec<&'static JobDefinition> {
    list_jobs().into_iter().filter(|j| j.category == category).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_expected_jobs() {
        let r = job_registry();
        for id in ["oracle_updater", "funding_keeper", "liq_executor", "market_maker", "glv_manager"] {
            assert!(r.contains_key(id), "missing job {id}");
        }
    }

    #[test]
    fn category_filter_works() {
        let keepers = list_jobs_by_category(JobCategory::Keeper);
        assert!(keepers.iter().all(|j| matches!(j.category, JobCategory::Keeper)));
        assert!(keepers.iter().any(|j| j.job_id == "oracle_updater"));
    }

    #[test]
    fn get_job_returns_err_on_unknown() {
        assert!(get_job("does-not-exist").is_err());
    }
}

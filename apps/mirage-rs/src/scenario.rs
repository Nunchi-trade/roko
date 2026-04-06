//! Scenario definition and execution support.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_primitives::{Address, Bytes, U256, hex};
use parking_lot::RwLock;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    MirageError, TransactionRequest,
    cow::CowState,
    fork::{
        ClassificationConfig, DiffClassifier, DirtyAccount, DirtyStore, EvmExecutor, ForkState,
        HybridDB, MirageState, ReadCache, lock_state_writes,
    },
    replay::LogEntry,
};

const DEFAULT_SCENARIO_TIMEOUT: Duration = Duration::from_secs(15);

/// Execution mode for a scenario set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    /// Reuse one baseline and revert between scenarios.
    Sequential,
    /// Clone the baseline and run branches independently.
    Parallel,
}

/// Lifecycle state for a scenario set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScenarioSetStatus {
    /// Waiting for scenarios to be defined.
    Draft,
    /// Currently executing.
    Running,
    /// Execution finished successfully.
    Complete,
    /// Execution failed.
    Failed,
}

/// One scenario inside a scenario set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scenario {
    /// Stable scenario ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Transactions to execute.
    pub transactions: Vec<TransactionRequest>,
    /// Addresses whose balances should be captured after execution.
    pub track_addresses: Vec<Address>,
    /// Optional gas ceiling.
    pub max_gas: Option<u64>,
    /// Scenario timeout.
    pub timeout: Duration,
    /// Optional post-run assertions used by fixture-driven scenario tests.
    #[serde(default)]
    pub assertions: ScenarioAssertions,
}

/// Post-run assertions for built-in scenario fixtures.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioAssertions {
    /// Addresses that should appear in the watch list after execution.
    #[serde(default)]
    #[serde(alias = "watch_list_contains")]
    pub watch_list_contains: Vec<Address>,
    /// Optional token balance lower-bound check.
    #[serde(alias = "token_balance_gte")]
    pub token_balance_gte: Option<TokenBalanceAssertion>,
}

/// Lower-bound token balance assertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalanceAssertion {
    /// Token contract to inspect.
    pub token: Address,
    /// Account expected to hold the balance.
    pub address: Address,
    /// Lower bound for the balance.
    pub amount: U256,
}

#[derive(Debug, Deserialize)]
struct ScenarioFixture {
    scenario: ScenarioMeta,
    #[serde(default)]
    transactions: Vec<TransactionToml>,
    #[serde(default)]
    assertions: ScenarioAssertions,
    track: TrackConfig,
}

#[derive(Debug, Deserialize)]
struct ScenarioMeta {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(
        default,
        alias = "maxGas",
        deserialize_with = "deserialize_optional_u64_quantity"
    )]
    max_gas: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_duration")]
    timeout: Option<Duration>,
    #[serde(default, alias = "trackAddresses")]
    track_addresses: Vec<Address>,
}

#[derive(Debug, Default, Deserialize)]
struct TrackConfig {
    #[serde(default)]
    addresses: Vec<Address>,
}

#[derive(Debug, Deserialize)]
struct TransactionToml {
    from: Address,
    to: Option<Address>,
    #[serde(default, alias = "input")]
    data: Bytes,
    #[serde(default)]
    value: U256,
    #[serde(deserialize_with = "deserialize_u64_quantity")]
    gas: u64,
}

fn deserialize_u64_quantity<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_optional_u64_quantity(deserializer)?
        .ok_or_else(|| serde::de::Error::custom("missing numeric value"))
}

fn deserialize_optional_u64_quantity<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum QuantityValue {
        Number(u64),
        Text(String),
        Null,
    }

    match Option::<QuantityValue>::deserialize(deserializer)? {
        None | Some(QuantityValue::Null) => Ok(None),
        Some(QuantityValue::Number(value)) => Ok(Some(value)),
        Some(QuantityValue::Text(text)) => parse_u64_quantity(&text)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

fn deserialize_optional_duration<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum DurationValue {
        Number(u64),
        Text(String),
        Null,
    }

    match Option::<DurationValue>::deserialize(deserializer)? {
        None | Some(DurationValue::Null) => Ok(None),
        Some(DurationValue::Number(seconds)) => Ok(Some(Duration::from_secs(seconds))),
        Some(DurationValue::Text(text)) => parse_duration(&text)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

fn parse_u64_quantity(text: &str) -> std::result::Result<u64, String> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        text.parse::<u64>().map_err(|error| error.to_string())
    }
}

fn parse_duration(text: &str) -> std::result::Result<Duration, String> {
    let text = text.trim();
    if let Some(milliseconds) = text.strip_suffix("ms") {
        return parse_u64_quantity(milliseconds.trim()).map(Duration::from_millis);
    }
    if let Some(seconds) = text.strip_suffix('s') {
        return parse_u64_quantity(seconds.trim()).map(Duration::from_secs);
    }
    if let Some(minutes) = text.strip_suffix('m') {
        let minutes = parse_u64_quantity(minutes.trim())?;
        let seconds = minutes
            .checked_mul(60)
            .ok_or_else(|| "duration overflow".to_owned())?;
        return Ok(Duration::from_secs(seconds));
    }
    parse_u64_quantity(text).map(Duration::from_secs)
}

/// Group of scenarios sharing one baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSet {
    /// Stable set ID.
    pub id: String,
    /// Snapshot ID used as the baseline.
    pub baseline_snapshot_id: u64,
    /// Full fork baseline used for deterministic parallel/sequential branching.
    #[serde(skip_serializing, skip_deserializing, default)]
    pub baseline_fork: Option<ForkState>,
    /// Scenarios defined in the set.
    pub scenarios: Vec<Scenario>,
    /// Current set status.
    pub status: ScenarioSetStatus,
}

/// Result status for one scenario.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScenarioStatus {
    /// Scenario is queued or running (not yet terminal).
    Pending,
    /// Execution completed without revert.
    Success,
    /// Transaction reverted.
    Reverted,
    /// Execution exceeded the timeout.
    Timeout,
    /// Execution exceeded a gas bound.
    GasExceeded,
    /// Unhandled execution error.
    Error(String),
}

/// Returns true when `status` is a finished scenario outcome (not [`ScenarioStatus::Pending`]).
#[must_use]
pub fn is_terminal_scenario_status(status: &ScenarioStatus) -> bool {
    !matches!(status, ScenarioStatus::Pending)
}

/// INV-013: only `None`/`Pending` may advance; terminal states cannot transition further.
#[must_use]
pub fn scenario_status_transition_valid(
    from: Option<&ScenarioStatus>,
    to: &ScenarioStatus,
) -> bool {
    match from {
        None => matches!(to, ScenarioStatus::Pending),
        Some(ScenarioStatus::Pending) => matches!(
            to,
            ScenarioStatus::Success
                | ScenarioStatus::Reverted
                | ScenarioStatus::Timeout
                | ScenarioStatus::GasExceeded
                | ScenarioStatus::Error(_)
        ),
        Some(_) => false,
    }
}

/// Result for one scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioResult {
    /// Scenario ID.
    pub scenario_id: String,
    /// Scenario name.
    pub name: String,
    /// Final scenario status.
    pub status: ScenarioStatus,
    /// Aggregate gas used.
    pub gas_used: u64,
    /// Wall-clock time in milliseconds.
    pub wall_time_ms: u64,
    /// Approximate peak memory during execution.
    pub peak_memory_bytes: u64,
    /// Net profit/loss across tracked balances, measured in wei.
    pub pnl_wei: i128,
    /// Number of accounts touched by the scenario.
    pub state_diff_accounts: usize,
    /// Number of storage slots written by the scenario.
    pub state_diff_storage_slots: usize,
    /// Final balances for tracked addresses.
    pub final_balances: HashMap<Address, U256>,
    /// Opaque protocol-specific state payload.
    pub position_state: serde_json::Value,
    /// Logs emitted by the scenario.
    pub logs: Vec<LogEntry>,
    /// Optional revert reason.
    pub revert_reason: Option<String>,
}

/// Background scenario job status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    /// Still running.
    Running,
    /// Finished successfully.
    Complete,
    /// Failed before completion.
    Failed,
}

/// Async execution tracking payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioJob {
    /// Job ID.
    pub job_id: String,
    /// Scenario set ID.
    pub set_id: String,
    /// Job status.
    pub status: JobStatus,
    /// Results once complete.
    pub results: Option<Vec<ScenarioResult>>,
    /// Aggregate runtime once complete.
    pub total_wall_time_ms: Option<u64>,
}

/// Sorts scenario results by the comparison contract used by the RPC API.
pub(crate) fn rank_scenario_results(mut results: Vec<ScenarioResult>) -> Vec<ScenarioResult> {
    results.sort_by(|left, right| {
        right
            .pnl_wei
            .cmp(&left.pnl_wei)
            .then_with(|| left.gas_used.cmp(&right.gas_used))
            .then_with(|| {
                left.state_diff_storage_slots
                    .cmp(&right.state_diff_storage_slots)
            })
            .then_with(|| left.state_diff_accounts.cmp(&right.state_diff_accounts))
            .then_with(|| left.wall_time_ms.cmp(&right.wall_time_ms))
    });
    results
}

/// Scenario runner using the shared fork state.
#[derive(Debug, Clone)]
pub struct ScenarioRunner {
    state: Arc<RwLock<MirageState>>,
}

impl ScenarioRunner {
    /// Creates a new runner.
    #[must_use]
    pub(crate) const fn new(state: Arc<RwLock<MirageState>>) -> Self {
        Self { state }
    }

    /// Executes each scenario with snapshot/revert between branches.
    pub async fn run_sequential(&self, set: &ScenarioSet) -> Vec<ScenarioResult> {
        let baseline = set
            .baseline_fork
            .clone()
            .unwrap_or_else(|| self.state.read().fork.clone());
        let set = set.clone();
        match tokio::task::spawn_blocking(move || execute_scenario_set(baseline, &set)).await {
            Ok((results, executed_fork)) => {
                let _writer_guard = lock_state_writes(&self.state).await;
                self.state.write().fork.adopt_executed_branch(executed_fork);
                results
            }
            Err(error) => vec![ScenarioResult {
                scenario_id: "join-error".to_owned(),
                name: "join-error".to_owned(),
                status: ScenarioStatus::Error(error.to_string()),
                gas_used: 0,
                wall_time_ms: 0,
                peak_memory_bytes: 0,
                pnl_wei: 0,
                state_diff_accounts: 0,
                state_diff_storage_slots: 0,
                final_balances: HashMap::new(),
                position_state: serde_json::json!({"mode": "raw-balances"}),
                logs: Vec::new(),
                revert_reason: Some(error.to_string()),
            }],
        }
    }

    /// Executes each scenario on an isolated [`CowState`] branch built from a shared storage baseline.
    ///
    /// The template fork is kept behind an [`Arc`] so branches avoid deep-cloning the full
    /// [`ForkState`] for every scenario; only [`fork_from_template_and_cow`] materialization runs per task.
    pub async fn run_parallel(&self, set: &ScenarioSet) -> Vec<ScenarioResult> {
        let baseline = set
            .baseline_fork
            .clone()
            .unwrap_or_else(|| self.state.read().fork.clone());
        let template = Arc::new(baseline);
        let storage_baseline = Arc::new(storage_baseline_from_fork(template.as_ref()));
        let tasks = set.scenarios.clone().into_iter().map(|scenario| {
            let template = Arc::clone(&template);
            let storage_baseline = Arc::clone(&storage_baseline);
            tokio::task::spawn_blocking(move || {
                let cow = CowState::branch(storage_baseline);
                let mut fork = fork_from_template_and_cow(template.as_ref(), &cow);
                execute_scenario(&mut fork, &scenario)
            })
        });
        let joined = futures::future::join_all(tasks).await;
        joined
            .into_iter()
            .map(|result| match result {
                Ok(value) => value,
                Err(error) => ScenarioResult {
                    scenario_id: "join-error".to_owned(),
                    name: "join-error".to_owned(),
                    status: ScenarioStatus::Error(error.to_string()),
                    gas_used: 0,
                    wall_time_ms: 0,
                    peak_memory_bytes: 0,
                    pnl_wei: 0,
                    state_diff_accounts: 0,
                    state_diff_storage_slots: 0,
                    final_balances: HashMap::new(),
                    position_state: serde_json::json!({"mode": "raw-balances"}),
                    logs: Vec::new(),
                    revert_reason: Some(error.to_string()),
                },
            })
            .collect()
    }
}

/// Builds the frozen storage map used as a [`CowState`] baseline for parallel scenario branches.
fn storage_baseline_from_fork(fork: &ForkState) -> HashMap<(Address, U256), U256> {
    let mut storage = HashMap::new();
    for (address, account) in &fork.db.dirty.accounts {
        for (slot, value) in &account.storage {
            storage.insert((*address, *slot), *value);
        }
    }
    storage
}

/// Materializes a runnable fork: account metadata from `template`, storage slots from `cow` reads.
fn fork_from_template_and_cow(template: &ForkState, cow: &CowState) -> ForkState {
    let mut dirty = DirtyStore::default();
    dirty.demote_protocols_to_slot_only = template.db.dirty.demote_protocols_to_slot_only;
    for (address, account) in &template.db.dirty.accounts {
        let mut storage = HashMap::new();
        for slot in account.storage.keys() {
            if let Some(value) = cow.read(*address, *slot) {
                storage.insert(*slot, value);
            }
        }
        dirty.accounts.insert(
            *address,
            DirtyAccount {
                balance: account.balance,
                nonce: account.nonce,
                code: account.code.clone(),
                code_hash: account.code_hash,
                erc20_balance_slot: account.erc20_balance_slot,
                erc20_balances: account.erc20_balances.clone(),
                storage,
            },
        );
    }
    dirty.watch_list = template.db.dirty.watch_list.clone();
    dirty.unwatch_list = template.db.dirty.unwatch_list.clone();
    dirty.total_dirty_slots = template.db.dirty.total_dirty_slots;

    let db = HybridDB {
        dirty,
        read_cache: ReadCache::new(
            template.db.read_cache.entry_count().max(1),
            template.db.cache_ttl,
        ),
        bytecode_cache: Arc::clone(&template.db.bytecode_cache),
        upstream: Arc::clone(&template.db.upstream),
        pinned_block: template.db.pinned_block,
        cache_ttl: template.db.cache_ttl,
        chain_id: template.db.chain_id,
    };

    let mut fork = ForkState::new(db, template.local_block_number, template.chain_id);
    fork.timestamp = template.timestamp;
    fork.next_base_fee_per_gas = template.next_base_fee_per_gas;
    fork.coinbase = template.coinbase;
    fork.prev_randao = template.prev_randao;
    fork.receipts = template.receipts.clone();
    fork.transactions = template.transactions.clone();
    fork.blocks_by_number = template.blocks_by_number.clone();
    fork.blocks_by_hash = template.blocks_by_hash.clone();
    fork.impersonated_accounts = template.impersonated_accounts.clone();
    fork.strict_nonce = template.strict_nonce;
    fork.strict_balance = template.strict_balance;
    fork.verify_signatures = template.verify_signatures;
    fork
}

fn execute_scenario_set(fork_in: ForkState, set: &ScenarioSet) -> (Vec<ScenarioResult>, ForkState) {
    let mut working = match &set.baseline_fork {
        Some(baseline) => baseline.clone(),
        None => fork_in,
    };
    let mut results = Vec::with_capacity(set.scenarios.len());

    for scenario in &set.scenarios {
        if let Some(baseline) = &set.baseline_fork {
            working = baseline.clone();
        }

        let snapshot_id = if set.baseline_snapshot_id == 0 {
            working.snapshot()
        } else {
            set.baseline_snapshot_id
        };

        let result = execute_scenario(&mut working, scenario);
        results.push(result);

        if set.baseline_snapshot_id == 0 {
            let _ = working.revert(snapshot_id);
        }
    }

    (results, working)
}

impl Scenario {
    /// Parses a built-in TOML scenario fixture.
    pub fn from_toml(id: impl Into<String>, input: &str) -> Result<Self, MirageError> {
        let fixture: ScenarioFixture = toml::from_str(input)?;
        let mut track_addresses = fixture.scenario.track_addresses;
        for address in fixture.track.addresses {
            if !track_addresses.contains(&address) {
                track_addresses.push(address);
            }
        }
        Ok(Self {
            id: id.into(),
            name: if fixture.scenario.description.is_empty() {
                fixture.scenario.name
            } else {
                format!(
                    "{} — {}",
                    fixture.scenario.name, fixture.scenario.description
                )
            },
            transactions: fixture
                .transactions
                .into_iter()
                .map(|tx| TransactionRequest {
                    from: Some(tx.from),
                    to: tx.to,
                    gas: Some(tx.gas),
                    value: Some(tx.value),
                    data: Some(tx.data),
                    gas_price: None,
                    nonce: None,
                    chain_id: None,
                })
                .collect(),
            track_addresses,
            max_gas: fixture.scenario.max_gas,
            timeout: fixture.scenario.timeout.unwrap_or(DEFAULT_SCENARIO_TIMEOUT),
            assertions: fixture.assertions,
        })
    }

    /// Evaluates the fixture assertions against the current fork state.
    pub fn evaluate_assertions(
        &self,
        state: &mut crate::fork::ForkState,
    ) -> Result<(), MirageError> {
        for address in &self.assertions.watch_list_contains {
            if !state.db.dirty.watch_list.contains_key(address) {
                return Err(MirageError::Unsupported(format!(
                    "watch list missing {address}"
                )));
            }
        }
        if let Some(assertion) = &self.assertions.token_balance_gte {
            let balance = state
                .db
                .erc20_balance_of(assertion.token, assertion.address)?;
            if balance < assertion.amount {
                return Err(MirageError::Unsupported(format!(
                    "token balance {} below required {}",
                    balance, assertion.amount
                )));
            }
        }
        Ok(())
    }
}

fn execute_scenario(fork: &mut crate::fork::ForkState, scenario: &Scenario) -> ScenarioResult {
    let started = Instant::now();
    let mut gas_used = 0_u64;
    let mut logs = Vec::new();
    let initial_balances = tracked_balances(fork, &scenario.track_addresses);
    let mut state_diff_accounts = 0_usize;
    let mut state_diff_storage_slots = 0_usize;
    let mut status = ScenarioStatus::Success;
    let mut revert_reason = None;
    for request in &scenario.transactions {
        let Some(from) = request.from else {
            status = ScenarioStatus::Error("missing from".to_owned());
            break;
        };
        let to = request.to;
        let value = request.value.unwrap_or(U256::ZERO);
        let gas_limit = request.gas.unwrap_or(21_000);
        let input = request.data.clone().unwrap_or_default();

        match EvmExecutor::transact(fork, from, to, input, value, gas_limit) {
            Ok((_, diff)) => {
                if !diff.success {
                    status = ScenarioStatus::Reverted;
                    revert_reason = Some(format!("0x{}", hex::encode(diff.output.as_ref())));
                    gas_used = gas_used.saturating_add(diff.gas_used);
                    logs.extend(diff.logs);
                    break;
                }
                let classifier = DiffClassifier::new(ClassificationConfig::default());
                let _ = classifier.apply(&mut fork.db.dirty, &diff, fork.local_block_number);
                state_diff_accounts = state_diff_accounts.saturating_add(diff.accounts.len());
                state_diff_storage_slots = state_diff_storage_slots.saturating_add(
                    diff.accounts
                        .values()
                        .map(|account| account.storage_written.len())
                        .sum::<usize>(),
                );
                gas_used = gas_used.saturating_add(diff.gas_used);
                logs.extend(diff.logs);
                if let Some(max_gas) = scenario.max_gas {
                    if gas_used > max_gas {
                        status = ScenarioStatus::GasExceeded;
                        break;
                    }
                }
            }
            Err(error) => {
                status = ScenarioStatus::Error(error.to_string());
                revert_reason = Some(error.to_string());
                break;
            }
        }
        if started.elapsed() > scenario.timeout {
            status = ScenarioStatus::Timeout;
            break;
        }
    }

    if matches!(status, ScenarioStatus::Success) {
        if let Err(error) = scenario.evaluate_assertions(fork) {
            status = ScenarioStatus::Error(error.to_string());
            revert_reason = Some(error.to_string());
        }
    }

    let final_balances = tracked_balances(fork, &scenario.track_addresses);
    let pnl_wei = balance_delta(&initial_balances, &final_balances);

    ScenarioResult {
        scenario_id: scenario.id.clone(),
        name: scenario.name.clone(),
        status,
        gas_used,
        wall_time_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        peak_memory_bytes: crate::resources::ResourceModel::current_process_memory_bytes(),
        pnl_wei,
        state_diff_accounts,
        state_diff_storage_slots,
        final_balances,
        position_state: serde_json::json!({"mode": "raw-balances"}),
        logs,
        revert_reason,
    }
}

fn tracked_balances(
    fork: &mut crate::fork::ForkState,
    track_addresses: &[Address],
) -> HashMap<Address, U256> {
    track_addresses
        .iter()
        .map(|address| {
            let balance = fork
                .db
                .basic(*address)
                .ok()
                .flatten()
                .map_or(U256::ZERO, |info| info.balance);
            (*address, balance)
        })
        .collect()
}

fn balance_delta(
    initial_balances: &HashMap<Address, U256>,
    final_balances: &HashMap<Address, U256>,
) -> i128 {
    let initial = initial_balances
        .values()
        .copied()
        .fold(U256::ZERO, U256::saturating_add);
    let final_total = final_balances
        .values()
        .copied()
        .fold(U256::ZERO, U256::saturating_add);
    u256_to_i128_saturating(final_total).saturating_sub(u256_to_i128_saturating(initial))
}

fn u256_to_i128_saturating(value: U256) -> i128 {
    let bytes = value.to_be_bytes::<32>();
    if bytes[..16].iter().any(|byte| *byte != 0) {
        return i128::MAX;
    }
    let mut lower = [0_u8; 16];
    lower.copy_from_slice(&bytes[16..]);
    let value = u128::from_be_bytes(lower);
    i128::try_from(value).unwrap_or(i128::MAX)
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroUsize, sync::Arc, time::Duration};

    use alloy_primitives::{U256, address};

    use super::{
        RunMode, Scenario, ScenarioAssertions, ScenarioResult, ScenarioRunner, ScenarioSet,
        ScenarioSetStatus, ScenarioStatus, is_terminal_scenario_status, rank_scenario_results,
        scenario_status_transition_valid,
    };
    use crate::{
        fork::{ForkState, HybridDB, MirageFork, simple_transaction},
        provider::UpstreamRpc,
        resources::{MirageMode, Profile, ResourceModel},
    };

    #[tokio::test]
    async fn test_scenario_runner_basic() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let runner = ScenarioRunner::new(mirage.state());
        let sender = address!("0x1000000000000000000000000000000000000001");
        let receiver = address!("0x1000000000000000000000000000000000000002");
        let set = ScenarioSet {
            id: "set-1".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "scenario-1".to_owned(),
                name: "transfer".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(5_u64))],
                track_addresses: vec![sender, receiver],
                max_gas: Some(30_000),
                timeout: Duration::from_secs(1),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let results = runner.run_sequential(&set).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::Success));
    }

    #[tokio::test]
    async fn test_scenario_runner_revert_restores_baseline() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let sender = address!("0x2000000000000000000000000000000000000001");
        let receiver = address!("0x2000000000000000000000000000000000000002");
        let set = ScenarioSet {
            id: "set-2".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "scenario-2".to_owned(),
                name: "transfer".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(7_u64))],
                track_addresses: vec![sender, receiver],
                max_gas: Some(30_000),
                timeout: Duration::from_secs(1),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let runner = ScenarioRunner::new(mirage.state());
        let _ = runner.run_sequential(&set).await;
        let balance = {
            let state = mirage.state();
            let maybe_info = state
                .read()
                .fork
                .db
                .upstream
                .get_account_info(sender, crate::provider::BlockTag::Latest);
            let info = match maybe_info {
                Ok(Some(info)) => info,
                Ok(None) => panic!("mock account exists"),
                Err(error) => panic!("mock read succeeds: {error}"),
            };
            info.balance
        };
        assert_eq!(balance, U256::from(1_000_000_000_000_000_000_u64));

        let _ = RunMode::Parallel;
    }

    /// INV-014: each sequential scenario starts from the reverted baseline; cumulative debits would fail the second run.
    #[tokio::test]
    async fn test_scenario_sequential_second_scenario_needs_fresh_baseline() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let sender = address!("0x2500000000000000000000000000000000000001");
        let receiver = address!("0x2500000000000000000000000000000000000002");
        let large = U256::from(900_000_000_000_000_000_u64);
        let set = ScenarioSet {
            id: "set-two-big".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![
                Scenario {
                    id: "big-1".to_owned(),
                    name: "drain most balance".to_owned(),
                    transactions: vec![simple_transaction(sender, receiver, large)],
                    track_addresses: vec![sender],
                    max_gas: Some(500_000),
                    timeout: Duration::from_secs(1),
                    assertions: ScenarioAssertions::default(),
                },
                Scenario {
                    id: "big-2".to_owned(),
                    name: "same again only if baseline restored".to_owned(),
                    transactions: vec![simple_transaction(sender, receiver, large)],
                    track_addresses: vec![sender],
                    max_gas: Some(500_000),
                    timeout: Duration::from_secs(1),
                    assertions: ScenarioAssertions::default(),
                },
            ],
            status: ScenarioSetStatus::Draft,
        };

        let runner = ScenarioRunner::new(mirage.state());
        let results = runner.run_sequential(&set).await;
        assert_eq!(results.len(), 2);
        assert!(
            matches!(results[0].status, ScenarioStatus::Success),
            "first scenario should succeed: {:?}",
            results[0].status
        );
        assert!(
            matches!(results[1].status, ScenarioStatus::Success),
            "second scenario requires revert-to-baseline between runs: {:?}",
            results[1].status
        );
    }

    #[tokio::test]
    async fn scenario_runner_sequential_releases_state_write_lock_during_execution() {
        let sender = address!("0x2100000000000000000000000000000000000001");
        let receiver = address!("0x2100000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let set = ScenarioSet {
            id: "set-3".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "scenario-3".to_owned(),
                name: "transfer".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(3_u64))],
                track_addresses: vec![sender, receiver],
                max_gas: Some(30_000),
                timeout: Duration::from_secs(1),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let state = mirage.state();
        let state_for_task = Arc::clone(&state);
        let set_for_task = set.clone();
        let task = tokio::spawn(async move {
            ScenarioRunner::new(state_for_task)
                .run_sequential(&set_for_task)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        let read_guard = state.read();
        assert!(started.elapsed() < Duration::from_millis(75));
        drop(read_guard);

        let results = task
            .await
            .unwrap_or_else(|error| panic!("join scenario task: {error}"));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::Success));
    }

    #[tokio::test]
    async fn scenario_runner_releases_writer_gate_during_execution() {
        let sender = address!("0x2200000000000000000000000000000000000001");
        let receiver = address!("0x2200000000000000000000000000000000000002");
        let upstream = Arc::new(UpstreamRpc::mock(1));
        upstream.set_mock_delay(Duration::from_millis(150));
        let db = HybridDB::new(
            Arc::clone(&upstream),
            32,
            Duration::from_secs(12),
            NonZeroUsize::MIN,
            1,
        );
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let set = ScenarioSet {
            id: "set-4".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "scenario-4".to_owned(),
                name: "transfer".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(4_u64))],
                track_addresses: vec![sender, receiver],
                max_gas: Some(30_000),
                timeout: Duration::from_secs(1),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let state = mirage.state();
        let state_for_task = Arc::clone(&state);
        let set_for_task = set.clone();
        let task = tokio::spawn(async move {
            ScenarioRunner::new(state_for_task)
                .run_sequential(&set_for_task)
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        let started = std::time::Instant::now();
        crate::fork::with_state_write(&state, |state| {
            state.reject_new_forks = !state.reject_new_forks;
        })
        .await;
        assert!(started.elapsed() < Duration::from_millis(75));

        let results = task
            .await
            .unwrap_or_else(|error| panic!("join scenario task: {error}"));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::Success));
    }

    #[test]
    fn scenario_results_rank_by_pnl_then_cost() {
        let low_value = ScenarioResult {
            scenario_id: "low".to_owned(),
            name: "low".to_owned(),
            status: ScenarioStatus::Success,
            gas_used: 200,
            wall_time_ms: 10,
            peak_memory_bytes: 0,
            pnl_wei: 20,
            state_diff_accounts: 1,
            state_diff_storage_slots: 1,
            final_balances: Default::default(),
            position_state: serde_json::json!({}),
            logs: Vec::new(),
            revert_reason: None,
        };
        let high_value = ScenarioResult {
            scenario_id: "high".to_owned(),
            name: "high".to_owned(),
            status: ScenarioStatus::Success,
            gas_used: 400,
            wall_time_ms: 10,
            peak_memory_bytes: 0,
            pnl_wei: 30,
            state_diff_accounts: 3,
            state_diff_storage_slots: 5,
            final_balances: Default::default(),
            position_state: serde_json::json!({}),
            logs: Vec::new(),
            revert_reason: None,
        };

        let ranked = rank_scenario_results(vec![low_value.clone(), high_value.clone()]);
        assert_eq!(ranked[0].scenario_id, high_value.scenario_id);
    }

    #[test]
    fn scenario_fixture_supports_assertions() {
        let fixture = r#"
[scenario]
name = "uniswap_v3_lp_entry"
description = "Add concentrated liquidity"

[[transactions]]
from = "0x3000000000000000000000000000000000000001"
to = "0x3000000000000000000000000000000000000010"
value = "0x0"
gas = 500000
data = "0xa9059cbb00000000000000000000000030000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000005"

[assertions]
watch_list_contains = ["0x3000000000000000000000000000000000000010"]

[assertions.token_balance_gte]
token = "0x3000000000000000000000000000000000000010"
address = "0x3000000000000000000000000000000000000020"
amount = "0x5"

[track]
addresses = ["0x3000000000000000000000000000000000000010"]
"#;

        let scenario = Scenario::from_toml("fixture-1", fixture)
            .unwrap_or_else(|error| panic!("fixture parses: {error}"));
        assert_eq!(scenario.transactions.len(), 1);
        assert_eq!(scenario.assertions.watch_list_contains.len(), 1);
        assert!(scenario.assertions.token_balance_gte.is_some());
    }

    #[test]
    fn scenario_fixture_supports_plan_style_quantities() {
        let fixture = r#"
[scenario]
name = "router_stress"
description = "Hex gas and explicit timeout"
maxGas = "0x5208"
timeout = "1500ms"
track_addresses = ["0x3100000000000000000000000000000000000003"]

[[transactions]]
from = "0x3100000000000000000000000000000000000001"
to = "0x3100000000000000000000000000000000000002"
value = "0x1"
gas = "0x5208"
input = "0x"

[track]
addresses = ["0x3100000000000000000000000000000000000002"]
"#;

        let scenario = Scenario::from_toml("fixture-hex", fixture)
            .unwrap_or_else(|error| panic!("fixture parses: {error}"));
        assert_eq!(scenario.max_gas, Some(21_000));
        assert_eq!(scenario.timeout, Duration::from_millis(1_500));
        assert_eq!(scenario.transactions.len(), 1);
        assert_eq!(scenario.transactions[0].gas, Some(21_000));
        assert_eq!(scenario.track_addresses.len(), 2);
        assert!(
            scenario
                .track_addresses
                .contains(&address!("0x3100000000000000000000000000000000000002"))
        );
        assert!(
            scenario
                .track_addresses
                .contains(&address!("0x3100000000000000000000000000000000000003"))
        );
    }

    #[test]
    fn built_in_fixtures_parse() {
        let fixtures = [
            (
                "eth-crash",
                include_str!("../tests/scenarios/eth_crash.toml"),
            ),
            (
                "volume-spike",
                include_str!("../tests/scenarios/volume_spike.toml"),
            ),
            (
                "uniswap-v3-entry",
                include_str!("../tests/scenarios/uniswap_v3_entry.toml"),
            ),
            (
                "aave-liquidation",
                include_str!("../tests/scenarios/aave_liquidation.toml"),
            ),
            ("new-pool", include_str!("../tests/scenarios/new_pool.toml")),
        ];

        for (id, fixture) in fixtures {
            let scenario = Scenario::from_toml(id, fixture)
                .unwrap_or_else(|error| panic!("fixture {id} parses: {error}"));
            assert!(!scenario.transactions.is_empty());
            assert!(!scenario.track_addresses.is_empty());
        }
    }

    #[test]
    fn built_in_fixtures_match_expected_scale() {
        let eth_crash = Scenario::from_toml(
            "eth-crash",
            include_str!("../tests/scenarios/eth_crash.toml"),
        )
        .unwrap_or_else(|error| panic!("eth crash parses: {error}"));
        assert!((20..=40).contains(&eth_crash.transactions.len()));

        let volume_spike = Scenario::from_toml(
            "volume-spike",
            include_str!("../tests/scenarios/volume_spike.toml"),
        )
        .unwrap_or_else(|error| panic!("volume spike parses: {error}"));
        assert_eq!(volume_spike.transactions.len(), 100);

        let aave = Scenario::from_toml(
            "aave-liquidation",
            include_str!("../tests/scenarios/aave_liquidation.toml"),
        )
        .unwrap_or_else(|error| panic!("aave liquidation parses: {error}"));
        assert!(aave.transactions.len() >= 3);

        let new_pool =
            Scenario::from_toml("new-pool", include_str!("../tests/scenarios/new_pool.toml"))
                .unwrap_or_else(|error| panic!("new pool parses: {error}"));
        assert!(new_pool.transactions.iter().any(|tx| tx.to.is_none()));
    }

    #[test]
    fn test_scenario_status_valid_transitions() {
        // ScenarioSetStatus: Draft → Running → Complete | Failed
        let draft = ScenarioSetStatus::Draft;
        let running = ScenarioSetStatus::Running;
        let complete = ScenarioSetStatus::Complete;
        let failed = ScenarioSetStatus::Failed;

        assert_ne!(draft, running);
        assert_ne!(running, complete);
        assert_ne!(running, failed);
        assert_ne!(complete, failed);

        // INV-013: Pending → terminal only; no terminal → terminal
        let pending = ScenarioStatus::Pending;
        let success = ScenarioStatus::Success;
        let timeout = ScenarioStatus::Timeout;
        let gas_exceeded = ScenarioStatus::GasExceeded;
        let reverted = ScenarioStatus::Reverted;
        let err = ScenarioStatus::Error("x".into());

        assert!(!is_terminal_scenario_status(&pending));
        assert!(is_terminal_scenario_status(&success));
        assert!(scenario_status_transition_valid(None, &pending));
        assert!(scenario_status_transition_valid(Some(&pending), &success));
        assert!(scenario_status_transition_valid(Some(&pending), &reverted));
        assert!(scenario_status_transition_valid(Some(&pending), &timeout));
        assert!(scenario_status_transition_valid(
            Some(&pending),
            &gas_exceeded
        ));
        assert!(scenario_status_transition_valid(Some(&pending), &err));
        assert!(
            !scenario_status_transition_valid(Some(&pending), &pending),
            "Pending → Pending is not a valid progress transition"
        );
        assert!(
            !scenario_status_transition_valid(Some(&success), &timeout),
            "terminal → terminal forbidden"
        );
        assert!(!scenario_status_transition_valid(Some(&reverted), &success));

        assert!(matches!(success, ScenarioStatus::Success));
        assert!(matches!(timeout, ScenarioStatus::Timeout));
        assert!(matches!(gas_exceeded, ScenarioStatus::GasExceeded));
        assert!(matches!(reverted, ScenarioStatus::Reverted));
        assert!(matches!(err, ScenarioStatus::Error(_)));
    }

    #[tokio::test]
    async fn test_scenario_timeout_enforced() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let sender = address!("0x5000000000000000000000000000000000000001");
        let receiver = address!("0x5000000000000000000000000000000000000002");
        let set = ScenarioSet {
            id: "timeout-set".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "timeout-scenario".to_owned(),
                name: "should timeout".to_owned(),
                transactions: vec![
                    simple_transaction(sender, receiver, U256::from(1_u64)),
                    simple_transaction(sender, receiver, U256::from(1_u64)),
                    simple_transaction(sender, receiver, U256::from(1_u64)),
                ],
                track_addresses: vec![sender],
                max_gas: None,
                timeout: Duration::ZERO,
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let runner = ScenarioRunner::new(mirage.state());
        let results = runner.run_sequential(&set).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::Timeout));
    }

    #[tokio::test]
    async fn test_scenario_gas_exceeded() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let sender = address!("0x6000000000000000000000000000000000000001");
        let receiver = address!("0x6000000000000000000000000000000000000002");
        let set = ScenarioSet {
            id: "gas-set".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "gas-scenario".to_owned(),
                name: "should exceed gas".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(1_u64))],
                track_addresses: vec![sender],
                max_gas: Some(1),
                timeout: Duration::from_secs(10),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let runner = ScenarioRunner::new(mirage.state());
        let results = runner.run_sequential(&set).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::GasExceeded));
    }

    #[tokio::test]
    async fn test_scenario_tracks_all_addresses() {
        let upstream = Arc::new(UpstreamRpc::mock(1));
        let db = HybridDB::new(upstream, 32, Duration::from_secs(12), NonZeroUsize::MIN, 1);
        let fork = ForkState::new(db, 0, 1);
        let mirage = MirageFork::new(
            fork,
            ResourceModel::for_profile(Profile::Standard, Duration::from_secs(12)),
            MirageMode::Live,
        );
        let sender = address!("0x7000000000000000000000000000000000000001");
        let receiver = address!("0x7000000000000000000000000000000000000002");
        let bystander = address!("0x7000000000000000000000000000000000000003");
        let set = ScenarioSet {
            id: "track-set".to_owned(),
            baseline_snapshot_id: 0,
            baseline_fork: None,
            scenarios: vec![Scenario {
                id: "track-scenario".to_owned(),
                name: "track all".to_owned(),
                transactions: vec![simple_transaction(sender, receiver, U256::from(1_u64))],
                track_addresses: vec![sender, receiver, bystander],
                max_gas: None,
                timeout: Duration::from_secs(10),
                assertions: ScenarioAssertions::default(),
            }],
            status: ScenarioSetStatus::Draft,
        };

        let runner = ScenarioRunner::new(mirage.state());
        let results = runner.run_sequential(&set).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, ScenarioStatus::Success));
        assert_eq!(results[0].final_balances.len(), 3);
        assert!(results[0].final_balances.contains_key(&sender));
        assert!(results[0].final_balances.contains_key(&receiver));
        assert!(results[0].final_balances.contains_key(&bystander));
    }
}

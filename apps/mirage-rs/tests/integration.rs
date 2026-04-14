//! Process-spawning integration tests for `mirage-rs`.

#![allow(clippy::default_trait_access, clippy::expect_used)]

use std::{path::PathBuf, sync::LazyLock, time::Duration};

use alloy_primitives::{Address, Bytes, U256, address};
use futures_util::StreamExt;
use mirage_rs::{
    EventFilter, EventSource, JobStatus, MirageClient, MirageConfig, MirageStatus,
    MirageTestInstance, MultiVersionStore, PositionRequest, RunMode, Scenario, ScenarioAssertions,
    StateDiff, TransactionRequest, VersionEntry,
    rpc::{from_sqrt_price_x96, to_sqrt_price_x96},
    spawn_mirage_test_instance,
};
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

/// Serializes subprocess spawns so parallel `cargo test` does not starve ports or the host.
static MIRAGE_SPAWN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn spawn_mirage_serial(
    rpc_url: Option<&str>,
    port: Option<u16>,
) -> mirage_rs::Result<MirageTestInstance> {
    let _guard = MIRAGE_SPAWN_LOCK.lock().await;
    spawn_mirage_test_instance(rpc_url, port).await
}

fn reserve_free_local_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("resolve ephemeral port")
        .port()
}

#[tokio::test]
async fn integration_spawn_and_ready() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let mut cfg = MirageConfig::default_local();
    cfg.url = format!("http://127.0.0.1:{port}");
    let client = MirageClient::new(cfg).await.expect("construct client");
    client
        .wait_ready(Duration::from_secs(10))
        .await
        .expect("instance ready within 10s");

    instance.shutdown().await.expect("shutdown instance");
}

/// INV-035 (process integration): same ordering guarantees as the in-process RPC harness, against
/// the spawned `mirage-rs` binary. Uses a dedicated port so this can run while other tests use
/// `18545` (`integration_spawn_and_ready` holds the default-local URL contract there).
#[tokio::test]
async fn test_local_tx_event_sequence() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");

    let mut cfg = MirageConfig::default_local();
    cfg.url = format!("http://127.0.0.1:{port}");
    let client = MirageClient::new(cfg).await.expect("construct client");
    client
        .wait_ready(Duration::from_secs(10))
        .await
        .expect("instance ready");

    let sender = address!("0x3600000000000000000000000000000000000001");
    let contract = address!("0x3600000000000000000000000000000000000002");
    let mut events = client
        .subscribe_events(EventFilter {
            addresses: Some(vec![contract]),
            topics: None,
        })
        .await
        .expect("subscribe events");

    let tx_hash = client
        .eth_send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(contract),
            gas: Some(150_000),
            value: Some(U256::ZERO),
            data: Some(Bytes::from_static(&[0x41, 0x4b, 0xf3, 0x89, 0x00, 0x01])),
            ..Default::default()
        })
        .await
        .expect("send protocol touch tx");

    let event = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("event timeout")
        .expect("event stream closed");

    let dirty_slots: serde_json::Value = rpc_call(
        &instance.config().url,
        "mirage_getDirtySlots",
        serde_json::json!([contract]),
    )
    .await
    .expect("load dirty slots after tx");
    let watch_list: Vec<serde_json::Value> = rpc_call(
        &instance.config().url,
        "mirage_getWatchList",
        serde_json::json!([]),
    )
    .await
    .expect("load watch list after tx");
    let receipt: serde_json::Value = rpc_call(
        &instance.config().url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await
    .expect("load receipt after tx");

    assert_eq!(event.tx_hash, tx_hash);
    assert_eq!(event.contract, contract);
    assert_eq!(event.log_index, 0);
    assert!(matches!(event.source, EventSource::LocalTx));
    assert!(
        dirty_slots
            .as_object()
            .is_some_and(|slots| slots.len() >= 3),
        "protocol touch should dirty the selector, caller, and calldata slots"
    );
    assert!(
        watch_list
            .iter()
            .any(|entry| entry.get("address") == Some(&serde_json::json!(contract))),
        "classifier should watch the touched protocol contract before the receipt is observed"
    );
    assert_eq!(
        receipt.get("transactionHash"),
        Some(&serde_json::json!(tx_hash))
    );
    assert_eq!(receipt.get("to"), Some(&serde_json::json!(Some(contract))));
    assert_eq!(
        receipt
            .get("logs")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or_default(),
        1
    );

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_unknown_method_returns_32601_and_status_matches_server() {
    // Use an ephemeral port so this test doesn't collide with
    // `rpc::mirage_instance_or_env()`'s default fallback (`18552`) when
    // workspace tests run in parallel.
    let port = reserve_free_local_port();
    let status_path = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));
    let _ = tokio::fs::remove_file(&status_path).await;

    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    assert_jsonrpc_unknown_method_returns_32601(&instance.config().url).await;
    assert_mirage_status_matches_health(&client, &instance.config().url).await;

    let startup_status = serde_json::from_str::<serde_json::Value>(
        &tokio::fs::read_to_string(&status_path)
            .await
            .expect("status artifact should exist"),
    )
    .expect("status artifact should be valid json");
    assert_eq!(startup_status["status"], "ready");
    assert_eq!(startup_status["ready"], true);
    assert_eq!(startup_status["port"], serde_json::json!(port));

    instance.shutdown().await.expect("shutdown instance");
}

/// Plan V2: HTTP JSON-RPC `POST` for a nonexistent method returns `-32601` / "Method not found".
#[tokio::test]
async fn test_jsonrpc_unknown_method_returns_32601() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    assert_jsonrpc_unknown_method_returns_32601(&instance.config().url).await;

    instance.shutdown().await.expect("shutdown instance");
}

/// Plan V2: `MirageClient::mirage_status` matches `GET /health` for the fields clients rely on.
#[tokio::test]
async fn test_mirage_client_status_matches_server() {
    let port = reserve_free_local_port();
    let status_path = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));
    let _ = tokio::fs::remove_file(&status_path).await;

    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    assert_mirage_status_matches_health(&client, &instance.config().url).await;

    let startup_status = serde_json::from_str::<serde_json::Value>(
        &tokio::fs::read_to_string(&status_path)
            .await
            .expect("status artifact should exist"),
    )
    .expect("status artifact should be valid json");
    assert_eq!(startup_status["status"], "ready");
    assert_eq!(startup_status["ready"], true);
    assert_eq!(startup_status["port"], serde_json::json!(port));

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn test_pool_slot0_matches_expected_price() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let pool = address!("0x50000000000000000000000000000000000000f0");
    let price = 1_800.0;
    let expected_sqrt_price_x96 = to_sqrt_price_x96(price);

    let set_storage = rpc_call::<bool>(
        &instance.config().url,
        "mirage_setStorageAt",
        serde_json::json!([pool, "0x0", format!("0x{:064x}", expected_sqrt_price_x96)]),
    )
    .await
    .expect("set pool slot0");
    assert!(set_storage);

    let stored = rpc_call::<String>(
        &instance.config().url,
        "eth_getStorageAt",
        serde_json::json!([pool, "0x0", "latest"]),
    )
    .await
    .expect("read pool slot0");
    let stored = parse_u256(&stored);
    assert_eq!(stored, expected_sqrt_price_x96);

    let decoded_price = from_sqrt_price_x96(stored);
    let relative_error = (decoded_price - price).abs() / price;
    assert!(
        relative_error < 1e-9,
        "decoded_price={decoded_price} price={price} relative_error={relative_error}"
    );

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_eth_get_block_by_number_falls_back_to_upstream_view() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let block = rpc_call::<serde_json::Value>(
        &instance.config().url,
        "eth_getBlockByNumber",
        serde_json::json!(["latest", true]),
    )
    .await
    .expect("fetch latest block");
    assert_eq!(block["number"], "0x0");
    assert!(block["hash"].is_string());
    assert!(block["transactions"].is_array());

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_mirage_mint_erc20_returns_success_and_marks_dirty_slots() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let token = address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    let owner = address!("0x3300000000000000000000000000000000000002");

    let set_code = rpc_call::<bool>(
        &instance.config().url,
        "mirage_setCode",
        serde_json::json!([token, "0x6001600055"]),
    )
    .await
    .expect("set token code");
    assert!(set_code);

    let minted = rpc_call::<bool>(
        &instance.config().url,
        "mirage_mintERC20",
        serde_json::json!([token, owner, "0x10"]),
    )
    .await
    .expect("mint token");
    assert!(minted);

    let dirty_slots = rpc_call::<serde_json::Map<String, serde_json::Value>>(
        &instance.config().url,
        "mirage_getDirtySlots",
        serde_json::json!([token]),
    )
    .await
    .expect("read dirty slots");
    assert!(!dirty_slots.is_empty());

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_mirage_mint_erc20_supports_arbitrary_token_balance_overrides() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let token = address!("0x4400000000000000000000000000000000000004");
    let owner = address!("0x4400000000000000000000000000000000000005");

    let set_code = rpc_call::<bool>(
        &instance.config().url,
        "mirage_setCode",
        serde_json::json!([token, "0x6001600055"]),
    )
    .await
    .expect("set token code");
    assert!(set_code);

    let minted = rpc_call::<bool>(
        &instance.config().url,
        "mirage_mintERC20",
        serde_json::json!([token, owner, "0x2a"]),
    )
    .await
    .expect("mint token");
    assert!(minted);

    let balance = rpc_call::<String>(
        &instance.config().url,
        "eth_call",
        serde_json::json!([{
            "from": owner,
            "to": token,
            "data": format!("0x70a08231000000000000000000000000{}", owner.to_string().trim_start_matches("0x").to_lowercase()),
        }, "latest"]),
    )
    .await
    .expect("read synthetic token balance");
    assert_eq!(parse_u256(&balance), U256::from(42_u64));

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_eth_transfer_state_diff() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let sender = address!("0x1000000000000000000000000000000000000001");
    let receiver = address!("0x1000000000000000000000000000000000000002");
    let before_sender = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([sender, "latest"]),
    )
    .await
    .expect("read sender balance before");
    let before_receiver = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([receiver, "latest"]),
    )
    .await
    .expect("read receiver balance before");

    let tx_hash = client
        .eth_send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(receiver),
            gas: Some(21_000),
            value: Some(U256::from(25_u64)),
            data: Some(Default::default()),
            gas_price: None,
            nonce: None,
            chain_id: Some(1),
        })
        .await
        .expect("submit transfer");

    let receipt = rpc_call::<serde_json::Value>(
        &instance.config().url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await
    .expect("get receipt");
    assert_eq!(receipt["status"], "0x1");

    let after_sender = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([sender, "latest"]),
    )
    .await
    .expect("read sender balance after");
    let after_receiver = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([receiver, "latest"]),
    )
    .await
    .expect("read receiver balance after");

    assert!(parse_u256(&after_sender) < parse_u256(&before_sender));
    assert!(parse_u256(&after_receiver) > parse_u256(&before_receiver));

    let diff: StateDiff = rpc_call(
        &instance.config().url,
        "mirage_getLastStateDiff",
        serde_json::json!([]),
    )
    .await
    .expect("mirage_getLastStateDiff");
    let sender_entry = diff
        .accounts
        .get(&sender)
        .expect("StateDiff should record sender balance change");
    let receiver_entry = diff
        .accounts
        .get(&receiver)
        .expect("StateDiff should record receiver balance change");
    assert!(
        sender_entry.new_balance.is_some(),
        "sender AccountDiff should include new_balance"
    );
    assert!(
        receiver_entry.new_balance.is_some(),
        "receiver AccountDiff should include new_balance"
    );
    assert_eq!(sender_entry.new_balance.unwrap(), parse_u256(&after_sender));
    assert_eq!(
        receiver_entry.new_balance.unwrap(),
        parse_u256(&after_receiver)
    );

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_snapshot_revert() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let sender = address!("0x2000000000000000000000000000000000000001");
    let receiver = address!("0x2000000000000000000000000000000000000002");
    let snapshot = client.evm_snapshot().await.expect("take snapshot");
    client
        .eth_send_transaction(TransactionRequest {
            from: Some(sender),
            to: Some(receiver),
            gas: Some(21_000),
            value: Some(U256::from(10_u64)),
            data: Some(Default::default()),
            gas_price: None,
            nonce: None,
            chain_id: Some(1),
        })
        .await
        .expect("submit transfer");
    let changed = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([receiver, "latest"]),
    )
    .await
    .expect("read changed balance");
    assert!(parse_u256(&changed) > U256::from(1_000_000_000_000_000_000_u64));

    assert!(client.evm_revert(snapshot).await.expect("revert snapshot"));
    let reverted = rpc_call::<String>(
        &instance.config().url,
        "eth_getBalance",
        serde_json::json!([receiver, "latest"]),
    )
    .await
    .expect("read reverted balance");
    assert_eq!(
        parse_u256(&reverted),
        U256::from(1_000_000_000_000_000_000_u64)
    );

    instance.shutdown().await.expect("shutdown instance");
}

#[tokio::test]
async fn integration_scenario_runner_cow_isolation() {
    let port = reserve_free_local_port();
    let mut instance = spawn_mirage_serial(None, Some(port))
        .await
        .expect("spawn test instance");
    let client = MirageClient::new(instance.config())
        .await
        .expect("construct client");
    client
        .wait_ready(Duration::from_secs(5))
        .await
        .expect("instance ready");

    let sender = address!("0x3000000000000000000000000000000000000001");
    let left = address!("0x3000000000000000000000000000000000000002");
    let right = address!("0x3000000000000000000000000000000000000003");
    let set_id = client
        .mirage_begin_scenario_set("latest")
        .await
        .expect("begin set");

    let left_scenario = Scenario {
        id: "left-branch".to_owned(),
        name: "left transfer".to_owned(),
        transactions: vec![tx(sender, left, 4)],
        track_addresses: vec![sender, left],
        max_gas: Some(30_000),
        timeout: Duration::from_secs(1),
        assertions: ScenarioAssertions::default(),
    };
    let right_scenario = Scenario {
        id: "right-branch".to_owned(),
        name: "right transfer".to_owned(),
        transactions: vec![tx(sender, right, 9)],
        track_addresses: vec![sender, right],
        max_gas: Some(30_000),
        timeout: Duration::from_secs(1),
        assertions: ScenarioAssertions::default(),
    };
    client
        .mirage_define_scenario(&set_id, &left_scenario)
        .await
        .expect("define left");
    client
        .mirage_define_scenario(&set_id, &right_scenario)
        .await
        .expect("define right");

    let job_id = client
        .mirage_run_scenario_set(&set_id, RunMode::Parallel)
        .await
        .expect("run scenario set");
    let job = wait_for_job(&client, &job_id).await;
    assert!(matches!(job.status, JobStatus::Complete));
    assert_eq!(job.results.as_ref().expect("results present").len(), 2);

    let owner_view = client
        .mirage_get_position(PositionRequest {
            owner: sender,
            protocol_type: "raw-balances".to_owned(),
            contract: None,
            token_addresses: vec![sender, left, right],
        })
        .await
        .expect("read position snapshot");
    assert_eq!(owner_view.protocol_type, "raw-balances");

    instance.shutdown().await.expect("shutdown instance");
}

#[test]
fn integration_block_stm_conflict_rate_uses_transaction_reexecutions() {
    let store = MultiVersionStore::default();
    let address = address!("0x4100000000000000000000000000000000000004");
    let slot_a = U256::from(1_u64);
    let slot_b = U256::from(2_u64);
    let total_txs = 50;

    store.record(
        address,
        slot_a,
        VersionEntry {
            tx_index: 0,
            value: U256::from(10_u64),
            incarnation: 0,
        },
    );
    store.record(
        address,
        slot_a,
        VersionEntry {
            tx_index: 1,
            value: U256::from(20_u64),
            incarnation: 0,
        },
    );
    store.record(
        address,
        slot_b,
        VersionEntry {
            tx_index: 1,
            value: U256::from(30_u64),
            incarnation: 0,
        },
    );
    store.record(
        address,
        slot_a,
        VersionEntry {
            tx_index: 1,
            value: U256::from(21_u64),
            incarnation: 1,
        },
    );
    store.record(
        address,
        slot_b,
        VersionEntry {
            tx_index: 1,
            value: U256::from(31_u64),
            incarnation: 1,
        },
    );

    assert_eq!(store.re_execution_count(), 1);
    let conflict_rate = store.conflict_rate(total_txs);
    assert!((conflict_rate - (1.0 / total_txs as f64)).abs() < f64::EPSILON);
    assert!(conflict_rate < 0.05);
}

fn tx(from: Address, to: Address, value: u64) -> TransactionRequest {
    TransactionRequest {
        from: Some(from),
        to: Some(to),
        gas: Some(21_000),
        value: Some(U256::from(value)),
        data: Some(Default::default()),
        gas_price: None,
        nonce: None,
        chain_id: Some(1),
    }
}

async fn wait_for_job(client: &MirageClient, job_id: &str) -> mirage_rs::ScenarioJob {
    for _ in 0..20 {
        let job = client
            .mirage_get_scenario_results(job_id)
            .await
            .expect("poll scenario job");
        if matches!(job.status, JobStatus::Complete | JobStatus::Failed) {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("scenario job did not complete in time");
}

async fn rpc_call<T: DeserializeOwned>(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<T> {
    let value = rpc_response(url, method, params).await?;
    if let Some(error) = value.get("error") {
        anyhow::bail!("rpc error for {method}: {error}");
    }
    let result = value
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing result for {method}"))?;
    Ok(serde_json::from_value(result)?)
}

async fn rpc_response(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let response = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .send()
        .await?;
    Ok(response.json::<serde_json::Value>().await?)
}

async fn assert_jsonrpc_unknown_method_returns_32601(base_url: &str) {
    let v = rpc_response(base_url, "eth_fooNonExistent", serde_json::json!([]))
        .await
        .expect("unknown method response");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["error"]["code"], -32601);
    assert_eq!(v["error"]["message"], "Method not found");
}

async fn assert_mirage_status_matches_health(client: &MirageClient, base_url: &str) {
    let rpc_status = client.mirage_status().await.expect("mirage status rpc");
    let health_status = reqwest::Client::new()
        .get(format!("{base_url}/health"))
        .send()
        .await
        .expect("health request")
        .json::<MirageStatus>()
        .await
        .expect("decode health payload");
    assert_eq!(rpc_status.status, "ready");
    assert_eq!(rpc_status.status, health_status.status);
    assert_eq!(rpc_status.chain_id, health_status.chain_id);
    assert_eq!(rpc_status.block_number, health_status.block_number);
    assert_eq!(rpc_status.watch_list_size, health_status.watch_list_size);
    assert_eq!(
        rpc_status.dirty_account_count,
        health_status.dirty_account_count
    );
    assert_eq!(rpc_status.dirty_slot_count, health_status.dirty_slot_count);
    assert_eq!(
        rpc_status.upstream_connected,
        health_status.upstream_connected
    );
    assert_eq!(
        rpc_status.divergence_detected,
        health_status.divergence_detected
    );
    assert_eq!(rpc_status.mode, health_status.mode);
}

fn parse_u256(text: &str) -> U256 {
    U256::from_str_radix(text.trim_start_matches("0x"), 16).expect("valid U256 quantity")
}

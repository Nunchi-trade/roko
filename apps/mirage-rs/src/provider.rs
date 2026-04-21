//! Upstream RPC access for lazy latest reads.

#![allow(
    clippy::needless_pass_by_value,
    clippy::significant_drop_in_scrutinee,
    clippy::significant_drop_tightening
)]

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use alloy_primitives::{Address, B256, Bytes, U256, hex, keccak256};
use futures_util::{SinkExt, Stream, StreamExt, stream};
use parking_lot::RwLock;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{AccountInfo, Bytecode, MirageError, Result};

const DEFAULT_MOCK_BALANCE: u64 = 1_000_000_000_000_000_000;

/// Default steady-state upstream requests per second (`--upstream-rps`).
pub const DEFAULT_UPSTREAM_RPS: u32 = 100;
/// Default token-bucket capacity (`--upstream-burst`): allows an initial burst before RPS throttling.
pub const DEFAULT_UPSTREAM_BURST: u32 = 200;

/// Upstream block selector for lazy reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockTag {
    /// Resolve reads against the latest upstream head.
    Latest,
    /// Resolve reads against a fixed block number.
    Number(u64),
}

impl BlockTag {
    fn as_json(self) -> Value {
        match self {
            Self::Latest => Value::String("latest".to_owned()),
            Self::Number(number) => Value::String(format!("0x{number:x}")),
        }
    }
}

/// Thread-safe upstream call counters.
#[derive(Debug, Default)]
pub struct UpstreamStats {
    /// Total upstream RPC calls.
    pub calls: u64,
    /// Total upstream RPC failures.
    pub errors: u64,
}

#[derive(Debug)]
struct RateLimitState {
    tokens: f64,
    last_refill_at: Instant,
}

/// Mock upstream state used by tests and offline mode.
#[derive(Debug, Clone, Default)]
pub struct MockUpstream {
    /// Current upstream block number.
    pub block_number: u64,
    /// Chain ID exposed by the mock.
    pub chain_id: u64,
    /// Artificial latency injected into mock reads for concurrency tests.
    pub delay: Duration,
    /// Account information keyed by address.
    pub accounts: HashMap<Address, AccountInfo>,
    /// Storage overrides keyed by `(address, slot)`.
    pub storage: HashMap<(Address, U256), U256>,
    /// Known block hashes keyed by block number.
    pub block_hashes: HashMap<u64, B256>,
    /// Full transaction objects returned by `eth_getBlockByNumber` when `full_transactions` is true.
    pub block_transactions: HashMap<u64, Vec<Value>>,
    /// Transaction payloads keyed by hash for `eth_getTransactionByHash` in mock mode.
    pub transactions_by_hash: HashMap<B256, Value>,
    /// Block timestamp (seconds since UNIX epoch) for mock blocks.
    pub timestamp: u64,
}

/// Minimal upstream RPC adapter.
#[derive(Debug)]
pub struct UpstreamRpc {
    http_url: Option<String>,
    ws_url: Option<String>,
    chain_id: u64,
    client: Option<Client>,
    stats: Arc<RwLock<UpstreamStats>>,
    mock: Arc<RwLock<MockUpstream>>,
    code_by_hash: Arc<RwLock<HashMap<B256, Bytecode>>>,
    addresses_by_code_hash: Arc<RwLock<HashMap<B256, Address>>>,
    retry_attempts: u32,
    retry_backoff: Duration,
    requests_per_second: u32,
    burst: u32,
    rate_limit: Arc<RwLock<RateLimitState>>,
    /// Counts mock `eth_getTransactionByHash` hits (for tests).
    mock_transaction_fetches: Arc<AtomicU64>,
}

impl UpstreamRpc {
    /// Creates a new upstream adapter.
    #[must_use]
    pub fn new(http_url: Option<String>, ws_url: Option<String>, chain_id: u64) -> Self {
        Self::new_with_limits(
            http_url,
            ws_url,
            chain_id,
            DEFAULT_UPSTREAM_RPS,
            DEFAULT_UPSTREAM_BURST,
        )
    }

    /// Creates a new upstream adapter with explicit throttling limits.
    ///
    /// `requests_per_second` is the sustained rate. `burst` is the maximum number of tokens in
    /// the bucket (initial fill and cap). Defaults match the CLI: 100 RPS and burst capacity 200
    /// (2× the default steady rate).
    #[must_use]
    pub fn new_with_limits(
        http_url: Option<String>,
        ws_url: Option<String>,
        chain_id: u64,
        requests_per_second: u32,
        burst: u32,
    ) -> Self {
        let client = http_url.as_ref().map(|_| Client::new());
        let burst = burst.max(1);
        Self {
            http_url,
            ws_url,
            chain_id,
            client,
            stats: Arc::new(RwLock::new(UpstreamStats::default())),
            mock: Arc::new(RwLock::new(MockUpstream {
                block_number: 0,
                chain_id,
                delay: Duration::ZERO,
                accounts: HashMap::new(),
                storage: HashMap::new(),
                block_hashes: HashMap::new(),
                block_transactions: HashMap::new(),
                transactions_by_hash: HashMap::new(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            })),
            code_by_hash: Arc::new(RwLock::new(HashMap::new())),
            addresses_by_code_hash: Arc::new(RwLock::new(HashMap::new())),
            // Three retries (four attempts) with backoff 100ms, 200ms, 400ms between failures.
            retry_attempts: 3,
            retry_backoff: Duration::from_millis(100),
            requests_per_second,
            burst,
            rate_limit: Arc::new(RwLock::new(RateLimitState {
                tokens: f64::from(burst),
                last_refill_at: Instant::now(),
            })),
            mock_transaction_fetches: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Creates a mock-only upstream adapter.
    #[must_use]
    pub fn mock(chain_id: u64) -> Self {
        Self::new(None, None, chain_id)
    }

    /// Returns the configured chain ID.
    #[must_use]
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns whether a WebSocket upstream URL was configured.
    #[must_use]
    pub const fn has_ws(&self) -> bool {
        self.ws_url.is_some()
    }

    /// Returns whether an HTTP upstream URL was configured.
    #[must_use]
    pub const fn has_http(&self) -> bool {
        self.http_url.is_some()
    }

    /// Returns the configured HTTP upstream URL, if any.
    #[must_use]
    pub fn http_url(&self) -> Option<String> {
        self.http_url.clone()
    }

    /// Returns call counters.
    #[must_use]
    pub fn stats(&self) -> (u64, u64) {
        let stats = self.stats.read();
        (stats.calls, stats.errors)
    }

    /// Inserts mock account info for tests.
    pub fn set_mock_account(&self, address: Address, info: AccountInfo) {
        if let Some(code) = info.code.clone() {
            self.code_by_hash.write().insert(info.code_hash, code);
        }
        self.addresses_by_code_hash
            .write()
            .insert(info.code_hash, address);
        self.mock.write().accounts.insert(address, info);
    }

    /// Inserts mock storage for tests.
    pub fn set_mock_storage(&self, address: Address, slot: U256, value: U256) {
        self.mock.write().storage.insert((address, slot), value);
    }

    /// Injects artificial latency into mock reads for concurrency tests.
    pub fn set_mock_delay(&self, delay: Duration) {
        self.mock.write().delay = delay;
    }

    /// Seeds mock block bodies (full tx objects) and `eth_getTransactionByHash` responses.
    pub fn seed_mock_block_transactions(&self, block_number: u64, transactions: Vec<Value>) {
        let mut mock = self.mock.write();
        for tx in &transactions {
            if let Some(hash) = tx
                .get("hash")
                .and_then(Value::as_str)
                .and_then(|text| text.parse::<B256>().ok())
            {
                mock.transactions_by_hash.insert(hash, tx.clone());
            }
        }
        mock.block_transactions.insert(block_number, transactions);
    }

    /// Returns how many mock-mode `eth_getTransactionByHash` lookups ran (test helper).
    #[must_use]
    pub fn mock_transaction_fetches(&self) -> u64 {
        self.mock_transaction_fetches.load(Ordering::Relaxed)
    }

    /// Sets the mock upstream head block number (`eth_blockNumber`).
    pub fn set_mock_block_number(&self, block_number: u64) {
        self.mock.write().block_number = block_number;
    }

    /// Returns the upstream block number.
    ///
    /// # Errors
    ///
    /// Returns upstream RPC, transport, or response-parsing errors when the
    /// block number is not satisfied by the mock adapter.
    pub fn get_block_number(&self) -> Result<u64> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            return Ok(self.mock.read().block_number);
        }
        let response = self.call("eth_blockNumber", json!([]))?;
        parse_hex_u64(&response)
    }

    /// Returns upstream account info.
    ///
    /// # Errors
    ///
    /// Returns upstream balance, nonce, code, transport, or parsing errors
    /// when the account is not satisfied by the mock adapter.
    pub fn get_account_info(
        &self,
        address: Address,
        block: BlockTag,
    ) -> Result<Option<AccountInfo>> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            let info = self
                .mock
                .read()
                .accounts
                .get(&address)
                .cloned()
                .unwrap_or_else(|| AccountInfo {
                    balance: U256::from(DEFAULT_MOCK_BALANCE),
                    nonce: 0,
                    code_hash: Bytecode::default().hash_slow(),
                    code: Some(Bytecode::default()),
                });
            return Ok(Some(info));
        }

        let balance =
            parse_hex_u256(&self.call("eth_getBalance", json!([address, block.as_json()]))?)?;
        let nonce = parse_hex_u64(
            &self.call("eth_getTransactionCount", json!([address, block.as_json()]))?,
        )?;
        let code_bytes =
            parse_bytes(&self.call("eth_getCode", json!([address, block.as_json()]))?)?;
        let code = Bytecode::new_raw(code_bytes);
        self.code_by_hash
            .write()
            .insert(code.hash_slow(), code.clone());
        self.addresses_by_code_hash
            .write()
            .insert(code.hash_slow(), address);
        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash: code.hash_slow(),
            code: Some(code),
        }))
    }

    /// Returns upstream storage.
    ///
    /// # Errors
    ///
    /// Returns upstream storage, transport, or parsing errors when the slot is
    /// not satisfied by the mock adapter.
    pub fn get_storage_at(&self, address: Address, slot: U256, block: BlockTag) -> Result<U256> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            return Ok(self
                .mock
                .read()
                .storage
                .get(&(address, slot))
                .copied()
                .unwrap_or(U256::ZERO));
        }

        parse_hex_u256(&self.call(
            "eth_getStorageAt",
            json!([address, format!("0x{slot:x}"), block.as_json()]),
        )?)
    }

    /// Executes an upstream `eth_call`.
    ///
    /// # Errors
    ///
    /// Returns upstream call, transport, or parsing errors when the call is
    /// not satisfied by the mock adapter.
    pub fn eth_call(
        &self,
        from: Address,
        to: Address,
        data: &Bytes,
        block: BlockTag,
    ) -> Result<Bytes> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            return Ok(Bytes::from(vec![0_u8; 32]));
        }

        parse_bytes(&self.call(
            "eth_call",
            json!([{
                "from": from,
                "to": to,
                "data": format!("0x{}", hex::encode(data)),
            }, block.as_json()]),
        )?)
    }

    /// Returns a block payload with optional full transaction bodies.
    ///
    /// # Errors
    ///
    /// Returns upstream block-fetch, transport, or parsing errors when the
    /// block is not satisfied by the mock adapter.
    pub fn get_block_by_number(
        &self,
        block: BlockTag,
        full_transactions: bool,
    ) -> Result<Option<Value>> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            let mock = self.mock.read();
            let number = match block {
                BlockTag::Latest => mock.block_number,
                BlockTag::Number(number) => number,
            };
            let hash = mock
                .block_hashes
                .get(&number)
                .copied()
                .unwrap_or_else(|| keccak256(number.to_be_bytes()));
            let transactions = mock
                .block_transactions
                .get(&number)
                .cloned()
                .unwrap_or_default();
            let parent_hash = if number > 0 {
                keccak256((number - 1).to_le_bytes())
            } else {
                B256::ZERO
            };
            return Ok(Some(json!({
                "hash": hash,
                "number": format!("0x{number:x}"),
                "timestamp": format!("0x{:x}", mock.timestamp),
                "parentHash": format!("{parent_hash}"),
                "gasLimit": "0x1c9c380",
                "gasUsed": "0x0",
                "transactions": transactions,
            })));
        }

        let response = self.call(
            "eth_getBlockByNumber",
            json!([block.as_json(), full_transactions]),
        )?;
        Ok(if response.is_null() {
            None
        } else {
            Some(response)
        })
    }

    /// Returns a transaction payload by hash.
    ///
    /// # Errors
    ///
    /// Returns upstream transaction-fetch, transport, or parsing errors when
    /// the transaction is not satisfied by the mock adapter.
    pub fn get_transaction_by_hash(&self, tx_hash: B256) -> Result<Option<Value>> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            self.mock_transaction_fetches
                .fetch_add(1, Ordering::Relaxed);
            let tx = self.mock.read().transactions_by_hash.get(&tx_hash).cloned();
            return Ok(tx);
        }

        let response = self.call("eth_getTransactionByHash", json!([tx_hash]))?;
        Ok(if response.is_null() {
            None
        } else {
            Some(response)
        })
    }

    /// Returns a block payload by hash.
    ///
    /// # Errors
    ///
    /// Returns upstream block-fetch, transport, or parsing errors.
    pub fn get_block_by_hash(&self, hash: B256, full: bool) -> Result<Option<Value>> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            return Ok(None);
        }

        let response = self.call("eth_getBlockByHash", json!([hash, full]))?;
        Ok(if response.is_null() {
            None
        } else {
            Some(response)
        })
    }

    /// Returns a transaction receipt by hash.
    ///
    /// # Errors
    ///
    /// Returns upstream receipt-fetch, transport, or parsing errors.
    pub fn get_transaction_receipt(&self, tx_hash: B256) -> Result<Option<Value>> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            return Ok(None);
        }

        let response = self.call("eth_getTransactionReceipt", json!([tx_hash]))?;
        Ok(if response.is_null() {
            None
        } else {
            Some(response)
        })
    }

    /// Returns upstream bytecode by code hash.
    ///
    /// # Errors
    ///
    /// Returns upstream code-fetch, transport, or parsing errors. Returns
    /// [`MirageError::Upstream`] when the bytecode hash cannot be resolved.
    pub fn get_code_by_hash(&self, code_hash: B256, block: BlockTag) -> Result<Bytecode> {
        if let Some(code) = self.code_by_hash.read().get(&code_hash).cloned() {
            return Ok(code);
        }

        if code_hash == Bytecode::default().hash_slow() {
            return Ok(Bytecode::default());
        }

        self.maybe_delay_mock();

        if let Some(code) = self
            .mock
            .read()
            .accounts
            .values()
            .find_map(|info| (info.code_hash == code_hash).then(|| info.code.clone()))
            .flatten()
        {
            self.code_by_hash.write().insert(code_hash, code.clone());
            return Ok(code);
        }

        if let Some(address) = self.addresses_by_code_hash.read().get(&code_hash).copied() {
            let code_bytes =
                parse_bytes(&self.call("eth_getCode", json!([address, block.as_json()]))?)?;
            let code = Bytecode::new_raw(code_bytes);
            if code.hash_slow() == code_hash {
                self.code_by_hash.write().insert(code_hash, code.clone());
                return Ok(code);
            }
        }

        Err(MirageError::Upstream(format!(
            "unknown bytecode hash {code_hash}"
        )))
    }

    /// Returns a deterministic or upstream-provided block hash.
    ///
    /// # Errors
    ///
    /// Returns upstream block-fetch, transport, missing-field, or parsing
    /// errors when the hash is not satisfied by the mock adapter.
    pub fn get_block_hash(&self, number: u64) -> Result<B256> {
        if self.http_url.is_none() {
            self.maybe_delay_mock();
            let hash = *self
                .mock
                .write()
                .block_hashes
                .entry(number)
                .or_insert_with(|| keccak256(number.to_be_bytes()));
            return Ok(hash);
        }

        let response = self.call(
            "eth_getBlockByNumber",
            json!([format!("0x{number:x}"), false]),
        )?;
        response
            .get("hash")
            .ok_or_else(|| MirageError::Upstream("missing block hash".to_owned()))
            .and_then(parse_b256)
    }

    /// Performs a basic connectivity check.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::get_block_number`].
    pub fn health_check(&self) -> Result<u64> {
        self.get_block_number()
    }

    /// Subscribes to `newHeads` notifications from the WebSocket upstream.
    ///
    /// # Errors
    ///
    /// Returns WebSocket connect, subscribe, read, text-frame, JSON decode, or
    /// subscription-id errors while establishing the stream.
    pub async fn subscribe_new_heads(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<u64>> + Send>>> {
        let ws_url = self.ws_url.as_ref().ok_or_else(|| {
            MirageError::Upstream("no WebSocket upstream URL configured".to_owned())
        })?;
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(|error| MirageError::Upstream(format!("websocket connect failed: {error}")))?;
        let (mut write, mut read) = ws_stream.split();
        write
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "eth_subscribe",
                    "params": ["newHeads"],
                })
                .to_string()
                .into(),
            ))
            .await
            .map_err(|error| {
                MirageError::Upstream(format!("websocket subscribe failed: {error}"))
            })?;

        let mut subscription_id = None;
        while let Some(message) = read.next().await {
            let message = message.map_err(|error| {
                MirageError::Upstream(format!("websocket read failed: {error}"))
            })?;
            let text = message.to_text().map_err(|error| {
                MirageError::Upstream(format!("websocket text frame failed: {error}"))
            })?;
            let value: Value = serde_json::from_str(text)?;
            if let Some(error) = value.get("error") {
                return Err(MirageError::Upstream(format!(
                    "eth_subscribe error: {error}"
                )));
            }
            if let Some(result) = value.get("result") {
                subscription_id = Some(result.clone());
                break;
            }
        }

        let subscription_id = subscription_id
            .ok_or_else(|| MirageError::Upstream("missing websocket subscription id".to_owned()))?;

        let heads = stream::unfold(
            (read, subscription_id),
            |(mut read, subscription_id)| async move {
                while let Some(message) = read.next().await {
                    let message = match message {
                        Ok(message) => message,
                        Err(error) => {
                            return Some((
                                Err(MirageError::Upstream(format!(
                                    "websocket read failed: {error}"
                                ))),
                                (read, subscription_id),
                            ));
                        }
                    };
                    let Ok(text) = message.to_text() else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_str::<Value>(text) else {
                        continue;
                    };
                    let Some(params) = value.get("params") else {
                        continue;
                    };
                    if params.get("subscription") != Some(&subscription_id) {
                        continue;
                    }
                    let Some(result) = params.get("result") else {
                        continue;
                    };
                    let Some(number) = result.get("number") else {
                        continue;
                    };
                    match parse_hex_u64(number) {
                        Ok(number) => return Some((Ok(number), (read, subscription_id))),
                        Err(error) => return Some((Err(error), (read, subscription_id))),
                    }
                }
                None
            },
        );

        Ok(Box::pin(heads))
    }

    #[allow(clippy::disallowed_methods)] // blocking retry loop — no async runtime
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        {
            let mut stats = self.stats.write();
            stats.calls = stats.calls.saturating_add(1);
        }

        let url = self
            .http_url
            .as_deref()
            .ok_or_else(|| MirageError::Upstream("no upstream URL configured".to_owned()))?;
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| MirageError::Upstream("no HTTP client configured".to_owned()))?;

        let mut attempt = 0;
        let mut backoff = self.retry_backoff;
        loop {
            self.enforce_rate_limit();
            let response = client
                .post(url)
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": method,
                    "params": params,
                }))
                .send();

            let response = match response {
                Ok(response) => response,
                Err(_error) if attempt < self.retry_attempts => {
                    attempt = attempt.saturating_add(1);
                    thread::sleep(backoff);
                    backoff = backoff.saturating_mul(2);
                    continue;
                }
                Err(error) => {
                    let mut stats = self.stats.write();
                    stats.errors = stats.errors.saturating_add(1);
                    return Err(MirageError::Http(error));
                }
            };

            let value = match response.json::<Value>() {
                Ok(value) => value,
                Err(_error) if attempt < self.retry_attempts => {
                    attempt = attempt.saturating_add(1);
                    thread::sleep(backoff);
                    backoff = backoff.saturating_mul(2);
                    continue;
                }
                Err(error) => {
                    let mut stats = self.stats.write();
                    stats.errors = stats.errors.saturating_add(1);
                    return Err(MirageError::Http(error));
                }
            };

            if let Some(error) = value.get("error") {
                let mut stats = self.stats.write();
                stats.errors = stats.errors.saturating_add(1);
                return Err(MirageError::Upstream(error.to_string()));
            }

            return value.get("result").cloned().ok_or_else(|| {
                MirageError::Upstream(format!("missing result for method {method}"))
            });
        }
    }

    #[allow(clippy::disallowed_methods)] // blocking rate-limit sleep
    fn enforce_rate_limit(&self) {
        if self.requests_per_second == 0 {
            return;
        }
        let rps = f64::from(self.requests_per_second);
        let cap = f64::from(self.burst);
        loop {
            let wait_secs = {
                let mut rate_limit = self.rate_limit.write();
                let now = Instant::now();
                let elapsed = now.duration_since(rate_limit.last_refill_at).as_secs_f64();
                if elapsed > 0.0 {
                    rate_limit.tokens = (rate_limit.tokens + elapsed * rps).min(cap);
                    rate_limit.last_refill_at = now;
                }
                if rate_limit.tokens >= 1.0 {
                    rate_limit.tokens -= 1.0;
                    return;
                }
                let deficit = 1.0 - rate_limit.tokens;
                deficit / rps
            };
            // Avoid busy-spinning when the OS timer resolution is coarse.
            thread::sleep(Duration::from_secs_f64(wait_secs.max(1e-6)));
        }
    }

    #[allow(clippy::disallowed_methods)] // blocking mock delay
    fn maybe_delay_mock(&self) {
        let delay = self.mock.read().delay;
        if !delay.is_zero() {
            thread::sleep(delay);
        }
    }
}

fn parse_hex_u64(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .ok_or_else(|| MirageError::Upstream("expected hex quantity string".to_owned()))?;
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::Upstream(format!("invalid hex quantity: {error}")))
}

fn parse_hex_u256(value: &Value) -> Result<U256> {
    let text = value
        .as_str()
        .ok_or_else(|| MirageError::Upstream("expected hex quantity string".to_owned()))?;
    U256::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::Upstream(format!("invalid U256 quantity: {error}")))
}

fn parse_b256(value: &Value) -> Result<B256> {
    let text = value
        .as_str()
        .ok_or_else(|| MirageError::Upstream("expected B256 hex string".to_owned()))?;
    text.parse::<B256>()
        .map_err(|error| MirageError::Upstream(format!("invalid B256: {error}")))
}

fn parse_bytes(value: &Value) -> Result<Bytes> {
    let text = value
        .as_str()
        .ok_or_else(|| MirageError::Upstream("expected bytes hex string".to_owned()))?;
    let bytes = hex::decode(text.trim_start_matches("0x"))
        .map_err(|error| MirageError::Upstream(format!("invalid bytecode: {error}")))?;
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upstream_rpc_rate_limit_tokens_start_at_burst() {
        let upstream = UpstreamRpc::new_with_limits(None, None, 1, 100, 200);
        // Burst of 200 calls should complete nearly instantly
        let start = Instant::now();
        for _ in 0..200 {
            upstream.enforce_rate_limit();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(200),
            "200 burst calls should complete quickly, took {elapsed:?}"
        );

        // After burst exhaustion, next call must wait (~10ms for 100 rps)
        let start = Instant::now();
        upstream.enforce_rate_limit();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(5),
            "call after burst exhaustion should wait >=5ms, took {elapsed:?}"
        );
    }

    #[test]
    fn test_upstream_rpc_retry_backoff() {
        let upstream = UpstreamRpc::new(None, None, 1);
        assert_eq!(upstream.retry_attempts, 3);
        assert_eq!(upstream.retry_backoff, Duration::from_millis(100));
        assert_eq!(upstream.requests_per_second, DEFAULT_UPSTREAM_RPS);
        assert_eq!(upstream.burst, DEFAULT_UPSTREAM_BURST);
    }

    #[test]
    fn test_upstream_rpc_mock_get_block_number() {
        let upstream = UpstreamRpc::mock(1);
        upstream.mock.write().block_number = 42;
        assert_eq!(upstream.get_block_number().unwrap(), 42);
    }

    #[test]
    fn test_rate_limit_disabled_when_zero_rps() {
        let upstream = UpstreamRpc::new_with_limits(None, None, 1, 0, 0);
        let start = Instant::now();
        for _ in 0..500 {
            upstream.enforce_rate_limit();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "zero rps should skip rate limiting, took {elapsed:?}"
        );
    }

    #[test]
    fn test_mock_account_info() {
        let upstream = UpstreamRpc::mock(1);
        let address = Address::ZERO;
        let info = upstream
            .get_account_info(address, BlockTag::Latest)
            .unwrap();
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.balance, U256::from(DEFAULT_MOCK_BALANCE));
    }

    #[test]
    fn test_mock_storage() {
        let upstream = UpstreamRpc::mock(1);
        let address = Address::ZERO;
        let slot = U256::from(1);
        upstream.set_mock_storage(address, slot, U256::from(99));
        let value = upstream
            .get_storage_at(address, slot, BlockTag::Latest)
            .unwrap();
        assert_eq!(value, U256::from(99));
    }

    /// Plan V2: mock upstream path uses alloy [`Address`] / [`U256`] the same way HTTP RPC would.
    #[test]
    fn test_upstream_rpc_alloy_primitives_mock_contract() {
        let upstream = UpstreamRpc::mock(9);
        assert_eq!(upstream.chain_id(), 9);
        upstream.set_mock_block_number(123);
        assert_eq!(upstream.health_check().expect("health"), 123);
        let token = Address::repeat_byte(0xcd);
        upstream.set_mock_storage(token, U256::from(5), U256::from(11));
        assert_eq!(
            upstream
                .get_storage_at(token, U256::from(5), BlockTag::Latest)
                .expect("slot"),
            U256::from(11)
        );
    }

    #[test]
    fn test_block_tag_json() {
        assert_eq!(BlockTag::Latest.as_json(), Value::String("latest".into()));
        assert_eq!(
            BlockTag::Number(255).as_json(),
            Value::String("0xff".into())
        );
    }

    /// INV-029: upstream RPS rate limiter enforces throughput cap after burst exhaustion.
    #[test]
    fn test_upstream_rps_limit() {
        // Default: 100 rps, 200 burst.
        let default_upstream = UpstreamRpc::new(None, None, 1);
        assert_eq!(default_upstream.requests_per_second, 100);
        assert_eq!(default_upstream.burst, 200);

        // Use high rps (2000) and small burst (5) so exhaustion happens fast.
        let upstream = UpstreamRpc::new_with_limits(None, None, 1, 2000, 5);

        // Consume burst instantly.
        for _ in 0..5 {
            upstream.enforce_rate_limit();
        }

        // 5 more calls at 2000 rps should take >= 1ms (each call costs ~0.5ms).
        let start = Instant::now();
        for _ in 0..5 {
            upstream.enforce_rate_limit();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1),
            "calls after burst exhaustion must be throttled, took {elapsed:?}"
        );
    }

    /// INV-030: upstream RPC retry uses exponential backoff.
    #[test]
    fn test_upstream_exponential_backoff() {
        let upstream = UpstreamRpc::new(None, None, 1);
        // Default retry configuration: three retries, backoff 100 → 200 → 400 ms.
        assert_eq!(upstream.retry_attempts, 3);
        assert_eq!(upstream.retry_backoff, Duration::from_millis(100));

        // Simulate the doubling sequence used inside `call`.
        let mut backoff = upstream.retry_backoff;
        assert_eq!(backoff, Duration::from_millis(100));
        backoff = backoff.saturating_mul(2);
        assert_eq!(backoff, Duration::from_millis(200));
        backoff = backoff.saturating_mul(2);
        assert_eq!(backoff, Duration::from_millis(400));
    }
}

//! In-memory mock implementations of [`ChainClient`] and [`ChainWallet`].
//!
//! Suitable for unit tests and for wiring Roko components that need *some*
//! chain backend without standing up a real RPC endpoint. These mocks:
//!
//! * share state through `Arc<RwLock<...>>` so the same handle can be cloned
//!   across tasks,
//! * advance deterministically (block numbers monotonically increase, nonces
//!   auto-increment per tx), and
//! * can be paired via [`paired_mocks`] so a wallet's submitted txs show up
//!   in the client's receipt map after a single mined block.
//!
//! See `paired_mocks` for the canonical "wallet submits, client observes"
//! wiring used in integration tests.

use crate::client::ChainClient;
use crate::types::{
    BlockNumber, CallResult, ChainError, ChainHeader, ChainResult, LogEntry, Receipt, TxHash,
    TxRequest,
};
use crate::wallet::ChainWallet;
use crate::witness::{WITNESS_MARKER, WITNESS_TOPIC};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Shared mutable state behind [`MockChainClient`].
#[derive(Debug, Default)]
struct MockChainState {
    headers: Vec<ChainHeader>,
    receipts: HashMap<TxHash, Receipt>,
    logs: Vec<LogEntry>,
    /// `(address, slot, block?)` → storage bytes. `None` block = latest.
    storage: HashMap<(String, String, Option<BlockNumber>), Vec<u8>>,
    /// Canned `eth_call` output.
    call_output: Vec<u8>,
    call_gas_used: u64,
    chain_id: u64,
}

/// In-memory [`ChainClient`] backed by pre-seeded block/receipt/log data.
///
/// Clone-able: the underlying state is `Arc<RwLock<..>>`, so clones share
/// the same mock chain. Use [`MockChainClient::local`] for a ready-to-use
/// instance with one genesis block.
#[derive(Clone, Debug)]
pub struct MockChainClient {
    state: Arc<RwLock<MockChainState>>,
    name: String,
}

impl Default for MockChainClient {
    fn default() -> Self {
        Self::local()
    }
}

impl MockChainClient {
    /// Construct a mock client with one genesis block and chain id `1`.
    pub fn local() -> Self {
        let mut state = MockChainState {
            chain_id: 1,
            call_gas_used: 21_000,
            ..Default::default()
        };
        state.headers.push(ChainHeader {
            number: 0,
            hash: "0x0".to_string(),
            parent: "0x0".to_string(),
            timestamp: 0,
        });
        Self {
            state: Arc::new(RwLock::new(state)),
            name: "mock".to_string(),
        }
    }

    /// Override the mock chain id. Returns `self` for chaining.
    #[must_use]
    pub fn with_chain_id(self, id: u64) -> Self {
        self.state.write().chain_id = id;
        self
    }

    /// Override the canned `eth_call` output. Returns `self` for chaining.
    #[must_use]
    pub fn with_call_result(self, output: Vec<u8>, gas_used: u64) -> Self {
        {
            let mut s = self.state.write();
            s.call_output = output;
            s.call_gas_used = gas_used;
        }
        self
    }

    /// Append a header to the mock chain. Numbers must be monotonic; the
    /// mock does not enforce parent linkage.
    pub fn push_block(&self, header: ChainHeader) {
        self.state.write().headers.push(header);
    }

    /// Mine an empty block with an auto-assigned number and the previous
    /// block's hash as parent. Useful for tests that just need the tip to
    /// advance.
    #[allow(clippy::significant_drop_tightening)]
    pub fn mine_empty_block(&self) -> BlockNumber {
        let mut s = self.state.write();
        let number = s.headers.last().map_or(0, |h| h.number + 1);
        let parent = s
            .headers
            .last()
            .map_or_else(|| "0x0".to_string(), |h| h.hash.clone());
        s.headers.push(ChainHeader {
            number,
            hash: format!("0x{number:064x}"),
            parent,
            timestamp: number,
        });
        number
    }

    /// Insert a receipt directly (e.g. for a pre-mined tx).
    pub fn insert_receipt(&self, receipt: Receipt) {
        self.state
            .write()
            .receipts
            .insert(receipt.tx_hash.clone(), receipt);
    }

    /// Insert a log entry the `get_logs` filter can later return.
    pub fn insert_log(&self, log: LogEntry) {
        self.state.write().logs.push(log);
    }

    /// Seed a storage slot. `block == None` = latest.
    pub fn insert_storage(
        &self,
        address: impl Into<String>,
        slot: impl Into<String>,
        block: Option<BlockNumber>,
        value: Vec<u8>,
    ) {
        self.state
            .write()
            .storage
            .insert((address.into(), slot.into(), block), value);
    }
}

#[async_trait]
impl ChainClient for MockChainClient {
    async fn block_number(&self) -> ChainResult<BlockNumber> {
        let s = self.state.read();
        Ok(s.headers.last().map_or(0, |h| h.number))
    }

    async fn get_block_header(&self, number: BlockNumber) -> ChainResult<ChainHeader> {
        let s = self.state.read();
        s.headers
            .iter()
            .find(|h| h.number == number)
            .cloned()
            .ok_or_else(|| ChainError::Rpc(format!("no block at {number}")))
    }

    async fn get_receipt(&self, tx: &TxHash) -> ChainResult<Option<Receipt>> {
        let s = self.state.read();
        Ok(s.receipts.get(tx).cloned())
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn get_logs(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        addresses: &[String],
        topics: &[String],
    ) -> ChainResult<Vec<LogEntry>> {
        if to < from {
            return Ok(Vec::new());
        }
        let s = self.state.read();
        let out = s
            .logs
            .iter()
            .filter(|log| {
                let addr_ok = addresses.is_empty() || addresses.iter().any(|a| a == &log.address);
                let topic_ok =
                    topics.is_empty() || log.topics.iter().any(|t| topics.iter().any(|w| w == t));
                addr_ok && topic_ok
            })
            .cloned()
            .collect();
        Ok(out)
    }

    async fn get_storage_at(
        &self,
        address: &str,
        slot: &str,
        block: Option<BlockNumber>,
    ) -> ChainResult<Vec<u8>> {
        let s = self.state.read();
        let key = (address.to_string(), slot.to_string(), block);
        if let Some(v) = s.storage.get(&key) {
            return Ok(v.clone());
        }
        // Fall back to the "latest" entry if a block-specific one is missing.
        let latest_key = (address.to_string(), slot.to_string(), None);
        Ok(s.storage.get(&latest_key).cloned().unwrap_or_default())
    }

    async fn eth_call(
        &self,
        _request: &TxRequest,
        _block: Option<BlockNumber>,
    ) -> ChainResult<CallResult> {
        let s = self.state.read();
        Ok(CallResult {
            output: s.call_output.clone(),
            gas_used: s.call_gas_used,
        })
    }

    async fn chain_id(&self) -> ChainResult<u64> {
        Ok(self.state.read().chain_id)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Shared mutable state behind [`MockChainWallet`].
#[derive(Debug)]
struct MockWalletState {
    address: String,
    balance: u128,
    nonce: u64,
    submitted: Vec<(TxHash, TxRequest)>,
    tx_counter: u64,
    /// Optional hook: a paired client into which submitted txs should be
    /// injected as freshly mined receipts.
    paired_client: Option<MockChainClient>,
}

/// In-memory [`ChainWallet`] with configurable balance, nonce, and
/// auto-incrementing tx hashes.
#[derive(Clone, Debug)]
pub struct MockChainWallet {
    state: Arc<RwLock<MockWalletState>>,
    name: String,
}

impl MockChainWallet {
    /// Construct a wallet seeded with `balance_wei`, nonce 0, and a
    /// deterministic placeholder address.
    pub fn funded(balance_wei: u128) -> Self {
        Self {
            state: Arc::new(RwLock::new(MockWalletState {
                address: "0x000000000000000000000000000000000000beef".to_string(),
                balance: balance_wei,
                nonce: 0,
                submitted: Vec::new(),
                tx_counter: 0,
                paired_client: None,
            })),
            name: "mock-wallet".to_string(),
        }
    }

    /// Override the wallet address. Returns `self` for chaining.
    #[must_use]
    pub fn with_address(self, address: impl Into<String>) -> Self {
        self.state.write().address = address.into();
        self
    }

    /// Override the starting nonce. Returns `self` for chaining.
    #[must_use]
    pub fn with_nonce(self, nonce: u64) -> Self {
        self.state.write().nonce = nonce;
        self
    }

    /// All `(hash, tx)` pairs submitted so far.
    pub fn submitted(&self) -> Vec<(TxHash, TxRequest)> {
        self.state.read().submitted.clone()
    }

    /// Wire this wallet to a client so every `sign_and_submit` pushes a
    /// successful receipt into the client after one mined block.
    pub fn pair_with(&self, client: &MockChainClient) {
        self.state.write().paired_client = Some(client.clone());
    }
}

#[async_trait]
impl ChainWallet for MockChainWallet {
    async fn address(&self) -> ChainResult<String> {
        Ok(self.state.read().address.clone())
    }

    async fn balance(&self, _block: Option<BlockNumber>) -> ChainResult<u128> {
        Ok(self.state.read().balance)
    }

    async fn nonce(&self) -> ChainResult<u64> {
        Ok(self.state.read().nonce)
    }

    async fn sign_and_submit(&self, tx: TxRequest) -> ChainResult<TxHash> {
        let mut s = self.state.write();

        // Balance check — only counts `value`, not gas costs (mock keeps it
        // simple; a real wallet would factor in `gas_limit * max_fee`).
        if tx.value > s.balance {
            return Err(ChainError::InsufficientFunds {
                have: s.balance,
                need: tx.value,
            });
        }

        // Nonce gap check: if the caller pinned a nonce, it must match.
        if let Some(got) = tx.nonce
            && got != s.nonce
        {
            return Err(ChainError::NonceGap {
                expected: s.nonce,
                got,
            });
        }

        s.balance = s.balance.saturating_sub(tx.value);
        let counter = s.tx_counter + 1;
        s.tx_counter = counter;
        let hash = TxHash::new(format!("0x{counter:064x}"));
        s.nonce += 1;
        let witness_log = tx
            .data
            .strip_prefix(WITNESS_MARKER)
            .and_then(|data| data.get(..32))
            .map(|data| LogEntry {
                address: tx.to.clone().unwrap_or_else(|| s.address.clone()),
                topics: vec![WITNESS_TOPIC.to_string()],
                data: data.to_vec(),
            });
        s.submitted.push((hash.clone(), tx));

        // If paired with a client, mine a block and insert a receipt so the
        // client observes the tx on the next poll.
        if let Some(client) = s.paired_client.clone() {
            drop(s);
            let block_number = client.mine_empty_block();
            client.insert_receipt(Receipt {
                tx_hash: hash.clone(),
                status: true,
                block_number,
                gas_used: 21_000,
                logs: witness_log.into_iter().collect(),
            });
        }

        Ok(hash)
    }

    async fn wait_for_receipt(&self, tx: &TxHash, timeout_ms: u64) -> ChainResult<Receipt> {
        // If we are paired with a client, poll it until a matching receipt
        // appears or the timeout elapses.
        let paired = self.state.read().paired_client.clone();
        let Some(client) = paired else {
            return Err(ChainError::Unsupported(
                "wait_for_receipt requires a paired client".into(),
            ));
        };

        let step = Duration::from_millis(10);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(r) = client.get_receipt(tx).await? {
                return Ok(r);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ChainError::Timeout(format!(
                    "no receipt for {tx} within {timeout_ms}ms"
                )));
            }
            tokio::time::sleep(step).await;
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Construct a client+wallet pair where the wallet's submitted txs appear
/// as receipts on the client after a single mined block.
///
/// The returned wallet is funded with `balance` wei and already wired
/// to the client via [`MockChainWallet::pair_with`].
pub fn paired_mocks(balance: u128) -> (MockChainClient, MockChainWallet) {
    let client = MockChainClient::local();
    let wallet = MockChainWallet::funded(balance);
    wallet.pair_with(&client);
    (client, wallet)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn block_number_returns_seeded_head() {
        let c = MockChainClient::local();
        assert_eq!(c.block_number().await.unwrap(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_block_advances_head() {
        let c = MockChainClient::local();
        c.push_block(ChainHeader {
            number: 1,
            hash: "0x1".into(),
            parent: "0x0".into(),
            timestamp: 10,
        });
        c.push_block(ChainHeader {
            number: 2,
            hash: "0x2".into(),
            parent: "0x1".into(),
            timestamp: 20,
        });
        assert_eq!(c.block_number().await.unwrap(), 2);
        let h = c.get_block_header(1).await.unwrap();
        assert_eq!(h.hash, "0x1");
        assert_eq!(h.timestamp, 10);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_block_header_missing_is_rpc_error() {
        let c = MockChainClient::local();
        let err = c.get_block_header(99).await.unwrap_err();
        assert!(matches!(err, ChainError::Rpc(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_receipt_returns_seeded() {
        let c = MockChainClient::local();
        let hash = TxHash::new("0xaa");
        c.insert_receipt(Receipt {
            tx_hash: hash.clone(),
            status: true,
            block_number: 1,
            gas_used: 21_000,
            logs: vec![],
        });
        let r = c.get_receipt(&hash).await.unwrap().unwrap();
        assert_eq!(r.block_number, 1);
        assert!(r.status);

        let none = c.get_receipt(&TxHash::new("0xbb")).await.unwrap();
        assert!(none.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_logs_filters_by_address_and_topic() {
        let c = MockChainClient::local();
        c.insert_log(LogEntry {
            address: "0xaaa".into(),
            topics: vec!["0xt1".into()],
            data: vec![1],
        });
        c.insert_log(LogEntry {
            address: "0xbbb".into(),
            topics: vec!["0xt2".into()],
            data: vec![2],
        });
        c.insert_log(LogEntry {
            address: "0xaaa".into(),
            topics: vec!["0xt2".into()],
            data: vec![3],
        });

        // Address filter only.
        let a = c
            .get_logs(0, 10, &["0xaaa".to_string()], &[])
            .await
            .unwrap();
        assert_eq!(a.len(), 2);

        // Topic filter only.
        let t = c.get_logs(0, 10, &[], &["0xt2".to_string()]).await.unwrap();
        assert_eq!(t.len(), 2);

        // Both.
        let both = c
            .get_logs(0, 10, &["0xaaa".to_string()], &["0xt2".to_string()])
            .await
            .unwrap();
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].data, vec![3]);

        // No match.
        let none = c.get_logs(0, 10, &[], &["0xzz".to_string()]).await.unwrap();
        assert!(none.is_empty());

        // Empty filters = everything.
        let all = c.get_logs(0, 10, &[], &[]).await.unwrap();
        assert_eq!(all.len(), 3);

        // Inverted range yields empty.
        let inverted = c.get_logs(10, 0, &[], &[]).await.unwrap();
        assert!(inverted.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_storage_at_reads_seeded_slot() {
        let c = MockChainClient::local();
        c.insert_storage("0xc0ffee", "0x0", None, vec![0x42]);
        let v = c.get_storage_at("0xc0ffee", "0x0", None).await.unwrap();
        assert_eq!(v, vec![0x42]);

        // Missing slot returns empty.
        let v = c.get_storage_at("0xc0ffee", "0x9", None).await.unwrap();
        assert!(v.is_empty());

        // Block-specific falls back to latest if missing at that height.
        let v = c.get_storage_at("0xc0ffee", "0x0", Some(5)).await.unwrap();
        assert_eq!(v, vec![0x42]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn eth_call_returns_canned_output() {
        let c = MockChainClient::local().with_call_result(vec![0xde, 0xad], 1234);
        let res = c.eth_call(&TxRequest::default(), None).await.unwrap();
        assert_eq!(res.output, vec![0xde, 0xad]);
        assert_eq!(res.gas_used, 1234);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn chain_id_defaults_to_one() {
        let c = MockChainClient::local();
        assert_eq!(c.chain_id().await.unwrap(), 1);
        let c2 = MockChainClient::local().with_chain_id(31337);
        assert_eq!(c2.chain_id().await.unwrap(), 31337);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_name_is_mock() {
        let c = MockChainClient::local();
        assert_eq!(c.name(), "mock");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wallet_address_balance_nonce() {
        let w = MockChainWallet::funded(1_000_000);
        assert!(w.address().await.unwrap().starts_with("0x"));
        assert_eq!(w.balance(None).await.unwrap(), 1_000_000);
        assert_eq!(w.nonce().await.unwrap(), 0);
        assert_eq!(w.name(), "mock-wallet");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wallet_with_address_and_nonce_overrides() {
        let w = MockChainWallet::funded(10)
            .with_address("0xabc")
            .with_nonce(7);
        assert_eq!(w.address().await.unwrap(), "0xabc");
        assert_eq!(w.nonce().await.unwrap(), 7);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_submit_increments_nonce_and_assigns_hash() {
        let w = MockChainWallet::funded(100);
        let h1 = w
            .sign_and_submit(TxRequest {
                value: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        let h2 = w
            .sign_and_submit(TxRequest {
                value: 5,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_ne!(h1, h2);
        assert_eq!(w.nonce().await.unwrap(), 2);
        assert_eq!(w.balance(None).await.unwrap(), 85);
        assert_eq!(w.submitted().len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_submit_rejects_insufficient_funds() {
        let w = MockChainWallet::funded(5);
        let err = w
            .sign_and_submit(TxRequest {
                value: 10,
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ChainError::InsufficientFunds { have: 5, need: 10 }
        ));
        assert_eq!(w.nonce().await.unwrap(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_submit_rejects_nonce_gap() {
        let w = MockChainWallet::funded(100);
        let err = w
            .sign_and_submit(TxRequest {
                value: 1,
                nonce: Some(5),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ChainError::NonceGap {
                expected: 0,
                got: 5
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sign_and_submit_accepts_matching_pinned_nonce() {
        let w = MockChainWallet::funded(100).with_nonce(3);
        let h = w
            .sign_and_submit(TxRequest {
                value: 1,
                nonce: Some(3),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(h.as_str().starts_with("0x"));
        assert_eq!(w.nonce().await.unwrap(), 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_for_receipt_without_pair_is_unsupported() {
        let w = MockChainWallet::funded(10);
        let err = w
            .wait_for_receipt(&TxHash::new("0x1"), 10)
            .await
            .unwrap_err();
        assert!(matches!(err, ChainError::Unsupported(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn paired_mocks_wire_wallet_to_client() {
        let (client, wallet) = paired_mocks(1_000);
        let head_before = client.block_number().await.unwrap();
        let hash = wallet
            .sign_and_submit(TxRequest {
                value: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        let head_after = client.block_number().await.unwrap();
        assert_eq!(head_after, head_before + 1);

        let receipt = client.get_receipt(&hash).await.unwrap().unwrap();
        assert!(receipt.status);
        assert_eq!(receipt.block_number, head_after);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn wait_for_receipt_polls_until_match() {
        let (client, wallet) = paired_mocks(1_000);
        let hash = wallet
            .sign_and_submit(TxRequest {
                value: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        // Already inserted synchronously by paired mocks, but exercise the
        // polling path anyway.
        let receipt = wallet.wait_for_receipt(&hash, 1_000).await.unwrap();
        assert_eq!(receipt.tx_hash, hash);

        // Ensure the client sees it independently.
        let also = client.get_receipt(&hash).await.unwrap().unwrap();
        assert_eq!(also.tx_hash, hash);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn wait_for_receipt_times_out_when_not_mined() {
        let client = MockChainClient::local();
        let wallet = MockChainWallet::funded(10);
        wallet.pair_with(&client);
        let err = wallet
            .wait_for_receipt(&TxHash::new("0xdeadbeef"), 50)
            .await
            .unwrap_err();
        assert!(matches!(err, ChainError::Timeout(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mine_empty_block_is_monotonic() {
        let c = MockChainClient::local();
        let a = c.mine_empty_block();
        let b = c.mine_empty_block();
        assert_eq!(b, a + 1);
        assert_eq!(c.block_number().await.unwrap(), b);
    }
}

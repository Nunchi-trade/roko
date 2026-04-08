//! Alloy-backed [`ChainClient`](crate::ChainClient) and [`ChainWallet`](crate::ChainWallet).
//!
//! Feature-gated behind `alloy-backend` so mock-only consumers don't take an
//! alloy + hyper + reqwest tax. Talks to any JSON-RPC HTTP endpoint (mirage-rs,
//! anvil, live testnets).

use std::sync::Arc;
use std::time::Duration;

use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::eth::{BlockId, BlockNumberOrTag, TransactionRequest as AlloyTxRequest};
use alloy::signers::local::PrivateKeySigner;
use async_trait::async_trait;

use crate::{
    ChainClient, ChainError, ChainResult, ChainWallet,
    types::{BlockNumber, CallResult, ChainHeader, LogEntry, Receipt, TxHash, TxRequest},
};

fn to_rpc_err<E: std::fmt::Display>(err: E) -> ChainError {
    ChainError::Rpc(err.to_string())
}

fn parse_hex_address(s: &str) -> ChainResult<Address> {
    s.parse::<Address>()
        .map_err(|e| ChainError::InvalidAddress(format!("{s}: {e}")))
}

fn parse_hex_b256(s: &str) -> ChainResult<B256> {
    s.parse::<B256>()
        .map_err(|e| ChainError::Rpc(format!("invalid b256 {s}: {e}")))
}

fn tx_request_to_alloy(tx: &TxRequest) -> ChainResult<AlloyTxRequest> {
    let mut req = AlloyTxRequest::default();
    if let Some(from) = tx.from.as_deref() {
        req = req.with_from(parse_hex_address(from)?);
    }
    if let Some(to) = tx.to.as_deref() {
        req = req.with_to(parse_hex_address(to)?);
    }
    req = req.with_value(U256::from(tx.value));
    req = req.with_input(Bytes::from(tx.data.clone()));
    if let Some(gas) = tx.gas_limit {
        req = req.with_gas_limit(gas);
    }
    if let Some(max_fee) = tx.max_fee_per_gas {
        req = req.with_max_fee_per_gas(max_fee);
    }
    if let Some(prio) = tx.max_priority_fee_per_gas {
        req = req.with_max_priority_fee_per_gas(prio);
    }
    if let Some(nonce) = tx.nonce {
        req = req.with_nonce(nonce);
    }
    Ok(req)
}

/// Read-only JSON-RPC chain client.
#[derive(Clone)]
pub struct AlloyChainClient {
    provider: Arc<DynProvider>,
    name: String,
}

impl AlloyChainClient {
    /// Construct an HTTP-backed client pointing at `rpc_url`.
    pub fn http(rpc_url: &str) -> ChainResult<Self> {
        let url = reqwest::Url::parse(rpc_url)
            .map_err(|e| ChainError::Rpc(format!("invalid rpc url {rpc_url}: {e}")))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Self {
            provider: Arc::new(provider),
            name: format!("alloy-http({rpc_url})"),
        })
    }

    /// Share this client's provider (for building typed contract instances at call-site).
    pub fn provider(&self) -> Arc<DynProvider> {
        Arc::clone(&self.provider)
    }
}

#[async_trait]
impl ChainClient for AlloyChainClient {
    async fn block_number(&self) -> ChainResult<BlockNumber> {
        self.provider.get_block_number().await.map_err(to_rpc_err)
    }

    async fn get_block_header(&self, number: BlockNumber) -> ChainResult<ChainHeader> {
        let block = self
            .provider
            .get_block(BlockId::Number(BlockNumberOrTag::Number(number)))
            .await
            .map_err(to_rpc_err)?
            .ok_or_else(|| ChainError::Rpc(format!("block {number} not found")))?;
        let header = block.header;
        Ok(ChainHeader {
            number: header.number,
            hash: format!("{:#x}", header.hash),
            parent: format!("{:#x}", header.parent_hash),
            timestamp: header.timestamp,
        })
    }

    async fn get_receipt(&self, tx: &TxHash) -> ChainResult<Option<Receipt>> {
        let hash = parse_hex_b256(tx.as_str())?;
        let Some(r) = self
            .provider
            .get_transaction_receipt(hash)
            .await
            .map_err(to_rpc_err)?
        else {
            return Ok(None);
        };
        let logs = r
            .inner
            .logs()
            .iter()
            .map(|l| LogEntry {
                address: format!("{:#x}", l.inner.address),
                topics: l
                    .inner
                    .data
                    .topics()
                    .iter()
                    .map(|t| format!("{t:#x}"))
                    .collect(),
                data: l.inner.data.data.to_vec(),
            })
            .collect();
        Ok(Some(Receipt {
            tx_hash: tx.clone(),
            status: r.status(),
            block_number: r.block_number.unwrap_or(0),
            gas_used: r.gas_used,
            logs,
        }))
    }

    async fn get_logs(
        &self,
        _from: BlockNumber,
        _to: BlockNumber,
        _addresses: &[String],
        _topics: &[String],
    ) -> ChainResult<Vec<LogEntry>> {
        // Implemented via eth_getLogs when needed by scenarios; left as
        // Unsupported until a concrete caller requires the full filter surface.
        Err(ChainError::Unsupported("get_logs".into()))
    }

    async fn get_storage_at(
        &self,
        address: &str,
        slot: &str,
        block: Option<BlockNumber>,
    ) -> ChainResult<Vec<u8>> {
        let addr = parse_hex_address(address)?;
        let slot_u = U256::from_str_radix(slot.trim_start_matches("0x"), 16)
            .map_err(|e| ChainError::Rpc(format!("invalid slot {slot}: {e}")))?;
        let block_id = block.map_or(BlockId::Number(BlockNumberOrTag::Latest), |n| {
            BlockId::Number(BlockNumberOrTag::Number(n))
        });
        let v = self
            .provider
            .get_storage_at(addr, slot_u)
            .block_id(block_id)
            .await
            .map_err(to_rpc_err)?;
        Ok(v.to_be_bytes_vec())
    }

    async fn eth_call(
        &self,
        request: &TxRequest,
        block: Option<BlockNumber>,
    ) -> ChainResult<CallResult> {
        let req = tx_request_to_alloy(request)?;
        let mut call = self.provider.call(req);
        if let Some(n) = block {
            call = call.block(BlockId::Number(BlockNumberOrTag::Number(n)));
        }
        let bytes = call.await.map_err(to_rpc_err)?;
        Ok(CallResult {
            output: bytes.to_vec(),
            gas_used: 0,
        })
    }

    async fn chain_id(&self) -> ChainResult<u64> {
        self.provider.get_chain_id().await.map_err(to_rpc_err)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Signs + submits transactions using a locally-held private key.
#[derive(Clone)]
pub struct AlloyChainWallet {
    provider: Arc<DynProvider>,
    address: Address,
    chain_id: u64,
    name: String,
}

impl AlloyChainWallet {
    /// Build a wallet from a hex-encoded private key.
    pub fn from_hex_key(rpc_url: &str, key_hex: &str, chain_id: u64) -> ChainResult<Self> {
        let url = reqwest::Url::parse(rpc_url)
            .map_err(|e| ChainError::Rpc(format!("invalid rpc url {rpc_url}: {e}")))?;
        let trimmed = key_hex.trim_start_matches("0x");
        let signer: PrivateKeySigner = trimmed
            .parse()
            .map_err(|e| ChainError::Rpc(format!("invalid private key: {e}")))?;
        let address = signer.address();
        let eth_wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(eth_wallet)
            .connect_http(url)
            .erased();
        Ok(Self {
            provider: Arc::new(provider),
            address,
            chain_id,
            name: format!("alloy-wallet({address:#x})"),
        })
    }

    /// Address this wallet signs as.
    pub const fn address_typed(&self) -> Address {
        self.address
    }

    /// Share this wallet's provider (use this to send contract txs via alloy `sol!` bindings
    /// so signing, nonce management and gas estimation all go through one path).
    pub fn provider(&self) -> Arc<DynProvider> {
        Arc::clone(&self.provider)
    }
}

#[async_trait]
impl ChainWallet for AlloyChainWallet {
    async fn address(&self) -> ChainResult<String> {
        Ok(format!("{:#x}", self.address))
    }

    async fn balance(&self, block: Option<BlockNumber>) -> ChainResult<u128> {
        let mut call = self.provider.get_balance(self.address);
        if let Some(n) = block {
            call = call.block_id(BlockId::Number(BlockNumberOrTag::Number(n)));
        }
        let bal = call.await.map_err(to_rpc_err)?;
        Ok(u128::try_from(bal).unwrap_or(u128::MAX))
    }

    async fn nonce(&self) -> ChainResult<u64> {
        self.provider
            .get_transaction_count(self.address)
            .await
            .map_err(to_rpc_err)
    }

    async fn sign_and_submit(&self, tx: TxRequest) -> ChainResult<TxHash> {
        let mut req = tx_request_to_alloy(&tx)?;
        req = req.with_from(self.address);
        req = req.with_chain_id(self.chain_id);
        let pending = self
            .provider
            .send_transaction(req)
            .await
            .map_err(to_rpc_err)?;
        Ok(TxHash::new(format!("{:#x}", *pending.tx_hash())))
    }

    async fn wait_for_receipt(&self, tx: &TxHash, timeout_ms: u64) -> ChainResult<Receipt> {
        let hash = parse_hex_b256(tx.as_str())?;
        let deadline = Duration::from_millis(timeout_ms);
        let start = std::time::Instant::now();
        loop {
            if let Some(r) = self
                .provider
                .get_transaction_receipt(hash)
                .await
                .map_err(to_rpc_err)?
            {
                let logs = r
                    .inner
                    .logs()
                    .iter()
                    .map(|l| LogEntry {
                        address: format!("{:#x}", l.inner.address),
                        topics: l
                            .inner
                            .data
                            .topics()
                            .iter()
                            .map(|t| format!("{t:#x}"))
                            .collect(),
                        data: l.inner.data.data.to_vec(),
                    })
                    .collect();
                return Ok(Receipt {
                    tx_hash: tx.clone(),
                    status: r.status(),
                    block_number: r.block_number.unwrap_or(0),
                    gas_used: r.gas_used,
                    logs,
                });
            }
            if start.elapsed() >= deadline {
                return Err(ChainError::Timeout(format!(
                    "receipt for {tx} after {timeout_ms}ms"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

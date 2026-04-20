//! Chain tx submitter abstraction. Concrete alloy impl lands with the
//! `roko-chain` close-out; tests use [`MockSubmitter`].

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::types::Transaction;

/// Result of submitting a transaction. `status = 1` means success.
#[derive(Debug, Clone)]
pub struct TxReceipt {
    pub tx_hash: String,
    pub status: u8,
    pub gas_used: u64,
}

/// Submits a signed transaction and waits for a receipt.
#[async_trait]
pub trait ChainTxSubmitter: Send + Sync {
    async fn submit(&self, tx: &Transaction) -> Result<TxReceipt, String>;
}

/// In-memory submitter for tests. Captures every tx and returns a canned
/// receipt.
#[derive(Debug, Default, Clone)]
pub struct MockSubmitter {
    pub txs: Arc<Mutex<Vec<Transaction>>>,
    pub next_status: u8,
    pub next_gas: u64,
}

impl MockSubmitter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            txs: Arc::new(Mutex::new(vec![])),
            next_status: 1,
            next_gas: 100_000,
        }
    }

    #[must_use]
    pub fn recorded(&self) -> Vec<Transaction> {
        self.txs.lock().expect("mock submitter mutex").clone()
    }
}

#[async_trait]
impl ChainTxSubmitter for MockSubmitter {
    async fn submit(&self, tx: &Transaction) -> Result<TxReceipt, String> {
        self.txs.lock().expect("mock submitter mutex").push(tx.clone());
        Ok(TxReceipt {
            tx_hash: format!("0x{:064x}", self.txs.lock().expect("mock submitter mutex").len()),
            status: self.next_status,
            gas_used: self.next_gas,
        })
    }
}

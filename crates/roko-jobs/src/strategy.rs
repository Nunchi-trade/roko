//! Keeper / operator strategy trait. Ports `KeeperStrategy` from
//! `cli/jobs/strategy_interfaces.py`.

use crate::types::{ChainEvent, KeeperContext, Transaction};

/// A keeper / operator strategy — event-driven, stateless per event.
pub trait KeeperStrategy: Send + Sync {
    /// Evaluate a chain event and return transactions to submit.
    fn should_execute(&self, event: &ChainEvent, context: &KeeperContext) -> Vec<Transaction>;

    /// Optional: called after a tx settles (or fails). Default = no-op.
    fn on_execution_result(&self, _tx_hash: &str, _success: bool, _gas_used: u64) {}
}

//! `CustodyGate` — validates that a pending on-chain tx respects a
//! [`CustodyPolicy`]. Wraps [`roko_custody::CustodyGuard`] so trading
//! strategies can run through roko's standard verify → persist → policy
//! pipeline.
//!
//! Input signal body is expected to be JSON-deserialisable as a
//! [`roko_custody::Transaction`]. See `plans/P08-trading-surface.md` T11.

use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, Verdict};
use roko_custody::{CustodyGuard, CustodyPolicy, Transaction};
use std::sync::Mutex;

pub struct CustodyGate {
    name: String,
    guard: Mutex<CustodyGuard>,
}

impl CustodyGate {
    #[must_use]
    pub fn new(policy: CustodyPolicy) -> Self {
        Self {
            name: "custody".into(),
            guard: Mutex::new(CustodyGuard::new(policy)),
        }
    }

    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Reset the per-block rate-limit counter. Call at the start of each new
    /// block — the conductor can wire this to a block-watcher signal.
    pub fn reset_rate_limit(&self) {
        if let Ok(mut g) = self.guard.lock() {
            g.reset_rate_limit();
        }
    }
}

#[async_trait]
impl Gate for CustodyGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let tx: Transaction = match signal.body.as_json() {
            Ok(tx) => tx,
            Err(err) => {
                return Verdict::fail(&self.name, format!("signal body not a Transaction: {err}"));
            }
        };
        let mut guard = match self.guard.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.validate(&tx) {
            Ok(()) => Verdict::pass(&self.name),
            Err(err) => Verdict::fail(&self.name, err.to_string()),
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn tx_signal(to: &str, data: &str, value_wei: u128) -> Engram {
        let tx = Transaction {
            to: to.into(),
            data: data.into(),
            value_wei,
            gas_limit: None,
            chain_id: None,
        };
        Engram::builder(Kind::Task)
            .body(Body::from_json(&tx).expect("serialise tx"))
            .build()
    }

    #[tokio::test]
    async fn passes_when_policy_permits() {
        let gate = CustodyGate::new(CustodyPolicy {
            destinations: vec!["0xaa".into()],
            rate_limit_per_block: 10,
            ..CustodyPolicy::default()
        });
        let v = gate.verify(&tx_signal("0xaa", "0x", 0), &Context::now()).await;
        assert!(v.passed);
    }

    #[tokio::test]
    async fn fails_when_destination_unknown() {
        let gate = CustodyGate::new(CustodyPolicy {
            destinations: vec!["0xaa".into()],
            rate_limit_per_block: 10,
            ..CustodyPolicy::default()
        });
        let v = gate.verify(&tx_signal("0xbb", "0x", 0), &Context::now()).await;
        assert!(!v.passed);
        assert!(v.reason.contains("destination"));
    }
}

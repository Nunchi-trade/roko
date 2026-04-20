//! Custody policy + pre-signing guard. Ports
//! `offchainservices-agent/cli/jobs/custody.py` and the §Custody section of
//! `contracts-core/docs/agent_cli_jobs_spec.tex`.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// On-chain custody constraints enforced by `JobRegistry` + [`CustodyGuard`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustodyPolicy {
    pub destinations: Vec<String>,
    pub selectors: Vec<String>,
    pub value_cap_eth: f64,
    pub rate_limit_per_block: u32,
}

impl Default for CustodyPolicy {
    fn default() -> Self {
        Self {
            destinations: vec![],
            selectors: vec![],
            value_cap_eth: 0.0,
            rate_limit_per_block: u32::MAX,
        }
    }
}

/// A transaction the guard must validate. Matches
/// `cli/jobs/strategy_interfaces.Transaction`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub to: String,
    pub data: String,
    pub value_wei: u128,
    pub gas_limit: Option<u64>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Error)]
pub enum CustodyViolation {
    #[error("destination {to} not in allowed set: {destinations:?}")]
    Destination {
        to: String,
        destinations: Vec<String>,
    },
    #[error("selector {selector} not in allowed set: {selectors:?}")]
    Selector {
        selector: String,
        selectors: Vec<String>,
    },
    #[error("value {value_wei} wei exceeds cap {cap_wei} wei ({cap_eth} ETH)")]
    ValueCap {
        value_wei: u128,
        cap_wei: u128,
        cap_eth: f64,
    },
    #[error("rate limit exceeded: {count}/{limit} txs this block")]
    RateLimit { count: u32, limit: u32 },
}

/// Pre-signing custody policy enforcement.
pub struct CustodyGuard {
    policy: CustodyPolicy,
    tx_count_this_block: u32,
}

impl CustodyGuard {
    #[must_use]
    pub fn new(policy: CustodyPolicy) -> Self {
        Self {
            policy,
            tx_count_this_block: 0,
        }
    }

    /// Validate a transaction against the active policy.
    pub fn validate(&mut self, tx: &Transaction) -> Result<(), CustodyViolation> {
        if !self.policy.destinations.is_empty() {
            let to_lower = tx.to.to_lowercase();
            if !self
                .policy
                .destinations
                .iter()
                .any(|d| d.to_lowercase() == to_lower)
            {
                return Err(CustodyViolation::Destination {
                    to: tx.to.clone(),
                    destinations: self.policy.destinations.clone(),
                });
            }
        }

        if !self.policy.selectors.is_empty() && tx.data.len() >= 10 {
            let selector = tx.data[..10].to_lowercase();
            if !self.policy.selectors.iter().any(|s| s.to_lowercase() == selector) {
                return Err(CustodyViolation::Selector {
                    selector,
                    selectors: self.policy.selectors.clone(),
                });
            }
        }

        if self.policy.value_cap_eth > 0.0 {
            let cap_wei = (self.policy.value_cap_eth * 1e18) as u128;
            if tx.value_wei > cap_wei {
                return Err(CustodyViolation::ValueCap {
                    value_wei: tx.value_wei,
                    cap_wei,
                    cap_eth: self.policy.value_cap_eth,
                });
            }
        }

        if self.tx_count_this_block >= self.policy.rate_limit_per_block {
            return Err(CustodyViolation::RateLimit {
                count: self.tx_count_this_block,
                limit: self.policy.rate_limit_per_block,
            });
        }

        self.tx_count_this_block += 1;
        Ok(())
    }

    pub fn reset_rate_limit(&mut self) {
        self.tx_count_this_block = 0;
    }

    #[must_use]
    pub const fn policy(&self) -> &CustodyPolicy {
        &self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_tx(to: &str, data: &str, value_wei: u128) -> Transaction {
        Transaction {
            to: to.into(),
            data: data.into(),
            value_wei,
            gas_limit: None,
            chain_id: None,
        }
    }

    #[test]
    fn destination_must_be_whitelisted() {
        let mut g = CustodyGuard::new(CustodyPolicy {
            destinations: vec!["0xaa".into()],
            rate_limit_per_block: 10,
            ..CustodyPolicy::default()
        });
        let tx = mk_tx("0xbb", "0x", 0);
        assert!(matches!(g.validate(&tx), Err(CustodyViolation::Destination { .. })));
    }

    #[test]
    fn selector_must_be_whitelisted() {
        let mut g = CustodyGuard::new(CustodyPolicy {
            selectors: vec!["0xdeadbeef".into()],
            rate_limit_per_block: 10,
            ..CustodyPolicy::default()
        });
        let tx = mk_tx("0xaa", "0x11223344aa", 0);
        assert!(matches!(g.validate(&tx), Err(CustodyViolation::Selector { .. })));
    }

    #[test]
    fn value_cap_enforced() {
        let mut g = CustodyGuard::new(CustodyPolicy {
            value_cap_eth: 1.0,
            rate_limit_per_block: 10,
            ..CustodyPolicy::default()
        });
        let tx = mk_tx("0xaa", "0x", 2_000_000_000_000_000_000); // 2 ETH
        assert!(matches!(g.validate(&tx), Err(CustodyViolation::ValueCap { .. })));
    }

    #[test]
    fn rate_limit_counts_per_block() {
        let mut g = CustodyGuard::new(CustodyPolicy {
            rate_limit_per_block: 2,
            ..CustodyPolicy::default()
        });
        let tx = mk_tx("0xaa", "0x", 0);
        assert!(g.validate(&tx).is_ok());
        assert!(g.validate(&tx).is_ok());
        assert!(matches!(g.validate(&tx), Err(CustodyViolation::RateLimit { .. })));
        g.reset_rate_limit();
        assert!(g.validate(&tx).is_ok());
    }
}

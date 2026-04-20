//! `TradingAgent` — composes roko-venue + roko-strategy + roko-custody.
//!
//! One `step()` = one strategy tick against a live market snapshot. The
//! agent serialises strategy decisions through the venue adapter and hands
//! back a summary of what happened.
//!
//! See `plans/P08-trading-surface.md` T10. A `roko_agent::Agent` impl that
//! wraps this lands when the event-to-Engram encoding is settled.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

use std::sync::Arc;

use roko_custody::{CustodyGuard, CustodyPolicy, CustodyViolation};
use roko_strategy::{Strategy, StrategyContext, StrategyDecision};
use roko_venue::{Fill, MarketSnapshot, VenueAdapter, VenueError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentStepError {
    #[error("venue error: {0}")]
    Venue(#[from] VenueError),
    #[error("custody violation: {0}")]
    Custody(String),
}

/// Summary of a single trading-agent tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickOutcome {
    pub decisions_considered: usize,
    pub decisions_accepted: usize,
    pub decisions_rejected_by_custody: usize,
    pub fills: Vec<Fill>,
    pub custody_violations: Vec<String>,
}

pub struct TradingAgent {
    name: String,
    strategy: Box<dyn Strategy>,
    venue: Arc<dyn VenueAdapter>,
    custody: CustodyGuard,
}

impl TradingAgent {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        strategy: Box<dyn Strategy>,
        venue: Arc<dyn VenueAdapter>,
        custody_policy: CustodyPolicy,
    ) -> Self {
        Self {
            name: name.into(),
            strategy,
            venue,
            custody: CustodyGuard::new(custody_policy),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Run one strategy tick against the snapshot, forwarding each approved
    /// decision to the venue adapter. Custody violations are recorded but
    /// don't abort the tick — the venue/on-chain layers are the ultimate
    /// authority.
    pub async fn step(
        &mut self,
        snapshot: &MarketSnapshot,
        context: &StrategyContext,
    ) -> Result<TickOutcome, AgentStepError> {
        let decisions = self.strategy.on_tick(snapshot, context);
        let mut outcome = TickOutcome {
            decisions_considered: decisions.len(),
            ..TickOutcome::default()
        };

        self.custody.reset_rate_limit();

        for decision in decisions {
            if decision.action != "place_order" {
                outcome.decisions_accepted += 1;
                continue;
            }

            let pseudo_tx = custody_tx_for(&decision);
            if let Err(err) = self.custody.validate(&pseudo_tx) {
                outcome.decisions_rejected_by_custody += 1;
                outcome.custody_violations.push(format_violation(&err));
                continue;
            }

            let instrument = roko_venue::Instrument::new(decision.instrument.clone());
            match self
                .venue
                .place_order(
                    &instrument,
                    decision.side,
                    decision.size,
                    decision.limit_price,
                    decision.order_type.into(),
                    None,
                )
                .await
            {
                Ok(Some(fill)) => {
                    outcome.fills.push(fill);
                    outcome.decisions_accepted += 1;
                }
                Ok(None) => {
                    outcome.decisions_accepted += 1;
                }
                Err(err) => return Err(AgentStepError::Venue(err)),
            }
        }

        Ok(outcome)
    }

    #[must_use]
    pub fn venue(&self) -> &Arc<dyn VenueAdapter> {
        &self.venue
    }
}

fn custody_tx_for(decision: &StrategyDecision) -> roko_custody::Transaction {
    roko_custody::Transaction {
        to: decision.instrument.clone(),
        data: String::new(),
        value_wei: 0,
        gas_limit: None,
        chain_id: None,
    }
}

fn format_violation(err: &CustodyViolation) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_strategy::SimpleMMStrategy;
    use roko_venue::mock::MockVenue;

    #[tokio::test]
    async fn agent_forwards_decisions_to_venue() {
        let venue: Arc<dyn VenueAdapter> = Arc::new(MockVenue::with_mid(2500.0));
        let mut agent = TradingAgent::new(
            "simple-mm-test",
            Box::new(SimpleMMStrategy::new(10.0, 0.1)),
            venue,
            CustodyPolicy::default(),
        );
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            bid: 2499.5,
            ask: 2500.5,
            ..Default::default()
        };
        let outcome = agent.step(&snap, &StrategyContext::default()).await.unwrap();
        assert_eq!(outcome.decisions_considered, 2);
        assert_eq!(outcome.decisions_accepted, 2);
        assert_eq!(outcome.fills.len(), 2);
    }

    #[tokio::test]
    async fn rate_limit_rejects_extra_orders() {
        let venue: Arc<dyn VenueAdapter> = Arc::new(MockVenue::with_mid(2500.0));
        let mut agent = TradingAgent::new(
            "rl-test",
            Box::new(SimpleMMStrategy::new(10.0, 0.1)),
            venue,
            CustodyPolicy {
                rate_limit_per_block: 1,
                ..CustodyPolicy::default()
            },
        );
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            bid: 2499.5,
            ask: 2500.5,
            ..Default::default()
        };
        let outcome = agent.step(&snap, &StrategyContext::default()).await.unwrap();
        assert_eq!(outcome.decisions_considered, 2);
        assert_eq!(outcome.decisions_accepted, 1);
        assert_eq!(outcome.decisions_rejected_by_custody, 1);
    }
}

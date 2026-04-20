//! `RiskLimitGate` — enforces portfolio-level risk limits using
//! [`roko_execution::PortfolioRiskManager`]. Signal body must be a JSON
//! object with the fields expected by `PortfolioRiskManager::assess`.
//!
//! See `plans/P08-trading-surface.md` T11.

use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, Verdict};
use roko_execution::{PortfolioRiskConfig, PortfolioRiskManager, PositionSummary};
use serde::Deserialize;
use std::collections::HashMap;

pub struct RiskLimitGate {
    name: String,
    manager: PortfolioRiskManager,
}

impl RiskLimitGate {
    #[must_use]
    pub fn new(config: PortfolioRiskConfig) -> Self {
        Self {
            name: "risk_limit".into(),
            manager: PortfolioRiskManager::new(config),
        }
    }
}

#[derive(Deserialize)]
struct RiskPayload {
    positions: HashMap<String, PositionSummary>,
    #[serde(default)]
    account_value: Option<f64>,
    #[serde(default)]
    total_margin: Option<f64>,
}

#[async_trait]
impl Gate for RiskLimitGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let payload: RiskPayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(err) => {
                return Verdict::fail(&self.name, format!("signal body not a RiskPayload: {err}"));
            }
        };
        let state = self
            .manager
            .assess(payload.positions, payload.account_value, payload.total_margin);
        if state.blocked {
            return Verdict::fail(&self.name, state.block_reason);
        }
        if !state.warnings.is_empty() {
            // Warnings don't fail the gate but surface on the verdict.
            return Verdict::pass(&self.name).with_detail(state.warnings.join("\n"));
        }
        Verdict::pass(&self.name)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use serde_json::json;

    fn build_signal(body: serde_json::Value) -> Engram {
        Engram::builder(Kind::Task)
            .body(Body::from_json(&body).expect("serialize body"))
            .build()
    }

    #[tokio::test]
    async fn blocked_when_margin_exceeds_limit() {
        let gate = RiskLimitGate::new(PortfolioRiskConfig::default());
        let body = json!({
            "positions": { "ETH-PERP": { "direction": "long", "notional": 1000.0 } },
            "account_value": 1000.0,
            "total_margin": 950.0,
        });
        let v = gate.verify(&build_signal(body), &Context::now()).await;
        assert!(!v.passed);
    }

    #[tokio::test]
    async fn passes_when_well_below_limits() {
        let gate = RiskLimitGate::new(PortfolioRiskConfig::default());
        let body = json!({
            "positions": { "ETH-PERP": { "direction": "long", "notional": 1000.0 } },
            "account_value": 10_000.0,
            "total_margin": 200.0,
        });
        let v = gate.verify(&build_signal(body), &Context::now()).await;
        assert!(v.passed);
    }
}

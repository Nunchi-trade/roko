//! `HedgeAgent` — deterministic inventory hedger.
//!
//! Port of `strategies/hedge_agent.py`.

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct HedgeAgent {
    id: String,
    pub inventory_threshold: f64,
    pub urgency_factor: f64,
    pub max_hedge_size: f64,
    pub slippage_bps: f64,
}

impl HedgeAgent {
    #[must_use]
    pub fn new(inventory_threshold: f64, urgency_factor: f64, max_hedge_size: f64, slippage_bps: f64) -> Self {
        Self {
            id: "hedge_agent".into(),
            inventory_threshold,
            urgency_factor,
            max_hedge_size,
            slippage_bps,
        }
    }
}

impl Default for HedgeAgent {
    fn default() -> Self {
        Self::new(3.0, 0.5, 5.0, 10.0)
    }
}

impl Strategy for HedgeAgent {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snap: &MarketSnapshot, ctx: &StrategyContext) -> Vec<StrategyDecision> {
        if snap.mid_price <= 0.0 {
            return vec![];
        }
        let q = ctx.position_qty;
        if q.abs() <= self.inventory_threshold {
            return vec![];
        }
        let excess = q.abs() - self.inventory_threshold;
        let mut hedge_size = (excess * self.urgency_factor).min(self.max_hedge_size);
        hedge_size = round_to(hedge_size, 6);
        if hedge_size <= 0.001 {
            return vec![];
        }

        let slip = snap.mid_price * (self.slippage_bps / 10_000.0);
        let (side, price, signal) = if q > 0.0 {
            (OrderSide::Sell, round_to(snap.mid_price - slip, 2), "hedge_sell")
        } else {
            (OrderSide::Buy, round_to(snap.mid_price + slip, 2), "hedge_buy")
        };

        vec![StrategyDecision::place(snap.instrument.clone(), side, hedge_size, price)
            .with_order_type(StrategyOrderType::Ioc)
            .with_meta("signal", signal)
            .with_meta("inventory", q)
            .with_meta("excess", round_to(excess, 4))
            .with_meta("urgency", self.urgency_factor)]
    }
}

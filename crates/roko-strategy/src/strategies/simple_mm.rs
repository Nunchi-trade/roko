//! `SimpleMMStrategy` — symmetric bid/ask quoting around mid.
//!
//! Port of `strategies/simple_mm.py`.

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision};

#[derive(Debug, Clone)]
pub struct SimpleMMStrategy {
    id: String,
    pub spread_bps: f64,
    pub size: f64,
}

impl SimpleMMStrategy {
    #[must_use]
    pub fn new(spread_bps: f64, size: f64) -> Self {
        Self {
            id: "simple_mm".into(),
            spread_bps,
            size,
        }
    }
}

impl Default for SimpleMMStrategy {
    fn default() -> Self {
        Self::new(10.0, 1.0)
    }
}

impl Strategy for SimpleMMStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snapshot: &MarketSnapshot, _ctx: &StrategyContext) -> Vec<StrategyDecision> {
        if snapshot.mid_price <= 0.0 {
            return vec![];
        }
        let half_spread = snapshot.mid_price * (self.spread_bps / 10_000.0) / 2.0;
        let bid = round_to(snapshot.mid_price - half_spread, 2);
        let ask = round_to(snapshot.mid_price + half_spread, 2);
        vec![
            StrategyDecision::place(snapshot.instrument.clone(), OrderSide::Buy, self.size, bid),
            StrategyDecision::place(snapshot.instrument.clone(), OrderSide::Sell, self.size, ask),
        ]
    }
}

#[inline]
pub(crate) fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_mid_returns_nothing() {
        let mut s = SimpleMMStrategy::default();
        let snap = MarketSnapshot::default();
        assert!(s.on_tick(&snap, &StrategyContext::default()).is_empty());
    }

    #[test]
    fn places_symmetric_pair() {
        let mut s = SimpleMMStrategy::new(10.0, 1.0);
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            ..Default::default()
        };
        let ds = s.on_tick(&snap, &StrategyContext::default());
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].side, OrderSide::Buy);
        assert_eq!(ds[1].side, OrderSide::Sell);
        // half_spread = 2500 * 10 / 10_000 / 2 = 1.25
        assert!((ds[0].limit_price - 2498.75).abs() < 1e-6);
        assert!((ds[1].limit_price - 2501.25).abs() < 1e-6);
    }
}

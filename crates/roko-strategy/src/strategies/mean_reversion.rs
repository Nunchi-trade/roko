//! `MeanReversionStrategy` — trade when price deviates from SMA.
//!
//! Port of `strategies/mean_reversion.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct MeanReversionStrategy {
    id: String,
    pub window: usize,
    pub threshold_bps: f64,
    pub size: f64,
    prices: VecDeque<f64>,
}

impl MeanReversionStrategy {
    #[must_use]
    pub fn new(window: usize, threshold_bps: f64, size: f64) -> Self {
        Self {
            id: "mean_reversion".into(),
            window,
            threshold_bps,
            size,
            prices: VecDeque::with_capacity(window),
        }
    }
}

impl Default for MeanReversionStrategy {
    fn default() -> Self {
        Self::new(20, 30.0, 1.0)
    }
}

impl Strategy for MeanReversionStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snapshot: &MarketSnapshot, _ctx: &StrategyContext) -> Vec<StrategyDecision> {
        if self.prices.len() == self.window {
            self.prices.pop_front();
        }
        self.prices.push_back(snapshot.mid_price);

        if self.prices.len() < self.window {
            return vec![];
        }

        let sum: f64 = self.prices.iter().sum();
        let sma = sum / self.prices.len() as f64;
        if sma <= 0.0 {
            return vec![];
        }
        let deviation_bps = (snapshot.mid_price - sma) / sma * 10_000.0;

        let (side, signal) = if deviation_bps > self.threshold_bps {
            (OrderSide::Sell, "overbought")
        } else if deviation_bps < -self.threshold_bps {
            (OrderSide::Buy, "oversold")
        } else {
            return vec![];
        };

        vec![StrategyDecision::place(
            snapshot.instrument.clone(),
            side,
            self.size,
            round_to(snapshot.mid_price, 2),
        )
        .with_order_type(StrategyOrderType::Ioc)
        .with_meta("signal", signal)
        .with_meta("deviation_bps", round_to(deviation_bps, 2))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(mid: f64) -> MarketSnapshot {
        MarketSnapshot { instrument: "ETH-PERP".into(), mid_price: mid, ..Default::default() }
    }

    #[test]
    fn waits_for_window() {
        let mut s = MeanReversionStrategy::new(5, 10.0, 1.0);
        let ctx = StrategyContext::default();
        for _ in 0..4 {
            assert!(s.on_tick(&snap(100.0), &ctx).is_empty());
        }
        assert!(s.on_tick(&snap(100.0), &ctx).is_empty()); // at window, deviation=0
    }

    #[test]
    fn sells_when_overbought() {
        let mut s = MeanReversionStrategy::new(5, 50.0, 1.0);
        let ctx = StrategyContext::default();
        for _ in 0..5 {
            s.on_tick(&snap(100.0), &ctx);
        }
        let ds = s.on_tick(&snap(102.0), &ctx); // +200bps vs SMA ≈ 100.4
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].side, OrderSide::Sell);
    }
}

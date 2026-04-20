//! `AvellanedaStoikovStrategy` — inventory-aware market maker.
//!
//! Port of `strategies/avellaneda_mm.py` (the `toxicity_scorer` hook from
//! the Python version is omitted — the roko equivalent will bind a
//! `roko-gate::CustodyGate` / anomaly gate in a follow-up).

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct AvellanedaStoikovStrategy {
    id: String,
    pub gamma: f64,
    pub k: f64,
    pub base_size: f64,
    pub max_inventory: f64,
    pub min_spread_bps: f64,
    pub max_spread_bps: f64,
    pub vol_window: usize,
    prices: VecDeque<f64>,
    log_returns: VecDeque<f64>,
}

impl AvellanedaStoikovStrategy {
    #[must_use]
    pub fn new(
        gamma: f64,
        k: f64,
        base_size: f64,
        max_inventory: f64,
        min_spread_bps: f64,
        max_spread_bps: f64,
        vol_window: usize,
    ) -> Self {
        Self {
            id: "avellaneda_mm".into(),
            gamma,
            k,
            base_size,
            max_inventory,
            min_spread_bps,
            max_spread_bps,
            vol_window,
            prices: VecDeque::with_capacity(vol_window),
            log_returns: VecDeque::with_capacity(vol_window),
        }
    }

    fn update_vol(&mut self, mid: f64) -> f64 {
        if let Some(&prev) = self.prices.back() {
            if prev > 0.0 && mid > 0.0 {
                if self.log_returns.len() == self.vol_window {
                    self.log_returns.pop_front();
                }
                self.log_returns.push_back((mid / prev).ln());
            }
        }
        if self.prices.len() == self.vol_window {
            self.prices.pop_front();
        }
        self.prices.push_back(mid);

        if self.log_returns.len() < 3 {
            return mid * (self.min_spread_bps / 10_000.0);
        }

        let n = self.log_returns.len() as f64;
        let mean = self.log_returns.iter().sum::<f64>() / n;
        let var = self.log_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        var.max(1e-12).sqrt() * mid
    }

    fn reservation_price(&self, mid: f64, q: f64, sigma: f64) -> f64 {
        let t = 1.0_f64;
        mid - q * self.gamma * sigma.powi(2) * t
    }

    fn optimal_spread(&self, sigma: f64) -> f64 {
        let t = 1.0_f64;
        let mut spread = self.gamma * sigma.powi(2) * t;
        if self.gamma > 0.0 {
            spread += (2.0 / self.gamma) * (1.0 + self.gamma / self.k).ln();
        }
        spread
    }

    fn clamp_spread(&self, spread: f64, mid: f64) -> f64 {
        let min_s = mid * (self.min_spread_bps / 10_000.0);
        let max_s = mid * (self.max_spread_bps / 10_000.0);
        spread.max(min_s).min(max_s)
    }

    fn scale_size(&self, q: f64) -> f64 {
        if self.max_inventory <= 0.0 {
            return self.base_size;
        }
        let utilization = q.abs() / self.max_inventory;
        let scale = (1.0 - utilization).max(0.1);
        round_to(self.base_size * scale, 6)
    }
}

impl Default for AvellanedaStoikovStrategy {
    fn default() -> Self {
        Self::new(0.1, 1.5, 1.0, 10.0, 5.0, 200.0, 30)
    }
}

impl Strategy for AvellanedaStoikovStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snap: &MarketSnapshot, ctx: &StrategyContext) -> Vec<StrategyDecision> {
        let mid = snap.mid_price;
        if mid <= 0.0 {
            return vec![];
        }

        let q = ctx.position_qty;
        let sigma = self.update_vol(mid);
        let r_price = self.reservation_price(mid, q, sigma);
        let raw_spread = self.optimal_spread(sigma);
        let half_spread = self.clamp_spread(raw_spread, mid) / 2.0;
        let bid = round_to(r_price - half_spread, 2);
        let ask = round_to(r_price + half_spread, 2);
        let size = self.scale_size(q);

        if ctx.reduce_only {
            if q > 0.0 {
                return vec![StrategyDecision::place(
                    snap.instrument.clone(),
                    OrderSide::Sell,
                    size.min(q.abs()),
                    ask,
                )
                .with_meta("signal", "reduce_only_sell")
                .with_meta("inventory", q)];
            } else if q < 0.0 {
                return vec![StrategyDecision::place(
                    snap.instrument.clone(),
                    OrderSide::Buy,
                    size.min(q.abs()),
                    bid,
                )
                .with_meta("signal", "reduce_only_buy")
                .with_meta("inventory", q)];
            } else {
                return vec![];
            }
        }

        vec![
            StrategyDecision::place(snap.instrument.clone(), OrderSide::Buy, size, bid)
                .with_meta("signal", "as_bid")
                .with_meta("reservation_price", round_to(r_price, 2))
                .with_meta("spread", round_to(raw_spread, 4))
                .with_meta("sigma", round_to(sigma, 4))
                .with_meta("inventory", q),
            StrategyDecision::place(snap.instrument.clone(), OrderSide::Sell, size, ask)
                .with_meta("signal", "as_ask")
                .with_meta("reservation_price", round_to(r_price, 2))
                .with_meta("spread", round_to(raw_spread, 4))
                .with_meta("sigma", round_to(sigma, 4))
                .with_meta("inventory", q),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_symmetric_when_flat() {
        let mut s = AvellanedaStoikovStrategy::default();
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            ..Default::default()
        };
        let ds = s.on_tick(&snap, &StrategyContext::default());
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].side, OrderSide::Buy);
        assert_eq!(ds[1].side, OrderSide::Sell);
    }

    #[test]
    fn reduce_only_when_long_sells_only() {
        let mut s = AvellanedaStoikovStrategy::default();
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            ..Default::default()
        };
        let mut ctx = StrategyContext::default();
        ctx.reduce_only = true;
        ctx.position_qty = 2.0;
        let ds = s.on_tick(&snap, &ctx);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].side, OrderSide::Sell);
    }
}

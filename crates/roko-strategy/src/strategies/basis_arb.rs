//! `BasisArbStrategy` — trade implied basis from funding rate.
//!
//! Port of `strategies/basis_arb.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct BasisArbStrategy {
    id: String,
    pub basis_threshold_bps: f64,
    pub size: f64,
    pub funding_window: usize,
    funding_history: VecDeque<f64>,
}

impl BasisArbStrategy {
    #[must_use]
    pub fn new(basis_threshold_bps: f64, size: f64, funding_window: usize) -> Self {
        Self {
            id: "basis_arb".into(),
            basis_threshold_bps,
            size,
            funding_window,
            funding_history: VecDeque::with_capacity(funding_window),
        }
    }
}

impl Default for BasisArbStrategy {
    fn default() -> Self {
        Self::new(5.0, 1.0, 10)
    }
}

impl Strategy for BasisArbStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snap: &MarketSnapshot, ctx: &StrategyContext) -> Vec<StrategyDecision> {
        if snap.mid_price <= 0.0 {
            return vec![];
        }
        if self.funding_history.len() == self.funding_window {
            self.funding_history.pop_front();
        }
        self.funding_history.push_back(snap.funding_rate);

        if self.funding_history.len() < 3 {
            return vec![];
        }

        let avg_funding = self.funding_history.iter().sum::<f64>() / self.funding_history.len() as f64;
        // Annualize 8h funding: 3× per day × 365 days
        let basis_ann_bps = avg_funding * 365.0 * 3.0 * 10_000.0;

        if basis_ann_bps.abs() < self.basis_threshold_bps {
            return vec![];
        }

        let mut orders = vec![];

        if basis_ann_bps > self.basis_threshold_bps {
            if ctx.position_qty <= 0.0 {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Sell,
                        self.size,
                        round_to(snap.bid, 2),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "short_contango")
                    .with_meta("basis_ann_bps", round_to(basis_ann_bps, 2))
                    .with_meta("avg_funding", round_to(avg_funding, 8)),
                );
            } else {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Sell,
                        ctx.position_qty.abs(),
                        round_to(snap.bid, 2),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "close_wrong_side")
                    .with_meta("basis_ann_bps", round_to(basis_ann_bps, 2)),
                );
            }
        } else if basis_ann_bps < -self.basis_threshold_bps {
            if ctx.position_qty >= 0.0 {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Buy,
                        self.size,
                        round_to(snap.ask, 2),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "long_backwardation")
                    .with_meta("basis_ann_bps", round_to(basis_ann_bps, 2))
                    .with_meta("avg_funding", round_to(avg_funding, 8)),
                );
            } else {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Buy,
                        ctx.position_qty.abs(),
                        round_to(snap.ask, 2),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "close_wrong_side")
                    .with_meta("basis_ann_bps", round_to(basis_ann_bps, 2)),
                );
            }
        }
        orders
    }
}

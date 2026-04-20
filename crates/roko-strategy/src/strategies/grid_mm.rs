//! `GridMMStrategy` — fixed-interval grid levels above and below mid.
//!
//! Port of `strategies/grid_mm.py`.

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct GridMMStrategy {
    id: String,
    pub grid_spacing_bps: f64,
    pub num_levels: u32,
    pub size_per_level: f64,
    pub max_position: f64,
}

impl GridMMStrategy {
    #[must_use]
    pub fn new(grid_spacing_bps: f64, num_levels: u32, size_per_level: f64, max_position: f64) -> Self {
        Self {
            id: "grid_mm".into(),
            grid_spacing_bps,
            num_levels,
            size_per_level,
            max_position,
        }
    }
}

impl Default for GridMMStrategy {
    fn default() -> Self {
        Self::new(10.0, 5, 0.5, 5.0)
    }
}

impl Strategy for GridMMStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snapshot: &MarketSnapshot, ctx: &StrategyContext) -> Vec<StrategyDecision> {
        let mid = snapshot.mid_price;
        if mid <= 0.0 {
            return vec![];
        }

        let mut orders = vec![];

        if ctx.reduce_only && ctx.position_qty != 0.0 {
            let close_side = if ctx.position_qty > 0.0 { OrderSide::Sell } else { OrderSide::Buy };
            let close_price = if close_side == OrderSide::Sell { snapshot.bid } else { snapshot.ask };
            orders.push(
                StrategyDecision::place(
                    snapshot.instrument.clone(),
                    close_side,
                    ctx.position_qty.abs(),
                    round_to(close_price, 2),
                )
                .with_meta("signal", "reduce_only_close"),
            );
            return orders;
        }

        let spacing = mid * self.grid_spacing_bps / 10_000.0;

        for i in 1..=self.num_levels {
            let i_f = f64::from(i);
            let bid_price = mid - spacing * i_f;
            let ask_price = mid + spacing * i_f;

            if ctx.position_qty + self.size_per_level <= self.max_position {
                orders.push(
                    StrategyDecision::place(
                        snapshot.instrument.clone(),
                        OrderSide::Buy,
                        self.size_per_level,
                        round_to(bid_price, 2),
                    )
                    .with_meta("signal", "grid_bid")
                    .with_meta("level", i),
                );
            }

            if ctx.position_qty - self.size_per_level >= -self.max_position {
                orders.push(
                    StrategyDecision::place(
                        snapshot.instrument.clone(),
                        OrderSide::Sell,
                        self.size_per_level,
                        round_to(ask_price, 2),
                    )
                    .with_meta("signal", "grid_ask")
                    .with_meta("level", i),
                );
            }
        }

        orders
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_ladder() {
        let mut s = GridMMStrategy::new(10.0, 3, 0.1, 10.0);
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            bid: 2499.5,
            ask: 2500.5,
            ..Default::default()
        };
        let ds = s.on_tick(&snap, &StrategyContext::default());
        // 3 levels × 2 sides = 6 orders when no position caps hit
        assert_eq!(ds.len(), 6);
    }

    #[test]
    fn reduce_only_closes_position() {
        let mut s = GridMMStrategy::default();
        let snap = MarketSnapshot {
            instrument: "ETH-PERP".into(),
            mid_price: 2500.0,
            bid: 2499.0,
            ask: 2501.0,
            ..Default::default()
        };
        let mut ctx = StrategyContext::default();
        ctx.reduce_only = true;
        ctx.position_qty = 0.5;
        let ds = s.on_tick(&snap, &ctx);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].side, OrderSide::Sell);
        assert!((ds[0].size - 0.5).abs() < 1e-9);
    }
}

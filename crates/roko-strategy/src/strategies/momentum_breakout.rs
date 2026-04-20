//! `MomentumBreakoutStrategy` — enter on volume + price breakout above/below
//! an N-period range with a simple trailing stop.
//!
//! Port of `strategies/momentum_breakout.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct MomentumBreakoutStrategy {
    id: String,
    pub lookback: usize,
    pub breakout_threshold_bps: f64,
    pub volume_surge_mult: f64,
    pub trailing_stop_bps: f64,
    pub size: f64,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    volumes: VecDeque<f64>,
}

impl MomentumBreakoutStrategy {
    #[must_use]
    pub fn new(
        lookback: usize,
        breakout_threshold_bps: f64,
        volume_surge_mult: f64,
        trailing_stop_bps: f64,
        size: f64,
    ) -> Self {
        Self {
            id: "momentum_breakout".into(),
            lookback,
            breakout_threshold_bps,
            volume_surge_mult,
            trailing_stop_bps,
            size,
            highs: VecDeque::with_capacity(lookback),
            lows: VecDeque::with_capacity(lookback),
            volumes: VecDeque::with_capacity(lookback),
        }
    }
}

impl Default for MomentumBreakoutStrategy {
    fn default() -> Self {
        Self::new(20, 50.0, 2.0, 30.0, 1.0)
    }
}

fn push_bounded(buf: &mut VecDeque<f64>, val: f64, cap: usize) {
    if buf.len() == cap {
        buf.pop_front();
    }
    buf.push_back(val);
}

impl Strategy for MomentumBreakoutStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snap: &MarketSnapshot, ctx: &StrategyContext) -> Vec<StrategyDecision> {
        let mid = snap.mid_price;
        if mid <= 0.0 {
            return vec![];
        }

        let high = if snap.ask > 0.0 { snap.ask } else { mid };
        let low = if snap.bid > 0.0 { snap.bid } else { mid };
        let vol = if snap.volume_24h > 0.0 { snap.volume_24h } else { snap.open_interest };

        if self.highs.len() < self.lookback {
            push_bounded(&mut self.highs, high, self.lookback);
            push_bounded(&mut self.lows, low, self.lookback);
            push_bounded(&mut self.volumes, vol, self.lookback);
            return vec![];
        }

        // Compute range from previous window before adding current tick.
        let period_high = self.highs.iter().copied().fold(f64::MIN, f64::max);
        let period_low = self.lows.iter().copied().fold(f64::MAX, f64::min);
        let avg_vol = if self.volumes.is_empty() {
            1.0
        } else {
            self.volumes.iter().sum::<f64>() / self.volumes.len() as f64
        };

        push_bounded(&mut self.highs, high, self.lookback);
        push_bounded(&mut self.lows, low, self.lookback);
        push_bounded(&mut self.volumes, vol, self.lookback);

        let vol_surge = avg_vol > 0.0 && vol > avg_vol * self.volume_surge_mult;

        let upside_bps = if period_high > 0.0 { (mid - period_high) / period_high * 10_000.0 } else { 0.0 };
        let downside_bps = if period_low > 0.0 { (period_low - mid) / period_low * 10_000.0 } else { 0.0 };

        let mut orders = vec![];

        // Trailing stop when holding a position.
        if ctx.position_qty != 0.0 {
            if ctx.position_qty > 0.0 {
                let stop_price = mid * (1.0 - self.trailing_stop_bps / 10_000.0);
                if snap.bid <= stop_price {
                    orders.push(
                        StrategyDecision::place(
                            snap.instrument.clone(),
                            OrderSide::Sell,
                            ctx.position_qty.abs(),
                            round_to(snap.bid, 2),
                        )
                        .with_order_type(StrategyOrderType::Ioc)
                        .with_meta("signal", "trailing_stop_long")
                        .with_meta("stop_price", round_to(stop_price, 2)),
                    );
                }
            } else {
                let stop_price = mid * (1.0 + self.trailing_stop_bps / 10_000.0);
                if snap.ask >= stop_price {
                    orders.push(
                        StrategyDecision::place(
                            snap.instrument.clone(),
                            OrderSide::Buy,
                            ctx.position_qty.abs(),
                            round_to(snap.ask, 2),
                        )
                        .with_order_type(StrategyOrderType::Ioc)
                        .with_meta("signal", "trailing_stop_short")
                        .with_meta("stop_price", round_to(stop_price, 2)),
                    );
                }
            }
            return orders;
        }

        // Flat — consider breakout entry.
        if upside_bps > self.breakout_threshold_bps && vol_surge {
            orders.push(
                StrategyDecision::place(
                    snap.instrument.clone(),
                    OrderSide::Buy,
                    self.size,
                    round_to(snap.ask, 2),
                )
                .with_order_type(StrategyOrderType::Ioc)
                .with_meta("signal", "breakout_long")
                .with_meta("breakout_bps", round_to(upside_bps, 2))
                .with_meta("volume_surge", true),
            );
        } else if downside_bps > self.breakout_threshold_bps && vol_surge {
            orders.push(
                StrategyDecision::place(
                    snap.instrument.clone(),
                    OrderSide::Sell,
                    self.size,
                    round_to(snap.bid, 2),
                )
                .with_order_type(StrategyOrderType::Ioc)
                .with_meta("signal", "breakout_short")
                .with_meta("breakout_bps", round_to(downside_bps, 2))
                .with_meta("volume_surge", true),
            );
        }

        orders
    }
}

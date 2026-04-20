//! `AggressiveTakerStrategy` — crosses the spread with sinusoidal directional bias.
//!
//! Port of `strategies/aggressive_taker.py`.

use std::f64::consts::PI;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

#[derive(Debug, Clone)]
pub struct AggressiveTakerStrategy {
    id: String,
    pub size: f64,
    pub skip_ticks: u32,
    pub bias_amplitude: f64,
    pub bias_period: u32,
    tick_count: u32,
}

impl AggressiveTakerStrategy {
    #[must_use]
    pub fn new(size: f64, skip_ticks: u32, bias_amplitude: f64, bias_period: u32) -> Self {
        Self {
            id: "aggressive_taker".into(),
            size,
            skip_ticks,
            bias_amplitude,
            bias_period,
            tick_count: 0,
        }
    }
}

impl Default for AggressiveTakerStrategy {
    fn default() -> Self {
        Self::new(2.0, 0, 0.35, 4)
    }
}

impl Strategy for AggressiveTakerStrategy {
    fn strategy_id(&self) -> &str {
        &self.id
    }

    fn on_tick(&mut self, snap: &MarketSnapshot, _ctx: &StrategyContext) -> Vec<StrategyDecision> {
        if snap.mid_price <= 0.0 {
            return vec![];
        }
        self.tick_count += 1;
        if self.skip_ticks > 0 && self.tick_count % (self.skip_ticks + 1) != 0 {
            return vec![];
        }

        let phase = 2.0 * PI * f64::from(self.tick_count) / f64::from(self.bias_period);
        let bias = self.bias_amplitude * phase.sin();
        let buy_frac = 0.5 + bias;
        let sell_frac = 0.5 - bias;
        let buy_size = round_to((self.size * buy_frac).max(0.01), 4);
        let sell_size = round_to((self.size * sell_frac).max(0.01), 4);

        vec![
            StrategyDecision::place(
                snap.instrument.clone(),
                OrderSide::Buy,
                buy_size,
                round_to(snap.ask + 3.0, 2),
            )
            .with_order_type(StrategyOrderType::Ioc)
            .with_meta("signal", "aggressive_buy")
            .with_meta("bias", round_to(bias, 3)),
            StrategyDecision::place(
                snap.instrument.clone(),
                OrderSide::Sell,
                sell_size,
                round_to(snap.bid - 3.0, 2),
            )
            .with_order_type(StrategyOrderType::Ioc)
            .with_meta("signal", "aggressive_sell")
            .with_meta("bias", round_to(bias, 3)),
        ]
    }
}

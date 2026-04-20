//! Multi-level quote ladder. Ports `quoting_engine/ladder.py`.

use crate::config::LadderParams;

/// A single level of the quote ladder.
#[derive(Debug, Clone, Copy)]
pub struct LadderLevel {
    pub level: u32,
    pub bid_price: f64,
    pub bid_size: f64,
    pub ask_price: f64,
    pub ask_size: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct LadderBuilder {
    params: LadderParams,
    tick_size: f64,
}

impl LadderBuilder {
    #[must_use]
    pub const fn new(params: LadderParams, tick_size: f64) -> Self {
        Self { params, tick_size }
    }

    pub fn build(
        &self,
        fv: f64,
        half_spread: f64,
        mid: f64,
        bid_size_mult: f64,
        ask_size_mult: f64,
        num_levels_override: Option<u32>,
    ) -> Vec<LadderLevel> {
        if fv <= 0.0 || mid <= 0.0 {
            return vec![];
        }
        let delta = self.params.delta_bps * mid / 10_000.0;
        let num_levels = num_levels_override.unwrap_or(self.params.num_levels);
        let mut levels = Vec::with_capacity(num_levels as usize);
        for i in 0..num_levels {
            let i_f = f64::from(i);
            let offset = half_spread + i_f * delta;
            let min_size = self.params.min_size_ratio * self.params.s0;
            let base_size = (self.params.s0 * (-self.params.lam * i_f).exp()).max(min_size);
            let bid_price = self.round_to_tick(fv - offset);
            let ask_price = self.round_to_tick(fv + offset);
            let bid_size = round6(base_size * bid_size_mult);
            let ask_size = round6(base_size * ask_size_mult);
            if bid_size <= 0.0 && ask_size <= 0.0 {
                continue;
            }
            levels.push(LadderLevel {
                level: i,
                bid_price,
                bid_size: bid_size.max(0.0),
                ask_price,
                ask_size: ask_size.max(0.0),
            });
        }
        levels
    }

    fn round_to_tick(&self, price: f64) -> f64 {
        if self.tick_size <= 0.0 {
            return round8(price);
        }
        round8((price / self.tick_size).round() * self.tick_size)
    }
}

fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}
fn round8(x: f64) -> f64 {
    (x * 100_000_000.0).round() / 100_000_000.0
}

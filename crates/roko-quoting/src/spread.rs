//! Half-spread calculator. Ports `quoting_engine/spread.py`.

use crate::config::SpreadParams;

#[derive(Debug, Clone, Copy)]
pub struct SpreadCalculator {
    p: SpreadParams,
    tick_size: f64,
}

impl SpreadCalculator {
    #[must_use]
    pub const fn new(p: SpreadParams, tick_size: f64) -> Self {
        Self { p, tick_size }
    }

    pub fn compute(&self, mid: f64, sigma_price: f64, m_vol: f64, m_dd: f64, h_tox: f64, h_event: f64) -> f64 {
        if mid <= 0.0 {
            return 0.0;
        }
        let bps_to_price = mid / 10_000.0;
        let mut h_fee = self.p.h_fee_bps * bps_to_price;
        let mut rebate = self.p.rebate_credit_bps * bps_to_price;
        if self.p.growth_mode {
            h_fee *= self.p.growth_mode_scale;
            rebate *= self.p.growth_mode_scale;
        }
        let h_vol = sigma_price * self.p.vol_scale;
        let raw_sum = h_fee + h_vol + h_tox + h_event;
        let tick_floor = 0.5 * self.tick_size;
        let mut h_raw = raw_sum.max(tick_floor) - rebate;
        if h_raw < tick_floor {
            h_raw = tick_floor;
        }
        let h_amplified = h_raw * m_vol.sqrt() * m_dd;
        let h_min = (self.p.min_spread_bps / 2.0) * bps_to_price;
        let h_max = (self.p.max_spread_bps / 2.0) * bps_to_price;
        h_amplified.max(h_min).min(h_max)
    }
}

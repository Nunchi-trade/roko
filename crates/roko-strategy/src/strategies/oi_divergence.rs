//! `OIDivergenceStrategy` — filter real moves from fakeouts via price/OI correlation.
//!
//! Port of `strategies/oi_divergence.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

const LOOKBACK: usize = 24;
const VOLUME_AVG_WINDOW: usize = 36;
const VOLUME_SURGE_MULT: f64 = 1.3;
const MOM_THRESHOLD: f64 = 0.008;
const OI_CHANGE_THRESHOLD: f64 = 0.005;
const RSI_PERIOD: usize = 14;
const RSI_EXIT_HIGH: f64 = 75.0;
const RSI_EXIT_LOW: f64 = 25.0;
const ATR_LOOKBACK: usize = 24;
const ATR_STOP_MULT: f64 = 4.5;

fn min_history() -> usize {
    LOOKBACK.max(VOLUME_AVG_WINDOW).max(RSI_PERIOD + 1).max(ATR_LOOKBACK) + 1
}

fn rsi(closes: &[f64], period: usize) -> f64 {
    if closes.len() < period + 1 {
        return 50.0;
    }
    let start = closes.len() - period;
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in start..closes.len() {
        let d = closes[i] - closes[i - 1];
        if d > 0.0 {
            gains += d;
        } else {
            losses += -d;
        }
    }
    let avg_gain = gains / period as f64;
    let avg_loss = losses / period as f64;
    if avg_loss < 1e-10 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

fn calc_atr(highs: &[f64], lows: &[f64], closes: &[f64], lookback: usize) -> Option<f64> {
    if closes.len() < lookback + 1 {
        return None;
    }
    let start = closes.len() - lookback;
    let mut trs = Vec::with_capacity(lookback);
    for i in start..closes.len() {
        trs.push(
            (highs[i] - lows[i])
                .max((highs[i] - closes[i - 1]).abs())
                .max((lows[i] - closes[i - 1]).abs()),
        );
    }
    Some(trs.iter().sum::<f64>() / trs.len() as f64)
}

fn push_bounded(buf: &mut VecDeque<f64>, val: f64, cap: usize) {
    if buf.len() == cap {
        buf.pop_front();
    }
    buf.push_back(val);
}

#[derive(Debug, Clone)]
pub struct OIDivergenceStrategy {
    id: String,
    pub size: f64,
    closes: VecDeque<f64>,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    oi_values: VecDeque<f64>,
    volumes: VecDeque<f64>,
    direction: i8,
    entry_price: f64,
    peak_price: f64,
    atr_at_entry: f64,
    entry_oi_direction: i8,
}

impl OIDivergenceStrategy {
    #[must_use]
    pub fn new(size: f64) -> Self {
        let buf_len = min_history() + 5;
        Self {
            id: "oi_divergence".into(),
            size,
            closes: VecDeque::with_capacity(buf_len),
            highs: VecDeque::with_capacity(buf_len),
            lows: VecDeque::with_capacity(buf_len),
            oi_values: VecDeque::with_capacity(buf_len),
            volumes: VecDeque::with_capacity(VOLUME_AVG_WINDOW),
            direction: 0,
            entry_price: 0.0,
            peak_price: 0.0,
            atr_at_entry: 0.0,
            entry_oi_direction: 0,
        }
    }
}

impl Default for OIDivergenceStrategy {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Strategy for OIDivergenceStrategy {
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
        let cap = min_history() + 5;
        push_bounded(&mut self.closes, mid, cap);
        push_bounded(&mut self.highs, high, cap);
        push_bounded(&mut self.lows, low, cap);
        push_bounded(&mut self.oi_values, snap.open_interest, cap);
        push_bounded(&mut self.volumes, snap.volume_24h, VOLUME_AVG_WINDOW);
        if self.closes.len() < min_history() {
            return vec![];
        }

        let closes: Vec<f64> = self.closes.iter().copied().collect();
        let highs: Vec<f64> = self.highs.iter().copied().collect();
        let lows: Vec<f64> = self.lows.iter().copied().collect();
        let ois: Vec<f64> = self.oi_values.iter().copied().collect();
        let vols: Vec<f64> = self.volumes.iter().copied().collect();

        let n = closes.len();
        let base = closes[n - LOOKBACK];
        let price_return = if base > 0.0 { (closes[n - 1] - base) / base } else { 0.0 };
        let price_up = price_return > MOM_THRESHOLD;
        let price_down = price_return < -MOM_THRESHOLD;

        let oi_old = if ois.len() >= LOOKBACK && ois[n - LOOKBACK] > 0.0 { ois[n - LOOKBACK] } else { 0.0 };
        let oi_change = if oi_old > 0.0 { (ois[n - 1] - oi_old) / oi_old } else { 0.0 };
        let oi_up = oi_change > OI_CHANGE_THRESHOLD;
        let oi_down = oi_change < -OI_CHANGE_THRESHOLD;

        let avg_vol = if vols.is_empty() { 1.0 } else { vols.iter().sum::<f64>() / vols.len() as f64 };
        let vol_above_avg = avg_vol > 0.0 && snap.volume_24h > avg_vol * VOLUME_SURGE_MULT;
        let rsi_val = rsi(&closes, RSI_PERIOD);

        if ctx.position_qty > 0.0 {
            self.direction = 1;
        } else if ctx.position_qty < 0.0 {
            self.direction = -1;
        } else if ctx.position_qty == 0.0 {
            self.direction = 0;
        }

        let mut orders = vec![];

        if self.direction == 0 {
            if price_up && oi_up && vol_above_avg {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Buy,
                        self.size,
                        round_to(snap.ask, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "oi_agreement_long")
                    .with_meta("rsi", round_to(rsi_val, 1)),
                );
                self.direction = 1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.entry_oi_direction = 1;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            } else if price_down && oi_down && vol_above_avg {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Sell,
                        self.size,
                        round_to(snap.bid, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "oi_agreement_short")
                    .with_meta("rsi", round_to(rsi_val, 1)),
                );
                self.direction = -1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.entry_oi_direction = -1;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            }
        } else {
            let atr = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(self.atr_at_entry);
            let mut exit_signal: Option<&'static str> = None;
            if self.direction == 1 && oi_down {
                exit_signal = Some("oi_divergence");
            } else if self.direction == -1 && oi_up {
                exit_signal = Some("oi_divergence");
            }
            if exit_signal.is_none() {
                if self.direction == 1 && rsi_val > RSI_EXIT_HIGH {
                    exit_signal = Some("rsi_overbought");
                } else if self.direction == -1 && rsi_val < RSI_EXIT_LOW {
                    exit_signal = Some("rsi_oversold");
                }
            }
            if exit_signal.is_none() {
                if self.direction == 1 {
                    self.peak_price = self.peak_price.max(mid);
                    if mid < self.peak_price - ATR_STOP_MULT * atr {
                        exit_signal = Some("atr_trailing_stop");
                    }
                } else {
                    self.peak_price = self.peak_price.min(mid);
                    if mid > self.peak_price + ATR_STOP_MULT * atr {
                        exit_signal = Some("atr_trailing_stop");
                    }
                }
            }
            if let Some(sig) = exit_signal {
                let close_side = if self.direction == 1 { OrderSide::Sell } else { OrderSide::Buy };
                let close_price = if self.direction == 1 { snap.bid } else { snap.ask };
                let size = if ctx.position_qty != 0.0 { ctx.position_qty.abs() } else { self.size };
                orders.push(
                    StrategyDecision::place(snap.instrument.clone(), close_side, size, round_to(close_price, 8))
                        .with_order_type(StrategyOrderType::Ioc)
                        .with_meta("signal", sig),
                );
                self.direction = 0;
            }
        }
        orders
    }
}

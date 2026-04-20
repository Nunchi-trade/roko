//! `TrendFollowerStrategy` — EMA crossover + ADX filter + ATR trailing stop.
//!
//! Port of `strategies/trend_follower.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

const EMA_FAST: usize = 7;
const EMA_SLOW: usize = 26;
const ADX_PERIOD: usize = 14;
const ADX_ENTRY_THRESHOLD: f64 = 25.0;
const ADX_EXIT_THRESHOLD: f64 = 20.0;
const ATR_LOOKBACK: usize = 24;
const ATR_STOP_MULT: f64 = 4.5;
fn min_history() -> usize {
    let a = 2 * ADX_PERIOD + 5;
    let b = EMA_SLOW + 10;
    let c = ATR_LOOKBACK;
    a.max(b).max(c) + 1
}

fn ema(values: &[f64], span: usize) -> Vec<f64> {
    let alpha = 2.0 / (span as f64 + 1.0);
    let mut out = Vec::with_capacity(values.len());
    if let Some(&first) = values.first() {
        out.push(first);
        for v in &values[1..] {
            let prev = *out.last().unwrap();
            out.push(alpha * v + (1.0 - alpha) * prev);
        }
    }
    out
}

fn calc_adx(highs: &[f64], lows: &[f64], closes: &[f64], period: usize) -> f64 {
    let n = closes.len();
    if n < 2 * period + 2 {
        return 0.0;
    }
    let mut tr = Vec::with_capacity(n);
    let mut plus_dm = Vec::with_capacity(n);
    let mut minus_dm = Vec::with_capacity(n);
    for i in 1..n {
        let h = highs[i];
        let l = lows[i];
        let prev_c = closes[i - 1];
        tr.push((h - l).max((h - prev_c).abs()).max((l - prev_c).abs()));
        let up = highs[i] - highs[i - 1];
        let down = lows[i - 1] - lows[i];
        plus_dm.push(if up > down && up > 0.0 { up } else { 0.0 });
        minus_dm.push(if down > up && down > 0.0 { down } else { 0.0 });
    }
    if tr.len() < period {
        return 0.0;
    }
    let mut atr: f64 = tr[..period].iter().sum();
    let mut pdm: f64 = plus_dm[..period].iter().sum();
    let mut mdm: f64 = minus_dm[..period].iter().sum();
    let mut dx_values = Vec::with_capacity(tr.len() - period);
    for i in period..tr.len() {
        atr = atr - atr / period as f64 + tr[i];
        pdm = pdm - pdm / period as f64 + plus_dm[i];
        mdm = mdm - mdm / period as f64 + minus_dm[i];
        if atr > 0.0 {
            let plus_di = 100.0 * pdm / atr;
            let minus_di = 100.0 * mdm / atr;
            let di_sum = plus_di + minus_di;
            let dx = if di_sum > 0.0 { 100.0 * (plus_di - minus_di).abs() / di_sum } else { 0.0 };
            dx_values.push(dx);
        }
    }
    if dx_values.len() < period {
        return 0.0;
    }
    let mut adx = dx_values[..period].iter().sum::<f64>() / period as f64;
    for i in period..dx_values.len() {
        adx = (adx * (period as f64 - 1.0) + dx_values[i]) / period as f64;
    }
    adx
}

fn calc_atr(highs: &[f64], lows: &[f64], closes: &[f64], lookback: usize) -> Option<f64> {
    if closes.len() < lookback + 1 {
        return None;
    }
    let start = closes.len() - lookback;
    let mut trs = Vec::with_capacity(lookback);
    for i in start..closes.len() {
        let h = highs[i];
        let l = lows[i];
        let prev_c = closes[i - 1];
        trs.push((h - l).max((h - prev_c).abs()).max((l - prev_c).abs()));
    }
    Some(trs.iter().sum::<f64>() / trs.len() as f64)
}

#[derive(Debug, Clone)]
pub struct TrendFollowerStrategy {
    id: String,
    pub size: f64,
    closes: VecDeque<f64>,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    direction: i8,
    entry_price: f64,
    peak_price: f64,
    atr_at_entry: f64,
    prev_ema_cross: i8,
}

impl TrendFollowerStrategy {
    #[must_use]
    pub fn new(size: f64) -> Self {
        let buf_len = min_history() + 5;
        Self {
            id: "trend_follower".into(),
            size,
            closes: VecDeque::with_capacity(buf_len),
            highs: VecDeque::with_capacity(buf_len),
            lows: VecDeque::with_capacity(buf_len),
            direction: 0,
            entry_price: 0.0,
            peak_price: 0.0,
            atr_at_entry: 0.0,
            prev_ema_cross: 0,
        }
    }
}

impl Default for TrendFollowerStrategy {
    fn default() -> Self {
        Self::new(1.0)
    }
}

fn push_bounded(buf: &mut VecDeque<f64>, val: f64, cap: usize) {
    if buf.len() == cap {
        buf.pop_front();
    }
    buf.push_back(val);
}

impl Strategy for TrendFollowerStrategy {
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
        if self.closes.len() < min_history() {
            return vec![];
        }

        let closes: Vec<f64> = self.closes.iter().copied().collect();
        let highs: Vec<f64> = self.highs.iter().copied().collect();
        let lows: Vec<f64> = self.lows.iter().copied().collect();

        let start = closes.len().saturating_sub(EMA_SLOW + 10);
        let segment = &closes[start..];
        let ema_f = ema(segment, EMA_FAST);
        let ema_s = ema(segment, EMA_SLOW);
        let current_cross: i8 = if ema_f.last().unwrap() > ema_s.last().unwrap() { 1 } else { -1 };
        let crossover_up = current_cross == 1 && self.prev_ema_cross == -1;
        let crossover_down = current_cross == -1 && self.prev_ema_cross == 1;
        self.prev_ema_cross = current_cross;

        let adx = calc_adx(&highs, &lows, &closes, ADX_PERIOD);

        // Sync direction from context.
        if ctx.position_qty > 0.0 {
            self.direction = 1;
        } else if ctx.position_qty < 0.0 {
            self.direction = -1;
        } else if ctx.position_qty == 0.0 {
            self.direction = 0;
        }

        let mut orders = vec![];

        if self.direction == 0 {
            if crossover_up && adx > ADX_ENTRY_THRESHOLD {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Buy,
                        self.size,
                        round_to(snap.ask, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "trend_long")
                    .with_meta("adx", round_to(adx, 1)),
                );
                self.direction = 1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            } else if crossover_down && adx > ADX_ENTRY_THRESHOLD {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Sell,
                        self.size,
                        round_to(snap.bid, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "trend_short")
                    .with_meta("adx", round_to(adx, 1)),
                );
                self.direction = -1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            }
        } else {
            let atr = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(self.atr_at_entry);
            let mut exit_signal: Option<&'static str> = None;
            if self.direction == 1 && crossover_down {
                exit_signal = Some("ema_cross_exit");
            } else if self.direction == -1 && crossover_up {
                exit_signal = Some("ema_cross_exit");
            }
            if exit_signal.is_none() && adx < ADX_EXIT_THRESHOLD {
                exit_signal = Some("adx_weak");
            }
            if exit_signal.is_none() {
                if self.direction == 1 {
                    self.peak_price = self.peak_price.max(mid);
                    let stop = self.peak_price - ATR_STOP_MULT * atr;
                    if mid < stop {
                        exit_signal = Some("atr_trailing_stop");
                    }
                } else {
                    self.peak_price = self.peak_price.min(mid);
                    let stop = self.peak_price + ATR_STOP_MULT * atr;
                    if mid > stop {
                        exit_signal = Some("atr_trailing_stop");
                    }
                }
            }
            if let Some(signal) = exit_signal {
                let close_side = if self.direction == 1 { OrderSide::Sell } else { OrderSide::Buy };
                let close_price = if self.direction == 1 { snap.bid } else { snap.ask };
                let size = if ctx.position_qty != 0.0 { ctx.position_qty.abs() } else { self.size };
                orders.push(
                    StrategyDecision::place(snap.instrument.clone(), close_side, size, round_to(close_price, 8))
                        .with_order_type(StrategyOrderType::Ioc)
                        .with_meta("signal", signal),
                );
                self.direction = 0;
            }
        }
        orders
    }
}

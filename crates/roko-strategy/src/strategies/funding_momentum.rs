//! `FundingMomentumStrategy` — z-score on funding rate + EMA confirmation.
//!
//! Port of `strategies/funding_momentum.py`.

use std::collections::VecDeque;

use roko_venue::{MarketSnapshot, OrderSide};

use crate::strategy::{Strategy, StrategyContext, StrategyDecision, StrategyOrderType};

use super::simple_mm::round_to;

const FUNDING_LOOKBACK: usize = 48;
const EMA_FAST: usize = 12;
const EMA_SLOW: usize = 26;
const ZSCORE_ENTRY: f64 = 2.0;
const ZSCORE_EXIT: f64 = 1.0;
const ATR_LOOKBACK: usize = 24;
const ATR_STOP_MULT: f64 = 4.0;

fn min_history() -> usize {
    FUNDING_LOOKBACK.max(EMA_SLOW + 10).max(ATR_LOOKBACK) + 1
}

fn ema_last_pair(values: &[f64], fast: usize, slow: usize) -> (f64, f64) {
    let ema = |span: usize| {
        let alpha = 2.0 / (span as f64 + 1.0);
        let mut acc = values[0];
        for v in &values[1..] {
            acc = alpha * v + (1.0 - alpha) * acc;
        }
        acc
    };
    (ema(fast), ema(slow))
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
pub struct FundingMomentumStrategy {
    id: String,
    pub size: f64,
    closes: VecDeque<f64>,
    highs: VecDeque<f64>,
    lows: VecDeque<f64>,
    funding_rates: VecDeque<f64>,
    direction: i8,
    entry_price: f64,
    peak_price: f64,
    atr_at_entry: f64,
}

impl FundingMomentumStrategy {
    #[must_use]
    pub fn new(size: f64) -> Self {
        let buf = min_history() + 5;
        Self {
            id: "funding_momentum".into(),
            size,
            closes: VecDeque::with_capacity(buf),
            highs: VecDeque::with_capacity(buf),
            lows: VecDeque::with_capacity(buf),
            funding_rates: VecDeque::with_capacity(FUNDING_LOOKBACK),
            direction: 0,
            entry_price: 0.0,
            peak_price: 0.0,
            atr_at_entry: 0.0,
        }
    }
}

impl Default for FundingMomentumStrategy {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Strategy for FundingMomentumStrategy {
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
        push_bounded(&mut self.funding_rates, snap.funding_rate, FUNDING_LOOKBACK);
        if self.closes.len() < min_history() || self.funding_rates.len() < FUNDING_LOOKBACK {
            return vec![];
        }

        let closes: Vec<f64> = self.closes.iter().copied().collect();
        let highs: Vec<f64> = self.highs.iter().copied().collect();
        let lows: Vec<f64> = self.lows.iter().copied().collect();
        let rates: Vec<f64> = self.funding_rates.iter().copied().collect();
        let n = rates.len() as f64;
        let mean = rates.iter().sum::<f64>() / n;
        let var = rates.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        let std = var.sqrt().max(1e-10);
        let last = *rates.last().unwrap();
        let zscore = (last - mean) / std;

        let start = closes.len().saturating_sub(EMA_SLOW + 10);
        let segment = &closes[start..];
        let (ema_f, ema_s) = ema_last_pair(segment, EMA_FAST, EMA_SLOW);
        let bullish = ema_f > ema_s;
        let bearish = ema_f < ema_s;

        if ctx.position_qty > 0.0 {
            self.direction = 1;
        } else if ctx.position_qty < 0.0 {
            self.direction = -1;
        } else if ctx.position_qty == 0.0 {
            self.direction = 0;
        }

        let mut orders = vec![];
        if self.direction == 0 {
            if zscore < -ZSCORE_ENTRY && bullish {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Buy,
                        self.size,
                        round_to(snap.ask, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "funding_long")
                    .with_meta("funding_zscore", round_to(zscore, 2)),
                );
                self.direction = 1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            } else if zscore > ZSCORE_ENTRY && bearish {
                orders.push(
                    StrategyDecision::place(
                        snap.instrument.clone(),
                        OrderSide::Sell,
                        self.size,
                        round_to(snap.bid, 8),
                    )
                    .with_order_type(StrategyOrderType::Ioc)
                    .with_meta("signal", "funding_short")
                    .with_meta("funding_zscore", round_to(zscore, 2)),
                );
                self.direction = -1;
                self.entry_price = mid;
                self.peak_price = mid;
                self.atr_at_entry = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(mid * 0.02);
            }
        } else {
            let atr = calc_atr(&highs, &lows, &closes, ATR_LOOKBACK).unwrap_or(self.atr_at_entry);
            let mut exit_signal: Option<&'static str> = None;
            if self.direction == 1 && zscore > -ZSCORE_EXIT {
                exit_signal = Some("funding_normalized");
            } else if self.direction == -1 && zscore < ZSCORE_EXIT {
                exit_signal = Some("funding_normalized");
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

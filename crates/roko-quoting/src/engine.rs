//! Top-level [`QuotingEngine`] orchestrator. Ports `quoting_engine/engine.py`.
//!
//! Feature parity vs Python includes: OI-drop & mid-burst liquidation
//! detection, session-regime overrides, FV-band clamping, disagreement mode,
//! funding-boundary handling, inventory cap states with micro-clip unwind.
//!
//! External-feed hooks (oracle-freshness monitor, L2 microprice, cross-venue
//! funding feed) are first-class inputs on [`QuotingEngine::tick`] rather
//! than injected components — callers supply whatever they have; defaults
//! mirror the Python stub modules.

use std::collections::VecDeque;

use chrono::{DateTime, Datelike, Timelike, Utc};
use roko_strategy::{dd_multiplier, DdBin, VolBin, VolBinClassifier};
use serde::{Deserialize, Serialize};

use crate::config::{MarketConfig, RegimeOverride};
use crate::fair_value::FairValueCalculator;
use crate::inventory::{InventorySkewer, InventoryState, MicroClipOrder};
use crate::ladder::{LadderBuilder, LadderLevel};
use crate::spread::SpreadCalculator;
use crate::vol_estimator::RollingVolEstimator;

/// Per-tick metadata surfaced for observability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotingMeta {
    pub h_tox: f64,
    pub h_event: f64,
    pub vol_ready: bool,
    pub oracle_zone: String,
    pub oracle_age_ms: i64,
    pub microprice_source: String,
    pub funding_source: String,
    pub regime_name: String,
    pub session_mult: f64,
    pub liq_triggered: bool,
    pub liq_mid_burst: bool,
    pub liq_escalated: bool,
    pub liq_cooldown_remaining: u32,
    pub disagree_active: bool,
    pub in_funding_boundary: bool,
    pub inv_state: &'static str,
    pub micro_clip_side: Option<&'static str>,
    pub micro_clip_size: Option<f64>,
}

/// Output of a single [`QuotingEngine::tick`] invocation.
#[derive(Debug, Clone, Default)]
pub struct QuoteResult {
    pub fv_raw: f64,
    pub fv_skewed: f64,
    pub half_spread: f64,
    pub sigma_price: f64,
    pub sigma_log: f64,
    pub m_vol: f64,
    pub vol_bin: String,
    pub m_dd: f64,
    pub dd_bin: String,
    pub levels: Vec<LadderLevel>,
    pub halted: bool,
    pub reduce_only: bool,
    pub meta: QuotingMeta,
}

/// Stateful quoting engine — one per market.
pub struct QuotingEngine {
    cfg: MarketConfig,
    fv: FairValueCalculator,
    spread: SpreadCalculator,
    ladder: LadderBuilder,
    skewer: InventorySkewer,
    vol: RollingVolEstimator,
    vol_bin: VolBinClassifier,
    prev_oi: f64,
    liq_cooldown_remaining: u32,
    liq_total_cooldown: u32,
    mid_history_short: VecDeque<f64>,
    tick_count: u64,
}

impl QuotingEngine {
    #[must_use]
    pub fn new(cfg: MarketConfig) -> Self {
        let fv = FairValueCalculator::new(cfg.fv_weights);
        let spread = SpreadCalculator::new(cfg.spread, cfg.tick_size);
        let ladder = LadderBuilder::new(cfg.ladder, cfg.tick_size);
        let skewer = InventorySkewer::new(cfg.skew);
        let vol = RollingVolEstimator::new(cfg.vol_window as usize);
        let window = cfg.liquidation_detector.mid_burst_window.max(1);
        Self {
            cfg,
            fv,
            spread,
            ladder,
            skewer,
            vol,
            vol_bin: VolBinClassifier::new(),
            prev_oi: 0.0,
            liq_cooldown_remaining: 0,
            liq_total_cooldown: 0,
            mid_history_short: VecDeque::with_capacity(window),
            tick_count: 0,
        }
    }

    /// Optional external toxicity component, matches
    /// `quoting_engine.toxicity.StubToxicityScorer`.
    fn h_tox(_mid: f64, _bid: f64, _ask: f64) -> f64 {
        0.0
    }

    /// Optional external event-risk component (stub).
    fn h_event() -> f64 {
        0.0
    }

    fn in_funding_boundary(&self, now_ms: i64) -> bool {
        let cfg = &self.cfg.funding_boundary;
        if !cfg.enabled || now_ms <= 0 {
            return false;
        }
        let Some(dt) = DateTime::<Utc>::from_timestamp_millis(now_ms) else {
            return false;
        };
        let secs_into_hour = i64::from(dt.minute()) * 60 + i64::from(dt.second());
        secs_into_hour >= 3600 - cfg.pre_window_s || secs_into_hour < cfg.post_window_s
    }

    fn resolve_regime(&self, now_ms: i64) -> (String, RegimeOverride) {
        let sr = &self.cfg.session_regime;
        if !sr.enabled || now_ms <= 0 {
            return ("OPEN".into(), RegimeOverride::identity());
        }
        let Some(dt) = DateTime::<Utc>::from_timestamp_millis(now_ms) else {
            return ("OPEN".into(), RegimeOverride::identity());
        };
        let weekday = dt.weekday().num_days_from_monday();
        if sr.weekend_days.contains(&weekday) {
            if let Some(ro) = sr.regimes.get("WEEKEND") {
                return ("WEEKEND".into(), ro.clone());
            }
            return (
                "WEEKEND".into(),
                RegimeOverride {
                    spread_mult: sr.off_session_spread_mult,
                    size_mult: 1.0,
                    num_levels: None,
                    w_oracle_override: None,
                    reduce_only: false,
                },
            );
        }
        if let Some(&max_weekend) = sr.weekend_days.iter().max() {
            if weekday == (max_weekend + 1) % 7 {
                let minutes_since_midnight = i64::from(dt.hour()) * 60 + i64::from(dt.minute());
                if minutes_since_midnight < sr.reopen_window_minutes {
                    if let Some(ro) = sr.regimes.get("REOPEN_WINDOW") {
                        return ("REOPEN_WINDOW".into(), ro.clone());
                    }
                    return (
                        "REOPEN_WINDOW".into(),
                        RegimeOverride {
                            spread_mult: 2.0,
                            size_mult: 0.5,
                            num_levels: None,
                            w_oracle_override: None,
                            reduce_only: false,
                        },
                    );
                }
            }
        }
        let current_minutes = i64::from(dt.hour()) * 60 + i64::from(dt.minute());
        let (start, end) = parse_hhmm_range(&sr.in_session_start_utc, &sr.in_session_end_utc);
        if current_minutes >= start && current_minutes < end {
            if let Some(ro) = sr.regimes.get("OPEN") {
                return ("OPEN".into(), ro.clone());
            }
            return ("OPEN".into(), RegimeOverride::identity());
        }
        if let Some(ro) = sr.regimes.get("CLOSE") {
            return ("CLOSE".into(), ro.clone());
        }
        (
            "CLOSE".into(),
            RegimeOverride {
                spread_mult: sr.off_session_spread_mult,
                size_mult: 1.0,
                num_levels: None,
                w_oracle_override: None,
                reduce_only: false,
            },
        )
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub fn tick(
        &mut self,
        mid: f64,
        bid: f64,
        ask: f64,
        inventory: f64,
        daily_drawdown_pct: f64,
        mut reduce_only: bool,
        _timestamp_ms: i64,
        mut external_ref: f64,
        now_ms: i64,
        open_interest: f64,
    ) -> QuoteResult {
        self.tick_count += 1;

        // No oracle monitor / microprice / funding feed plumbing in this port —
        // callers pass `external_ref` directly if they have a cross-venue signal,
        // same as `StubToxicityScorer` + `StubEventSchedule` in the Python stub
        // flow. Full external-feed wiring lands in a follow-up commit.
        let oracle_zone = "fresh";
        let oracle_age_ms = 0i64;
        let oracle_spread_mult = 1.0_f64;

        let microprice_source = "bid_ask_proxy".to_string();
        let funding_source = "external_ref".to_string();

        if self.cfg.funding_dampening > 0.0 && external_ref != 0.0 {
            external_ref *= 1.0 / self.cfg.funding_dampening;
        }

        let (sigma_price, sigma_log) = self.vol.update(mid);
        let VolBin { multiplier: m_vol, name: vol_bin } = self.vol_bin.classify(sigma_log);
        let DdBin { multiplier: m_dd, name: dd_bin } = dd_multiplier(daily_drawdown_pct);

        if m_dd.is_infinite() {
            let mut result = QuoteResult {
                fv_raw: mid,
                fv_skewed: mid,
                sigma_price,
                sigma_log,
                m_vol,
                vol_bin: vol_bin.into(),
                m_dd,
                dd_bin: dd_bin.into(),
                halted: true,
                reduce_only: true,
                ..QuoteResult::default()
            };
            result.meta.oracle_zone = oracle_zone.into();
            return result;
        }
        if m_dd >= 2.0 {
            reduce_only = true;
        }

        let inv_state = self.skewer.inventory_state(inventory);
        if inv_state == InventoryState::HardBreach {
            let mut result = QuoteResult {
                fv_raw: mid,
                fv_skewed: mid,
                sigma_price,
                sigma_log,
                m_vol,
                vol_bin: vol_bin.into(),
                m_dd,
                dd_bin: dd_bin.into(),
                halted: true,
                reduce_only: true,
                ..QuoteResult::default()
            };
            result.meta.inv_state = "hard_breach";
            return result;
        }
        if inv_state == InventoryState::SoftBreach {
            reduce_only = true;
        }

        let (regime_name, regime) = self.resolve_regime(now_ms);

        let mut fv_raw = self.fv.compute(
            mid,
            bid,
            ask,
            external_ref,
            0.0,
            None,
            regime.w_oracle_override,
        );

        let fv_band_cfg = self.cfg.fv_band;
        if fv_band_cfg.enabled && mid > 0.0 {
            let disagree_abs = if external_ref > 0.0 && (external_ref - mid).abs() > f64::EPSILON {
                (mid - external_ref).abs()
            } else {
                0.0
            };
            let band = [
                fv_band_cfg.band_min_bps * mid / 10_000.0,
                fv_band_cfg.k_sigma * sigma_price,
                fv_band_cfg.k_disagree * disagree_abs,
            ]
            .into_iter()
            .fold(0.0_f64, f64::max);
            fv_raw = (mid - band).max(fv_raw.min(mid + band));
        }

        let in_boundary = self.in_funding_boundary(now_ms);
        if in_boundary && self.cfg.funding_boundary.pin_fv_to_oracle {
            fv_raw = mid;
        }

        let fv_skewed = self.skewer.price_skew(fv_raw, inventory, sigma_price);

        let h_tox = Self::h_tox(mid, bid, ask);
        let h_event = Self::h_event();
        let mut half_spread = self.spread.compute(mid, sigma_price, m_vol, m_dd, h_tox, h_event);
        half_spread *= oracle_spread_mult;
        half_spread *= regime.spread_mult;
        if regime.reduce_only {
            reduce_only = true;
        }

        let liq_cfg = self.cfg.liquidation_detector;
        let mut liq_triggered = false;
        let mut liq_mid_burst = false;
        let mut liq_escalated = false;

        if liq_cfg.enabled {
            if open_interest > 0.0 {
                if self.prev_oi > 0.0 {
                    let oi_change_pct = (open_interest - self.prev_oi) / self.prev_oi * 100.0;
                    if oi_change_pct <= -liq_cfg.oi_drop_threshold_pct {
                        self.liq_cooldown_remaining = liq_cfg.cooldown_ticks;
                        self.liq_total_cooldown = liq_cfg.cooldown_ticks;
                        liq_triggered = true;
                    }
                }
                self.prev_oi = open_interest;
            }
            if self.mid_history_short.len() == liq_cfg.mid_burst_window.max(1) {
                self.mid_history_short.pop_front();
            }
            self.mid_history_short.push_back(mid);
            if liq_cfg.mid_burst_bps > 0.0 && self.mid_history_short.len() >= liq_cfg.mid_burst_window {
                let hi = self.mid_history_short.iter().copied().fold(f64::MIN, f64::max);
                let lo = self.mid_history_short.iter().copied().fold(f64::MAX, f64::min);
                let burst = hi - lo;
                let threshold = liq_cfg.mid_burst_bps * mid / 10_000.0;
                if burst > threshold {
                    if self.liq_cooldown_remaining == 0 {
                        self.liq_cooldown_remaining = liq_cfg.cooldown_ticks;
                        self.liq_total_cooldown = liq_cfg.cooldown_ticks;
                    }
                    liq_mid_burst = true;
                }
            }
        }

        if self.liq_cooldown_remaining > 0 {
            half_spread *= liq_cfg.spread_mult;
            self.liq_cooldown_remaining -= 1;
            if liq_cfg.escalation_ticks > 0 {
                let in_cooldown = self.liq_total_cooldown - self.liq_cooldown_remaining;
                if in_cooldown >= liq_cfg.escalation_ticks {
                    reduce_only = true;
                    liq_escalated = true;
                }
            }
        }

        let disagree_cfg = self.cfg.disagreement;
        let mut disagree_active = false;
        if disagree_cfg.enabled && external_ref > 0.0 && (external_ref - mid).abs() > f64::EPSILON && mid > 0.0 {
            let disagree_bps = (mid - external_ref).abs() / mid * 10_000.0;
            if disagree_bps > disagree_cfg.threshold_bps {
                half_spread *= disagree_cfg.spread_mult;
                disagree_active = true;
            }
        }

        let (mut bid_mult, mut ask_mult) = self.skewer.size_skew(1.0, 1.0, inventory);
        if disagree_active {
            bid_mult *= disagree_cfg.size_mult;
            ask_mult *= disagree_cfg.size_mult;
        }
        if in_boundary {
            bid_mult *= self.cfg.funding_boundary.size_mult;
            ask_mult *= self.cfg.funding_boundary.size_mult;
        }
        let liq_in_cooldown = self.liq_cooldown_remaining > 0 || liq_triggered;
        if liq_in_cooldown {
            bid_mult *= liq_cfg.size_mult;
            ask_mult *= liq_cfg.size_mult;
        }
        bid_mult *= regime.size_mult;
        ask_mult *= regime.size_mult;

        let mut levels = self.ladder.build(fv_skewed, half_spread, mid, bid_mult, ask_mult, regime.num_levels);

        if liq_in_cooldown && liq_cfg.liq_catcher_levels > 0 && levels.len() > liq_cfg.liq_catcher_levels as usize {
            let pull = levels.len() - liq_cfg.liq_catcher_levels as usize;
            for (i, level) in levels.iter_mut().enumerate() {
                if i < pull {
                    level.bid_size = 0.0;
                    level.ask_size = 0.0;
                } else {
                    level.bid_size = round6(level.bid_size * liq_cfg.liq_catcher_size_mult);
                    level.ask_size = round6(level.ask_size * liq_cfg.liq_catcher_size_mult);
                }
            }
        }

        let micro_clip = self.skewer.micro_clip_order(inventory, self.tick_count);

        QuoteResult {
            fv_raw,
            fv_skewed,
            half_spread,
            sigma_price,
            sigma_log,
            m_vol,
            vol_bin: vol_bin.into(),
            m_dd,
            dd_bin: dd_bin.into(),
            levels,
            halted: false,
            reduce_only,
            meta: QuotingMeta {
                h_tox,
                h_event,
                vol_ready: self.vol.ready(),
                oracle_zone: oracle_zone.into(),
                oracle_age_ms,
                microprice_source,
                funding_source,
                regime_name,
                session_mult: regime.spread_mult,
                liq_triggered,
                liq_mid_burst,
                liq_escalated,
                liq_cooldown_remaining: self.liq_cooldown_remaining,
                disagree_active,
                in_funding_boundary: in_boundary,
                inv_state: match inv_state {
                    InventoryState::Normal => "normal",
                    InventoryState::SoftBreach => "soft_breach",
                    InventoryState::HardBreach => "hard_breach",
                },
                micro_clip_side: micro_clip.map(|c| c.side),
                micro_clip_size: micro_clip.map(|c: MicroClipOrder| c.size),
            },
            ..Default::default()
        }
    }

    #[must_use]
    pub fn config(&self) -> &MarketConfig {
        &self.cfg
    }
}

fn parse_hhmm_range(start: &str, end: &str) -> (i64, i64) {
    (parse_hhmm(start), parse_hhmm(end))
}

fn parse_hhmm(s: &str) -> i64 {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return 0;
    }
    let h: i64 = parts[0].parse().unwrap_or(0);
    let m: i64 = parts[1].parse().unwrap_or(0);
    h * 60 + m
}

fn round6(x: f64) -> f64 {
    (x * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_n_ticks(engine: &mut QuotingEngine, mid: f64, ticks: u32) -> QuoteResult {
        let mut last = QuoteResult::default();
        for i in 0..ticks {
            last = engine.tick(mid + f64::from(i) * 0.01, mid - 1.0, mid + 1.0, 0.0, 0.0, false, 0, 0.0, 0, 0.0);
        }
        last
    }

    #[test]
    fn basic_tick_produces_ladder() {
        let cfg = MarketConfig::default();
        let mut engine = QuotingEngine::new(cfg);
        let result = run_n_ticks(&mut engine, 2500.0, 10);
        assert!(!result.halted);
        assert!(!result.levels.is_empty());
        assert!(result.fv_raw > 0.0);
    }

    #[test]
    fn hard_dd_halts() {
        let cfg = MarketConfig::default();
        let mut engine = QuotingEngine::new(cfg);
        let r = engine.tick(2500.0, 2499.0, 2501.0, 0.0, 10.0, false, 0, 0.0, 0, 0.0);
        assert!(r.halted);
        assert!(r.m_dd.is_infinite());
    }

    #[test]
    fn liquidation_oi_drop_triggers_cooldown() {
        let mut cfg = MarketConfig::default();
        cfg.liquidation_detector.enabled = true;
        cfg.liquidation_detector.oi_drop_threshold_pct = 5.0;
        cfg.liquidation_detector.cooldown_ticks = 3;
        cfg.liquidation_detector.spread_mult = 2.0;
        cfg.liquidation_detector.size_mult = 0.5;
        let mut engine = QuotingEngine::new(cfg);
        // seed OI
        engine.tick(2500.0, 2499.0, 2501.0, 0.0, 0.0, false, 0, 0.0, 0, 1_000_000.0);
        // drop OI 10% → triggers cooldown
        let r = engine.tick(2500.0, 2499.0, 2501.0, 0.0, 0.0, false, 0, 0.0, 0, 900_000.0);
        assert!(r.meta.liq_triggered);
    }
}

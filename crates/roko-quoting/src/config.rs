//! Quoting-engine configuration models. Mirrors
//! `offchainservices-agent/quoting_engine/config.py`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FairValueWeights {
    pub w_oracle: f64,
    pub w_external: f64,
    pub w_microprice: f64,
    pub w_inventory: f64,
}

impl Default for FairValueWeights {
    fn default() -> Self {
        Self {
            w_oracle: 0.50,
            w_external: 0.00,
            w_microprice: 0.30,
            w_inventory: 0.20,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpreadParams {
    pub h_fee_bps: f64,
    pub vol_scale: f64,
    pub rebate_credit_bps: f64,
    pub min_spread_bps: f64,
    pub max_spread_bps: f64,
    pub growth_mode: bool,
    pub growth_mode_scale: f64,
}

impl Default for SpreadParams {
    fn default() -> Self {
        Self {
            h_fee_bps: 1.0,
            vol_scale: 1.0,
            rebate_credit_bps: 0.0,
            min_spread_bps: 2.0,
            max_spread_bps: 50.0,
            growth_mode: false,
            growth_mode_scale: 0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LadderParams {
    pub num_levels: u32,
    pub delta_bps: f64,
    pub s0: f64,
    pub lam: f64,
    pub min_size_ratio: f64,
}

impl Default for LadderParams {
    fn default() -> Self {
        Self {
            num_levels: 3,
            delta_bps: 1.5,
            s0: 1.0,
            lam: 0.5,
            min_size_ratio: 0.1,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkewMode {
    Price,
    Size,
    Both,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SkewParams {
    pub k_inv: f64,
    pub inv_limit: f64,
    pub mode: SkewMode,
    pub size_skew_factor: f64,
    pub soft_cap: f64,
    pub hard_cap: f64,
    pub micro_clip_size: f64,
    pub micro_clip_interval: u32,
}

impl Default for SkewParams {
    fn default() -> Self {
        Self {
            k_inv: 0.5,
            inv_limit: 10.0,
            mode: SkewMode::Both,
            size_skew_factor: 0.3,
            soft_cap: 0.0,
            hard_cap: 0.0,
            micro_clip_size: 0.0,
            micro_clip_interval: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FairValueBandConfig {
    pub enabled: bool,
    pub band_min_bps: f64,
    pub k_sigma: f64,
    pub k_disagree: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct DisagreementConfig {
    pub enabled: bool,
    pub threshold_bps: f64,
    pub spread_mult: f64,
    pub size_mult: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct FundingBoundaryConfig {
    pub enabled: bool,
    pub pre_window_s: i64,
    pub post_window_s: i64,
    pub size_mult: f64,
    pub pin_fv_to_oracle: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegimeOverride {
    pub spread_mult: f64,
    pub size_mult: f64,
    pub num_levels: Option<u32>,
    pub w_oracle_override: Option<f64>,
    pub reduce_only: bool,
}

impl RegimeOverride {
    #[must_use]
    pub fn identity() -> Self {
        Self {
            spread_mult: 1.0,
            size_mult: 1.0,
            num_levels: None,
            w_oracle_override: None,
            reduce_only: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionRegimeConfig {
    pub enabled: bool,
    pub in_session_start_utc: String,
    pub in_session_end_utc: String,
    pub off_session_spread_mult: f64,
    #[serde(default)]
    pub regimes: std::collections::HashMap<String, RegimeOverride>,
    #[serde(default)]
    pub weekend_days: Vec<u32>,
    pub reopen_window_minutes: i64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LiquidationDetectorConfig {
    pub enabled: bool,
    pub oi_drop_threshold_pct: f64,
    pub spread_mult: f64,
    pub size_mult: f64,
    pub cooldown_ticks: u32,
    pub mid_burst_bps: f64,
    pub mid_burst_window: usize,
    pub liq_catcher_levels: u32,
    pub liq_catcher_size_mult: f64,
    pub escalation_ticks: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OracleMonitorConfig {
    pub warning_ms: i64,
    pub stale_ms: i64,
    pub kill_ms: i64,
    pub enabled: bool,
}

impl Default for OracleMonitorConfig {
    fn default() -> Self {
        Self {
            warning_ms: 5_000,
            stale_ms: 15_000,
            kill_ms: 60_000,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeedConfig {
    pub oracle_monitor: OracleMonitorConfig,
    #[serde(default)]
    pub event_calendar_path: String,
    #[serde(default)]
    pub microprice_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketConfig {
    pub market_name: String,
    pub instrument: String,
    pub tick_size: f64,
    pub round_duration_s: f64,
    pub fv_weights: FairValueWeights,
    pub spread: SpreadParams,
    pub ladder: LadderParams,
    pub skew: SkewParams,
    pub feeds: FeedConfig,
    pub fv_band: FairValueBandConfig,
    pub disagreement: DisagreementConfig,
    pub funding_boundary: FundingBoundaryConfig,
    pub session_regime: SessionRegimeConfig,
    pub liquidation_detector: LiquidationDetectorConfig,
    pub vol_window: u32,
    pub funding_dampening: f64,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            market_name: "funding_rate".into(),
            instrument: "FR-PERP".into(),
            tick_size: 0.01,
            round_duration_s: 20.0,
            fv_weights: FairValueWeights::default(),
            spread: SpreadParams::default(),
            ladder: LadderParams::default(),
            skew: SkewParams::default(),
            feeds: FeedConfig::default(),
            fv_band: FairValueBandConfig::default(),
            disagreement: DisagreementConfig::default(),
            funding_boundary: FundingBoundaryConfig::default(),
            session_regime: SessionRegimeConfig {
                weekend_days: vec![5, 6],
                in_session_start_utc: "14:30".into(),
                in_session_end_utc: "21:00".into(),
                reopen_window_minutes: 30,
                off_session_spread_mult: 3.0,
                ..Default::default()
            },
            liquidation_detector: LiquidationDetectorConfig::default(),
            vol_window: 30,
            funding_dampening: 0.0,
        }
    }
}

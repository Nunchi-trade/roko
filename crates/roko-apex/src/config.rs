//! APEX configuration + preset map. Ports `modules/apex_config.py`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApexConfig {
    pub total_budget: f64,
    pub max_slots: u32,
    pub leverage: f64,
    pub margin_per_slot: f64,

    pub radar_score_threshold: u32,
    pub pulse_immediate_auto_entry: bool,
    pub pulse_confidence_threshold: f64,

    pub conviction_collapse_minutes: u32,
    pub stagnation_minutes: u32,
    pub stagnation_min_roe: f64,
    pub max_negative_roe: f64,

    pub min_hold_ms: u64,
    pub slot_cooldown_ms: u64,

    pub daily_loss_limit: f64,
    pub max_same_direction: u32,

    pub cooldown_duration_ms: u64,
    pub cooldown_trigger_losses: u32,
    pub cooldown_drawdown_pct: f64,

    pub guard_preset: String,
    pub guard_leverage_override: Option<f64>,

    pub tick_interval_s: f64,
    pub radar_interval_ticks: u32,
    pub watchdog_interval_ticks: u32,

    pub reflect_interval_ticks: u32,
    pub reflect_min_round_trips: u32,
    pub reflect_auto_adjust: bool,

    pub daily_reset_hour: u32,
    pub reflect_report_hour: u32,

    #[serde(default)]
    pub obsidian_vault_path: String,
}

impl ApexConfig {
    pub fn auto_compute_margin(&mut self) {
        if self.max_slots > 0 {
            self.margin_per_slot = self.total_budget / f64::from(self.max_slots);
        }
    }
}

impl Default for ApexConfig {
    fn default() -> Self {
        let mut cfg = Self {
            total_budget: 10_000.0,
            max_slots: 3,
            leverage: 10.0,
            margin_per_slot: 0.0,
            radar_score_threshold: 170,
            pulse_immediate_auto_entry: true,
            pulse_confidence_threshold: 70.0,
            conviction_collapse_minutes: 30,
            stagnation_minutes: 60,
            stagnation_min_roe: 3.0,
            max_negative_roe: -5.0,
            min_hold_ms: 2_700_000,
            slot_cooldown_ms: 300_000,
            daily_loss_limit: 500.0,
            max_same_direction: 2,
            cooldown_duration_ms: 1_800_000,
            cooldown_trigger_losses: 2,
            cooldown_drawdown_pct: 50.0,
            guard_preset: "tight".into(),
            guard_leverage_override: None,
            tick_interval_s: 60.0,
            radar_interval_ticks: 15,
            watchdog_interval_ticks: 5,
            reflect_interval_ticks: 240,
            reflect_min_round_trips: 5,
            reflect_auto_adjust: true,
            daily_reset_hour: 0,
            reflect_report_hour: 4,
            obsidian_vault_path: String::new(),
        };
        cfg.auto_compute_margin();
        cfg
    }
}

/// Named preset. Ports `APEX_PRESETS` from the Python module. Presets are
/// opinionated baselines — callers typically clone one and adjust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApexPreset {
    pub name: &'static str,
    pub config: ApexConfig,
}

fn build_presets() -> HashMap<String, ApexPreset> {
    let mut presets = HashMap::new();
    presets.insert(
        "default".into(),
        ApexPreset {
            name: "default",
            config: ApexConfig::default(),
        },
    );
    presets.insert(
        "aggressive".into(),
        ApexPreset {
            name: "aggressive",
            config: ApexConfig {
                total_budget: 25_000.0,
                max_slots: 5,
                leverage: 15.0,
                daily_loss_limit: 1_000.0,
                radar_score_threshold: 150,
                ..ApexConfig::default()
            },
        },
    );
    presets.insert(
        "conservative".into(),
        ApexPreset {
            name: "conservative",
            config: ApexConfig {
                total_budget: 5_000.0,
                max_slots: 2,
                leverage: 5.0,
                daily_loss_limit: 150.0,
                radar_score_threshold: 200,
                pulse_confidence_threshold: 85.0,
                ..ApexConfig::default()
            },
        },
    );
    // Auto-compute margin_per_slot for every entry after construction.
    for preset in presets.values_mut() {
        preset.config.auto_compute_margin();
    }
    presets
}

static PRESETS: OnceLock<HashMap<String, ApexPreset>> = OnceLock::new();

#[allow(non_snake_case)]
pub fn APEX_PRESETS() -> &'static HashMap<String, ApexPreset> {
    apex_presets()
}

/// Canonical snake-case accessor for the preset map.
pub fn apex_presets() -> &'static HashMap<String, ApexPreset> {
    PRESETS.get_or_init(build_presets)
}

//! APEX runtime state + JSON persistence. Ports `modules/apex_state.py`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApexSlot {
    pub slot_id: u32,
    #[serde(default = "default_empty")]
    pub status: String,
    #[serde(default)]
    pub instrument: String,
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub entry_source: String,
    #[serde(default)]
    pub entry_signal_score: f64,
    #[serde(default = "default_wallet")]
    pub wallet_id: String,
    #[serde(default)]
    pub entry_price: f64,
    #[serde(default)]
    pub entry_size: f64,
    #[serde(default)]
    pub margin_allocated: f64,
    #[serde(default)]
    pub current_price: f64,
    #[serde(default)]
    pub current_roe: f64,
    #[serde(default)]
    pub high_water_roe: f64,
    #[serde(default)]
    pub last_progress_ts: u64,
    #[serde(default)]
    pub entry_ts: u64,
    #[serde(default)]
    pub close_ts: u64,
    #[serde(default)]
    pub close_reason: String,
    #[serde(default)]
    pub close_pnl: f64,
    #[serde(default)]
    pub last_signal_seen_ts: u64,
    #[serde(default)]
    pub signal_disappeared_ts: u64,
}

fn default_empty() -> String {
    "empty".into()
}
fn default_wallet() -> String {
    "default".into()
}

impl ApexSlot {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.status == "empty"
    }
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApexState {
    #[serde(default)]
    pub slots: Vec<ApexSlot>,
    #[serde(default)]
    pub tick_count: u64,
    #[serde(default)]
    pub start_ts: u64,
    #[serde(default)]
    pub daily_pnl: f64,
    #[serde(default)]
    pub daily_loss_triggered: bool,
    #[serde(default)]
    pub total_trades: u64,
    #[serde(default)]
    pub total_pnl: f64,
    #[serde(default)]
    pub entry_queue: Vec<serde_json::Value>,
}

impl ApexState {
    #[must_use]
    pub fn new(max_slots: u32) -> Self {
        Self {
            slots: (0..max_slots).map(|i| ApexSlot { slot_id: i, status: "empty".into(), wallet_id: "default".into(), ..ApexSlot::default() }).collect(),
            tick_count: 0,
            start_ts: now_ms(),
            ..ApexState::default()
        }
    }

    #[must_use]
    pub fn active_slots(&self) -> Vec<&ApexSlot> {
        self.slots.iter().filter(|s| s.is_active()).collect()
    }

    #[must_use]
    pub fn active_instruments(&self) -> std::collections::BTreeSet<String> {
        self.active_slots().iter().map(|s| s.instrument.clone()).collect()
    }

    #[must_use]
    pub fn direction_count(&self, direction: &str) -> usize {
        self.active_slots().iter().filter(|s| s.direction == direction).count()
    }

    pub fn get_empty_slot(&mut self, now_ms: u64, cooldown_ms: u64) -> Option<&mut ApexSlot> {
        for slot in self.slots.iter_mut() {
            if !slot.is_empty() {
                continue;
            }
            if cooldown_ms > 0 && slot.close_ts > 0 && now_ms > 0 && now_ms - slot.close_ts < cooldown_ms {
                continue;
            }
            return Some(slot);
        }
        None
    }
}

pub struct ApexStateStore {
    path: PathBuf,
}

impl ApexStateStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn save(&self, state: &ApexState) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
        fs::write(&self.path, body)
    }

    pub fn load(&self) -> Option<ApexState> {
        let body = fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&body).ok()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

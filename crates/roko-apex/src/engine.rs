//! APEX orchestrator engine. Minimal port of `modules/apex_engine.py`'s
//! public surface — the detailed Radar/Pulse/Guard evaluation logic binds
//! in once those modules port (plans/P08-trading-surface.md T8 close-out).

use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::ApexConfig;
use crate::state::{ApexSlot, ApexState};

pub struct ApexEngine {
    pub config: ApexConfig,
    pub state: ApexState,
}

impl ApexEngine {
    #[must_use]
    pub fn new(mut config: ApexConfig) -> Self {
        config.auto_compute_margin();
        let state = ApexState::new(config.max_slots);
        Self { config, state }
    }

    /// Advance the orchestrator by one tick.
    ///
    /// Returns the slot ids that were touched this tick (entered or closed).
    /// The detailed entry/exit rules bind when Radar/Pulse/Guard port;
    /// today the tick only enforces the daily-loss-limit brake and advances
    /// the tick counter.
    pub fn tick(&mut self) -> Vec<u32> {
        self.state.tick_count += 1;
        if self.state.daily_pnl <= -self.config.daily_loss_limit {
            self.state.daily_loss_triggered = true;
        }
        Vec::new()
    }

    /// Open a slot for a given instrument/direction. Returns the slot id or
    /// `None` if no empty slot is available.
    pub fn enter(
        &mut self,
        instrument: &str,
        direction: &str,
        entry_source: &str,
        signal_score: f64,
        entry_price: f64,
        entry_size: f64,
    ) -> Option<u32> {
        if self.state.daily_loss_triggered {
            return None;
        }
        if self.state.direction_count(direction) >= self.config.max_same_direction as usize {
            return None;
        }
        let now = now_ms();
        let cooldown = self.config.slot_cooldown_ms;
        let slot = self.state.get_empty_slot(now, cooldown)?;
        slot.status = "active".into();
        slot.instrument = instrument.into();
        slot.direction = direction.into();
        slot.entry_source = entry_source.into();
        slot.entry_signal_score = signal_score;
        slot.entry_price = entry_price;
        slot.entry_size = entry_size;
        slot.margin_allocated = self.config.margin_per_slot;
        slot.current_price = entry_price;
        slot.entry_ts = now;
        slot.last_progress_ts = now;
        Some(slot.slot_id)
    }

    /// Close an active slot.
    pub fn close(&mut self, slot_id: u32, pnl: f64, reason: &str) -> Option<&ApexSlot> {
        let now = now_ms();
        let slot = self.state.slots.iter_mut().find(|s| s.slot_id == slot_id)?;
        if !slot.is_active() {
            return None;
        }
        slot.status = "closed".into();
        slot.close_ts = now;
        slot.close_reason = reason.into();
        slot.close_pnl = pnl;
        self.state.total_trades += 1;
        self.state.total_pnl += pnl;
        self.state.daily_pnl += pnl;
        self.state.slots.iter().find(|s| s.slot_id == slot_id)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_then_close_updates_totals() {
        let mut engine = ApexEngine::new(ApexConfig::default());
        let sid = engine.enter("ETH-PERP", "long", "pulse_immediate", 175.0, 2500.0, 0.1).unwrap();
        assert_eq!(engine.state.active_slots().len(), 1);
        engine.close(sid, 25.0, "tp").unwrap();
        assert_eq!(engine.state.total_trades, 1);
        assert!((engine.state.total_pnl - 25.0).abs() < 1e-9);
    }

    #[test]
    fn daily_loss_blocks_new_entry() {
        let mut engine = ApexEngine::new(ApexConfig {
            daily_loss_limit: 10.0,
            ..ApexConfig::default()
        });
        let sid = engine.enter("ETH-PERP", "long", "radar", 180.0, 2500.0, 0.1).unwrap();
        engine.close(sid, -20.0, "sl").unwrap();
        engine.tick();
        assert!(engine.state.daily_loss_triggered);
        // New entry should be blocked.
        assert!(engine.enter("BTC-PERP", "short", "radar", 190.0, 60_000.0, 0.01).is_none());
    }
}

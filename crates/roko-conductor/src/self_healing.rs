//! Self-healing conductor policy (COND-06).
//!
//! Recovery strategies for when the conductor detects stuck states:
//! - Reset watchers that are oscillating
//! - Reduce intervention sensitivity temporarily (cooldown)
//! - Auto-restart with fresh state after N consecutive failures

use serde::{Deserialize, Serialize};

/// React configuration for self-healing behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfHealingPolicy {
    /// Maximum oscillation count before a watcher is reset.
    /// An oscillation is when a watcher alternates between firing and
    /// not firing within consecutive ticks.
    pub max_oscillations: u32,
    /// Number of ticks to suppress interventions after a reset
    /// (sensitivity cooldown).
    pub cooldown_ticks: u64,
    /// After this many consecutive failures, the conductor resets to
    /// fresh state and re-evaluates from scratch.
    pub auto_restart_threshold: u32,
}

impl Default for SelfHealingPolicy {
    fn default() -> Self {
        Self {
            max_oscillations: 5,
            cooldown_ticks: 10,
            auto_restart_threshold: 3,
        }
    }
}

/// Tracks per-watcher oscillation and failure state for self-healing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelfHealingState {
    /// Per-watcher oscillation tracking: (watcher_name, was_firing_last_tick).
    watcher_states: Vec<(String, bool)>,
    /// Per-watcher oscillation counts.
    oscillation_counts: Vec<(String, u32)>,
    /// Watchers currently in cooldown with remaining ticks.
    cooldowns: Vec<(String, u64)>,
    /// Consecutive failure count (reset on success).
    consecutive_failures: u32,
    /// Whether the conductor has been auto-restarted.
    restarted: bool,
    /// Total number of self-healing interventions performed.
    total_interventions: u64,
}

/// Actions the self-healing subsystem can recommend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealingAction {
    /// Reset a specific watcher that is oscillating.
    ResetWatcher(String),
    /// Suppress interventions from a watcher for N ticks.
    CooldownWatcher {
        /// Name of the watcher to suppress.
        watcher: String,
        /// Number of ticks to suppress interventions for.
        ticks: u64,
    },
    /// Auto-restart the conductor with fresh state.
    AutoRestart,
    /// No healing action needed.
    None,
}

impl SelfHealingState {
    /// Create a new self-healing state tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tick observation for a watcher.
    ///
    /// `firing` indicates whether the watcher produced any outputs this tick.
    /// Returns a healing action if an oscillation threshold is exceeded.
    pub fn observe_watcher(
        &mut self,
        watcher: &str,
        firing: bool,
        policy: &SelfHealingPolicy,
    ) -> HealingAction {
        // Check if watcher is in cooldown.
        if self.is_in_cooldown(watcher) {
            return HealingAction::None;
        }

        // Find or create watcher state.
        let was_firing = self
            .watcher_states
            .iter()
            .find(|(name, _)| name == watcher)
            .map(|(_, was)| *was);

        // Update state.
        let found = self
            .watcher_states
            .iter_mut()
            .find(|(name, _)| name == watcher);
        if let Some(entry) = found {
            entry.1 = firing;
        } else {
            self.watcher_states.push((watcher.to_string(), firing));
        }

        // Detect oscillation (state changed since last tick).
        if let Some(was) = was_firing {
            if was != firing {
                let count = self.increment_oscillation(watcher);
                if count >= policy.max_oscillations {
                    self.total_interventions += 1;
                    self.enter_cooldown(watcher, policy.cooldown_ticks);
                    return HealingAction::ResetWatcher(watcher.to_string());
                }
            } else {
                // Stable — decay oscillation count.
                self.decay_oscillation(watcher);
            }
        }

        HealingAction::None
    }

    /// Record a conductor-level failure (e.g., a plan failed after intervention).
    ///
    /// Returns [`HealingAction::AutoRestart`] if the consecutive failure
    /// threshold is reached.
    pub fn record_failure(&mut self, policy: &SelfHealingPolicy) -> HealingAction {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= policy.auto_restart_threshold {
            self.consecutive_failures = 0;
            self.restarted = true;
            self.total_interventions += 1;
            HealingAction::AutoRestart
        } else {
            HealingAction::None
        }
    }

    /// Record a success — resets the consecutive failure counter.
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Advance cooldown timers by one tick.
    ///
    /// Returns watchers that exited cooldown.
    pub fn tick_cooldowns(&mut self) -> Vec<String> {
        let mut exited = Vec::new();
        for (name, remaining) in &mut self.cooldowns {
            *remaining = remaining.saturating_sub(1);
            if *remaining == 0 {
                exited.push(name.clone());
            }
        }
        self.cooldowns.retain(|(_, remaining)| *remaining > 0);
        exited
    }

    /// Whether a specific watcher is currently in cooldown.
    #[must_use]
    pub fn is_in_cooldown(&self, watcher: &str) -> bool {
        self.cooldowns
            .iter()
            .any(|(name, remaining)| name == watcher && *remaining > 0)
    }

    /// Current consecutive failure count.
    #[must_use]
    pub const fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Whether the conductor has been auto-restarted.
    #[must_use]
    pub const fn was_restarted(&self) -> bool {
        self.restarted
    }

    /// Total number of self-healing interventions.
    #[must_use]
    pub const fn total_interventions(&self) -> u64 {
        self.total_interventions
    }

    fn increment_oscillation(&mut self, watcher: &str) -> u32 {
        let found = self
            .oscillation_counts
            .iter_mut()
            .find(|(name, _)| name == watcher);
        if let Some(entry) = found {
            entry.1 += 1;
            entry.1
        } else {
            self.oscillation_counts.push((watcher.to_string(), 1));
            1
        }
    }

    fn decay_oscillation(&mut self, watcher: &str) {
        if let Some(entry) = self
            .oscillation_counts
            .iter_mut()
            .find(|(name, _)| name == watcher)
        {
            entry.1 = entry.1.saturating_sub(1);
        }
    }

    fn enter_cooldown(&mut self, watcher: &str, ticks: u64) {
        // Replace existing cooldown if present.
        if let Some(entry) = self.cooldowns.iter_mut().find(|(name, _)| name == watcher) {
            entry.1 = ticks;
        } else {
            self.cooldowns.push((watcher.to_string(), ticks));
        }
        // Reset oscillation count.
        if let Some(entry) = self
            .oscillation_counts
            .iter_mut()
            .find(|(name, _)| name == watcher)
        {
            entry.1 = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> SelfHealingPolicy {
        SelfHealingPolicy::default()
    }

    #[test]
    fn no_action_on_stable_watcher() {
        let mut state = SelfHealingState::new();
        let policy = default_policy();

        // First observation — no prior state.
        let action = state.observe_watcher("ghost-turn", true, &policy);
        assert_eq!(action, HealingAction::None);

        // Same state next tick — stable, no action.
        let action = state.observe_watcher("ghost-turn", true, &policy);
        assert_eq!(action, HealingAction::None);
    }

    #[test]
    fn oscillation_triggers_reset_after_threshold() {
        let mut state = SelfHealingState::new();
        let policy = SelfHealingPolicy {
            max_oscillations: 3,
            cooldown_ticks: 5,
            auto_restart_threshold: 10,
        };

        // Alternate: true, false, true, false, true, false
        let mut last_action = HealingAction::None;
        for i in 0..7 {
            last_action = state.observe_watcher("ghost-turn", i % 2 == 0, &policy);
            if matches!(last_action, HealingAction::ResetWatcher(_)) {
                break;
            }
        }
        assert!(
            matches!(last_action, HealingAction::ResetWatcher(ref w) if w == "ghost-turn"),
            "expected reset, got {last_action:?}"
        );
    }

    #[test]
    fn cooldown_suppresses_observations() {
        let mut state = SelfHealingState::new();
        let policy = SelfHealingPolicy {
            max_oscillations: 2,
            cooldown_ticks: 3,
            auto_restart_threshold: 10,
        };

        // Trigger oscillation reset.
        for i in 0..10 {
            state.observe_watcher("w", i % 2 == 0, &policy);
        }
        assert!(state.is_in_cooldown("w"));

        // During cooldown, observations are ignored.
        let action = state.observe_watcher("w", true, &policy);
        assert_eq!(action, HealingAction::None);
    }

    #[test]
    fn cooldown_expires_after_ticks() {
        let mut state = SelfHealingState::new();
        let policy = SelfHealingPolicy {
            max_oscillations: 2,
            cooldown_ticks: 3,
            auto_restart_threshold: 10,
        };

        // Trigger reset to enter cooldown.
        for i in 0..10 {
            state.observe_watcher("w", i % 2 == 0, &policy);
        }
        assert!(state.is_in_cooldown("w"));

        // Tick through cooldown.
        state.tick_cooldowns();
        assert!(state.is_in_cooldown("w"));
        state.tick_cooldowns();
        assert!(state.is_in_cooldown("w"));
        let exited = state.tick_cooldowns();
        assert!(!state.is_in_cooldown("w"));
        assert!(exited.contains(&"w".to_string()));
    }

    #[test]
    fn consecutive_failures_trigger_auto_restart() {
        let mut state = SelfHealingState::new();
        let policy = SelfHealingPolicy {
            max_oscillations: 5,
            cooldown_ticks: 10,
            auto_restart_threshold: 3,
        };

        assert_eq!(state.record_failure(&policy), HealingAction::None);
        assert_eq!(state.record_failure(&policy), HealingAction::None);
        assert_eq!(state.record_failure(&policy), HealingAction::AutoRestart);
        assert!(state.was_restarted());
        // Counter resets after restart.
        assert_eq!(state.consecutive_failures(), 0);
    }

    #[test]
    fn success_resets_failure_counter() {
        let mut state = SelfHealingState::new();
        let policy = default_policy();

        state.record_failure(&policy);
        state.record_failure(&policy);
        assert_eq!(state.consecutive_failures(), 2);

        state.record_success();
        assert_eq!(state.consecutive_failures(), 0);
    }

    #[test]
    fn total_interventions_counted() {
        let mut state = SelfHealingState::new();
        let policy = SelfHealingPolicy {
            max_oscillations: 2,
            cooldown_ticks: 1,
            auto_restart_threshold: 2,
        };

        // Trigger oscillation reset.
        for i in 0..10 {
            state.observe_watcher("w", i % 2 == 0, &policy);
        }
        // Trigger auto-restart.
        state.record_failure(&policy);
        state.record_failure(&policy);

        assert!(state.total_interventions() >= 2);
    }

    #[test]
    fn default_policy_values() {
        let policy = SelfHealingPolicy::default();
        assert_eq!(policy.max_oscillations, 5);
        assert_eq!(policy.cooldown_ticks, 10);
        assert_eq!(policy.auto_restart_threshold, 3);
    }
}

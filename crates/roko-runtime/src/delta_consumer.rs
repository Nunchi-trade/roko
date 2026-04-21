//! Delta consolidation loop consumer (BEAT-02).
//!

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::return_self_not_must_use,
    clippy::unnecessary_literal_bound,
    clippy::unused_self,
    clippy::derive_partial_eq_without_eq
)]

//!
//! Runs a three-phase dream cycle at configurable intervals, triggered by:
//! - Idle timeout (default 300s)
//! - Episode count threshold (default 50)
//! - Scheduled UTC time
//! - Explicit trigger
//!
//! The dream cycle is non-blocking: it spawns as a background task.
//!
//! Three phases:
//! 1. NREM replay (Mattar-Daw priority replay)
//! 2. REM imagination (counterfactual generation)
//! 3. Integration/staging (promote validated knowledge)

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::heartbeat::{CorticalState, Regime};

/// Configuration for the delta consolidation loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaConfig {
    /// Episode count that triggers a delta cycle.
    pub episode_threshold: u64,
    /// Idle timeout in seconds that triggers a delta cycle.
    pub idle_timeout_secs: u64,
    /// Optional scheduled UTC hour for nightly consolidation (e.g. "02:00").
    pub scheduled_utc: Option<String>,
    /// Maximum duration for a dream cycle before it is cancelled.
    pub max_cycle_duration_secs: u64,
    /// Minimum interval between delta cycles to prevent thrashing.
    pub min_cycle_interval_secs: u64,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            episode_threshold: 50,
            idle_timeout_secs: 300,
            scheduled_utc: None,
            max_cycle_duration_secs: 600,
            min_cycle_interval_secs: 60,
        }
    }
}

/// Trigger that caused a delta cycle to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaTrigger {
    /// Episode count threshold was reached.
    /// Episode count at trigger time.
    EpisodeThreshold {
        /// Episode count at trigger time.
        count: u64,
    },
    /// Idle timeout expired.
    /// Idle duration in seconds at trigger time.
    IdleTimeout {
        /// Idle duration in seconds.
        idle_secs: u64,
    },
    /// Scheduled nightly consolidation.
    /// Nightly scheduled consolidation.
    Scheduled {
        /// UTC hour that triggered the cycle.
        hour: u8,
    },
    /// Explicit operator or system trigger.
    /// Explicit trigger from operator or system.
    Explicit {
        /// Reason the explicit trigger was issued.
        reason: String,
    },
}

/// Status of a running or completed delta cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaCycleStatus {
    /// Cycle has not yet started.
    Idle,
    /// NREM replay phase is running.
    NremReplay,
    /// REM imagination phase is running.
    RemImagination,
    /// Integration/staging phase is running.
    Integration,
    /// Cycle completed successfully.
    Completed,
    /// Cycle was cancelled or timed out.
    /// Cycle was cancelled or timed out.
    Cancelled {
        /// Why the cycle was cancelled.
        reason: String,
    },
}

/// Report produced by a completed delta cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaCycleReport {
    /// UTC timestamp when the cycle started.
    pub started_at: DateTime<Utc>,
    /// UTC timestamp when the cycle completed.
    pub completed_at: DateTime<Utc>,
    /// What triggered this cycle.
    pub trigger: DeltaTrigger,
    /// NREM phase stats.
    pub nrem: NremPhaseReport,
    /// REM phase stats.
    pub rem: RemPhaseReport,
    /// Integration phase stats.
    pub integration: IntegrationPhaseReport,
}

/// NREM replay phase report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NremPhaseReport {
    /// Number of episodes replayed.
    pub episodes_replayed: usize,
    /// Number of patterns extracted.
    pub patterns_extracted: usize,
    /// Number of playbook rules compiled.
    pub rules_compiled: usize,
}

/// REM imagination phase report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemPhaseReport {
    /// Number of counterfactual scenarios generated.
    pub counterfactuals_generated: usize,
    /// Number of novel insights discovered.
    pub novel_insights: usize,
}

/// Integration/staging phase report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntegrationPhaseReport {
    /// Number of entries promoted from transient to working tier.
    pub entries_promoted: usize,
    /// Number of stale entries pruned.
    pub entries_pruned: usize,
    /// Number of entries at each confidence stage.
    pub stage_counts: Vec<(String, usize)>,
}

/// The delta consolidation loop consumer.
pub struct DeltaConsumer {
    config: DeltaConfig,
    last_cycle_ms: i64,
    episodes_since_last: u64,
    last_activity: Instant,
    current_status: DeltaCycleStatus,
    cycle_count: u64,
}

impl DeltaConsumer {
    /// Create a new delta consumer with the given configuration.
    pub fn new(config: DeltaConfig) -> Self {
        Self {
            config,
            last_cycle_ms: 0,
            episodes_since_last: 0,
            last_activity: Instant::now(),
            current_status: DeltaCycleStatus::Idle,
            cycle_count: 0,
        }
    }

    /// Access the configuration.
    pub const fn config(&self) -> &DeltaConfig {
        &self.config
    }

    /// Millisecond timestamp of the last delta cycle.
    pub const fn last_cycle_ms(&self) -> i64 {
        self.last_cycle_ms
    }

    /// Current status of the delta consumer.
    pub fn status(&self) -> &DeltaCycleStatus {
        &self.current_status
    }

    /// Number of completed dream cycles.
    pub const fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    /// Record that an episode was completed (increments the trigger counter).
    pub fn record_episode(&mut self) {
        self.episodes_since_last += 1;
        self.last_activity = Instant::now();
    }

    /// Record that activity occurred (resets idle timer).
    pub fn record_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check whether a delta cycle should be triggered.
    pub fn should_trigger(&self) -> Option<DeltaTrigger> {
        // Don't trigger if a cycle is already running
        if !matches!(
            self.current_status,
            DeltaCycleStatus::Idle | DeltaCycleStatus::Completed
        ) {
            return None;
        }

        // Respect minimum interval
        if self.last_cycle_ms > 0 {
            let now_ms = Utc::now().timestamp_millis();
            let min_interval_ms = self.config.min_cycle_interval_secs as i64 * 1000;
            if now_ms - self.last_cycle_ms < min_interval_ms {
                return None;
            }
        }

        // Check episode threshold
        if self.episodes_since_last >= self.config.episode_threshold {
            return Some(DeltaTrigger::EpisodeThreshold {
                count: self.episodes_since_last,
            });
        }

        // Check idle timeout
        let idle_duration = self.last_activity.elapsed();
        if idle_duration >= Duration::from_secs(self.config.idle_timeout_secs) {
            return Some(DeltaTrigger::IdleTimeout {
                idle_secs: idle_duration.as_secs(),
            });
        }

        None
    }

    /// Run the three-phase dream cycle synchronously.
    ///
    /// In production this should be spawned via `tokio::spawn` so it does not
    /// block the gamma or theta loops.
    pub fn run_cycle(
        &mut self,
        trigger: DeltaTrigger,
        _cortical: &CorticalState,
    ) -> DeltaCycleReport {
        let started_at = Utc::now();

        // Phase 1: NREM replay
        self.current_status = DeltaCycleStatus::NremReplay;
        let nrem = self.run_nrem_phase();

        // Phase 2: REM imagination
        self.current_status = DeltaCycleStatus::RemImagination;
        let rem = self.run_rem_phase();

        // Phase 3: Integration/staging
        self.current_status = DeltaCycleStatus::Integration;
        let integration = self.run_integration_phase();

        // Mark cycle complete
        self.current_status = DeltaCycleStatus::Completed;
        self.last_cycle_ms = Utc::now().timestamp_millis();
        self.episodes_since_last = 0;
        self.cycle_count += 1;

        DeltaCycleReport {
            started_at,
            completed_at: Utc::now(),
            trigger,
            nrem,
            rem,
            integration,
        }
    }

    /// Phase 1: NREM replay — utility-based replay of high-value episodes.
    ///
    /// This is a stub that will be connected to `roko-dreams::replay` when the
    /// dream runner is wired.
    fn run_nrem_phase(&self) -> NremPhaseReport {
        // The actual implementation will call:
        // - roko_dreams::replay::select_replay_episodes()
        // - roko_dreams::replay::compute_replay_utility()
        // For now, return empty stats that downstream consumers can handle.
        NremPhaseReport {
            episodes_replayed: 0,
            patterns_extracted: 0,
            rules_compiled: 0,
        }
    }

    /// Phase 2: REM imagination — combinational/exploratory creativity.
    ///
    /// This is a stub that will be connected to `roko-dreams::imagination` when
    /// the dream runner is wired.
    fn run_rem_phase(&self) -> RemPhaseReport {
        // The actual implementation will call:
        // - roko_dreams::imagination::CausalModel::from_episodes()
        // - Generate counterfactual scenarios via Pearl SCM
        RemPhaseReport {
            counterfactuals_generated: 0,
            novel_insights: 0,
        }
    }

    /// Phase 3: Integration/staging — promote validated knowledge.
    ///
    /// This is a stub that will be connected to `roko-dreams::staging` and
    /// `roko-neuro::knowledge_store` when those systems are wired.
    fn run_integration_phase(&self) -> IntegrationPhaseReport {
        // The actual implementation will call:
        // - roko_dreams::staging::StagingBuffer::promote_validated()
        // - roko_neuro::knowledge_store promote_tier()
        IntegrationPhaseReport {
            entries_promoted: 0,
            entries_pruned: 0,
            stage_counts: vec![],
        }
    }

    /// Check whether the system is in a low-activity state suitable for dreaming.
    pub fn is_low_activity(cortical: &CorticalState) -> bool {
        let snapshot = cortical.snapshot();
        matches!(snapshot.regime, Regime::Calm | Regime::Normal) && snapshot.pad.arousal < 0.3
    }
}

impl Default for DeltaConsumer {
    fn default() -> Self {
        Self::new(DeltaConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_consumer_triggers_on_episode_threshold() {
        let mut consumer = DeltaConsumer::new(DeltaConfig {
            episode_threshold: 3,
            idle_timeout_secs: 9999,
            ..DeltaConfig::default()
        });

        assert!(consumer.should_trigger().is_none());
        consumer.record_episode();
        consumer.record_episode();
        assert!(consumer.should_trigger().is_none());
        consumer.record_episode();
        let trigger = consumer.should_trigger();
        assert!(matches!(
            trigger,
            Some(DeltaTrigger::EpisodeThreshold { count: 3 })
        ));
    }

    #[test]
    fn delta_consumer_runs_cycle() {
        let mut consumer = DeltaConsumer::default();
        let cortical = CorticalState::default();
        let trigger = DeltaTrigger::Explicit {
            reason: "test".into(),
        };
        let report = consumer.run_cycle(trigger, &cortical);
        assert!(report.completed_at >= report.started_at);
        assert_eq!(consumer.cycle_count(), 1);
        assert_eq!(consumer.episodes_since_last, 0);
    }

    #[test]
    fn delta_consumer_respects_min_interval() {
        let mut consumer = DeltaConsumer::new(DeltaConfig {
            episode_threshold: 1,
            min_cycle_interval_secs: 9999,
            ..DeltaConfig::default()
        });

        // Run a cycle first
        let cortical = CorticalState::default();
        let trigger = DeltaTrigger::Explicit {
            reason: "first".into(),
        };
        consumer.run_cycle(trigger, &cortical);

        // Now recording an episode shouldn't trigger because of min interval
        consumer.record_episode();
        assert!(consumer.should_trigger().is_none());
    }

    #[test]
    fn delta_consumer_status_transitions() {
        let consumer = DeltaConsumer::default();
        assert!(matches!(consumer.status(), DeltaCycleStatus::Idle));
    }

    #[test]
    fn delta_low_activity_check() {
        let cortical = CorticalState::default();
        // Default is Calm with moderate arousal
        assert!(DeltaConsumer::is_low_activity(&cortical));
    }
}

//! STATUS: WIRED -- called from `roko-serve::start_demurrage_timer()`.
//!
//! Demurrage heartbeat consumer (LIFE-10).
//!
//! Runs periodic knowledge demurrage at configurable intervals within the
//! Theta/Delta heartbeat loop. Each tick increments an iteration counter;
//! when `validation_interval` iterations elapse, the consumer applies
//! confidence decay to all supplied knowledge entries and emits a
//! demurrage event for observability.
//!
//! This module is self-contained within `roko-runtime` (no dependency on
//! `roko-agent`). The orchestrator wires the consumer by feeding it
//! knowledge entries from the Neuro store on each tick.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_const_for_fn,
    clippy::unused_self,
    clippy::derive_partial_eq_without_eq
)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Configuration ───��──────────────────────────────────────────────────

/// Demurrage configuration controlling how often and how aggressively
/// knowledge confidence decays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageConsumerConfig {
    /// Number of heartbeat iterations between demurrage passes.
    /// Default: 250 (~2.9 hours at 1 iteration/40s).
    pub validation_interval: u64,
    /// Confidence reduction per missed interval.
    /// Default: 0.03 (3%).
    pub decay_per_interval: f64,
    /// Below this confidence, entries are flagged for archival.
    /// Default: 0.1.
    pub archive_threshold: f64,
    /// Domain-specific decay multipliers. Volatile domains decay faster.
    /// Example: `gas_patterns` = 2.0, `protocol_behavior` = 0.5.
    pub domain_multipliers: HashMap<String, f64>,
}

impl Default for DemurrageConsumerConfig {
    fn default() -> Self {
        let mut domain_multipliers = HashMap::new();
        domain_multipliers.insert("gas_patterns".into(), 2.0);
        domain_multipliers.insert("price_direction".into(), 1.5);
        domain_multipliers.insert("volatility_regime".into(), 1.0);
        domain_multipliers.insert("yield_trends".into(), 0.8);
        domain_multipliers.insert("protocol_behavior".into(), 0.5);

        Self {
            validation_interval: 250,
            decay_per_interval: 0.03,
            archive_threshold: 0.1,
            domain_multipliers,
        }
    }
}

// ─── Knowledge entry abstraction ────────────────────────────────────────

/// Minimal knowledge entry representation for demurrage processing.
///
/// This is domain-agnostic so `roko-runtime` does not depend on `roko-neuro`
/// or `roko-agent`. The orchestrator converts between concrete Engram types
/// and this representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageEntry {
    /// Unique identifier (hash).
    pub id: String,
    /// Current effective confidence.
    pub confidence: f64,
    /// Domain tag used for multiplier lookup (e.g. "gas_patterns").
    pub domain: String,
    /// Iteration at which this entry was last validated.
    pub last_validated_at: u64,
    /// Whether this entry was re-validated since the last demurrage pass.
    pub validated_since_last: bool,
}

// ─── Report ──��──────────────────────────────���───────────────────────────

/// Report emitted after each demurrage pass for observability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageEvent {
    /// UTC timestamp of this demurrage pass.
    pub timestamp: DateTime<Utc>,
    /// Heartbeat iteration at which demurrage was applied.
    pub iteration: u64,
    /// Number of entries processed.
    pub entries_processed: u32,
    /// Number of entries decayed (confidence reduced).
    pub entries_decayed: u32,
    /// Number of entries flagged for archival (below threshold).
    pub entries_archived: u32,
    /// Total confidence lost across all entries.
    pub total_confidence_lost: f64,
    /// Average confidence after decay.
    pub average_confidence_after: f64,
}

// ─── Consumer ────────���──────────────────────────────────────────────────

/// Demurrage heartbeat consumer that periodically applies knowledge decay.
///
/// Wire this into the Theta or Delta heartbeat loop. Each call to [`tick`]
/// increments the iteration counter. When `validation_interval` iterations
/// elapse since the last demurrage pass, the consumer processes the supplied
/// entries and returns a [`DemurrageEvent`].
///
/// # Usage
///
/// ```ignore
/// let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig::default());
///
/// // In the heartbeat loop:
/// if let Some((updated_entries, event)) = consumer.tick(&entries) {
///     // Write updated entries back to the knowledge store
///     // Emit event to efficiency log
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageConsumer {
    /// Demurrage configuration.
    pub config: DemurrageConsumerConfig,
    /// Current iteration counter (incremented on each tick).
    pub iteration: u64,
    /// Iteration at which the last demurrage pass ran.
    pub last_demurrage_at: u64,
    /// Cumulative entries archived across all cycles.
    pub total_archived: u64,
    /// Cumulative confidence lost across all cycles.
    pub total_confidence_lost: f64,
    /// Total demurrage passes completed.
    pub total_passes: u64,
}

impl DemurrageConsumer {
    /// Create a new demurrage consumer from configuration.
    pub fn new(config: DemurrageConsumerConfig) -> Self {
        Self {
            config,
            iteration: 0,
            last_demurrage_at: 0,
            total_archived: 0,
            total_confidence_lost: 0.0,
            total_passes: 0,
        }
    }

    /// Current iteration count.
    pub const fn iteration(&self) -> u64 {
        self.iteration
    }

    /// Iterations until the next demurrage pass.
    pub fn next_demurrage_in(&self) -> u64 {
        if self.config.validation_interval == 0 {
            return u64::MAX;
        }
        let elapsed = self.iteration - self.last_demurrage_at;
        self.config.validation_interval.saturating_sub(elapsed)
    }

    /// Return true if demurrage is due on the next tick.
    pub fn is_due(&self) -> bool {
        self.next_demurrage_in() <= 1
    }

    /// Advance the iteration counter and, if a validation interval has
    /// elapsed, apply demurrage to all supplied entries.
    ///
    /// Returns `Some((updated_entries, event))` when demurrage was applied,
    /// `None` otherwise.
    pub fn tick(
        &mut self,
        entries: &[DemurrageEntry],
    ) -> Option<(Vec<DemurrageEntry>, DemurrageEvent)> {
        self.iteration += 1;

        if self.config.validation_interval == 0 {
            return None;
        }

        let elapsed = self.iteration - self.last_demurrage_at;
        if elapsed < self.config.validation_interval {
            return None;
        }

        // Apply demurrage to all un-validated entries.
        let mut updated = Vec::with_capacity(entries.len());
        let mut entries_decayed = 0u32;
        let mut entries_archived = 0u32;
        let mut total_confidence_lost = 0.0f64;
        let mut total_confidence_after = 0.0f64;

        for entry in entries {
            let mut new_entry = entry.clone();

            if entry.validated_since_last {
                // Re-validated entries are not decayed.
                new_entry.validated_since_last = false;
                total_confidence_after += new_entry.confidence;
                updated.push(new_entry);
                continue;
            }

            // Look up domain multiplier (default 1.0).
            let multiplier = self
                .config
                .domain_multipliers
                .get(&entry.domain)
                .copied()
                .unwrap_or(1.0);

            let decay = self.config.decay_per_interval * multiplier;
            let old_confidence = new_entry.confidence;
            new_entry.confidence = (new_entry.confidence - decay).max(0.0);
            new_entry.last_validated_at = self.iteration;

            let lost = old_confidence - new_entry.confidence;
            if lost > 0.0 {
                entries_decayed += 1;
                total_confidence_lost += lost;
            }

            if new_entry.confidence < self.config.archive_threshold {
                entries_archived += 1;
            }

            total_confidence_after += new_entry.confidence;
            updated.push(new_entry);
        }

        let entries_processed = entries.len() as u32;
        let average_confidence_after = if entries_processed > 0 {
            total_confidence_after / f64::from(entries_processed)
        } else {
            0.0
        };

        let event = DemurrageEvent {
            timestamp: Utc::now(),
            iteration: self.iteration,
            entries_processed,
            entries_decayed,
            entries_archived,
            total_confidence_lost,
            average_confidence_after,
        };

        self.last_demurrage_at = self.iteration;
        self.total_archived += u64::from(entries_archived);
        self.total_confidence_lost += total_confidence_lost;
        self.total_passes += 1;

        Some((updated, event))
    }
}

impl Default for DemurrageConsumer {
    fn default() -> Self {
        Self::new(DemurrageConsumerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, confidence: f64, domain: &str) -> DemurrageEntry {
        DemurrageEntry {
            id: id.into(),
            confidence,
            domain: domain.into(),
            last_validated_at: 0,
            validated_since_last: false,
        }
    }

    #[test]
    fn no_demurrage_before_interval() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 10,
            ..DemurrageConsumerConfig::default()
        });
        let entries = vec![make_entry("a", 0.9, "general")];

        // Tick 9 times — no demurrage yet.
        for _ in 0..9 {
            assert!(consumer.tick(&entries).is_none());
        }
        assert_eq!(consumer.iteration(), 9);
    }

    #[test]
    fn demurrage_fires_at_interval() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 5,
            decay_per_interval: 0.1,
            archive_threshold: 0.1,
            domain_multipliers: HashMap::new(),
        });
        let entries = vec![make_entry("a", 0.9, "general")];

        // First 4 ticks: no demurrage.
        for _ in 0..4 {
            assert!(consumer.tick(&entries).is_none());
        }

        // 5th tick: demurrage fires.
        let result = consumer.tick(&entries);
        assert!(result.is_some());
        let (updated, event) = result.unwrap();
        assert_eq!(event.entries_processed, 1);
        assert_eq!(event.entries_decayed, 1);
        assert!((updated[0].confidence - 0.8).abs() < 1e-6);
    }

    #[test]
    fn domain_multiplier_applies() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 1,
            decay_per_interval: 0.1,
            archive_threshold: 0.0,
            domain_multipliers: {
                let mut m = HashMap::new();
                m.insert("volatile".into(), 2.0);
                m
            },
        });
        let entries = vec![
            make_entry("normal", 1.0, "general"),
            make_entry("vol", 1.0, "volatile"),
        ];

        let (updated, _) = consumer.tick(&entries).unwrap();
        // Normal domain: decay = 0.1 * 1.0 = 0.1 -> 0.9
        assert!((updated[0].confidence - 0.9).abs() < 1e-6);
        // Volatile domain: decay = 0.1 * 2.0 = 0.2 -> 0.8
        assert!((updated[1].confidence - 0.8).abs() < 1e-6);
    }

    #[test]
    fn validated_entries_skip_decay() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 1,
            decay_per_interval: 0.1,
            archive_threshold: 0.0,
            domain_multipliers: HashMap::new(),
        });
        let entries = vec![DemurrageEntry {
            id: "validated".into(),
            confidence: 0.9,
            domain: "general".into(),
            last_validated_at: 0,
            validated_since_last: true,
        }];

        let (updated, event) = consumer.tick(&entries).unwrap();
        assert_eq!(event.entries_decayed, 0);
        assert!((updated[0].confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn archival_flagging() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 1,
            decay_per_interval: 0.05,
            archive_threshold: 0.1,
            domain_multipliers: HashMap::new(),
        });
        let entries = vec![
            make_entry("low", 0.12, "general"), // 0.12 - 0.05 = 0.07 < 0.1 -> archived
            make_entry("high", 0.8, "general"), // 0.8 - 0.05 = 0.75 > 0.1 -> not archived
        ];

        let (_, event) = consumer.tick(&entries).unwrap();
        assert_eq!(event.entries_archived, 1);
        assert_eq!(event.entries_decayed, 2);
    }

    #[test]
    fn cumulative_stats_track_across_passes() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 1,
            decay_per_interval: 0.05,
            archive_threshold: 0.0,
            domain_multipliers: HashMap::new(),
        });
        let entries = vec![make_entry("a", 0.5, "general")];

        consumer.tick(&entries).unwrap();
        consumer.tick(&entries).unwrap();

        assert_eq!(consumer.total_passes, 2);
        assert!(consumer.total_confidence_lost > 0.0);
    }

    #[test]
    fn zero_interval_disables_demurrage() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 0,
            ..DemurrageConsumerConfig::default()
        });
        let entries = vec![make_entry("a", 0.5, "general")];

        for _ in 0..100 {
            assert!(consumer.tick(&entries).is_none());
        }
    }

    #[test]
    fn next_demurrage_in_counts_down() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 10,
            ..DemurrageConsumerConfig::default()
        });
        let entries = vec![make_entry("a", 0.5, "general")];

        assert_eq!(consumer.next_demurrage_in(), 10);

        consumer.tick(&entries);
        assert_eq!(consumer.next_demurrage_in(), 9);

        // Advance to trigger.
        for _ in 0..9 {
            consumer.tick(&entries);
        }
        // After firing, countdown resets.
        assert_eq!(consumer.next_demurrage_in(), 10);
    }

    #[test]
    fn is_due_on_last_tick_before_interval() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 3,
            ..DemurrageConsumerConfig::default()
        });
        let entries = vec![make_entry("a", 0.5, "general")];

        consumer.tick(&entries); // iter 1, next in 2
        assert!(!consumer.is_due());
        consumer.tick(&entries); // iter 2, next in 1
        assert!(consumer.is_due());
    }

    #[test]
    fn event_has_correct_fields() {
        let mut consumer = DemurrageConsumer::new(DemurrageConsumerConfig {
            validation_interval: 1,
            decay_per_interval: 0.1,
            archive_threshold: 0.05,
            domain_multipliers: HashMap::new(),
        });
        let entries = vec![
            make_entry("a", 0.5, "general"),
            make_entry("b", 0.08, "general"),
        ];

        let (_, event) = consumer.tick(&entries).unwrap();
        assert_eq!(event.iteration, 1);
        assert_eq!(event.entries_processed, 2);
        assert!(event.total_confidence_lost > 0.0);
        assert!(event.average_confidence_after > 0.0);
    }
}

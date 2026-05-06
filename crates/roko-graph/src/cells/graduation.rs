//! GraduationCell -- a React Cell that promotes qualifying Pulses to Signals.
//!
//! GraduationCell runs as a background React Cell in the Engine. On each
//! tick it:
//! 1. Receives Pulses from the Bus (via `decide_with_pulses`)
//! 2. Evaluates each Pulse against the configured GraduationPolicies
//! 3. Calls `Pulse::graduate()` for matching Pulses
//! 4. Outputs the graduated Signals for the Engine to persist via the Store
//!
//! # Policy precedence
//!
//! Policy evaluation is delegated to [`GraduationConfig::should_graduate`],
//! which scans all matching policies before deciding. A `never` policy
//! blocks an `always` policy for the same topic. See
//! `crates/roko-core/src/config/graduation.rs` for the full precedence rules.
//!
//! # TODO
//!
//! - `roko learn graduation` subcommand for inspecting graduation stats
//!   (deferred; not part of P3-5/P3-6/P3-7).

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use roko_core::config::graduation::GraduationConfig;
use roko_core::traits::React;
use roko_core::{Context, Engram, PolicyOutputs, Provenance, Pulse, Score};

use crate::cell::{Cell, CellContext, CellVersion};

/// Watches the Bus and promotes qualifying Pulses to Signals.
///
/// Register this cell with the Engine at startup to enable automatic
/// graduation. The policies are loaded from `roko.toml` `[graduation]`
/// section via [`GraduationConfig`].
pub struct GraduationCell {
    /// Configured graduation policies.
    config: GraduationConfig,
    /// Rolling counter used for telemetry (not for sampling -- sampling
    /// uses `pulse.seq` for determinism across restarts).
    pulse_counter: AtomicU64,
    /// Provenance tag attached to all graduated Signals.
    provenance_tag: String,
}

impl GraduationCell {
    /// Create a new GraduationCell with the given policies.
    #[must_use]
    pub fn new(config: GraduationConfig) -> Self {
        Self {
            config,
            pulse_counter: AtomicU64::new(0),
            provenance_tag: "graduation-policy".into(),
        }
    }

    /// Create a GraduationCell with the default v2 spec policies.
    #[must_use]
    pub fn with_default_policies() -> Self {
        Self::new(GraduationConfig::default_policies())
    }

    /// Evaluate a single Pulse against all policies and return whether it
    /// should graduate.
    ///
    /// Sampling uses `pulse.seq` for deterministic behavior across restarts
    /// and tests.
    #[must_use]
    pub fn should_graduate(&self, pulse: &Pulse) -> bool {
        self.config.should_graduate(&pulse.topic, pulse.seq)
    }

    /// Graduate a Pulse to a Signal with the cell's default provenance and
    /// a neutral score (downstream scorers will re-score based on content).
    fn graduate_pulse(&self, pulse: &Pulse) -> Engram {
        let score = Score::NEUTRAL;
        let provenance = Provenance::trusted(self.provenance_tag.clone());
        pulse.graduate(provenance, score)
    }

    /// Access the graduation config.
    #[must_use]
    pub fn config(&self) -> &GraduationConfig {
        &self.config
    }

    /// Total pulses processed (telemetry counter).
    #[must_use]
    pub fn pulses_processed(&self) -> u64 {
        self.pulse_counter.load(Ordering::Relaxed)
    }
}

// ---- roko-graph Cell trait ------------------------------------------------

#[async_trait]
impl Cell for GraduationCell {
    fn cell_id(&self) -> &'static str {
        "graduation-policy"
    }

    fn cell_name(&self) -> &'static str {
        "Graduation Policy"
    }

    fn cell_version(&self) -> CellVersion {
        (0, 1, 0)
    }

    fn protocols(&self) -> &[&str] {
        &["React"]
    }

    async fn execute(
        &self,
        input: Vec<Engram>,
        _ctx: &CellContext,
    ) -> roko_core::error::Result<Vec<Engram>> {
        // In graph execution mode, we just pass through input engrams.
        // The real graduation work happens via decide_with_pulses() when
        // the Bus delivers pulses.
        Ok(input)
    }
}

// ---- roko-core React trait ------------------------------------------------

impl React for GraduationCell {
    fn decide(&self, _stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        // React::decide only receives Signals; graduation is pulse-driven.
        // Return empty -- the real work happens in decide_with_pulses.
        Vec::new()
    }

    fn decide_with_pulses(
        &self,
        _signals: &[Engram],
        pulses: &[Pulse],
        _ctx: &Context,
    ) -> PolicyOutputs {
        let mut graduated = Vec::new();

        for pulse in pulses {
            self.pulse_counter.fetch_add(1, Ordering::Relaxed);
            if self.should_graduate(pulse) {
                graduated.push(self.graduate_pulse(pulse));
            }
        }

        PolicyOutputs {
            engrams: graduated,
            pulses: Vec::new(),
        }
    }

    fn name(&self) -> &'static str {
        "graduation-policy"
    }
}

// ---- Tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::graduation::{GraduationConfig, GraduationPolicy};
    use roko_core::pulse::{Topic, TopicFilter};
    use roko_core::{Body, Kind};

    fn make_pulse(topic: &str, seq: u64) -> Pulse {
        Pulse::builder(seq, Topic::new(topic), Kind::GateVerdict)
            .body(Body::text("test"))
            .created_at_ms(0)
            .build()
    }

    #[test]
    fn always_policy_graduates_matching_pulses() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("gate.verdict.".into()),
                always: true,
                never: false,
                sample_every: 1,
            }],
        };
        let cell = GraduationCell::new(config);
        let pulse = make_pulse("gate.verdict.emitted", 1);
        assert!(cell.should_graduate(&pulse));
    }

    #[test]
    fn never_policy_blocks_matching_pulses() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("heartbeat.".into()),
                always: false,
                never: true,
                sample_every: 1,
            }],
        };
        let cell = GraduationCell::new(config);
        let pulse = make_pulse("heartbeat.tick", 1);
        assert!(!cell.should_graduate(&pulse));
    }

    #[test]
    fn no_matching_policy_does_not_graduate() {
        let cell = GraduationCell::with_default_policies();
        let pulse = make_pulse("agent.output.token", 1);
        // "agent.output.*" is not in the default policy list.
        // Default = do not graduate.
        assert!(!cell.should_graduate(&pulse));
    }

    #[test]
    fn decide_with_pulses_promotes_to_engrams() {
        let cell = GraduationCell::with_default_policies();
        let ctx = Context::default();

        let graduating = make_pulse("gate.verdict.emitted", 1);
        let blocked = make_pulse("heartbeat.tick", 2);

        let outputs = cell.decide_with_pulses(&[], &[graduating, blocked], &ctx);

        assert_eq!(outputs.engrams.len(), 1);
        assert!(outputs.pulses.is_empty());
    }

    #[test]
    fn graduate_method_preserves_kind_and_body() {
        let cell = GraduationCell::with_default_policies();
        let pulse = make_pulse("gate.verdict.emitted", 1);
        let signal = cell.graduate_pulse(&pulse);

        assert_eq!(signal.kind, Kind::GateVerdict);
        assert_eq!(signal.body, Body::text("test"));
    }

    #[test]
    fn graduated_signal_has_audit_tags() {
        let cell = GraduationCell::with_default_policies();
        let pulse = make_pulse("gate.verdict.emitted", 42);
        let signal = cell.graduate_pulse(&pulse);

        assert_eq!(signal.tag("pulse_topic"), Some("gate.verdict.emitted"));
        assert_eq!(signal.tag("pulse_seq"), Some("42"));
    }

    #[test]
    fn pulse_counter_tracks_processed_count() {
        let cell = GraduationCell::with_default_policies();
        let ctx = Context::default();
        assert_eq!(cell.pulses_processed(), 0);

        let pulses = vec![
            make_pulse("gate.verdict.emitted", 1),
            make_pulse("heartbeat.tick", 2),
            make_pulse("safety.approval.granted", 3),
        ];

        let _ = cell.decide_with_pulses(&[], &pulses, &ctx);
        assert_eq!(cell.pulses_processed(), 3);
    }

    #[test]
    fn decide_without_pulses_returns_empty() {
        let cell = GraduationCell::with_default_policies();
        let ctx = Context::default();
        let result = cell.decide(&[], &ctx);
        assert!(result.is_empty());
    }

    #[test]
    fn cell_trait_metadata() {
        let cell = GraduationCell::with_default_policies();
        assert_eq!(cell.cell_id(), "graduation-policy");
        assert_eq!(cell.cell_name(), "Graduation Policy");
        assert_eq!(cell.protocols(), &["React"]);
    }

    #[test]
    fn never_overrides_always_in_cell() {
        let config = GraduationConfig {
            policies: vec![
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.".into()),
                    always: false,
                    never: true,
                    sample_every: 1,
                },
            ],
        };
        let cell = GraduationCell::new(config);
        let pulse = make_pulse("gate.verdict.emitted", 1);
        // never must win over always
        assert!(!cell.should_graduate(&pulse));
    }

    #[test]
    fn sampling_uses_pulse_seq_for_determinism() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("metric.".into()),
                always: false,
                never: false,
                sample_every: 5,
            }],
        };
        let cell = GraduationCell::new(config);

        // seq 0: 0 % 5 == 0 -> graduate
        assert!(cell.should_graduate(&make_pulse("metric.cpu", 0)));
        // seq 1: 1 % 5 != 0 -> skip
        assert!(!cell.should_graduate(&make_pulse("metric.cpu", 1)));
        // seq 5: 5 % 5 == 0 -> graduate
        assert!(cell.should_graduate(&make_pulse("metric.cpu", 5)));
    }
}

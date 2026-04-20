//! Verdict-as-signal reentry — publishes gate verdicts as `Pulse` events.
//!
//! When a gate pipeline produces a verdict, the publisher converts it into a
//! [`Pulse`] with `Kind::GateVerdict` and a topic like `gate.verdict.emitted`.
//! This enables downstream consumers (learning, conductor, router) to react to
//! verdicts in real time without polling.
//!
//! The publisher is optional: callers supply a callback via
//! [`VerdictPublisher`]. When no publisher is configured, verdicts are
//! produced but not broadcast.

use roko_core::{Body, Kind, Pulse, Topic, Verdict};
use serde::Serialize;
use std::sync::Arc;

/// Summary of a verdict suitable for Pulse body serialization.
#[derive(Clone, Debug, Serialize)]
pub struct VerdictSummary {
    /// Gate that produced the verdict.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Numeric quality score.
    pub score: f32,
    /// Human-readable reason (empty on pass).
    pub reason: String,
    /// Optional rung index.
    pub rung: Option<u32>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

impl VerdictSummary {
    /// Create a summary from a verdict and optional rung index.
    #[must_use]
    pub fn from_verdict(verdict: &Verdict, rung: Option<u32>) -> Self {
        Self {
            gate: verdict.gate.clone(),
            passed: verdict.passed,
            score: verdict.score,
            reason: verdict.reason.clone(),
            rung,
            duration_ms: verdict.duration_ms,
        }
    }
}

/// Callback type for publishing verdict pulses.
///
/// The callback receives a fully constructed `Pulse`. Implementations may
/// forward it to a `PulseBus`, log it, or feed it to downstream consumers.
pub type VerdictPublishFn = dyn Fn(Pulse) + Send + Sync;

/// Optional verdict publisher that wraps a callback.
///
/// Attach to a gate pipeline or rung dispatch to broadcast verdicts.
#[derive(Clone)]
pub struct VerdictPublisher {
    callback: Arc<VerdictPublishFn>,
    /// Monotonic sequence counter for pulses.
    seq: Arc<std::sync::atomic::AtomicU64>,
}

impl VerdictPublisher {
    /// Create a new publisher with the given callback.
    pub fn new(callback: Arc<VerdictPublishFn>) -> Self {
        Self {
            callback,
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Publish a verdict as a `Pulse`.
    ///
    /// The pulse is created with `Kind::GateVerdict` and topic
    /// `gate.verdict.emitted`. The verdict summary is serialized into the
    /// pulse body.
    pub fn publish(&self, verdict: &Verdict, rung: Option<u32>) {
        let summary = VerdictSummary::from_verdict(verdict, rung);
        let body = Body::from_json(&summary).unwrap_or_else(|_| {
            Body::text(format!(
                "gate={} passed={} score={}",
                summary.gate, summary.passed, summary.score
            ))
        });
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pulse = Pulse::new(
            seq,
            Topic::new("gate.verdict.emitted"),
            Kind::GateVerdict,
            body,
        );
        (self.callback)(pulse);
    }

    /// Publish multiple verdicts from a pipeline run.
    pub fn publish_all(&self, verdicts: &[Verdict], rung: Option<u32>) {
        for verdict in verdicts {
            self.publish(verdict, rung);
        }
    }
}

impl std::fmt::Debug for VerdictPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerdictPublisher")
            .field("seq", &self.seq.load(std::sync::atomic::Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn make_verdict(gate: &str, passed: bool) -> Verdict {
        if passed {
            Verdict::pass(gate).with_duration(42)
        } else {
            Verdict::fail(gate, "boom").with_duration(100)
        }
    }

    #[test]
    fn publish_creates_pulse_with_gate_verdict_kind() {
        let pulses: Arc<Mutex<Vec<Pulse>>> = Arc::new(Mutex::new(Vec::new()));
        let pulses_clone = Arc::clone(&pulses);
        let publisher = VerdictPublisher::new(Arc::new(move |pulse| {
            pulses_clone.lock().unwrap().push(pulse);
        }));

        let verdict = make_verdict("compile", true);
        publisher.publish(&verdict, Some(0));

        let captured = pulses.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].kind, Kind::GateVerdict);
        assert_eq!(captured[0].topic, Topic::new("gate.verdict.emitted"));
    }

    #[test]
    fn publish_all_emits_one_pulse_per_verdict() {
        let count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let count_clone = Arc::clone(&count);
        let publisher = VerdictPublisher::new(Arc::new(move |_pulse| {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));

        let verdicts = vec![
            make_verdict("compile", true),
            make_verdict("lint", true),
            make_verdict("test", false),
        ];
        publisher.publish_all(&verdicts, Some(2));
        assert_eq!(count.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[test]
    fn sequence_numbers_increment() {
        let seqs: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let seqs_clone = Arc::clone(&seqs);
        let publisher = VerdictPublisher::new(Arc::new(move |pulse| {
            seqs_clone.lock().unwrap().push(pulse.seq);
        }));

        publisher.publish(&make_verdict("a", true), None);
        publisher.publish(&make_verdict("b", true), None);
        publisher.publish(&make_verdict("c", false), None);

        let captured = seqs.lock().unwrap();
        assert_eq!(*captured, vec![0, 1, 2]);
    }

    #[test]
    fn summary_captures_verdict_fields() {
        let verdict = Verdict::fail("test", "3 tests failed").with_duration(500);
        let summary = VerdictSummary::from_verdict(&verdict, Some(2));
        assert_eq!(summary.gate, "test");
        assert!(!summary.passed);
        assert_eq!(summary.reason, "3 tests failed");
        assert_eq!(summary.rung, Some(2));
        assert_eq!(summary.duration_ms, 500);
    }

    #[test]
    fn debug_impl_works() {
        let publisher = VerdictPublisher::new(Arc::new(|_| {}));
        let debug = format!("{publisher:?}");
        assert!(debug.contains("VerdictPublisher"));
    }
}

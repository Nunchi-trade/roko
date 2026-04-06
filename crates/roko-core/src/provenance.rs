//! Provenance tracking — who produced a signal, and should we trust it?
//!
//! Every signal carries a [`Provenance`] record that answers:
//!
//! - **Who produced this?** (agent role, model, human user, external chain)
//! - **How trusted is that producer?** (trust in `[0..1]`)
//! - **Is the data tainted?** (from untrusted external source, needs validation)
//!
//! Provenance is how Roko implements taint analysis (anti-pattern: untrusted
//! data injected into prompts) and audit trails (tracing a decision back to
//! its inputs via lineage chains).

use serde::{Deserialize, Serialize};

/// Who produced a signal and how trustworthy they are.
///
/// Roko uses provenance to:
/// 1. Audit: trace decisions back to their source inputs
/// 2. Security: prevent untrusted data from reaching high-privilege contexts
/// 3. Reputation: weight signals by their author's track record
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Identifier of the producer (agent role, user email, chain address, etc.).
    pub author: String,

    /// Trust score \[0..1\] at time of emission.
    /// 1.0 = fully trusted (local code, verified gates)
    /// 0.5 = unverified but internal
    /// 0.0 = untrusted (user input, external APIs, chain pulls)
    pub trust: f32,

    /// Whether this signal contains data from an untrusted source.
    /// Tainted signals must be sanitized before they enter prompts or gates.
    pub tainted: bool,

    /// Optional: the agent session or run that produced this signal.
    /// Useful for grouping related signals and computing per-run metrics.
    pub session: Option<String>,
}

impl Provenance {
    /// Produced by trusted internal code (gates, composers, the orchestrator itself).
    #[must_use]
    pub fn trusted(author: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            trust: 1.0,
            tainted: false,
            session: None,
        }
    }

    /// Produced by an internal agent — trusted but not ground truth.
    #[must_use]
    pub fn agent(author: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            trust: 0.75,
            tainted: false,
            session: None,
        }
    }

    /// From an external/untrusted source — needs sanitization before use.
    #[must_use]
    pub fn external(author: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            trust: 0.1,
            tainted: true,
            session: None,
        }
    }

    /// From a user (higher trust than external, but still tainted for safety).
    #[must_use]
    pub fn user(author: impl Into<String>) -> Self {
        Self {
            author: author.into(),
            trust: 0.5,
            tainted: true,
            session: None,
        }
    }

    /// Attach a session identifier.
    #[must_use]
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// Override the trust value (clamped to `[0..1]`).
    #[must_use]
    pub const fn with_trust(mut self, trust: f32) -> Self {
        self.trust = trust.clamp(0.0, 1.0);
        self
    }

    /// Mark as tainted regardless of author.
    #[must_use]
    pub const fn with_taint(mut self, tainted: bool) -> Self {
        self.tainted = tainted;
        self
    }

    /// Is this provenance trusted enough to include in `min_trust` contexts?
    #[must_use]
    pub fn is_trusted(&self, min_trust: f32) -> bool {
        self.trust >= min_trust && !self.tainted
    }
}

impl Default for Provenance {
    fn default() -> Self {
        Self::trusted("roko")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_has_full_trust_no_taint() {
        let p = Provenance::trusted("gate:compile");
        assert_eq!(p.trust, 1.0);
        assert!(!p.tainted);
    }

    #[test]
    fn external_is_tainted_low_trust() {
        let p = Provenance::external("webhook");
        assert!(p.trust < 0.5);
        assert!(p.tainted);
    }

    #[test]
    fn user_is_tainted_medium_trust() {
        let p = Provenance::user("user:alice");
        assert!(p.tainted);
        assert!(p.trust > 0.0);
    }

    #[test]
    fn with_session_adds_session() {
        let p = Provenance::agent("impl:1").with_session("run-42");
        assert_eq!(p.session.as_deref(), Some("run-42"));
    }

    #[test]
    fn is_trusted_rejects_tainted() {
        let p = Provenance::user("alice");
        assert!(!p.is_trusted(0.0)); // tainted always fails is_trusted
    }

    #[test]
    fn is_trusted_respects_threshold() {
        let p = Provenance::agent("x").with_trust(0.8);
        assert!(p.is_trusted(0.5));
        assert!(p.is_trusted(0.8));
        assert!(!p.is_trusted(0.9));
    }

    #[test]
    fn with_trust_clamps() {
        let p = Provenance::external("x").with_trust(2.0);
        assert_eq!(p.trust, 1.0);
        let p = Provenance::external("x").with_trust(-1.0);
        assert_eq!(p.trust, 0.0);
    }
}

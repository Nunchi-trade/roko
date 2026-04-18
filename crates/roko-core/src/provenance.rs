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

use crate::ContentHash;
use serde::{Deserialize, Serialize};

/// Structured taint metadata that can travel with a signal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintInfo {
    /// Short machine-readable category (`external`, `user_input`, `propagated`, ...).
    pub category: String,
    /// Human-readable detail for audit logs and refusal messages.
    pub detail: String,
    /// Parent signals this taint was inherited from, when propagation occurred.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherited_from: Vec<ContentHash>,
}

impl TaintInfo {
    /// Create a new taint record.
    #[must_use]
    pub fn new(category: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            detail: detail.into(),
            inherited_from: Vec::new(),
        }
    }

    /// Taint originating from an external source.
    #[must_use]
    pub fn external(detail: impl Into<String>) -> Self {
        Self::new("external", detail)
    }

    /// Taint originating from user input.
    #[must_use]
    pub fn user_input(detail: impl Into<String>) -> Self {
        Self::new("user_input", detail)
    }

    /// Taint inherited from one or more parent signals.
    #[must_use]
    pub fn propagated(
        detail: impl Into<String>,
        inherited_from: impl IntoIterator<Item = ContentHash>,
    ) -> Self {
        Self {
            category: "propagated".to_string(),
            detail: detail.into(),
            inherited_from: inherited_from.into_iter().collect(),
        }
    }
}

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

    /// Optional structured taint metadata for audit and safety decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taint_info: Option<TaintInfo>,

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
            taint_info: None,
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
            taint_info: None,
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
            taint_info: Some(TaintInfo::external("external source")),
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
            taint_info: Some(TaintInfo::user_input("user input")),
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
    pub fn with_taint(mut self, tainted: bool) -> Self {
        self.tainted = tainted;
        if !tainted {
            self.taint_info = None;
        }
        self
    }

    /// Attach explicit taint metadata and mark the provenance tainted.
    #[must_use]
    pub fn with_taint_info(mut self, taint_info: TaintInfo) -> Self {
        self.tainted = true;
        self.taint_info = Some(taint_info);
        self
    }

    /// Is this provenance trusted enough to include in `min_trust` contexts?
    #[must_use]
    pub fn is_trusted(&self, min_trust: f32) -> bool {
        self.trust >= min_trust && !self.tainted
    }

    /// Coherence issues between trust, taint flags, and structured taint metadata.
    #[must_use]
    pub fn coherence_issues(&self) -> Vec<&'static str> {
        let mut issues = Vec::new();

        if !self.trust.is_finite() {
            issues.push("trust must be finite");
        }
        if self.tainted && self.taint_info.is_none() {
            issues.push("tainted provenance should carry taint_info");
        }
        if !self.tainted && self.taint_info.is_some() {
            issues.push("clean provenance cannot carry taint_info");
        }
        if self.trust >= 1.0 && self.tainted {
            issues.push("fully trusted provenance cannot also be tainted");
        }

        issues
    }

    /// Return whether the provenance fields form a coherent safety record.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        self.coherence_issues().is_empty()
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

    #[test]
    fn tainted_provenance_is_coherent_when_taint_info_is_present() {
        let provenance = Provenance::user("alice");
        assert!(provenance.is_coherent());
        assert!(provenance.taint_info.is_some());
    }

    #[test]
    fn missing_taint_info_is_flagged() {
        let provenance = Provenance::trusted("gate").with_taint(true);
        assert_eq!(
            provenance.coherence_issues(),
            vec![
                "tainted provenance should carry taint_info",
                "fully trusted provenance cannot also be tainted",
            ]
        );
    }

    #[test]
    fn taint_info_on_clean_provenance_is_incoherent() {
        let mut provenance = Provenance::trusted("gate");
        provenance.taint_info = Some(TaintInfo::external("webhook"));
        assert_eq!(
            provenance.coherence_issues(),
            vec!["clean provenance cannot carry taint_info"]
        );
    }
}

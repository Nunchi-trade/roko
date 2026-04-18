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

/// Structured taint metadata attached to a tainted provenance record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintInfo {
    /// Short machine-readable category such as `"external"` or `"propagated"`.
    pub category: String,
    /// Human-readable explanation kept short for logs and UI surfaces.
    pub detail: String,
    /// Upstream tainted signal that caused propagation, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<ContentHash>,
}

impl TaintInfo {
    /// Construct taint metadata for a provenance record.
    #[must_use]
    pub fn new(category: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            detail: detail.into(),
            inherited_from: None,
        }
    }

    /// Mark taint as coming from an external source.
    #[must_use]
    pub fn external(detail: impl Into<String>) -> Self {
        Self::new("external", detail)
    }

    /// Mark taint as coming from user input.
    #[must_use]
    pub fn user_input(detail: impl Into<String>) -> Self {
        Self::new("user_input", detail)
    }

    /// Mark taint as inherited from another signal.
    #[must_use]
    pub fn propagated(detail: impl Into<String>, inherited_from: ContentHash) -> Self {
        Self::new("propagated", detail).with_inherited_from(inherited_from)
    }

    /// Attach an upstream tainted signal reference.
    #[must_use]
    pub fn with_inherited_from(mut self, inherited_from: ContentHash) -> Self {
        self.inherited_from = Some(inherited_from);
        self
    }
}

/// Cohesion check for a provenance record's internal metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceCoherenceCheck {
    /// Machine-readable issues found while checking provenance consistency.
    pub issues: Vec<ProvenanceCoherenceIssue>,
}

impl ProvenanceCoherenceCheck {
    /// Returns `true` when no coherence issues were found.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        self.issues.is_empty()
    }

    /// Conservative scalar summary for ranking or diagnostics.
    #[must_use]
    pub fn score(&self) -> f32 {
        (1.0 - 0.25 * self.issues.len() as f32).clamp(0.0, 1.0)
    }
}

/// Coherence issue discovered while validating a [`Provenance`] record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceCoherenceIssue {
    /// The author field was blank.
    MissingAuthor,
    /// `tainted` was set without corresponding explanatory metadata.
    MissingTaintInfo,
    /// Taint metadata was attached while the provenance was marked clean.
    UnexpectedTaintInfo,
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

    /// Structured taint metadata for audits and propagation.
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
        let author = author.into();
        Self {
            taint_info: Some(TaintInfo::external(format!("source {}", author))),
            author,
            trust: 0.1,
            tainted: true,
            session: None,
        }
    }

    /// From a user (higher trust than external, but still tainted for safety).
    #[must_use]
    pub fn user(author: impl Into<String>) -> Self {
        let author = author.into();
        Self {
            taint_info: Some(TaintInfo::user_input(format!("source {}", author))),
            author,
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
    pub fn with_taint(mut self, tainted: bool) -> Self {
        self.tainted = tainted;
        if !tainted {
            self.taint_info = None;
        }
        self
    }

    /// Attach structured taint metadata and mark the provenance tainted.
    #[must_use]
    pub fn with_taint_info(mut self, taint_info: TaintInfo) -> Self {
        self.tainted = true;
        self.taint_info = Some(taint_info);
        self
    }

    /// Run a conservative consistency check over the provenance metadata.
    #[must_use]
    pub fn coherence_check(&self) -> ProvenanceCoherenceCheck {
        let mut issues = Vec::new();

        if self.author.trim().is_empty() {
            issues.push(ProvenanceCoherenceIssue::MissingAuthor);
        }
        if self.tainted && self.taint_info.is_none() {
            issues.push(ProvenanceCoherenceIssue::MissingTaintInfo);
        }
        if !self.tainted && self.taint_info.is_some() {
            issues.push(ProvenanceCoherenceIssue::UnexpectedTaintInfo);
        }

        ProvenanceCoherenceCheck { issues }
    }

    /// Shorthand for `coherence_check().score()`.
    #[must_use]
    pub fn coherence_score(&self) -> f32 {
        self.coherence_check().score()
    }

    /// Whether the provenance metadata is internally coherent.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        self.coherence_check().is_coherent()
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
    fn external_sets_taint_info() {
        let p = Provenance::external("webhook");
        assert_eq!(
            p.taint_info.as_ref().map(|info| info.category.as_str()),
            Some("external")
        );
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
    fn coherence_check_flags_missing_taint_info() {
        let p = Provenance::trusted("roko").with_taint(true);
        let check = p.coherence_check();
        assert!(!check.is_coherent());
        assert!(
            check
                .issues
                .contains(&ProvenanceCoherenceIssue::MissingTaintInfo)
        );
    }

    #[test]
    fn coherence_check_flags_unexpected_taint_info() {
        let p = Provenance {
            author: "roko".to_string(),
            trust: 1.0,
            tainted: false,
            taint_info: Some(TaintInfo::external("api")),
            session: None,
        };
        let check = p.coherence_check();
        assert!(!check.is_coherent());
        assert!(
            check
                .issues
                .contains(&ProvenanceCoherenceIssue::UnexpectedTaintInfo)
        );
    }

    #[test]
    fn coherence_check_detects_missing_author() {
        let p = Provenance::trusted("");
        let check = p.coherence_check();
        assert!(!check.is_coherent());
        assert!(
            check
                .issues
                .contains(&ProvenanceCoherenceIssue::MissingAuthor)
        );
        assert!(check.score() < 1.0);
    }
}

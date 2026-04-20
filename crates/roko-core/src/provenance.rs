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

/// Typed taint classification for provenance records.
///
/// Replaces the former `tainted: bool` + `TaintInfo { category, detail }` pair
/// with compile-time checked variants. Marked `#[non_exhaustive]` so new taint
/// reasons can be added without breaking downstream matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Taint {
    /// No taint — data is from a trusted, verified source.
    Clean,
    /// LLM-generated content that may contain hallucinated facts.
    LlmHallucination {
        /// Human-readable detail.
        detail: String,
    },
    /// A tool call failed or returned suspect data.
    ToolFailure {
        /// Human-readable detail.
        detail: String,
    },
    /// A human operator explicitly flagged this data.
    UserFlagged {
        /// Human-readable detail.
        detail: String,
    },
    /// Data has exceeded its freshness window.
    StaleData {
        /// Staleness threshold in milliseconds.
        threshold_ms: i64,
    },
    /// Data came from an unverified external source (API, webhook, chain).
    UnverifiedSource {
        /// Human-readable detail.
        detail: String,
    },
    /// Taint was inherited from an upstream tainted signal.
    Propagated {
        /// Human-readable explanation.
        detail: String,
        /// The upstream tainted signal hash, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inherited_from: Option<ContentHash>,
    },
    /// User-provided input (higher trust than external, but still tainted for safety).
    UserInput {
        /// Human-readable detail.
        detail: String,
    },
    /// Application-specific taint reason not covered by other variants.
    Custom(String),
}

impl Default for Taint {
    fn default() -> Self {
        Self::Clean
    }
}

impl Taint {
    /// Returns `true` when the data is tainted (any variant except `Clean`).
    #[must_use]
    pub const fn is_tainted(&self) -> bool {
        !matches!(self, Self::Clean)
    }

    /// Short machine-readable category string for logging and audit trails.
    #[must_use]
    pub fn category(&self) -> &str {
        match self {
            Self::Clean => "clean",
            Self::LlmHallucination { .. } => "llm_hallucination",
            Self::ToolFailure { .. } => "tool_failure",
            Self::UserFlagged { .. } => "user_flagged",
            Self::StaleData { .. } => "stale_data",
            Self::UnverifiedSource { .. } => "unverified_source",
            Self::Propagated { .. } => "propagated",
            Self::UserInput { .. } => "user_input",
            Self::Custom(_) => "custom",
        }
    }

    /// Human-readable detail string, if the variant carries one.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Clean => None,
            Self::LlmHallucination { detail }
            | Self::ToolFailure { detail }
            | Self::UserFlagged { detail }
            | Self::UnverifiedSource { detail }
            | Self::Propagated { detail, .. }
            | Self::UserInput { detail } => Some(detail),
            Self::StaleData { threshold_ms: _ } => None,
            Self::Custom(detail) => Some(detail),
        }
    }

    /// Upstream inherited-from hash, if this is a propagated taint.
    #[must_use]
    pub fn inherited_from(&self) -> Option<&ContentHash> {
        match self {
            Self::Propagated { inherited_from, .. } => inherited_from.as_ref(),
            _ => None,
        }
    }
}

/// Structured taint metadata attached to a tainted provenance record.
///
/// **Deprecated**: Use [`Taint`] enum directly. This struct is retained for
/// backward compatibility with existing serialized data and the
/// `TaintTracker` in roko-orchestrator. New code should use `Taint` variants.
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

    /// Convert this legacy `TaintInfo` into the typed [`Taint`] enum.
    #[must_use]
    pub fn to_taint(&self) -> Taint {
        match self.category.as_str() {
            "external" => Taint::UnverifiedSource {
                detail: self.detail.clone(),
            },
            "user_input" => Taint::UserInput {
                detail: self.detail.clone(),
            },
            "propagated" => Taint::Propagated {
                detail: self.detail.clone(),
                inherited_from: self.inherited_from,
            },
            other => Taint::Custom(format!("{}: {}", other, self.detail)),
        }
    }
}

impl From<&TaintInfo> for Taint {
    fn from(info: &TaintInfo) -> Self {
        info.to_taint()
    }
}

impl From<&Taint> for Option<TaintInfo> {
    fn from(taint: &Taint) -> Self {
        match taint {
            Taint::Clean => None,
            Taint::LlmHallucination { detail } => {
                Some(TaintInfo::new("llm_hallucination", detail.clone()))
            }
            Taint::ToolFailure { detail } => Some(TaintInfo::new("tool_failure", detail.clone())),
            Taint::UserFlagged { detail } => Some(TaintInfo::new("user_flagged", detail.clone())),
            Taint::StaleData { threshold_ms } => Some(TaintInfo::new(
                "stale_data",
                format!("threshold {}ms", threshold_ms),
            )),
            Taint::UnverifiedSource { detail } => Some(TaintInfo::new("external", detail.clone())),
            Taint::Propagated {
                detail,
                inherited_from,
            } => {
                let mut info = TaintInfo::new("propagated", detail.clone());
                info.inherited_from = *inherited_from;
                Some(info)
            }
            Taint::UserInput { detail } => Some(TaintInfo::new("user_input", detail.clone())),
            Taint::Custom(detail) => Some(TaintInfo::new("custom", detail.clone())),
        }
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

    /// Typed taint classification. `Taint::Clean` means untainted.
    ///
    /// Tainted signals must be sanitized before they enter prompts or gates.
    /// Use [`Provenance::is_tainted()`] for boolean checks.
    #[serde(default)]
    pub taint: Taint,

    /// **Deprecated** — legacy structured taint metadata.
    ///
    /// Retained for backward-compatible deserialization of old JSONL logs.
    /// New code should read [`taint`](Self::taint) directly. When both fields
    /// are present during deserialization, `taint` takes precedence.
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
            taint: Taint::Clean,
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
            taint: Taint::Clean,
            taint_info: None,
            session: None,
        }
    }

    /// From an external/untrusted source — needs sanitization before use.
    #[must_use]
    pub fn external(author: impl Into<String>) -> Self {
        let author = author.into();
        Self {
            taint: Taint::UnverifiedSource {
                detail: format!("source {}", author),
            },
            taint_info: None,
            author,
            trust: 0.1,
            session: None,
        }
    }

    /// From a user (higher trust than external, but still tainted for safety).
    #[must_use]
    pub fn user(author: impl Into<String>) -> Self {
        let author = author.into();
        Self {
            taint: Taint::UserInput {
                detail: format!("source {}", author),
            },
            taint_info: None,
            author,
            trust: 0.5,
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

    /// Set the typed taint classification.
    #[must_use]
    pub fn with_taint(mut self, taint: Taint) -> Self {
        self.taint = taint;
        self
    }

    /// Attach structured taint metadata (legacy API) and set the typed taint field.
    #[must_use]
    pub fn with_taint_info(mut self, taint_info: TaintInfo) -> Self {
        self.taint = taint_info.to_taint();
        self.taint_info = Some(taint_info);
        self
    }

    /// Returns `true` when this provenance carries active taint.
    #[must_use]
    pub fn is_tainted(&self) -> bool {
        self.taint.is_tainted()
    }

    /// Run a conservative consistency check over the provenance metadata.
    #[must_use]
    pub fn coherence_check(&self) -> ProvenanceCoherenceCheck {
        let mut issues = Vec::new();

        if self.author.trim().is_empty() {
            issues.push(ProvenanceCoherenceIssue::MissingAuthor);
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
        self.trust >= min_trust && !self.is_tainted()
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
        assert!(!p.is_tainted());
    }

    #[test]
    fn external_is_tainted_low_trust() {
        let p = Provenance::external("webhook");
        assert!(p.trust < 0.5);
        assert!(p.is_tainted());
    }

    #[test]
    fn user_is_tainted_medium_trust() {
        let p = Provenance::user("user:alice");
        assert!(p.is_tainted());
        assert!(p.trust > 0.0);
    }

    #[test]
    fn with_session_adds_session() {
        let p = Provenance::agent("impl:1").with_session("run-42");
        assert_eq!(p.session.as_deref(), Some("run-42"));
    }

    #[test]
    fn external_sets_taint_variant() {
        let p = Provenance::external("webhook");
        assert_eq!(p.taint.category(), "unverified_source");
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
    fn taint_enum_variants_have_correct_categories() {
        assert_eq!(Taint::Clean.category(), "clean");
        assert_eq!(
            Taint::LlmHallucination {
                detail: "x".into()
            }
            .category(),
            "llm_hallucination"
        );
        assert_eq!(
            Taint::ToolFailure {
                detail: "x".into()
            }
            .category(),
            "tool_failure"
        );
        assert_eq!(
            Taint::UserFlagged {
                detail: "x".into()
            }
            .category(),
            "user_flagged"
        );
        assert_eq!(
            Taint::StaleData { threshold_ms: 100 }.category(),
            "stale_data"
        );
        assert_eq!(
            Taint::UnverifiedSource {
                detail: "x".into()
            }
            .category(),
            "unverified_source"
        );
        assert_eq!(
            Taint::Custom("x".into()).category(),
            "custom"
        );
    }

    #[test]
    fn taint_info_converts_to_taint_enum() {
        let info = TaintInfo::external("webhook payload");
        let taint = info.to_taint();
        assert!(matches!(taint, Taint::UnverifiedSource { .. }));
        assert_eq!(taint.detail(), Some("webhook payload"));
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

    #[test]
    fn with_taint_info_sets_both_fields() {
        let p = Provenance::trusted("gateway")
            .with_taint_info(TaintInfo::external("webhook payload"));
        assert!(p.is_tainted());
        assert!(matches!(p.taint, Taint::UnverifiedSource { .. }));
        assert!(p.taint_info.is_some());
    }
}

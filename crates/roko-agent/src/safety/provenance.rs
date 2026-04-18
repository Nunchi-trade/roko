//! Provenance-facing safety records.
//!
//! The dispatcher already emits audit events, but several safety documents refer
//! to richer custody and taint records. These structs provide the documented
//! shapes inside the live safety crate without forcing a heavier persistence
//! backend into the runtime path.

use serde::{Deserialize, Serialize};

use crate::safety::authz::AuthorizationEvidence;

/// Trust label carried by an input or action lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Taint {
    /// No active taint.
    None,
    /// Data came directly from a human operator or user.
    UserInput,
    /// Data was fetched from an external source.
    ExternalFetch(String),
    /// Data was produced by a third-party plugin or extension.
    ThirdPartyPlugin(String),
    /// Data was imported from a legacy or foreign system.
    LegacyImport,
}

impl Taint {
    /// Returns `true` when the label denotes untrusted or review-worthy input.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Assurance tier for an audited record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationLevel {
    /// Local session-level attestation.
    LocalAgent,
    /// Human or organization-backed attestation.
    OrgRole,
    /// External witness or chain-backed attestation.
    ChainWitness,
}

/// Action-centric custody record for a safety-relevant operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custody {
    /// Stable identifier for the action being recorded.
    pub action: String,
    /// Principal that initiated the action.
    pub principal: String,
    /// Unix-millis timestamp for the action.
    pub when: i64,
    /// Authorization evidence captured at decision time.
    pub authorized: Vec<AuthorizationEvidence>,
    /// Heuristics that materially influenced the action.
    pub why_heuristics: Vec<String>,
    /// Claims or assertions that materially influenced the action.
    pub why_claims: Vec<String>,
    /// Optional simulation or dry-run identifier.
    pub simulation: Option<String>,
    /// Gate or review stages that passed before execution.
    pub gates_passed: Vec<String>,
    /// Taint state active for the action.
    pub taint: Option<Taint>,
    /// Optional result identifier or digest.
    pub result: Option<String>,
    /// Optional external witness identifier.
    pub witness: Option<String>,
    /// Optional attestation tier for the record.
    pub attestation: Option<AttestationLevel>,
}

impl Custody {
    /// Create a custody record with the required fields.
    #[must_use]
    pub fn new(
        action: impl Into<String>,
        principal: impl Into<String>,
        when: i64,
        authorized: Vec<AuthorizationEvidence>,
    ) -> Self {
        Self {
            action: action.into(),
            principal: principal.into(),
            when,
            authorized,
            why_heuristics: Vec::new(),
            why_claims: Vec::new(),
            simulation: None,
            gates_passed: Vec::new(),
            taint: None,
            result: None,
            witness: None,
            attestation: None,
        }
    }

    /// Attach active taint to the record.
    #[must_use]
    pub fn with_taint(mut self, taint: Taint) -> Self {
        if taint.is_active() {
            self.taint = Some(taint);
        }
        self
    }

    /// Attach a result identifier or digest.
    #[must_use]
    pub fn with_result(mut self, result: impl Into<String>) -> Self {
        self.result = Some(result.into());
        self
    }

    /// Attach an attestation level.
    #[must_use]
    pub fn with_attestation(mut self, attestation: AttestationLevel) -> Self {
        self.attestation = Some(attestation);
        self
    }
}


//! Provenance-facing safety records and custody logging.
//!
//! The dispatcher already emits audit events, but several safety documents refer
//! to richer custody and taint records. These structs provide the documented
//! shapes inside the live safety crate without forcing a heavier persistence
//! backend into the runtime path.
//!
//! The [`CustodyLogger`] provides append-only JSONL persistence for custody
//! records, following the same pattern as `EpisodeLogger`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::safety::authz::AuthorizationEvidence;

/// Trust label carried by an input or action lineage in the custody layer.
///
/// This is the **action-centric** taint classification for custody logging —
/// it classifies where an input came from (external fetch, plugin, user, etc.).
///
/// For the **signal-level** taint (hallucination tracking, stale data, propagation),
/// see `roko_core::Taint` which is the canonical provenance-layer type.
///
/// These two serve different architectural layers:
/// - `CustodyTaint`: local safety decision (should this action be restricted?)
/// - `roko_core::Taint`: global signal lineage (should downstream consumers trust this?)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustodyTaint {
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

/// Backwards-compatible type alias.
pub type Taint = CustodyTaint;

impl CustodyTaint {
    /// Returns `true` when the label denotes untrusted or review-worthy input.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Convert to the canonical `roko_core::Taint` for signal-level tracking.
    ///
    /// This bridges the custody layer to the provenance layer when a custody
    /// event needs to propagate taint information into the signal graph.
    #[must_use]
    pub fn to_signal_taint(&self) -> roko_core::Taint {
        match self {
            Self::None => roko_core::Taint::Clean,
            Self::UserInput => roko_core::Taint::UserInput {
                detail: "custody: user input".into(),
            },
            Self::ExternalFetch(url) => roko_core::Taint::UnverifiedSource {
                detail: format!("custody: external fetch from {url}"),
            },
            Self::ThirdPartyPlugin(name) => roko_core::Taint::UnverifiedSource {
                detail: format!("custody: third-party plugin {name}"),
            },
            Self::LegacyImport => roko_core::Taint::Custom("custody: legacy import".into()),
        }
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

    /// Attach gate names that passed before execution.
    #[must_use]
    pub fn with_gates_passed(mut self, gates: Vec<String>) -> Self {
        self.gates_passed = gates;
        self
    }

    /// Attach heuristic explanations.
    #[must_use]
    pub fn with_heuristics(mut self, heuristics: Vec<String>) -> Self {
        self.why_heuristics = heuristics;
        self
    }
}

// ─── CustodyLogger ──────────────────────────────────────────────────

/// Append-only JSONL logger for custody records.
///
/// Each call to [`CustodyLogger::log`] serializes a [`Custody`] record
/// as a single JSON line and appends it to the custody log file. The
/// logger creates the parent directory on first write if it does not
/// exist.
///
/// # Thread safety
///
/// The logger is `Send + Sync` and can be shared across threads. Each
/// `log` call opens the file in append mode, so concurrent writes are
/// safe on POSIX (atomic under `O_APPEND` for small writes).
#[derive(Debug, Clone)]
pub struct CustodyLogger {
    /// Path to the custody JSONL file.
    path: PathBuf,
}

impl CustodyLogger {
    /// Create a logger that writes to the given path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append a custody record to the log file.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be opened/written.
    pub fn log(&self, custody: &Custody) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(custody)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")
    }

    /// Read all custody records from the log file.
    ///
    /// Returns an empty vec if the file does not exist. Lines that fail
    /// to parse are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read.
    pub fn read_all(&self) -> std::io::Result<Vec<Custody>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        let records = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        Ok(records)
    }

    /// Return the path to the custody log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the number of records in the log.
    ///
    /// Returns 0 if the file does not exist or cannot be read.
    #[must_use]
    pub fn count(&self) -> usize {
        self.read_all().map(|records| records.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custody_record_round_trips_through_serde() {
        let custody = Custody::new("write_file", "agent-001", 1713600000000_i64, vec![])
            .with_taint(Taint::ExternalFetch("https://example.com".into()))
            .with_result("sha256:abc123")
            .with_attestation(AttestationLevel::LocalAgent)
            .with_gates_passed(vec!["compile".into(), "test".into()])
            .with_heuristics(vec!["irreversibility=0.2".into()]);

        let json = serde_json::to_string(&custody).unwrap();
        let decoded: Custody = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.action, "write_file");
        assert_eq!(decoded.principal, "agent-001");
        assert_eq!(decoded.gates_passed.len(), 2);
        assert!(decoded.taint.is_some());
        assert_eq!(decoded.result.as_deref(), Some("sha256:abc123"));
    }

    #[test]
    fn custody_logger_writes_and_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("custody.jsonl");
        let logger = CustodyLogger::new(&log_path);

        let c1 = Custody::new("bash", "agent-1", 100, vec![]);
        let c2 = Custody::new("write_file", "agent-2", 200, vec![]).with_taint(Taint::UserInput);

        logger.log(&c1).unwrap();
        logger.log(&c2).unwrap();

        let records = logger.read_all().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].action, "bash");
        assert_eq!(records[1].action, "write_file");
        assert!(records[1].taint.is_some());
    }

    #[test]
    fn custody_logger_count_returns_zero_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let logger = CustodyLogger::new(tmp.path().join("nonexistent.jsonl"));
        assert_eq!(logger.count(), 0);
    }

    #[test]
    fn taint_none_is_not_active() {
        assert!(!Taint::None.is_active());
    }

    #[test]
    fn taint_external_fetch_is_active() {
        assert!(Taint::ExternalFetch("url".into()).is_active());
    }

    #[test]
    fn taint_with_none_does_not_set_field() {
        let custody = Custody::new("test", "p", 0, vec![]).with_taint(Taint::None);
        assert!(custody.taint.is_none());
    }
}

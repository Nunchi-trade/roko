//! Audit log for secret accesses (items 43.12--43.14).
//!
//! [`SecretAuditLog`] is an append-only, in-memory log that records every
//! secret read, write, and delete. It provides filtered views by namespace
//! and by time range for monitoring and compliance.

use super::resolve::SecretSource;

/// What action was performed on a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum AuditAction {
    /// The secret was read / resolved.
    Read,
    /// The secret was written / created.
    Write,
    /// The secret was deleted / revoked.
    Delete,
}

impl AuditAction {
    /// Stable label for metrics / logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single audit log entry recording one secret access.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    /// The secret namespace (e.g. `"llm"`).
    pub namespace: String,
    /// The secret key within the namespace (e.g. `"anthropic"`).
    pub key: String,
    /// Which backend supplied (or would supply) the secret.
    pub source: SecretSource,
    /// Who performed the access (agent name, CLI user, system component).
    pub accessor: String,
    /// Unix epoch milliseconds when the access occurred.
    pub timestamp_ms: u64,
    /// What kind of access was performed.
    pub action: AuditAction,
}

impl AuditEntry {
    /// Create a new audit entry with all fields specified.
    #[must_use]
    pub fn new(
        namespace: impl Into<String>,
        key: impl Into<String>,
        source: SecretSource,
        accessor: impl Into<String>,
        timestamp_ms: u64,
        action: AuditAction,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            key: key.into(),
            source,
            accessor: accessor.into(),
            timestamp_ms,
            action,
        }
    }
}

/// Append-only in-memory log of secret accesses.
///
/// Thread-safe: uses [`parking_lot::RwLock`] internally. The log is not
/// persisted to disk by default; the embedding binary is responsible for
/// draining entries to a durable store if needed.
pub struct SecretAuditLog {
    entries: parking_lot::RwLock<Vec<AuditEntry>>,
}

impl std::fmt::Debug for SecretAuditLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.entries.read().len();
        f.debug_struct("SecretAuditLog")
            .field("entry_count", &len)
            .finish_non_exhaustive()
    }
}

impl SecretAuditLog {
    /// Create an empty audit log.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Append an entry to the log.
    pub fn record_access(&self, entry: AuditEntry) {
        self.entries.write().push(entry);
    }

    /// Return a snapshot of all entries (cloned).
    #[must_use]
    pub fn entries(&self) -> Vec<AuditEntry> {
        self.entries.read().clone()
    }

    /// Return entries matching the given namespace.
    #[must_use]
    pub fn entries_for_namespace(&self, namespace: &str) -> Vec<AuditEntry> {
        self.entries
            .read()
            .iter()
            .filter(|e| e.namespace == namespace)
            .cloned()
            .collect()
    }

    /// Return entries with `timestamp_ms >= since`.
    #[must_use]
    pub fn entries_since(&self, timestamp_ms: u64) -> Vec<AuditEntry> {
        self.entries
            .read()
            .iter()
            .filter(|e| e.timestamp_ms >= timestamp_ms)
            .cloned()
            .collect()
    }

    /// Total number of entries in the log.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Whether the log is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }
}

impl Default for SecretAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(ns: &str, key: &str, accessor: &str, ts: u64, action: AuditAction) -> AuditEntry {
        AuditEntry::new(ns, key, SecretSource::File, accessor, ts, action)
    }

    #[test]
    fn record_and_retrieve_entries() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "anthropic", "agent-1", 1000, AuditAction::Read));
        log.record_access(make_entry("rpc", "alchemy", "agent-2", 2000, AuditAction::Write));
        assert_eq!(log.len(), 2);
        let all = log.entries();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].namespace, "llm");
        assert_eq!(all[1].namespace, "rpc");
    }

    #[test]
    fn filter_by_namespace() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "anthropic", "a", 100, AuditAction::Read));
        log.record_access(make_entry("rpc", "alchemy", "b", 200, AuditAction::Read));
        log.record_access(make_entry("llm", "openai", "c", 300, AuditAction::Write));

        let llm = log.entries_for_namespace("llm");
        assert_eq!(llm.len(), 2);
        assert!(llm.iter().all(|e| e.namespace == "llm"));

        let rpc = log.entries_for_namespace("rpc");
        assert_eq!(rpc.len(), 1);
        assert_eq!(rpc[0].key, "alchemy");
    }

    #[test]
    fn filter_by_time() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "a", "x", 100, AuditAction::Read));
        log.record_access(make_entry("llm", "b", "x", 200, AuditAction::Read));
        log.record_access(make_entry("llm", "c", "x", 300, AuditAction::Read));

        let since_200 = log.entries_since(200);
        assert_eq!(since_200.len(), 2);
        assert!(since_200.iter().all(|e| e.timestamp_ms >= 200));
    }

    #[test]
    fn empty_log() {
        let log = SecretAuditLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert!(log.entries().is_empty());
        assert!(log.entries_for_namespace("llm").is_empty());
        assert!(log.entries_since(0).is_empty());
    }

    #[test]
    fn audit_action_display() {
        assert_eq!(AuditAction::Read.to_string(), "read");
        assert_eq!(AuditAction::Write.to_string(), "write");
        assert_eq!(AuditAction::Delete.to_string(), "delete");
    }

    #[test]
    fn audit_action_as_str() {
        assert_eq!(AuditAction::Read.as_str(), "read");
        assert_eq!(AuditAction::Write.as_str(), "write");
        assert_eq!(AuditAction::Delete.as_str(), "delete");
    }

    #[test]
    fn entry_records_accessor() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "anthropic", "conductor", 1000, AuditAction::Read));
        let entries = log.entries();
        assert_eq!(entries[0].accessor, "conductor");
    }

    #[test]
    fn entry_records_source() {
        let entry = AuditEntry::new(
            "llm", "anthropic",
            SecretSource::Environment,
            "agent-1", 1000,
            AuditAction::Read,
        );
        assert_eq!(entry.source, SecretSource::Environment);
    }

    #[test]
    fn entries_since_boundary_inclusive() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "a", "x", 100, AuditAction::Read));
        log.record_access(make_entry("llm", "b", "x", 200, AuditAction::Read));
        // since=200 should include the entry at exactly 200
        let since_200 = log.entries_since(200);
        assert_eq!(since_200.len(), 1);
        assert_eq!(since_200[0].key, "b");
    }

    #[test]
    fn default_creates_empty_log() {
        let log = SecretAuditLog::default();
        assert!(log.is_empty());
    }

    #[test]
    fn delete_action_recorded() {
        let log = SecretAuditLog::new();
        log.record_access(make_entry("llm", "anthropic", "admin", 500, AuditAction::Delete));
        let entries = log.entries();
        assert_eq!(entries[0].action, AuditAction::Delete);
    }
}

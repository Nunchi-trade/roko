use roko_core::foundation::GateVerdict;
use serde::{Deserialize, Serialize};

/// Typed gate result status that preserves skipped/not-wired/invalid distinctions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GateStatus {
    /// Gate ran and passed.
    Passed,
    /// Gate ran and failed.
    Failed,
    /// Gate was intentionally skipped for an operational reason.
    Skipped {
        /// Reason the gate was skipped.
        reason: String,
    },
    /// Gate could not run because no implementation or required input is wired.
    NotWired {
        /// Missing implementation/input reason.
        reason: String,
    },
    /// Gate could not run because its configuration is invalid.
    InvalidConfig {
        /// Configuration error reason.
        reason: String,
    },
}

impl GateStatus {
    /// Convert legacy `passed/skipped/skip_reason` fields to a typed status.
    #[must_use]
    pub fn from_legacy_fields(passed: bool, skipped: bool, skip_reason: Option<&str>) -> Self {
        if passed && !skipped {
            return Self::Passed;
        }

        if skipped {
            let reason = skip_reason.unwrap_or("skipped").to_string();
            let lower = reason.to_ascii_lowercase();
            if lower.contains("not wired")
                || lower.contains("not implemented")
                || lower.contains("requires explicit")
            {
                return Self::NotWired { reason };
            }
            if lower.contains("missing program") || lower.contains("invalid") {
                return Self::InvalidConfig { reason };
            }
            return Self::Skipped { reason };
        }

        Self::Failed
    }
}

impl From<&GateVerdict> for GateStatus {
    fn from(verdict: &GateVerdict) -> Self {
        Self::from_legacy_fields(
            verdict.passed,
            verdict.skipped,
            verdict.skip_reason.as_deref(),
        )
    }
}

/// Broad gate family used by the shared registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    /// Compilation/type-checking gate.
    Compile,
    /// Static lint gate.
    Lint,
    /// Test-suite gate.
    Test,
    /// Diff/review gate.
    Diff,
    /// Formatting gate.
    Format,
    /// Configured shell command gate.
    Shell,
    /// LLM/human-review judge gate.
    Judge,
}

/// Metadata for one known gate id and its aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateSpec {
    /// Canonical gate id.
    pub id: &'static str,
    /// Accepted legacy/config aliases for the gate.
    pub aliases: &'static [&'static str],
    /// Canonical rung index for ordering.
    pub rung: u8,
    /// Gate family.
    pub kind: GateKind,
    /// Inputs required before this gate can execute.
    pub required_inputs: &'static [&'static str],
}

/// Resolver for canonical gate ids, aliases, and rungs.
#[derive(Debug, Clone, Copy, Default)]
pub struct GateRegistry;

impl GateRegistry {
    /// Create a registry resolver over the static gate specs.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Return all known gate specs.
    #[must_use]
    pub fn all(&self) -> &'static [GateSpec] {
        GATE_SPECS
    }

    /// Resolve a canonical id or alias to a gate spec.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<&'static GateSpec> {
        GATE_SPECS
            .iter()
            .find(|spec| spec.id == name || spec.aliases.contains(&name))
    }

    /// Resolve a canonical id or alias to its rung index.
    #[must_use]
    pub fn rung_for_name(&self, name: &str) -> Option<u8> {
        self.resolve(name).map(|spec| spec.rung)
    }
}

/// Static registry of the currently known gate aliases and rungs.
pub const GATE_SPECS: &[GateSpec] = &[
    GateSpec {
        id: "compile",
        aliases: &["compile:cargo"],
        rung: 0,
        kind: GateKind::Compile,
        required_inputs: &["workdir"],
    },
    GateSpec {
        id: "clippy",
        aliases: &["clippy:cargo"],
        rung: 1,
        kind: GateKind::Lint,
        required_inputs: &["workdir"],
    },
    GateSpec {
        id: "test",
        aliases: &["test:cargo"],
        rung: 2,
        kind: GateKind::Test,
        required_inputs: &["workdir"],
    },
    GateSpec {
        id: "diff",
        aliases: &["diff:git"],
        rung: 3,
        kind: GateKind::Diff,
        required_inputs: &["workdir"],
    },
    GateSpec {
        id: "fmt",
        aliases: &["fmt:cargo", "format"],
        rung: 4,
        kind: GateKind::Format,
        required_inputs: &["workdir"],
    },
    GateSpec {
        id: "custom",
        aliases: &["custom:shell", "shell"],
        rung: 5,
        kind: GateKind::Shell,
        required_inputs: &["workdir", "shell_command"],
    },
    GateSpec {
        id: "judge",
        aliases: &["llm-judge"],
        rung: 6,
        kind: GateKind::Judge,
        required_inputs: &["workdir", "judge_payload"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_status_converts_legacy_passed() {
        assert_eq!(
            GateStatus::from_legacy_fields(true, false, None),
            GateStatus::Passed
        );
    }

    #[test]
    fn gate_status_converts_legacy_failed() {
        assert_eq!(
            GateStatus::from_legacy_fields(false, false, None),
            GateStatus::Failed
        );
    }

    #[test]
    fn gate_status_distinguishes_skipped_not_wired_and_invalid_config() {
        assert_eq!(
            GateStatus::from_legacy_fields(false, true, Some("adaptive: high pass rate")),
            GateStatus::Skipped {
                reason: "adaptive: high pass rate".to_string()
            }
        );
        assert_eq!(
            GateStatus::from_legacy_fields(false, true, Some("not wired")),
            GateStatus::NotWired {
                reason: "not wired".to_string()
            }
        );
        assert_eq!(
            GateStatus::from_legacy_fields(false, true, Some("missing program")),
            GateStatus::InvalidConfig {
                reason: "missing program".to_string()
            }
        );
    }

    #[test]
    fn gate_registry_resolves_aliases_and_rungs() {
        let registry = GateRegistry::new();
        let cases = [
            ("compile", 0, GateKind::Compile),
            ("compile:cargo", 0, GateKind::Compile),
            ("clippy", 1, GateKind::Lint),
            ("test", 2, GateKind::Test),
            ("diff:git", 3, GateKind::Diff),
            ("format", 4, GateKind::Format),
            ("custom:shell", 5, GateKind::Shell),
            ("llm-judge", 6, GateKind::Judge),
        ];

        for (name, rung, kind) in cases {
            let spec = registry.resolve(name).expect("gate should resolve");
            assert_eq!(spec.rung, rung);
            assert_eq!(spec.kind, kind);
            assert_eq!(registry.rung_for_name(name), Some(rung));
        }
    }

    #[test]
    fn gate_registry_unknown_gate_is_explicitly_absent() {
        let registry = GateRegistry::new();
        assert_eq!(registry.resolve("nonexistent"), None);
        assert_eq!(registry.rung_for_name("nonexistent"), None);
    }
}

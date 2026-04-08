//! Mortality subsystem scaffold.
//!
//! Placeholder API only; real lifecycle logic is intentionally not
//! implemented in this phase.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder mortality engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MortalityEngine;

impl MortalityEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Mortality;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Mortality";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: mortality";

    /// Construct a placeholder mortality engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this scaffold subsystem.
    #[must_use]
    pub const fn summary(self) -> GolemSubsystemSummary {
        GolemSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn pulse(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for MortalityEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}

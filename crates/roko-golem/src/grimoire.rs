//! Grimoire subsystem scaffold.
//!
//! Placeholder API only; memory lineage/evolution behavior is not yet
//! implemented.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder grimoire engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GrimoireEngine;

impl GrimoireEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Grimoire;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Grimoire";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: grimoire";

    /// Construct a placeholder grimoire engine.
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
    pub const fn evolve(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for GrimoireEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}

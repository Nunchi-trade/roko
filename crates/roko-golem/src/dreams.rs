//! Dreams subsystem scaffold.
//!
//! Placeholder API only; replay/imagination behavior is not yet implemented.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder dreams engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DreamsEngine;

impl DreamsEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Dreams;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Dreams";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: dreams";

    /// Construct a placeholder dreams engine.
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
    pub const fn replay(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for DreamsEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}

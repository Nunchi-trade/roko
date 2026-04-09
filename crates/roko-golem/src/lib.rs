//! `roko-golem` scaffold crate.
//!
//! This crate does **not** implement runtime Golem behavior yet. It exists to
//! provide a coherent, feature-gated scaffold surface for phase-2 subsystem
//! integration work.
//!
//! Enable `scaffold` to expose placeholder subsystem modules plus a shared
//! aggregator API:
//! - `mortality`
//! - `daimon`
//! - `dreams`
//! - `hypnagogia`
//! - `grimoire`
//! - `chain_witness`

/// Whether the scaffold API is compiled in this build.
pub const SCAFFOLD_ENABLED: bool = cfg!(feature = "scaffold");

/// High-level status of this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// Feature-gated placeholder APIs are available.
    ScaffoldOnly,
    /// Placeholder APIs are compiled out.
    Disabled,
}

/// Returns the current integration status.
#[must_use]
pub const fn status() -> IntegrationStatus {
    if SCAFFOLD_ENABLED {
        IntegrationStatus::ScaffoldOnly
    } else {
        IntegrationStatus::Disabled
    }
}

#[cfg(feature = "scaffold")]
/// Stable identifiers for the scaffolded Golem subsystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GolemSubsystemId {
    /// Mortality and lifecycle decay.
    Mortality,
    /// Affect and motivational state.
    Daimon,
    /// Replay and imagination loop.
    Dreams,
    /// Liminal interrupt / handoff state.
    Hypnagogia,
    /// Memory lineage and evolution.
    Grimoire,
    /// Chain-facing witness and triage entrypoint.
    ChainWitness,
}

#[cfg(feature = "scaffold")]
impl GolemSubsystemId {
    /// Human-readable label for this subsystem.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mortality => "Mortality",
            Self::Daimon => "Daimon",
            Self::Dreams => "Dreams",
            Self::Hypnagogia => "Hypnagogia",
            Self::Grimoire => "Grimoire",
            Self::ChainWitness => "Chain Witness",
        }
    }

    /// Stable machine-friendly slug for this subsystem.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Mortality => "mortality",
            Self::Daimon => "daimon",
            Self::Dreams => "dreams",
            Self::Hypnagogia => "hypnagogia",
            Self::Grimoire => "grimoire",
            Self::ChainWitness => "chain-witness",
        }
    }
}

#[cfg(feature = "scaffold")]
/// Summary metadata for one scaffolded subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GolemSubsystemSummary {
    /// Stable subsystem identifier.
    pub id: GolemSubsystemId,
    /// Human-readable label.
    pub label: &'static str,
    /// Static marker string describing scaffold behavior.
    pub marker: &'static str,
}

#[cfg(feature = "scaffold")]
impl GolemSubsystemSummary {
    /// Construct a subsystem summary.
    #[must_use]
    pub const fn new(id: GolemSubsystemId, label: &'static str, marker: &'static str) -> Self {
        Self { id, label, marker }
    }
}

#[cfg(feature = "scaffold")]
/// Common trait for feature-gated scaffold engines.
pub trait ScaffoldEngine: Copy {
    /// Return summary metadata for this subsystem engine.
    fn summary(self) -> GolemSubsystemSummary;
}

#[cfg(feature = "scaffold")]
pub mod chain_witness;
#[cfg(feature = "scaffold")]
pub mod daimon;
#[cfg(feature = "scaffold")]
pub mod dreams;
#[cfg(feature = "scaffold")]
pub mod grimoire;
#[cfg(feature = "scaffold")]
pub mod hypnagogia;
#[cfg(feature = "scaffold")]
pub mod mortality;

#[cfg(feature = "scaffold")]
pub use chain_witness::ChainWitnessEngine;
#[cfg(feature = "scaffold")]
pub use daimon::{AffectEngine, AffectOctant, AffectState, DaimonEngine};
#[cfg(feature = "scaffold")]
pub use dreams::DreamsEngine;
#[cfg(feature = "scaffold")]
pub use grimoire::GrimoireEngine;
#[cfg(feature = "scaffold")]
pub use hypnagogia::HypnagogiaEngine;
#[cfg(feature = "scaffold")]
pub use mortality::MortalityEngine;

#[cfg(feature = "scaffold")]
/// Top-level aggregator for the scaffolded subsystem engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GolemScaffold {
    /// Mortality subsystem scaffold.
    pub mortality: MortalityEngine,
    /// Daimon subsystem scaffold.
    pub daimon: DaimonEngine,
    /// Dreams subsystem scaffold.
    pub dreams: DreamsEngine,
    /// Hypnagogia subsystem scaffold.
    pub hypnagogia: HypnagogiaEngine,
    /// Grimoire subsystem scaffold.
    pub grimoire: GrimoireEngine,
    /// Chain witness subsystem scaffold.
    pub chain_witness: ChainWitnessEngine,
}

#[cfg(feature = "scaffold")]
impl GolemScaffold {
    /// Construct the full scaffold with every subsystem engine.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mortality: MortalityEngine::new(),
            daimon: DaimonEngine::new(),
            dreams: DreamsEngine::new(),
            hypnagogia: HypnagogiaEngine::new(),
            grimoire: GrimoireEngine::new(),
            chain_witness: ChainWitnessEngine::new(),
        }
    }

    /// Return the fixed set of subsystem summaries in stable order.
    #[must_use]
    pub const fn summaries(self) -> [GolemSubsystemSummary; 6] {
        [
            self.mortality.summary(),
            self.daimon.summary(),
            self.dreams.summary(),
            self.hypnagogia.summary(),
            self.grimoire.summary(),
            self.chain_witness.summary(),
        ]
    }

    /// Collect the scaffold into a heap-backed summary view.
    #[must_use]
    pub fn summary(self) -> GolemScaffoldSummary {
        let subsystems = self.summaries().to_vec();
        GolemScaffoldSummary {
            subsystem_count: subsystems.len(),
            subsystems,
        }
    }
}

#[cfg(feature = "scaffold")]
/// Collected top-level summary for the scaffold API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GolemScaffoldSummary {
    /// Number of scaffolded subsystems.
    pub subsystem_count: usize,
    /// Per-subsystem summary entries.
    pub subsystems: Vec<GolemSubsystemSummary>,
}

#[cfg(feature = "scaffold")]
impl GolemScaffoldSummary {
    /// Return true when the summary contains the given subsystem id.
    #[must_use]
    pub fn contains(&self, id: GolemSubsystemId) -> bool {
        self.subsystems.iter().any(|subsystem| subsystem.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::{IntegrationStatus, SCAFFOLD_ENABLED, status};

    #[test]
    fn status_matches_feature_flag() {
        if SCAFFOLD_ENABLED {
            assert_eq!(status(), IntegrationStatus::ScaffoldOnly);
        } else {
            assert_eq!(status(), IntegrationStatus::Disabled);
        }
    }

    #[cfg(feature = "scaffold")]
    #[test]
    fn scaffold_summary_lists_all_subsystems() {
        use super::{GolemScaffold, GolemSubsystemId};

        let scaffold = GolemScaffold::new();
        let summary = scaffold.summary();

        assert_eq!(summary.subsystem_count, 6);
        assert!(summary.contains(GolemSubsystemId::Mortality));
        assert!(summary.contains(GolemSubsystemId::Daimon));
        assert!(summary.contains(GolemSubsystemId::Dreams));
        assert!(summary.contains(GolemSubsystemId::Hypnagogia));
        assert!(summary.contains(GolemSubsystemId::Grimoire));
        assert!(summary.contains(GolemSubsystemId::ChainWitness));
    }

    #[cfg(feature = "scaffold")]
    #[test]
    fn subsystem_labels_and_slugs_are_stable() {
        use super::GolemSubsystemId;

        assert_eq!(GolemSubsystemId::Mortality.label(), "Mortality");
        assert_eq!(GolemSubsystemId::ChainWitness.slug(), "chain-witness");
    }
}

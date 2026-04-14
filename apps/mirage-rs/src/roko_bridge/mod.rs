//! Roko integration bridge — implements roko-core traits over mirage internals.
//!
//! This module is gated behind the `roko` feature (which implies `chain`). It
//! lets a roko golem use mirage as:
//!
//! - a **[`SimulationGate`]**: a `Gate` trait implementation that runs a planned
//!   transaction through a mirage fork and returns a `Verdict` telling the agent
//!   whether the tx would succeed on mainnet.
//! - an **[`HdcSubstrate`]**: a `Substrate` trait implementation backed by an
//!   HDC similarity index. Signals are content-hashed, indexed by HDC vector
//!   (projected from their body text), and retrieved by semantic similarity.
//! - a **[`ChainSubstrate`]**: a richer `Substrate` wired to the chain
//!   knowledge-layer: signals posted here are promoted to `InsightEntry`s with
//!   a decay lifecycle, confirmations, and challenges.
//!
//! Engram lineage is preserved: the original `Engram` is stored verbatim and
//! returned by `get(content_hash)` byte-for-byte equal to the put input.
//!
//! # Conversion rules
//!
//! - `Engram` → HDC vector: `project_tokens(body.as_text())` if the body is
//!   text/JSON, otherwise `project_bytes(body.canonical_bytes())`.
//! - `Engram` → `KnowledgeKind`: inferred from `Engram.kind`, defaulting to
//!   `KnowledgeKind::Insight`. See [`map_kind`] for the table.
//!
//! # Concurrency
//!
//! All substrates use `parking_lot::RwLock` internally; the trait's `async`
//! signature is satisfied with synchronous wrappers (locks held across a single
//! method call are brief).

mod chain_substrate;
mod hdc_substrate;
mod simulation_gate;
mod subscription;

pub use chain_substrate::{ChainSubstrate, ChainSubstrateConfig};
pub use hdc_substrate::HdcSubstrate;
pub use simulation_gate::{SimulationGate, SimulationGateConfig};
pub use subscription::{
    BackpressurePolicy, BroadcastSink, InsightBus, InsightEvent, InsightSubscription, MpscSink,
    PheromoneBus, PheromoneEvent, PheromoneSubscription, SinkError, SubscriptionId,
    SubscriptionSink, SubscriptionStats, VecSink,
};

use crate::chain::KnowledgeKind;
use roko_core::Kind;

/// Maps a `roko_core::Kind` onto a [`KnowledgeKind`] for chain storage.
#[must_use]
pub fn map_kind(kind: &Kind) -> KnowledgeKind {
    match kind {
        Kind::Insight => KnowledgeKind::Insight,
        Kind::PlaybookRule | Kind::Skill => KnowledgeKind::Heuristic,
        Kind::CompileDiagnostic => KnowledgeKind::Warning,
        Kind::Task | Kind::Plan => KnowledgeKind::StrategyFragment,
        Kind::ExperimentResult => KnowledgeKind::CausalLink,
        Kind::Custom(s) if s.contains("anti") => KnowledgeKind::AntiKnowledge,
        _ => KnowledgeKind::Insight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::Kind;

    #[test]
    fn map_kind_covers_core_variants() {
        assert_eq!(map_kind(&Kind::Insight), KnowledgeKind::Insight);
        assert_eq!(map_kind(&Kind::PlaybookRule), KnowledgeKind::Heuristic);
        assert_eq!(map_kind(&Kind::Skill), KnowledgeKind::Heuristic);
        assert_eq!(map_kind(&Kind::CompileDiagnostic), KnowledgeKind::Warning);
        assert_eq!(map_kind(&Kind::Task), KnowledgeKind::StrategyFragment);
        assert_eq!(map_kind(&Kind::ExperimentResult), KnowledgeKind::CausalLink);
        assert_eq!(
            map_kind(&Kind::Custom("com.example.anti_insight".into())),
            KnowledgeKind::AntiKnowledge
        );
        assert_eq!(map_kind(&Kind::AgentMessage), KnowledgeKind::Insight);
    }
}

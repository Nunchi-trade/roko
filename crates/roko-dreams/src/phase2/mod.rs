//! Phase 2+ dream surfaces described in the docs but not yet implemented.
//!
//! These modules provide compile-clean type shells so the documented target
//! state is visible in the crate without widening the shipped runtime.

pub mod advanced;
pub mod cycle;
/// Divergence and alpha-convergence stubs.
pub mod divergence;
pub mod evolution;
/// Spectral provenance and hauntology stubs.
pub mod hauntology;
pub mod hypnagogia;
pub mod imagination;
pub mod integration;
/// Oneirography and dream-art stubs.
pub mod oneirography;
/// TUI rendering stubs for dream visualization.
pub mod rendering;
pub mod replay;
pub mod shared;
/// Sleep-time compute and precomputation stubs.
pub mod sleep_time;
pub mod synthesis;
pub mod threat;

pub use advanced::{
    ComputationalMicrostate, ConstitutionalPrinciple, ConstitutionalSelfCritique, DreamJournal,
    DreamJournalEntry, DreamTrendAnalysis, InsightEnvironmentSnapshot, LucidDreamMonitor,
    NeuroinformedLucidMonitor, NightmareClass, NightmareContainment, NightmareDecision,
    NightmareDetector, NightmareReport, PrincipleSeverity, TemporalValidityTracker,
};
pub use cycle::{
    DreamOutcomeEvent, DreamPhase, DreamQualityDashboard, DreamQualityTrend, ExtendedDreamPhase,
    MicroConsolidation,
};
pub use divergence::{DivergenceMetrics, DivergenceTargets};
pub use evolution::{
    ArchiveCell, BayesianMemeticFitness, DescriptorDimension, FitnessClassification,
    FitnessEvaluation, MapElitesArchive, TournamentRecombination,
};
pub use hauntology::{SpectralInfluenceMetrics, SpectralProvenance};
pub use hypnagogia::{
    HypnagogiaBudget, HypnagogiaConfig, HypnagogiaInsightPipeline, HypnagogicFragment,
    HypnagogicSession, HypnagogicTemperatureCurve, IncubationCueSource, NoveltyFilter,
    TargetedDreamIncubation,
};
pub use imagination::{
    Association, AssociationEngine, AssociationStrength, BacktrackingCounterfactualConfig,
    CausalEdge, CausalGraph, CausalVariable, CombinationalConfig, CounterfactualEngine,
    CounterfactualHypothesis, CounterfactualResult, CreativityMode, DeltaEpisodeEncoder,
    DepotentiationConfig, ExploratoryConfig, GenerationMode, ImaginationBudget,
    ImaginationModeAllocations, ImaginationQualityReport, ImaginationROITracker,
    ImaginationValidator, InteractiveCounterfactualConfig, InterventionEngine, ModeROI,
    TransformationalConfig, WorldModelConfig,
};
pub use integration::{
    CircadianScheduler, DreamShareConfig, DreamShareMode, DreamShareProtocol,
    FleetDreamCoordinator, SharedDreamInsight, SynapticRenormalization,
};
pub use oneirography::{
    AgentStateVector, ArtQualityAssessment, AuctionParams, AuctionType, ImageGenProvider,
    PortfolioAnalytics, SelfAppraisalAction,
};
pub use rendering::{AnimationType, BorderStyle, DreamRenderConfig, PhaseVisualSpec};
pub use replay::{
    AdaptiveRolloutConfig, EpisodicSpacingTracker, GoalEnsembleReplay, GoalHypothesis,
    InsightConsolidator, InsightRelation, ReplayFidelity, ReplayFidelityConfig, ReplayMode,
    ReplayScheduleConfig, RolloutResult, SpacingEntry, TripleCouplingScheduler,
};
pub use shared::{
    BasicEvent, BudgetSnapshot, CausalEdgeSnap, ColorPalette, DepotentiationSummary,
    EvolutionaryStrategy, HdcVector, Hypothesis, ImageGenRequest, ImageGenResult, InsightRecord,
    NeuroDigest, NeuroDigestEntry, PatternRecord, PrivacyLevel, ThreatTier,
};
pub use sleep_time::{PrecomputedSummary, SleepTimePrecompute};
pub use synthesis::{
    CounterfactualDiversityConfig, CounterfactualSet, GlobalCounterfactual,
    GlobalCounterfactualConfig, KMedoidsConfig, PlausibilityReport, PlausibilityScorer,
    TransportCostFunction, TransportCounterfactualConfig,
};
pub use threat::{
    AdvancedRedTeamConfig, AttackCategory, AttackPrimitive, AttackPrimitiveSource,
    ConstitutionalClassifierConfig, FailureMode, FaultGate, FaultGateType, FaultTree, RiskZone,
    ThreatAssessment, ThreatGenerator, ThreatSeverityAssessor,
};

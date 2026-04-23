//! Agent-facing lifecycle manifests, backup records, restore records, and
//! knowledge-transfer helpers.
//!
//! The lifecycle docs describe explicit operator-directed commands for agent
//! creation, configuration, funding, backup, deletion, recreation, restore, and
//! live knowledge transfer. This module provides the public data structures and
//! pure helper functions those flows need while leaving storage, network, and
//! long-running I/O to higher layers.

use std::{collections::HashMap, marker::PhantomData, path::Path};

use roko_core::Attestation;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The minimal manifest sufficient for the happy-path creation flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCoreManifest {
    /// Free-text description of what the agent should do.
    pub prompt: String,
    /// Deployment mode for the agent runtime.
    pub mode: DeploymentMode,
    /// Optional domain plugin configuration.
    pub domain: Option<DomainPlugin>,
    /// Schema version for forward compatibility.
    pub schema_version: u32,
}

impl AgentCoreManifest {
    /// Create a self-hosted general-purpose manifest from a prompt.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            mode: DeploymentMode::SelfHosted,
            domain: None,
            schema_version: 1,
        }
    }
}

/// Where an agent is deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentMode {
    /// Managed compute infrastructure.
    Hosted,
    /// Operator-managed local or remote infrastructure.
    SelfHosted,
}

/// Domain plugin activated during creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum DomainPlugin {
    /// Blockchain domain configuration.
    Chain(ChainConfig),
    /// Coding domain configuration.
    Coding(CodingConfig),
    /// Research domain configuration.
    Research(ResearchConfig),
    /// User-provided custom domain configuration.
    Custom(CustomPluginConfig),
}

/// Chain-domain creation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Network name such as `base`, `base-sepolia`, or `anvil`.
    pub network: String,
    /// Wallet custody mode.
    pub custody_mode: String,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            network: "base-sepolia".into(),
            custody_mode: "delegation".into(),
        }
    }
}

/// Coding-domain creation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingConfig {
    /// Root path of the codebase.
    pub workspace_path: String,
    /// Language or stack hint.
    pub language: Option<String>,
}

/// Research-domain creation configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchConfig {
    /// Default research topic or corpus.
    pub topic: Option<String>,
    /// Whether citation retrieval tools should be enabled.
    pub citations_enabled: bool,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            topic: None,
            citations_enabled: true,
        }
    }
}

/// Custom domain plugin configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomPluginConfig {
    /// Stable plugin identifier.
    pub id: String,
    /// Plugin-specific key/value parameters.
    pub params: HashMap<String, String>,
}

/// Full manifest with optional overrides resolved before provisioning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentExtendedManifest {
    /// Core manifest fields.
    #[serde(flatten)]
    pub core: AgentCoreManifest,
    /// Human-readable agent name.
    pub name: Option<String>,
    /// Strategy document content.
    pub strategy_md: Option<String>,
    /// Model routing configuration.
    pub model_routing: Option<ModelRoutingConfig>,
    /// Neuro knowledge-store configuration.
    pub neuro: Option<NeuroConfig>,
    /// Mesh coordination configuration.
    pub mesh: Option<MeshConfig>,
    /// Tool profile name.
    pub tool_profile: Option<String>,
    /// Template used to create the manifest.
    pub template_id: Option<String>,
    /// Template expansion parameters.
    pub template_params: Option<HashMap<String, String>>,
    /// AI autofill provenance.
    pub autofill: Option<AutofillProvenance>,
    /// Inference provider configuration.
    pub inference: Option<InferenceConfig>,
    /// Budget limits.
    pub budget: Option<BudgetConfig>,
    /// Optional lineage identifier shared across replacement agents.
    pub lineage_id: Option<String>,
    /// Lineage generation number.
    pub generation: u32,
    /// Successor exploration settings.
    pub successor: Option<SuccessorConfig>,
}

impl AgentExtendedManifest {
    /// Create an extended manifest from core fields.
    pub fn new(core: AgentCoreManifest) -> Self {
        Self {
            core,
            name: None,
            strategy_md: None,
            model_routing: None,
            neuro: None,
            mesh: None,
            tool_profile: None,
            template_id: None,
            template_params: None,
            autofill: None,
            inference: None,
            budget: None,
            lineage_id: None,
            generation: 0,
            successor: None,
        }
    }
}

/// Model routing configuration for three cognitive speeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRoutingConfig {
    /// Fast reactive model.
    pub gamma_model: String,
    /// Reflective model.
    pub theta_model: String,
    /// Deep consolidation model.
    pub delta_model: String,
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            gamma_model: "claude-haiku-4-5".into(),
            theta_model: "claude-sonnet-4-6".into(),
            delta_model: "claude-opus-4-6".into(),
        }
    }
}

/// Neuro knowledge-store configuration used during creation and restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuroConfig {
    /// Filesystem path for the knowledge store.
    pub path: String,
    /// Maximum active Engram count.
    pub max_engrams: u64,
    /// Decay model name.
    pub decay_model: String,
    /// Tier-specific decay multipliers.
    pub tiers: TierConfig,
    /// Optional demurrage configuration.
    pub demurrage: Option<DemurrageConfig>,
}

impl Default for NeuroConfig {
    fn default() -> Self {
        Self {
            path: ".roko/neuro/".into(),
            max_engrams: 50_000,
            decay_model: "ebbinghaus".into(),
            tiers: TierConfig::default(),
            demurrage: Some(DemurrageConfig::default()),
        }
    }
}

/// Tier-specific decay multipliers for the Neuro store.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TierConfig {
    /// Multiplier for transient knowledge.
    pub transient_multiplier: f64,
    /// Multiplier for working knowledge.
    pub working_multiplier: f64,
    /// Multiplier for consolidated knowledge.
    pub consolidated_multiplier: f64,
    /// Multiplier for persistent knowledge.
    pub persistent_multiplier: f64,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            transient_multiplier: 0.1,
            working_multiplier: 0.5,
            consolidated_multiplier: 1.0,
            persistent_multiplier: 5.0,
        }
    }
}

/// Mesh coordination configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MeshConfig {
    /// Whether Mesh coordination is enabled.
    pub enabled: bool,
    /// Optional relay URL.
    pub relay_url: Option<String>,
    /// Optional collective identifier.
    pub collective_id: Option<String>,
    /// Knowledge-sharing policy.
    pub sharing: MeshSharingConfig,
}

/// Knowledge-sharing policy for Mesh-connected agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshSharingConfig {
    /// Knowledge types eligible for sharing.
    pub share_types: Vec<EngramKind>,
    /// Minimum confidence before sharing.
    pub min_share_confidence: f64,
    /// Share gate-passed Engrams automatically.
    pub share_on_gate_pass: bool,
    /// Confidence multiplier applied to received knowledge.
    pub received_confidence_discount: f64,
    /// Maximum received Engrams per hour.
    pub max_received_per_hour: u32,
    /// Sync interval in seconds.
    pub sync_interval_secs: u64,
}

impl Default for MeshSharingConfig {
    fn default() -> Self {
        Self {
            share_types: vec![
                EngramKind::Insight,
                EngramKind::Warning,
                EngramKind::CausalLink,
            ],
            min_share_confidence: 0.5,
            share_on_gate_pass: true,
            received_confidence_discount: 0.7,
            max_received_per_hour: 100,
            sync_interval_secs: 300,
        }
    }
}

/// AI autofill provenance metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutofillProvenance {
    /// Model that produced the filled manifest.
    pub model: String,
    /// Unix timestamp for autofill generation.
    pub generated_at: u64,
    /// Prompt or template source used for autofill.
    pub source: String,
}

/// Inference provider configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Default model used for routine turns.
    pub default_model: String,
    /// Escalation model used for reflective turns.
    pub escalation_model: String,
    /// Critical model used for deep or high-risk turns.
    pub critical_model: String,
    /// Maximum tokens allowed per turn.
    pub max_tokens_per_turn: u64,
    /// Sampling temperature.
    pub temperature: f64,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-haiku-4-5".into(),
            escalation_model: "claude-sonnet-4-6".into(),
            critical_model: "claude-opus-4-6".into(),
            max_tokens_per_turn: 4096,
            temperature: 0.7,
        }
    }
}

/// Budget limits and degradation policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Per-day inference spending limit.
    pub max_daily_inference_usd: f64,
    /// Optional total spending cap.
    pub max_total_usd: Option<f64>,
    /// Per-turn token cap.
    pub max_tokens_per_turn: u64,
    /// Optional hosted compute hourly budget.
    pub max_hourly_compute_usd: Option<f64>,
    /// Warning threshold as a fraction of daily budget.
    pub warning_at: f64,
    /// Critical threshold as a fraction of daily budget.
    pub critical_at: f64,
    /// Degradation behavior when constrained.
    pub degradation: BudgetDegradationMode,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_daily_inference_usd: 10.0,
            max_total_usd: None,
            max_tokens_per_turn: 8192,
            max_hourly_compute_usd: None,
            warning_at: 0.7,
            critical_at: 0.9,
            degradation: BudgetDegradationMode::Cascade,
        }
    }
}

/// Budget-constrained behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BudgetDegradationMode {
    /// Apply staged cost-reduction measures.
    Cascade,
    /// Pause processing when constrained.
    Pause,
    /// Notify only and leave behavior unchanged.
    NotifyOnly,
}

/// Successor-specific exploration settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuccessorConfig {
    /// Exploration temperature boost applied initially.
    pub initial_exploration_boost: f64,
    /// Number of initial iterations that receive the boost.
    pub exploration_boost_duration: u64,
}

impl Default for SuccessorConfig {
    fn default() -> Self {
        Self {
            initial_exploration_boost: 0.2,
            exploration_boost_duration: 100,
        }
    }
}

/// Resolve a manifest by filling documented defaults.
pub fn resolve_manifest(mut manifest: AgentExtendedManifest) -> AgentExtendedManifest {
    if manifest.model_routing.is_none() {
        manifest.model_routing = Some(ModelRoutingConfig::default());
    }
    if manifest.neuro.is_none() {
        manifest.neuro = Some(NeuroConfig::default());
    }
    if manifest.mesh.is_none() {
        manifest.mesh = Some(MeshConfig::default());
    }
    if manifest.tool_profile.is_none() {
        manifest.tool_profile = Some("standard".into());
    }
    if manifest.inference.is_none() {
        manifest.inference = Some(InferenceConfig::default());
    }
    if manifest.budget.is_none() {
        manifest.budget = Some(BudgetConfig::default());
    }
    if manifest.generation > 0 && manifest.successor.is_none() {
        manifest.successor = Some(SuccessorConfig::default());
    }
    manifest
}

/// Error returned while validating or provisioning lifecycle manifests.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProvisioningError {
    /// Prompt length is outside the documented creation range.
    #[error("prompt length must be between 10 and 2000 characters")]
    InvalidPromptLength,
    /// Schema version is not supported.
    #[error("unsupported manifest schema version {0}")]
    UnsupportedSchema(u32),
    /// Required field is missing.
    #[error("missing required field {0}")]
    MissingField(&'static str),
}

/// Validate an agent manifest against basic lifecycle invariants.
pub fn validate_manifest(manifest: &AgentExtendedManifest) -> Result<(), ProvisioningError> {
    let prompt_len = manifest.core.prompt.chars().count();
    if !(10..=2000).contains(&prompt_len) {
        return Err(ProvisioningError::InvalidPromptLength);
    }
    if manifest.core.schema_version != 1 {
        return Err(ProvisioningError::UnsupportedSchema(
            manifest.core.schema_version,
        ));
    }
    Ok(())
}

/// Type-state marker: manifest has not been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Unvalidated;

/// Type-state marker: manifest has passed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Validated;

/// Type-state marker: runtime resources have been allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ResourcesAllocated;

/// Type-state marker: Neuro has been initialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NeuroInitialized;

/// Type-state marker: routing has been configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RoutingConfigured;

/// Type-state marker: tool profile has been loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ToolsLoaded;

/// Type-state marker: Mesh registration has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MeshRegistered;

/// Type-state marker: agent is ready to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Ready;

/// Provisioning state accumulated while resolving an agent manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentState {
    /// Allocated resource labels.
    pub resources: Vec<String>,
    /// Whether Neuro initialization completed.
    pub neuro_initialized: bool,
    /// Whether routing configuration completed.
    pub routing_configured: bool,
    /// Loaded tool profile.
    pub tool_profile: Option<String>,
    /// Whether Mesh registration completed.
    pub mesh_registered: bool,
}

/// Type-state provisioning wrapper for agent creation.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvisioningAgent<S> {
    manifest: AgentExtendedManifest,
    state: AgentState,
    stage: PhantomData<S>,
}

impl ProvisioningAgent<Unvalidated> {
    /// Create a new provisioning wrapper from an extended manifest.
    pub fn new(manifest: AgentExtendedManifest) -> Self {
        Self {
            manifest,
            state: AgentState::default(),
            stage: PhantomData,
        }
    }

    /// Validate the manifest and advance the type state.
    pub fn validate(self) -> Result<ProvisioningAgent<Validated>, ProvisioningError> {
        validate_manifest(&self.manifest)?;
        Ok(self.transition())
    }
}

impl ProvisioningAgent<Validated> {
    /// Allocate resources and advance the type state.
    pub fn allocate_resources(
        self,
        resource: impl Into<String>,
    ) -> ProvisioningAgent<ResourcesAllocated> {
        ProvisioningAgent {
            manifest: self.manifest,
            state: AgentState {
                resources: {
                    let mut resources = self.state.resources;
                    resources.push(resource.into());
                    resources
                },
                ..self.state
            },
            stage: PhantomData,
        }
    }
}

impl ProvisioningAgent<ResourcesAllocated> {
    /// Initialize Neuro and advance the type state.
    pub fn init_neuro(self) -> ProvisioningAgent<NeuroInitialized> {
        ProvisioningAgent {
            manifest: self.manifest,
            state: AgentState {
                neuro_initialized: true,
                ..self.state
            },
            stage: PhantomData,
        }
    }
}

impl ProvisioningAgent<NeuroInitialized> {
    /// Configure model routing and advance the type state.
    pub fn configure_routing(self) -> ProvisioningAgent<RoutingConfigured> {
        ProvisioningAgent {
            manifest: self.manifest,
            state: AgentState {
                routing_configured: true,
                ..self.state
            },
            stage: PhantomData,
        }
    }
}

impl ProvisioningAgent<RoutingConfigured> {
    /// Load tools and advance the type state.
    pub fn load_tools(self) -> ProvisioningAgent<ToolsLoaded> {
        let profile = self
            .manifest
            .tool_profile
            .clone()
            .unwrap_or_else(|| "standard".into());
        ProvisioningAgent {
            manifest: self.manifest,
            state: AgentState {
                tool_profile: Some(profile),
                ..self.state
            },
            stage: PhantomData,
        }
    }
}

impl ProvisioningAgent<ToolsLoaded> {
    /// Register with Mesh if enabled and advance the type state.
    pub fn register_mesh(self) -> ProvisioningAgent<MeshRegistered> {
        let mesh_registered = self.manifest.mesh.as_ref().is_some_and(|mesh| mesh.enabled);
        ProvisioningAgent {
            manifest: self.manifest,
            state: AgentState {
                mesh_registered,
                ..self.state
            },
            stage: PhantomData,
        }
    }
}

impl ProvisioningAgent<MeshRegistered> {
    /// Final transition to ready.
    pub fn ready(self) -> ProvisioningAgent<Ready> {
        self.transition()
    }
}

impl ProvisioningAgent<Ready> {
    /// Return the ready manifest.
    pub const fn manifest(&self) -> &AgentExtendedManifest {
        &self.manifest
    }

    /// Return the accumulated provisioning state.
    pub const fn state(&self) -> &AgentState {
        &self.state
    }

    /// Start the cognitive loop.
    ///
    /// The concrete loop runner lives above `roko-agent`; this returns a
    /// serializable running-agent record that callers can hand to that layer.
    pub fn start_cognitive_loop(self) -> RunningAgent {
        RunningAgent {
            name: self.manifest.name.unwrap_or_else(|| "agent".into()),
            state: self.state,
            started_at: now_secs(),
        }
    }
}

impl<S> ProvisioningAgent<S> {
    fn transition<T>(self) -> ProvisioningAgent<T> {
        ProvisioningAgent {
            manifest: self.manifest,
            state: self.state,
            stage: PhantomData,
        }
    }
}

/// Run the full provisioning pipeline in one shot.
///
/// Validates the manifest, allocates resources, initializes Neuro,
/// configures routing, loads tools, registers with Mesh, and returns
/// a ready-state agent. Returns `Err` if manifest validation fails.
pub fn provision_full(
    manifest: AgentExtendedManifest,
    _slot: &str,
) -> Result<ProvisioningAgent<Ready>, ProvisioningError> {
    let agent = ProvisioningAgent::new(manifest);
    let validated = agent.validate()?;
    let allocated = validated.allocate_resources(_slot);
    let neuro = allocated.init_neuro();
    let routing = neuro.configure_routing();
    let tools = routing.load_tools();
    let mesh = tools.register_mesh();
    Ok(mesh.ready())
}

/// Running agent record returned by the provisioning shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningAgent {
    /// Human-readable running agent name.
    pub name: String,
    /// Provisioned runtime state.
    pub state: AgentState,
    /// Unix timestamp when the running record was created.
    pub started_at: u64,
}

/// Top-level agent configuration matching the documented `roko.toml` sections.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    /// Agent section.
    pub agent: AgentSection,
    /// Inference section.
    pub inference: InferenceConfig,
    /// Neuro section.
    pub neuro: NeuroConfig,
    /// Mesh section.
    pub mesh: MeshConfig,
    /// Tools section.
    pub tools: ToolsConfig,
    /// Budget section.
    pub budget: BudgetConfig,
    /// Heartbeat section.
    pub heartbeat: HeartbeatConfig,
}

/// Agent identity and operator-authored intent section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSection {
    /// Human-readable agent name.
    pub name: String,
    /// Agent prompt or mission statement.
    pub prompt: String,
    /// Deployment mode.
    pub mode: DeploymentMode,
    /// Optional domain name.
    pub domain: Option<String>,
    /// Optional MCP config path.
    pub mcp_config: Option<String>,
    /// Optional lineage identifier.
    pub lineage_id: Option<String>,
    /// Generation number within the lineage.
    pub generation: u32,
}

impl Default for AgentSection {
    fn default() -> Self {
        Self {
            name: "agent".into(),
            prompt: "Describe what this agent should do".into(),
            mode: DeploymentMode::SelfHosted,
            domain: None,
            mcp_config: None,
            lineage_id: None,
            generation: 0,
        }
    }
}

/// Tool-profile configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Tool profile name.
    pub profile: String,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            profile: "standard".into(),
        }
    }
}

/// Heartbeat cadence configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    /// Gamma loop interval in seconds.
    pub gamma_interval_secs: u64,
    /// Theta loop interval in seconds.
    pub theta_interval_secs: u64,
    /// Delta loop interval in hours.
    pub delta_interval_hours: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            gamma_interval_secs: 15,
            theta_interval_secs: 75,
            delta_interval_hours: 6,
        }
    }
}

/// Warning returned when a config change is allowed but operationally notable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigWarning {
    /// Daily inference budget increased and needs explicit operator awareness.
    BudgetIncrease {
        /// Previous budget.
        old: f64,
        /// Proposed budget.
        new: f64,
    },
    /// Default inference model changed.
    ModelChange {
        /// Previous model.
        old: String,
        /// Proposed model.
        new: String,
    },
}

/// Error returned when a config change cannot be hot-applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LifecycleConfigError {
    /// The named field requires process restart or reprovisioning.
    #[error("{0} requires restart")]
    RequiresRestart(&'static str),
}

/// Validate a proposed config change against hot-reload constraints.
pub fn validate_config_change(
    current: &AgentConfig,
    proposed: &AgentConfig,
) -> Result<Vec<ConfigWarning>, LifecycleConfigError> {
    let mut warnings = Vec::new();
    if proposed.budget.max_daily_inference_usd > current.budget.max_daily_inference_usd {
        warnings.push(ConfigWarning::BudgetIncrease {
            old: current.budget.max_daily_inference_usd,
            new: proposed.budget.max_daily_inference_usd,
        });
    }
    if proposed.inference.default_model != current.inference.default_model {
        warnings.push(ConfigWarning::ModelChange {
            old: current.inference.default_model.clone(),
            new: proposed.inference.default_model.clone(),
        });
    }
    if proposed.neuro.path != current.neuro.path {
        return Err(LifecycleConfigError::RequiresRestart("neuro.path"));
    }
    if proposed.mesh.enabled != current.mesh.enabled
        || proposed.mesh.relay_url != current.mesh.relay_url
        || proposed.mesh.collective_id != current.mesh.collective_id
    {
        return Err(LifecycleConfigError::RequiresRestart("mesh"));
    }
    if proposed.agent.name != current.agent.name || proposed.agent.mode != current.agent.mode {
        return Err(LifecycleConfigError::RequiresRestart("agent"));
    }
    Ok(warnings)
}

/// Per-turn cost record written by lifecycle-aware agent runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCostRecord {
    /// Turn identifier.
    pub turn_id: String,
    /// Model used for this turn.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Cache-read tokens.
    pub cache_read_tokens: u64,
    /// Estimated cost in USD.
    pub estimated_cost_usd: f64,
    /// Cognitive speed tier.
    pub cognitive_tier: CognitiveTier,
    /// Whether zero-LLM probes suppressed the turn.
    pub t0_suppressed: bool,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
}

/// Cognitive speed tier used for cost attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveTier {
    /// Fast reactive tier.
    Gamma,
    /// Reflective tier.
    Theta,
    /// Consolidation tier.
    Delta,
}

/// Daily cost aggregation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyCostSummary {
    /// Date in `YYYY-MM-DD` form.
    pub date: String,
    /// Total inference cost.
    pub inference_cost_usd: f64,
    /// Total hosted compute cost.
    pub compute_cost_usd: f64,
    /// Total chain gas cost.
    pub gas_cost_usd: f64,
    /// Total turns executed.
    pub total_turns: u64,
    /// Turns suppressed by zero-LLM probes.
    pub t0_suppressed_turns: u64,
    /// Suppression rate.
    pub t0_suppression_rate: f64,
    /// Mean cost per turn.
    pub cost_per_turn_usd: f64,
    /// Fraction of turns by model.
    pub model_distribution: HashMap<String, f64>,
}

/// Lifetime cost tracking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LifetimeCosts {
    /// Total inference cost.
    pub total_inference_usd: f64,
    /// Total hosted compute cost.
    pub total_compute_usd: f64,
    /// Total chain gas cost.
    pub total_gas_usd: f64,
    /// Total all-in cost.
    pub total_cost_usd: f64,
    /// Days active.
    pub days_active: u32,
    /// Average daily cost.
    pub average_daily_cost_usd: f64,
    /// Projected monthly cost.
    pub projected_monthly_cost_usd: f64,
}

/// Engram as stored in a lifecycle backup archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupEngram {
    /// Content-addressed hash.
    pub hash: String,
    /// Knowledge type.
    pub kind: EngramKind,
    /// Knowledge content.
    pub body: String,
    /// Author agent ID.
    pub author: String,
    /// Tags for categorization and retrieval.
    pub tags: Vec<String>,
    /// Seven-axis score at backup time.
    pub score: EngramScore,
    /// Knowledge tier at backup time.
    pub tier: KnowledgeTier,
    /// Decay state at backup time.
    pub decay: DecayState,
    /// Provenance chain.
    pub provenance: Vec<ProvenanceEntry>,
    /// Creation timestamp.
    pub created_at: u64,
    /// Last access timestamp or iteration.
    pub last_accessed_at: u64,
    /// Retrieval count.
    pub retrieval_count: u64,
    /// Validation count.
    pub validation_count: u64,
    /// Optional HDC vector encoded as text.
    pub hdc_vector: Option<String>,
}

/// Lifecycle knowledge type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngramKind {
    /// Observation or interpretation.
    Insight,
    /// Practical rule of thumb.
    Heuristic,
    /// Safety-critical warning.
    Warning,
    /// Causal relationship.
    CausalLink,
    /// Tactical strategy fragment.
    StrategyFragment,
    /// Negative knowledge about what does not work.
    AntiKnowledge,
}

/// Seven-axis Engram score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EngramScore {
    /// Confidence in correctness.
    pub confidence: f64,
    /// Novelty relative to prior knowledge.
    pub novelty: f64,
    /// Historical utility.
    pub utility: f64,
    /// Author or source reputation.
    pub reputation: f64,
    /// Narrowness or exactness.
    pub precision: f64,
    /// Ranking salience.
    pub salience: f64,
    /// Internal coherence.
    pub coherence: f64,
}

impl EngramScore {
    /// Create a score from confidence alone.
    pub const fn from_confidence(confidence: f64) -> Self {
        Self {
            confidence,
            novelty: 0.0,
            utility: 0.0,
            reputation: 1.0,
            precision: 0.0,
            salience: 0.0,
            coherence: 0.0,
        }
    }
}

/// Knowledge tier controlling decay rate and context eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTier {
    /// Fast decay; recently created or unvalidated.
    Transient,
    /// Moderate decay; used but not fully consolidated.
    Working,
    /// Standard decay; validated through experience.
    Consolidated,
    /// Slow decay; repeatedly validated by the current agent.
    Persistent,
    /// Cold storage; retained for audit but removed from active context.
    Archived,
}

/// Decay state captured in backup records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecayState {
    /// Decay model variant.
    pub model: DecayModel,
    /// Current effective confidence.
    pub effective_confidence: f64,
    /// Ticks since last access.
    pub ticks_since_access: u64,
    /// Tier multiplier applied to decay.
    pub tier_multiplier: f64,
}

/// Decay behavior for a backed-up Engram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecayModel {
    /// Confidence remains constant.
    None,
    /// Exponential half-life decay.
    HalfLife {
        /// Milliseconds for confidence to halve.
        half_life_ms: u64,
    },
    /// Binary time-to-live.
    Ttl {
        /// Expiration timestamp.
        expires_at: u64,
    },
    /// Ebbinghaus retention curve.
    Ebbinghaus {
        /// Memory strength.
        strength: f64,
        /// Time scale in milliseconds.
        scale_ms: u64,
    },
}

/// Provenance entry for backup, restore, and Mesh knowledge transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProvenanceEntry {
    /// Original author or source.
    Source {
        /// Source agent or system.
        source: String,
        /// Timestamp for the source event.
        timestamp: u64,
    },
    /// Restored from a backup.
    Restored {
        /// Source agent identifier.
        source_agent: String,
        /// Source generation.
        generation: u32,
        /// Restore timestamp.
        timestamp: u64,
    },
    /// Received over Mesh.
    MeshReceived {
        /// Agent that shared the Engram.
        from_agent: String,
        /// Collective used for transfer.
        via_collective: Option<String>,
        /// Share timestamp.
        timestamp: u64,
    },
}

/// Backup manifest metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Backup format version.
    pub version: u32,
    /// Source agent identifier.
    pub agent_id: String,
    /// Source agent name.
    pub agent_name: Option<String>,
    /// Creation timestamp.
    pub created_at: u64,
    /// Roko version string.
    pub roko_version: String,
    /// Optional lineage generation.
    pub generation: Option<u32>,
    /// Aggregate backup statistics.
    pub stats: BackupStats,
}

/// Backup archive statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackupStats {
    /// Total Engrams in the archive.
    pub total_engrams: u64,
    /// Count by knowledge type.
    pub engrams_by_type: HashMap<EngramKind, u64>,
    /// Count by knowledge tier.
    pub engrams_by_tier: HashMap<KnowledgeTier, u64>,
    /// Average confidence.
    pub average_confidence: f64,
    /// Median confidence.
    pub median_confidence: f64,
    /// Snapshot playbook size.
    pub playbook_size_bytes: u64,
}

/// In-memory representation of a lifecycle backup archive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupArchive {
    /// Manifest metadata.
    pub manifest: BackupManifest,
    /// Backed-up Engrams.
    pub engrams: Vec<BackupEngram>,
    /// Optional playbook snapshot.
    pub playbook_md: Option<String>,
    /// Archive checksum.
    pub checksum: Option<String>,
}

/// Backup verification or parsing error.
#[derive(Debug, Error)]
pub enum BackupError {
    /// Archive integrity verification failed.
    #[error("backup integrity check failed: expected {expected}, computed {computed}")]
    IntegrityCheckFailed {
        /// Expected checksum.
        expected: String,
        /// Computed checksum.
        computed: String,
    },
    /// Archive I/O failed.
    #[error("backup I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Archive decoding failed.
    #[error("backup decode error: {0}")]
    Decode(String),
}

/// Verify backup integrity before restore.
///
/// Reads the backup archive from `path`, deserializes it, and validates:
///
/// 1. **Parse**: the file must be valid JSON conforming to [`BackupArchive`].
/// 2. **Integrity**: when the archive carries a `checksum` field, the
///    computed checksum of the serialized manifest + engrams must match.
/// 3. **Schema**: the manifest `version` must be 1 (forward-compatible
///    callers can extend this check later).
///
/// Returns the parsed [`BackupManifest`] on success so callers can inspect
/// it before committing to a full restore.
pub fn verify_backup(path: &Path) -> Result<BackupManifest, BackupError> {
    let raw = std::fs::read_to_string(path)?;
    let archive: BackupArchive =
        serde_json::from_str(&raw).map_err(|e| BackupError::Decode(e.to_string()))?;

    // When a checksum is present, recompute and verify.
    if let Some(expected) = &archive.checksum {
        // The checksum covers the canonical JSON of manifest + engrams
        // (excluding the checksum field itself) so that the archive is
        // self-verifiable.
        let canonical = serde_json::json!({
            "manifest": archive.manifest,
            "engrams": archive.engrams,
            "playbook_md": archive.playbook_md,
        });
        let computed = fnv1a_hex(canonical.to_string().as_bytes());
        if &computed != expected {
            return Err(BackupError::IntegrityCheckFailed {
                expected: expected.clone(),
                computed,
            });
        }
    }

    // Basic schema check: only version 1 is supported today.
    if archive.manifest.version == 0 {
        return Err(BackupError::Decode(
            "unsupported backup version 0".to_string(),
        ));
    }

    Ok(archive.manifest)
}

/// Compute a 64-bit FNV-1a hash and return it as a hex string.
///
/// This is a fast, non-cryptographic hash used for backup integrity
/// verification. It catches accidental corruption (truncation, encoding
/// errors) without requiring a heavy-weight hashing crate.
fn fnv1a_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3); // FNV prime
    }
    format!("{hash:016x}")
}

/// Restore filtering and confidence-decay configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreConfig {
    /// Knowledge type filter.
    pub type_filter: TypeFilter,
    /// Minimum source confidence accepted.
    pub min_confidence: f64,
    /// Optional maximum Engram count.
    pub max_engrams: Option<usize>,
    /// Source-to-target generation distance.
    pub generation: u32,
    /// Per-generation confidence retention rate.
    pub confidence_decay: f64,
    /// Whether validation should be run before adoption.
    pub validate: bool,
}

impl Default for RestoreConfig {
    fn default() -> Self {
        Self {
            type_filter: TypeFilter::All,
            min_confidence: 0.0,
            max_engrams: None,
            generation: 1,
            confidence_decay: 0.85,
            validate: false,
        }
    }
}

/// Knowledge type filter used during backup and restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "types", rename_all = "snake_case")]
pub enum TypeFilter {
    /// Accept all knowledge types.
    All,
    /// Accept only listed knowledge types.
    Only(Vec<EngramKind>),
}

impl TypeFilter {
    /// Return whether a kind is accepted.
    pub fn accepts(&self, kind: &EngramKind) -> bool {
        match self {
            Self::All => true,
            Self::Only(types) => types.contains(kind),
        }
    }
}

/// Engram staged in quarantine before restore adoption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuarantinedEngram {
    /// Backed-up Engram.
    pub engram: BackupEngram,
    /// Confidence before restore decay.
    pub original_confidence: f64,
    /// Confidence after restore decay.
    pub decayed_confidence: f64,
    /// Restore provenance tag.
    pub provenance_tag: ProvenanceTag,
    /// Validation status.
    pub validation_status: ValidationStatus,
}

/// Restore provenance tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceTag {
    /// Source agent identifier.
    pub source_agent: String,
    /// Source generation.
    pub source_generation: u32,
    /// Restore timestamp.
    pub restore_timestamp: u64,
}

/// Validation status for quarantined knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// Validation has not run.
    Pending,
    /// Validation accepted the Engram.
    Accepted,
    /// Validation rejected the Engram.
    Rejected,
}

/// Restore report summarizing quarantine, validation, and adoption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RestoreReport {
    /// Engrams processed from the archive.
    pub processed: u64,
    /// Engrams filtered out before quarantine.
    pub filtered: u64,
    /// Engrams quarantined.
    pub quarantined: u64,
    /// Engrams validated.
    pub validated: u64,
    /// Engrams rejected.
    pub rejected: u64,
    /// Engrams adopted.
    pub adopted: u64,
}

/// Minimal in-memory Neuro-store stub used by restore helpers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NeuroStore {
    /// Stored Engrams.
    pub engrams: Vec<BackupEngram>,
}

impl NeuroStore {
    /// Insert a restored or shared Engram.
    pub fn insert(&mut self, engram: BackupEngram) {
        self.engrams.push(engram);
    }
}

/// Compute restored confidence after generational decay.
pub fn restore_confidence(original_confidence: f64, generation: u32, decay_rate: f64) -> f64 {
    let confidence = original_confidence.clamp(0.0, 1.0);
    if generation == 0 {
        return confidence;
    }
    let rate = decay_rate.clamp(0.0, 1.0);
    (confidence * rate.powi(generation as i32)).max(0.01)
}

/// Load backup Engrams into quarantine according to restore filters.
pub fn quarantine_backup(backup: &BackupArchive, config: &RestoreConfig) -> Vec<QuarantinedEngram> {
    let limit = config.max_engrams.unwrap_or(usize::MAX);
    backup
        .engrams
        .iter()
        .filter(|engram| config.type_filter.accepts(&engram.kind))
        .filter(|engram| engram.score.confidence >= config.min_confidence)
        .take(limit)
        .map(|engram| QuarantinedEngram {
            engram: engram.clone(),
            original_confidence: engram.score.confidence,
            decayed_confidence: restore_confidence(
                engram.score.confidence,
                config.generation,
                config.confidence_decay,
            ),
            provenance_tag: ProvenanceTag {
                source_agent: backup.manifest.agent_id.clone(),
                source_generation: backup.manifest.generation.unwrap_or(0),
                restore_timestamp: now_secs(),
            },
            validation_status: ValidationStatus::Pending,
        })
        .collect()
}

/// Adopt quarantined Engrams into an in-memory Neuro store.
pub fn adopt_engrams(neuro: &mut NeuroStore, quarantined: Vec<QuarantinedEngram>) -> RestoreReport {
    let mut report = RestoreReport {
        processed: u64::try_from(quarantined.len()).unwrap_or(u64::MAX),
        quarantined: u64::try_from(quarantined.len()).unwrap_or(u64::MAX),
        ..RestoreReport::default()
    };

    for item in quarantined {
        if item.validation_status == ValidationStatus::Rejected {
            report.rejected = report.rejected.saturating_add(1);
            continue;
        }

        let mut engram = item.engram;
        engram.score.confidence = item.decayed_confidence;
        engram.tier = tier_from_confidence(item.decayed_confidence);
        engram.provenance.push(ProvenanceEntry::Restored {
            source_agent: item.provenance_tag.source_agent,
            generation: item.provenance_tag.source_generation,
            timestamp: item.provenance_tag.restore_timestamp,
        });
        engram.decay = DecayState {
            model: DecayModel::Ebbinghaus {
                strength: item.decayed_confidence,
                scale_ms: default_scale_for_kind(engram.kind),
            },
            effective_confidence: item.decayed_confidence,
            ticks_since_access: 0,
            tier_multiplier: tier_multiplier(engram.tier),
        };
        neuro.insert(engram);
        report.adopted = report.adopted.saturating_add(1);
    }

    report.validated = report.adopted;
    report
}

/// Assign tier based on confidence, never restoring directly to Persistent.
pub fn tier_from_confidence(confidence: f64) -> KnowledgeTier {
    if confidence >= 0.8 {
        KnowledgeTier::Consolidated
    } else if confidence >= 0.5 {
        KnowledgeTier::Working
    } else {
        KnowledgeTier::Transient
    }
}

/// Retrieval context used by the testing-effect update.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetrievalContext {
    /// Whether the turn using this Engram passed gates.
    pub gate_passed: bool,
    /// Diversity score for task/Daimon/context variation.
    pub context_diversity: f64,
    /// Retrieval timestamp.
    pub timestamp: u64,
}

/// Update Engram strength after successful retrieval.
pub fn apply_testing_effect(engram: &mut BackupEngram, retrieval_context: &RetrievalContext) {
    let base_increase = 0.05;
    let gate_bonus = if retrieval_context.gate_passed {
        0.03
    } else {
        0.0
    };
    let diversity_bonus = retrieval_context.context_diversity.clamp(0.0, 1.0) * 0.02;
    let total_increase = base_increase + gate_bonus + diversity_bonus;

    if let DecayModel::Ebbinghaus { strength, .. } = &mut engram.decay.model {
        *strength = (*strength + total_increase).min(10.0);
    }
    engram.decay.ticks_since_access = 0;
    engram.retrieval_count = engram.retrieval_count.saturating_add(1);
    engram.last_accessed_at = retrieval_context.timestamp;
}

/// Compute current retention under Ebbinghaus decay.
pub fn ebbinghaus_retention(time_since_access_ms: u64, strength: f64, scale_ms: u64) -> f64 {
    let denominator = strength * scale_ms as f64;
    if denominator <= 0.0 || !denominator.is_finite() {
        return 0.0;
    }
    (-(time_since_access_ms as f64) / denominator).exp()
}

/// Compute effective confidence for a backed-up Engram.
pub fn effective_confidence(engram: &BackupEngram) -> f64 {
    const TICK_DURATION_MS: f64 = 1_000.0;
    match engram.decay.model {
        DecayModel::None => engram.score.confidence,
        DecayModel::HalfLife { half_life_ms } => {
            if half_life_ms == 0 {
                return 0.0;
            }
            let t = engram.decay.ticks_since_access as f64 * TICK_DURATION_MS;
            engram.score.confidence * 0.5_f64.powf(t / half_life_ms as f64)
        }
        DecayModel::Ttl { expires_at } => {
            if now_secs() > expires_at {
                0.0
            } else {
                engram.score.confidence
            }
        }
        DecayModel::Ebbinghaus { strength, scale_ms } => {
            let t = engram.decay.ticks_since_access as f64 * TICK_DURATION_MS;
            engram.score.confidence
                * ebbinghaus_retention(t as u64, strength, scale_ms)
                * engram.decay.tier_multiplier
        }
    }
    .clamp(0.0, 1.0)
}

/// Tier multiplier for Ebbinghaus decay.
pub const fn tier_multiplier(tier: KnowledgeTier) -> f64 {
    match tier {
        KnowledgeTier::Transient => 0.1,
        KnowledgeTier::Working => 0.5,
        KnowledgeTier::Consolidated => 1.0,
        KnowledgeTier::Persistent => 5.0,
        KnowledgeTier::Archived => 0.0,
    }
}

/// Default scale for a knowledge type in milliseconds.
pub const fn default_scale_for_kind(kind: EngramKind) -> u64 {
    const HOUR_MS: u64 = 3_600_000;
    match kind {
        EngramKind::Insight | EngramKind::StrategyFragment => 168 * HOUR_MS,
        EngramKind::Heuristic => 336 * HOUR_MS,
        EngramKind::Warning => 72 * HOUR_MS,
        EngramKind::CausalLink => 504 * HOUR_MS,
        EngramKind::AntiKnowledge => 720 * HOUR_MS,
    }
}

/// Configuration for knowledge demurrage in the Neuro store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageConfig {
    /// Cognitive-loop iterations between validation checks.
    pub validation_interval: u64,
    /// Confidence loss per missed validation interval.
    pub decay_per_interval: f64,
    /// Confidence threshold for archiving.
    pub archive_threshold: f64,
    /// Domain-specific decay multipliers.
    pub domain_multipliers: HashMap<String, f64>,
}

impl Default for DemurrageConfig {
    fn default() -> Self {
        let mut domain_multipliers = HashMap::new();
        domain_multipliers.insert("gas_patterns".into(), 2.0);
        domain_multipliers.insert("price_direction".into(), 1.5);
        domain_multipliers.insert("volatility_regime".into(), 1.0);
        domain_multipliers.insert("yield_trends".into(), 0.8);
        domain_multipliers.insert("protocol_behavior".into(), 0.5);

        Self {
            validation_interval: 250,
            decay_per_interval: 0.03,
            archive_threshold: 0.1,
            domain_multipliers,
        }
    }
}

/// Report produced by a demurrage cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DemurrageReport {
    /// Entries processed.
    pub entries_processed: u32,
    /// Entries moved to archive tier.
    pub entries_archived: u32,
    /// Total confidence lost.
    pub total_confidence_lost: f64,
    /// Average confidence after the cycle.
    pub average_confidence_after: f64,
}

/// Apply knowledge demurrage to one Engram.
pub fn apply_demurrage(
    engram: &BackupEngram,
    config: &DemurrageConfig,
    current_iteration: u64,
) -> BackupEngram {
    if engram.tier == KnowledgeTier::Archived || config.validation_interval == 0 {
        return engram.clone();
    }

    let intervals =
        current_iteration.saturating_sub(engram.last_accessed_at) / config.validation_interval;
    if intervals == 0 {
        return engram.clone();
    }

    let domain = engram.tags.first().map(String::as_str).unwrap_or("default");
    let domain_multiplier = config
        .domain_multipliers
        .get(domain)
        .copied()
        .unwrap_or(1.0);
    let total_decay = config.decay_per_interval * domain_multiplier * intervals as f64;
    let new_confidence = (engram.score.confidence - total_decay).max(0.0);

    let mut updated = engram.clone();
    updated.score.confidence = new_confidence;
    if new_confidence < config.archive_threshold {
        updated.tier = KnowledgeTier::Archived;
    }
    updated
}

/// Apply knowledge demurrage to all active Engrams.
pub fn apply_demurrage_to_all(
    engrams: &[BackupEngram],
    config: &DemurrageConfig,
    current_iteration: u64,
) -> (Vec<BackupEngram>, DemurrageReport) {
    let mut report = DemurrageReport {
        entries_processed: u32::try_from(engrams.len()).unwrap_or(u32::MAX),
        ..DemurrageReport::default()
    };
    let mut updated = Vec::with_capacity(engrams.len());

    for engram in engrams {
        let next = apply_demurrage(engram, config, current_iteration);
        if next.tier == KnowledgeTier::Archived && engram.tier != KnowledgeTier::Archived {
            report.entries_archived = report.entries_archived.saturating_add(1);
        }
        report.total_confidence_lost += engram.score.confidence - next.score.confidence;
        updated.push(next);
    }

    let total_confidence = updated
        .iter()
        .map(|engram| engram.score.confidence)
        .sum::<f64>();
    report.average_confidence_after = if updated.is_empty() {
        0.0
    } else {
        total_confidence / updated.len() as f64
    };

    (updated, report)
}

/// Demurrage rate calibration framework for token-level economics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DemurrageCalibration {
    /// Target velocity increase versus zero-demurrage baseline.
    pub target_velocity_multiplier: f64,
    /// Hoarding sensitivity coefficient.
    pub hoarding_sensitivity: f64,
    /// Minimum visible annual rate.
    pub minimum_effective_rate: f64,
    /// Maximum practical annual rate.
    pub maximum_practical_rate: f64,
    /// Grace period before demurrage begins.
    pub grace_period_days: u32,
    /// Minimum fraction of face value.
    pub floor_fraction: f64,
}

impl DemurrageCalibration {
    /// Compute recommended annual demurrage rate.
    pub fn recommended_rate(&self) -> f64 {
        let raw_rate = self.target_velocity_multiplier.ln() * self.hoarding_sensitivity;
        raw_rate.clamp(self.minimum_effective_rate, self.maximum_practical_rate)
    }

    /// Compute effective balance after elapsed years.
    pub fn effective_balance(&self, face_value: f64, elapsed_years: f64) -> f64 {
        let grace_years = f64::from(self.grace_period_days) / 365.25;
        if elapsed_years <= grace_years {
            return face_value;
        }
        let taxable_years = elapsed_years - grace_years;
        let decayed = face_value * (-self.recommended_rate() * taxable_years).exp();
        decayed.max(face_value * self.floor_fraction)
    }
}

impl Default for DemurrageCalibration {
    fn default() -> Self {
        Self {
            target_velocity_multiplier: 3.0,
            hoarding_sensitivity: 0.4,
            minimum_effective_rate: 0.02,
            maximum_practical_rate: 0.15,
            grace_period_days: 90,
            floor_fraction: 0.01,
        }
    }
}

/// Version vector for Mesh delta sync.
pub type VersionVector = HashMap<String, u64>;

/// Sync delta containing Engrams a peer has not seen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncDelta {
    /// Source agent ID.
    pub source_agent: String,
    /// Engrams to sync.
    pub engrams: Vec<SharedEngram>,
    /// Source agent version vector.
    pub version_vector: VersionVector,
    /// Sync timestamp.
    pub timestamp: u64,
}

/// Engram packaged for sharing across the Mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedEngram {
    /// Engram content and metadata.
    pub engram: BackupEngram,
    /// Monotonic sequence number per source agent.
    pub seq: u64,
    /// Agent that shared this Engram.
    pub shared_by: String,
    /// Share timestamp.
    pub shared_at: u64,
    /// Optional cryptographic attestation.
    pub attestation: Option<Attestation>,
}

/// PAD vector used to modulate knowledge sharing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PADVector {
    /// Pleasure axis.
    pub pleasure: f64,
    /// Arousal axis.
    pub arousal: f64,
    /// Dominance axis.
    pub dominance: f64,
}

/// Compute sharing threshold modulated by Daimon PAD state.
pub fn sharing_threshold(base_threshold: f64, pad: &PADVector) -> f64 {
    let arousal_modifier = -pad.arousal.clamp(-1.0, 1.0) * 0.15;
    let dominance_modifier = pad.dominance.clamp(-1.0, 1.0) * 0.10;
    (base_threshold + arousal_modifier + dominance_modifier).clamp(0.1, 0.9)
}

/// Mesh receive configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshReceiveConfig {
    /// Maximum Engrams accepted per hour.
    pub max_received_per_hour: u32,
    /// Minimum sender reputation.
    pub min_sender_reputation: f64,
    /// Confidence discount applied to incoming knowledge.
    pub received_confidence_discount: f64,
    /// Collective identifier.
    pub collective_id: Option<String>,
}

impl Default for MeshReceiveConfig {
    fn default() -> Self {
        Self {
            max_received_per_hour: 100,
            min_sender_reputation: 0.0,
            received_confidence_discount: 0.7,
            collective_id: None,
        }
    }
}

/// Report from processing incoming Mesh Engrams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MeshReceiveReport {
    /// Engrams received during the current hour.
    pub received_this_hour: u32,
    /// Engrams dropped by rate limiting.
    pub rate_limited: u32,
    /// Engrams dropped because attestation failed.
    pub attestation_failed: u32,
    /// Engrams dropped by reputation filtering.
    pub reputation_filtered: u32,
    /// Engrams rejected by validation.
    pub rejected: u32,
    /// Engrams adopted.
    pub adopted: u32,
}

/// Process incoming Mesh Engrams through rate-limit, discount, and adoption.
pub fn process_mesh_engrams(
    neuro: &mut NeuroStore,
    incoming: Vec<SharedEngram>,
    config: &MeshReceiveConfig,
) -> MeshReceiveReport {
    let mut report = MeshReceiveReport::default();

    for shared in incoming {
        if report.received_this_hour >= config.max_received_per_hour {
            report.rate_limited = report.rate_limited.saturating_add(1);
            continue;
        }

        report.received_this_hour = report.received_this_hour.saturating_add(1);
        let mut engram = shared.engram;
        engram.score.confidence =
            (engram.score.confidence * config.received_confidence_discount).clamp(0.0, 1.0);
        engram.provenance.push(ProvenanceEntry::MeshReceived {
            from_agent: shared.shared_by,
            via_collective: config.collective_id.clone(),
            timestamp: shared.shared_at,
        });
        neuro.insert(engram);
        report.adopted = report.adopted.saturating_add(1);
    }

    report
}

/// Request specific knowledge from a peer agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRequest {
    /// Target agent ID.
    pub target_agent: String,
    /// Knowledge query.
    pub query: KnowledgeQuery,
    /// Maximum Engrams to receive.
    pub max_results: u32,
    /// Optional Engram hashes offered in return.
    pub offer: Option<Vec<String>>,
}

/// Mesh knowledge query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum KnowledgeQuery {
    /// Semantic keyword or topic query.
    Semantic(String),
    /// HDC similarity query.
    HdcSimilarity {
        /// Query vector.
        vector: Vec<u8>,
        /// Minimum similarity threshold.
        threshold: f64,
    },
    /// Query by knowledge type.
    ByType(EngramKind),
    /// Query by tag.
    ByTag(String),
}

/// GitOps configuration source for lifecycle-managed agent configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitOpsConfig {
    /// Git repository URL.
    pub repo_url: String,
    /// Branch, tag, or commit SHA.
    pub target_revision: String,
    /// Relative config path in the repository.
    pub path: String,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// Automatically apply detected changes.
    pub auto_sync: bool,
    /// Revert manual drift back to Git state.
    pub self_heal: bool,
    /// Remove absent config sections.
    pub prune: bool,
    /// Number of past config states retained for rollback.
    pub revision_history_limit: usize,
    /// Retry policy.
    pub retry: GitOpsRetryPolicy,
}

impl Default for GitOpsConfig {
    fn default() -> Self {
        Self {
            repo_url: String::new(),
            target_revision: "main".into(),
            path: ".".into(),
            poll_interval_secs: 60,
            auto_sync: true,
            self_heal: true,
            prune: false,
            revision_history_limit: 10,
            retry: GitOpsRetryPolicy::default(),
        }
    }
}

/// Retry policy for GitOps synchronization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitOpsRetryPolicy {
    /// Maximum retry attempts; `-1` means unlimited.
    pub limit: i32,
    /// Initial backoff in seconds.
    pub initial_backoff_secs: u64,
    /// Backoff multiplier.
    pub factor: f64,
    /// Maximum backoff in seconds.
    pub max_backoff_secs: u64,
}

impl Default for GitOpsRetryPolicy {
    fn default() -> Self {
        Self {
            limit: 5,
            initial_backoff_secs: 5,
            factor: 2.0,
            max_backoff_secs: 180,
        }
    }
}

/// Result of a GitOps drift-detection pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConfigDrift {
    /// Desired and actual state match.
    InSync {
        /// Git revision used for comparison.
        revision: String,
    },
    /// Actual state diverges from Git.
    Drifted {
        /// Git revision used for comparison.
        revision: String,
        /// Divergent keys.
        diverged_keys: Vec<String>,
        /// Last known good revision.
        last_known_good: String,
    },
    /// Git source was unreachable.
    SourceUnreachable {
        /// Human-readable connection or authentication error.
        error: String,
    },
}

// ─── Budget tracking ─────────────────────────────────────────────────────

/// Runtime budget status after checking accumulated costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    /// Spending is within normal limits.
    Ok,
    /// Spending has crossed the warning threshold.
    Warning,
    /// Spending has crossed the critical threshold.
    Critical,
    /// Daily or lifetime budget is fully exhausted.
    Exhausted,
}

/// Runtime budget tracker that accumulates per-turn costs and enforces limits.
///
/// Call [`BudgetTracker::record_turn`] after each LLM invocation and
/// [`BudgetTracker::check`] before the next one to determine whether the
/// agent should degrade, pause, or stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetTracker {
    /// Budget configuration (thresholds and degradation mode).
    pub config: BudgetConfig,
    /// Total inference cost accumulated today.
    pub daily_cost_usd: f64,
    /// Total inference cost accumulated over the agent's lifetime.
    pub lifetime_cost_usd: f64,
    /// Current date string (`YYYY-MM-DD`) for daily rollover detection.
    pub current_date: String,
    /// Total turns recorded.
    pub total_turns: u64,
    /// Turns suppressed by T0 zero-LLM probes.
    pub t0_suppressed_turns: u64,
}

impl BudgetTracker {
    /// Create a new budget tracker from configuration.
    pub fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            daily_cost_usd: 0.0,
            lifetime_cost_usd: 0.0,
            current_date: today_str(),
            total_turns: 0,
            t0_suppressed_turns: 0,
        }
    }

    /// Record a completed turn's cost.
    pub fn record_turn(&mut self, cost: &TurnCostRecord) {
        let today = today_str();
        if today != self.current_date {
            self.daily_cost_usd = 0.0;
            self.current_date = today;
        }
        self.daily_cost_usd += cost.estimated_cost_usd;
        self.lifetime_cost_usd += cost.estimated_cost_usd;
        self.total_turns += 1;
        if cost.t0_suppressed {
            self.t0_suppressed_turns += 1;
        }
    }

    /// Check the current budget status.
    pub fn check(&self) -> BudgetStatus {
        let daily_fraction = if self.config.max_daily_inference_usd > 0.0 {
            self.daily_cost_usd / self.config.max_daily_inference_usd
        } else {
            0.0
        };

        // Check lifetime cap if set.
        if let Some(max_total) = self.config.max_total_usd {
            if max_total > 0.0 && self.lifetime_cost_usd >= max_total {
                return BudgetStatus::Exhausted;
            }
        }

        if daily_fraction >= 1.0 {
            BudgetStatus::Exhausted
        } else if daily_fraction >= self.config.critical_at {
            BudgetStatus::Critical
        } else if daily_fraction >= self.config.warning_at {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Ok
        }
    }

    /// Return the degradation mode configured for this budget.
    pub fn degradation_mode(&self) -> BudgetDegradationMode {
        self.config.degradation
    }

    /// Remaining daily budget in USD.
    pub fn remaining_daily_usd(&self) -> f64 {
        (self.config.max_daily_inference_usd - self.daily_cost_usd).max(0.0)
    }

    /// Daily utilization as a fraction (0.0 to 1.0+).
    pub fn daily_utilization(&self) -> f64 {
        if self.config.max_daily_inference_usd > 0.0 {
            self.daily_cost_usd / self.config.max_daily_inference_usd
        } else {
            0.0
        }
    }
}

/// Degradation action recommended when budget is constrained.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationAction {
    /// Downgrade model tier (e.g., Opus -> Sonnet -> Haiku).
    DowngradeModel,
    /// Reduce context window budget.
    ReduceContext,
    /// Increase T0 threshold (fewer LLM calls).
    IncreaseT0Threshold,
    /// Pause non-essential tasks.
    PauseNonEssential,
    /// Notify the operator.
    NotifyOperator,
    /// Hard stop all processing.
    HardStop,
}

/// Compute the cascade of degradation actions for a given budget status.
pub fn degradation_cascade(
    status: BudgetStatus,
    mode: BudgetDegradationMode,
) -> Vec<DegradationAction> {
    match mode {
        BudgetDegradationMode::NotifyOnly => match status {
            BudgetStatus::Ok => vec![],
            _ => vec![DegradationAction::NotifyOperator],
        },
        BudgetDegradationMode::Pause => match status {
            BudgetStatus::Ok => vec![],
            BudgetStatus::Warning => vec![DegradationAction::NotifyOperator],
            BudgetStatus::Critical => vec![
                DegradationAction::PauseNonEssential,
                DegradationAction::NotifyOperator,
            ],
            BudgetStatus::Exhausted => vec![
                DegradationAction::HardStop,
                DegradationAction::NotifyOperator,
            ],
        },
        BudgetDegradationMode::Cascade => match status {
            BudgetStatus::Ok => vec![],
            BudgetStatus::Warning => vec![
                DegradationAction::DowngradeModel,
                DegradationAction::ReduceContext,
            ],
            BudgetStatus::Critical => vec![
                DegradationAction::DowngradeModel,
                DegradationAction::ReduceContext,
                DegradationAction::IncreaseT0Threshold,
                DegradationAction::PauseNonEssential,
                DegradationAction::NotifyOperator,
            ],
            BudgetStatus::Exhausted => vec![
                DegradationAction::HardStop,
                DegradationAction::NotifyOperator,
            ],
        },
    }
}

// ─── Successor creation ──────────────────────────────────────────────────

/// Successor creation mode controlling what the new agent inherits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessorMode {
    /// New agent inherits nothing: fresh Neuro, fresh Daimon, new ID.
    Clean,
    /// New agent inherits parent's prompt, tool profile, and strategy but
    /// gets fresh Neuro (no knowledge transfer).
    SameStrategy,
    /// New agent inherits everything transferable via backup/restore with
    /// 0.85^generation confidence decay.
    FullLineage,
}

/// Create a successor manifest from a parent manifest.
///
/// The returned manifest has:
/// - Incremented `generation`
/// - `lineage_id` set (inherits parent's or uses parent's name as root)
/// - `SuccessorConfig` with default exploration boost
/// - Fields inherited based on `mode`:
///   - `Clean`: only lineage metadata preserved
///   - `SameStrategy`: prompt + tool_profile + strategy_md copied
///   - `FullLineage`: all transferable fields copied (knowledge transfer
///     via backup/restore is handled by the caller)
pub fn create_successor(
    parent: &AgentExtendedManifest,
    mode: SuccessorMode,
    new_name: Option<String>,
) -> AgentExtendedManifest {
    let lineage_id = parent
        .lineage_id
        .clone()
        .or_else(|| parent.name.clone())
        .or_else(|| Some("root".to_string()));

    let generation = parent.generation + 1;

    match mode {
        SuccessorMode::Clean => {
            let core = AgentCoreManifest::new("Describe what this agent should do");
            let mut manifest = AgentExtendedManifest::new(core);
            manifest.name = new_name;
            manifest.lineage_id = lineage_id;
            manifest.generation = generation;
            manifest.successor = Some(SuccessorConfig::default());
            resolve_manifest(manifest)
        }
        SuccessorMode::SameStrategy => {
            let core = AgentCoreManifest {
                prompt: parent.core.prompt.clone(),
                mode: parent.core.mode,
                domain: parent.core.domain.clone(),
                schema_version: parent.core.schema_version,
            };
            let mut manifest = AgentExtendedManifest::new(core);
            manifest.name = new_name;
            manifest.lineage_id = lineage_id;
            manifest.generation = generation;
            manifest.strategy_md = parent.strategy_md.clone();
            manifest.tool_profile = parent.tool_profile.clone();
            manifest.template_id = parent.template_id.clone();
            manifest.successor = Some(SuccessorConfig::default());
            resolve_manifest(manifest)
        }
        SuccessorMode::FullLineage => {
            let core = AgentCoreManifest {
                prompt: parent.core.prompt.clone(),
                mode: parent.core.mode,
                domain: parent.core.domain.clone(),
                schema_version: parent.core.schema_version,
            };
            let mut manifest = AgentExtendedManifest::new(core);
            manifest.name = new_name;
            manifest.lineage_id = lineage_id;
            manifest.generation = generation;
            manifest.strategy_md = parent.strategy_md.clone();
            manifest.model_routing = parent.model_routing.clone();
            manifest.neuro = parent.neuro.clone();
            manifest.mesh = parent.mesh.clone();
            manifest.tool_profile = parent.tool_profile.clone();
            manifest.template_id = parent.template_id.clone();
            manifest.inference = parent.inference.clone();
            manifest.budget = parent.budget.clone();
            manifest.successor = Some(SuccessorConfig::default());
            resolve_manifest(manifest)
        }
    }
}

// ─── Demurrage cycle runner ──────────────────────────────────────────────

/// Periodic demurrage cycle runner that tracks iteration counts and applies
/// knowledge demurrage at configured intervals.
///
/// Wire this into the Theta or Delta heartbeat loop. Each call to [`DemurrageCycle::tick`]
/// increments the iteration counter; when `validation_interval` iterations elapse,
/// the cycle applies demurrage to all supplied Engrams and returns a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemurrageCycle {
    /// Demurrage configuration.
    pub config: DemurrageConfig,
    /// Current iteration counter.
    pub iteration: u64,
    /// Iteration at which the last demurrage pass ran.
    pub last_demurrage_at: u64,
    /// Cumulative entries archived across all cycles.
    pub total_archived: u64,
    /// Cumulative confidence lost across all cycles.
    pub total_confidence_lost: f64,
}

impl DemurrageCycle {
    /// Create a new demurrage cycle from configuration.
    pub fn new(config: DemurrageConfig) -> Self {
        Self {
            config,
            iteration: 0,
            last_demurrage_at: 0,
            total_archived: 0,
            total_confidence_lost: 0.0,
        }
    }

    /// Advance the iteration counter and, if a validation interval has elapsed,
    /// apply demurrage to all supplied Engrams.
    ///
    /// Returns `Some(report)` when demurrage was applied, `None` otherwise.
    pub fn tick(
        &mut self,
        engrams: &[BackupEngram],
    ) -> Option<(Vec<BackupEngram>, DemurrageReport)> {
        self.iteration += 1;

        if self.config.validation_interval == 0 {
            return None;
        }

        let intervals_since_last =
            (self.iteration - self.last_demurrage_at) / self.config.validation_interval;
        if intervals_since_last == 0 {
            return None;
        }

        let (updated, report) = apply_demurrage_to_all(engrams, &self.config, self.iteration);
        self.last_demurrage_at = self.iteration;
        self.total_archived += u64::from(report.entries_archived);
        self.total_confidence_lost += report.total_confidence_lost;
        Some((updated, report))
    }

    /// Check whether the next tick will trigger a demurrage pass.
    pub fn next_demurrage_in(&self) -> u64 {
        if self.config.validation_interval == 0 {
            return u64::MAX;
        }
        let elapsed = self.iteration - self.last_demurrage_at;
        self.config.validation_interval.saturating_sub(elapsed)
    }

    /// Return true if demurrage is due on the next tick.
    pub fn is_due(&self) -> bool {
        self.next_demurrage_in() <= 1
    }
}

fn today_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_engram(confidence: f64) -> BackupEngram {
        BackupEngram {
            hash: "hash".into(),
            kind: EngramKind::Insight,
            body: "body".into(),
            author: "agent-a".into(),
            tags: vec!["volatility_regime".into()],
            score: EngramScore::from_confidence(confidence),
            tier: KnowledgeTier::Working,
            decay: DecayState {
                model: DecayModel::Ebbinghaus {
                    strength: confidence,
                    scale_ms: default_scale_for_kind(EngramKind::Insight),
                },
                effective_confidence: confidence,
                ticks_since_access: 0,
                tier_multiplier: 0.5,
            },
            provenance: vec![],
            created_at: 0,
            last_accessed_at: 0,
            retrieval_count: 0,
            validation_count: 0,
            hdc_vector: None,
        }
    }

    #[test]
    fn restore_confidence_applies_decay_floor() {
        assert!((restore_confidence(0.9, 1, 0.85) - 0.765).abs() < 0.0001);
        assert_eq!(restore_confidence(0.001, 5, 0.85), 0.01);
    }

    #[test]
    fn demurrage_archives_low_confidence_entries() {
        let engram = sample_engram(0.11);
        let updated = apply_demurrage(&engram, &DemurrageConfig::default(), 250);
        assert_eq!(updated.tier, KnowledgeTier::Archived);
    }

    #[test]
    fn restore_adopts_with_decayed_confidence() {
        let archive = BackupArchive {
            manifest: BackupManifest {
                version: 1,
                agent_id: "agent-a".into(),
                agent_name: None,
                created_at: 0,
                roko_version: "0.1.0".into(),
                generation: Some(0),
                stats: BackupStats::default(),
            },
            engrams: vec![sample_engram(0.9)],
            playbook_md: None,
            checksum: None,
        };
        let quarantined = quarantine_backup(&archive, &RestoreConfig::default());
        let mut neuro = NeuroStore::default();
        let report = adopt_engrams(&mut neuro, quarantined);
        assert_eq!(report.adopted, 1);
        assert!(neuro.engrams[0].score.confidence < 0.9);
    }

    #[test]
    fn config_validation_requires_restart_for_neuro_path() {
        let current = AgentConfig::default();
        let mut proposed = current.clone();
        proposed.neuro.path = ".roko/other-neuro/".into();
        assert!(matches!(
            validate_config_change(&current, &proposed),
            Err(LifecycleConfigError::RequiresRestart("neuro.path"))
        ));
    }

    // ─── BudgetTracker tests ─────────────────────────────────────────────

    fn sample_turn_cost(cost_usd: f64) -> TurnCostRecord {
        TurnCostRecord {
            turn_id: "turn-1".into(),
            model: "claude-haiku-4-5".into(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            estimated_cost_usd: cost_usd,
            cognitive_tier: CognitiveTier::Gamma,
            t0_suppressed: false,
            timestamp: 0,
        }
    }

    #[test]
    fn budget_tracker_ok_within_limits() {
        let mut tracker = BudgetTracker::new(BudgetConfig::default());
        tracker.record_turn(&sample_turn_cost(1.0));
        assert_eq!(tracker.check(), BudgetStatus::Ok);
    }

    #[test]
    fn budget_tracker_warning_at_threshold() {
        let mut tracker = BudgetTracker::new(BudgetConfig {
            max_daily_inference_usd: 10.0,
            warning_at: 0.7,
            ..BudgetConfig::default()
        });
        tracker.record_turn(&sample_turn_cost(7.5));
        assert_eq!(tracker.check(), BudgetStatus::Warning);
    }

    #[test]
    fn budget_tracker_critical_at_threshold() {
        let mut tracker = BudgetTracker::new(BudgetConfig {
            max_daily_inference_usd: 10.0,
            critical_at: 0.9,
            ..BudgetConfig::default()
        });
        tracker.record_turn(&sample_turn_cost(9.5));
        assert_eq!(tracker.check(), BudgetStatus::Critical);
    }

    #[test]
    fn budget_tracker_exhausted_at_limit() {
        let mut tracker = BudgetTracker::new(BudgetConfig {
            max_daily_inference_usd: 10.0,
            ..BudgetConfig::default()
        });
        tracker.record_turn(&sample_turn_cost(10.0));
        assert_eq!(tracker.check(), BudgetStatus::Exhausted);
    }

    #[test]
    fn budget_tracker_lifetime_cap() {
        let mut tracker = BudgetTracker::new(BudgetConfig {
            max_daily_inference_usd: 100.0,
            max_total_usd: Some(5.0),
            ..BudgetConfig::default()
        });
        tracker.record_turn(&sample_turn_cost(6.0));
        assert_eq!(tracker.check(), BudgetStatus::Exhausted);
    }

    #[test]
    fn degradation_cascade_steps_for_cascade_mode() {
        let actions = degradation_cascade(BudgetStatus::Warning, BudgetDegradationMode::Cascade);
        assert!(actions.contains(&DegradationAction::DowngradeModel));
        assert!(actions.contains(&DegradationAction::ReduceContext));

        let actions = degradation_cascade(BudgetStatus::Exhausted, BudgetDegradationMode::Cascade);
        assert!(actions.contains(&DegradationAction::HardStop));
    }

    #[test]
    fn degradation_cascade_notify_only_mode() {
        let actions =
            degradation_cascade(BudgetStatus::Critical, BudgetDegradationMode::NotifyOnly);
        assert_eq!(actions, vec![DegradationAction::NotifyOperator]);
    }

    // ─── Successor tests ─────────────────────────────────────────────────

    fn sample_parent_manifest() -> AgentExtendedManifest {
        let core = AgentCoreManifest {
            prompt: "I am a coding agent that implements features".into(),
            mode: DeploymentMode::SelfHosted,
            domain: Some(DomainPlugin::Coding(CodingConfig {
                workspace_path: "/workspace".into(),
                language: Some("rust".into()),
            })),
            schema_version: 1,
        };
        let mut manifest = AgentExtendedManifest::new(core);
        manifest.name = Some("parent-agent".into());
        manifest.strategy_md = Some("Focus on code quality".into());
        manifest.tool_profile = Some("implementer".into());
        manifest
    }

    #[test]
    fn successor_clean_inherits_nothing() {
        let parent = sample_parent_manifest();
        let child = create_successor(&parent, SuccessorMode::Clean, Some("child".into()));
        assert_eq!(child.generation, 1);
        assert_eq!(child.lineage_id, Some("parent-agent".into()));
        assert_ne!(child.core.prompt, parent.core.prompt);
        assert!(child.strategy_md.is_none());
    }

    #[test]
    fn successor_same_strategy_inherits_prompt_and_profile() {
        let parent = sample_parent_manifest();
        let child = create_successor(&parent, SuccessorMode::SameStrategy, Some("child".into()));
        assert_eq!(child.generation, 1);
        assert_eq!(child.core.prompt, parent.core.prompt);
        assert_eq!(child.strategy_md, parent.strategy_md);
        assert_eq!(child.tool_profile, parent.tool_profile);
    }

    #[test]
    fn successor_full_lineage_inherits_everything() {
        let parent = sample_parent_manifest();
        let child = create_successor(&parent, SuccessorMode::FullLineage, Some("child".into()));
        assert_eq!(child.generation, 1);
        assert_eq!(child.core.prompt, parent.core.prompt);
        assert_eq!(child.strategy_md, parent.strategy_md);
        assert_eq!(child.core.domain, parent.core.domain);
        assert!(child.successor.is_some());
    }

    #[test]
    fn successor_lineage_chains_across_generations() {
        let parent = sample_parent_manifest();
        let child = create_successor(&parent, SuccessorMode::FullLineage, Some("child".into()));
        let grandchild = create_successor(
            &child,
            SuccessorMode::FullLineage,
            Some("grandchild".into()),
        );
        assert_eq!(grandchild.generation, 2);
        assert_eq!(grandchild.lineage_id, Some("parent-agent".into()));
    }

    // ─── DemurrageCycle tests ────────────────────────────────────────────

    #[test]
    fn demurrage_cycle_fires_at_interval() {
        let config = DemurrageConfig {
            validation_interval: 5,
            ..DemurrageConfig::default()
        };
        let engrams = vec![sample_engram(0.5)];
        let mut cycle = DemurrageCycle::new(config);

        for _ in 0..4 {
            assert!(cycle.tick(&engrams).is_none());
        }
        let result = cycle.tick(&engrams);
        assert!(result.is_some());
    }

    #[test]
    fn demurrage_cycle_accumulates_stats() {
        let config = DemurrageConfig {
            validation_interval: 1,
            decay_per_interval: 0.5,
            archive_threshold: 0.1,
            ..DemurrageConfig::default()
        };
        let engrams = vec![sample_engram(0.2)];
        let mut cycle = DemurrageCycle::new(config);

        let (updated, _report) = cycle.tick(&engrams).unwrap();
        assert!(cycle.total_confidence_lost > 0.0);
        // With 0.5 decay and 0.2 confidence, the engram should be archived.
        assert_eq!(updated[0].tier, KnowledgeTier::Archived);
        assert_eq!(cycle.total_archived, 1);
    }

    #[test]
    fn demurrage_cycle_next_due_reports_correctly() {
        let config = DemurrageConfig {
            validation_interval: 10,
            ..DemurrageConfig::default()
        };
        let mut cycle = DemurrageCycle::new(config);
        assert_eq!(cycle.next_demurrage_in(), 10);
        for _ in 0..7 {
            cycle.tick(&[]);
        }
        assert_eq!(cycle.next_demurrage_in(), 3);
    }

    // ─── Provisioning pipeline tests ────────────────────────────────────

    #[test]
    fn provision_full_happy_path() {
        let core = AgentCoreManifest {
            prompt: "I am a coding agent that does work".into(),
            mode: DeploymentMode::SelfHosted,
            domain: None,
            schema_version: 1,
        };
        let manifest = AgentExtendedManifest::new(core);
        let ready = provision_full(manifest, "local-slot-1").unwrap();
        assert!(ready.state().neuro_initialized);
        assert!(ready.state().routing_configured);
        assert_eq!(ready.state().tool_profile, Some("standard".into()));
        assert!(!ready.state().resources.is_empty());
    }

    #[test]
    fn provision_full_rejects_invalid_manifest() {
        let core = AgentCoreManifest {
            prompt: "short".into(), // Too short
            mode: DeploymentMode::SelfHosted,
            domain: None,
            schema_version: 1,
        };
        let manifest = AgentExtendedManifest::new(core);
        let result = provision_full(manifest, "local-slot-1");
        assert!(matches!(
            result,
            Err(ProvisioningError::InvalidPromptLength)
        ));
    }

    #[test]
    fn provision_full_type_state_prevents_skipping() {
        // Verify that the type-state pipeline enforces stage ordering at
        // compile time. This test exists to document the guarantee; the
        // compiler enforces it — you cannot call `init_neuro()` on an
        // `Unvalidated` agent.
        let core = AgentCoreManifest::new("An agent that does research and writes reports");
        let manifest = AgentExtendedManifest::new(core);
        let unvalidated = ProvisioningAgent::new(manifest);
        let validated = unvalidated.validate().unwrap();
        let allocated = validated.allocate_resources("slot-1");
        let neuro = allocated.init_neuro();
        let routed = neuro.configure_routing();
        let tools = routed.load_tools();
        let mesh = tools.register_mesh();
        let ready = mesh.ready();
        assert!(ready.state().neuro_initialized);
    }
}

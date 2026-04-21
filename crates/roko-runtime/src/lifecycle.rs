//! Domain-neutral agent lifecycle records and provisioning primitives.
//!
//! The lifecycle docs model agent creation, provisioning, operation, graceful
//! shutdown, and restore as explicit operator-directed transitions. This module
//! provides the runtime-side data structures for those transitions without
//! depending on higher-level agent, Neuro, Mesh, or chain crates.

use std::{collections::HashMap, marker::PhantomData, time::Duration};

use serde::{Deserialize, Serialize};

/// Agent lifecycle states, informed by FIPA00023 with cloud-native extensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentLifecycleState {
    /// Manifest accepted, but the process is not yet running.
    Initiated,
    /// Infrastructure and runtime dependencies are being allocated.
    Provisioning,
    /// Agent registered, cognitive loop running, and accepting work.
    Active,
    /// Operator-initiated pause with state retained.
    Suspended,
    /// Agent is self-blocked on an external event.
    Waiting,
    /// Logical state preserved to cold storage while the process is stopped.
    Hibernated,
    /// Role, capability, or tool transition is in progress.
    Metamorphosing,
    /// Budget-constrained operation at reduced capability.
    Degraded {
        /// Current budget degradation stage.
        stage: DegradationStage,
    },
    /// Process terminated and runtime resources released.
    Deleted,
}

/// Degradation stage used when budget constraints reduce agent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationStage {
    /// Cheaper models are preferred.
    ModelDowngrade,
    /// Zero-LLM probes are emphasized.
    T0Emphasis,
    /// Runtime tick frequency is reduced.
    ReducedFrequency,
    /// Agent observes and reports but avoids taking actions.
    MonitoringOnly,
    /// Cognitive loop is paused until the budget window resets.
    BudgetPaused,
}

/// Hosted-machine lifecycle state for compute provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineLifecycleState {
    /// Manifest validated and resources requested.
    Provisioning,
    /// VM or local process has been spawned and is booting.
    Booting,
    /// Health checks pass and the agent can accept work.
    Ready,
    /// Deletion requested; work is draining before shutdown.
    Draining,
    /// Resources have been released.
    Destroyed,
    /// Supervisor restart budget was exceeded.
    Crashed,
}

/// Reason a lifecycle transition was requested or observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleTransitionReason {
    /// Operator created or initialized an agent.
    OperatorCreate,
    /// Manifest validation completed.
    ManifestValidated,
    /// Runtime dependencies completed startup.
    RuntimeReady,
    /// Operator paused processing.
    OperatorPause,
    /// Operator resumed processing.
    OperatorResume,
    /// Agent blocked on external work.
    ExternalWait,
    /// External work completed.
    ExternalReady,
    /// Operator requested deletion.
    OperatorDelete,
    /// Budget guardrail changed runtime capability.
    BudgetConstrained,
    /// Budget guardrail cleared and full capability can resume.
    BudgetRestored,
    /// Role or capability transition started.
    MetamorphosisStarted,
    /// Role or capability transition finished.
    MetamorphosisFinished,
    /// Runtime completed cleanup.
    CleanupComplete,
    /// A custom transition reason supplied by higher layers.
    Custom(String),
}

/// A serializable lifecycle transition record for event logs and replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleTransition {
    /// Stable agent identifier.
    pub agent_id: String,
    /// Previous lifecycle state.
    pub from: AgentLifecycleState,
    /// New lifecycle state.
    pub to: AgentLifecycleState,
    /// Why the transition occurred.
    pub reason: LifecycleTransitionReason,
    /// UTC timestamp for the transition.
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

impl LifecycleTransition {
    /// Create a new lifecycle transition stamped with the current UTC time.
    pub fn new(
        agent_id: impl Into<String>,
        from: AgentLifecycleState,
        to: AgentLifecycleState,
        reason: LifecycleTransitionReason,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            from,
            to,
            reason,
            occurred_at: chrono::Utc::now(),
        }
    }
}

/// OCI-inspired lifecycle hooks for agent creation and deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LifecycleHooks {
    /// Executes after manifest validation and before resource allocation.
    pub before_provision: Vec<HookSpec>,
    /// Executes after provisioning completes and before the cognitive loop starts.
    pub before_start: Vec<HookSpec>,
    /// Executes after the first cognitive loop iteration completes.
    pub after_start: Vec<HookSpec>,
    /// Executes after deletion is requested and before graceful shutdown begins.
    pub before_stop: Vec<HookSpec>,
    /// Executes after the agent is deleted and resources are released.
    pub after_stop: Vec<HookSpec>,
}

/// A lifecycle hook command specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    /// Program or executable to invoke.
    pub command: String,
    /// Arguments passed to the hook command.
    pub args: Vec<String>,
    /// Environment variables injected into the hook process.
    pub env: HashMap<String, String>,
    /// Maximum runtime for the hook in seconds.
    pub timeout_secs: u64,
}

impl HookSpec {
    /// Create a hook command with default timeout and no arguments.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            timeout_secs: 30,
        }
    }
}

/// Agent health probe configuration modeled after Kubernetes probes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthProbeConfig {
    /// Liveness probe: checks that the process is responsive.
    pub liveness: ProbeSpec,
    /// Readiness probe: checks whether the agent can accept new work.
    pub readiness: ProbeSpec,
    /// Startup probe: gates liveness and readiness during initial boot.
    pub startup: ProbeSpec,
}

impl Default for HealthProbeConfig {
    fn default() -> Self {
        Self {
            liveness: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay_secs: 15,
                period_secs: 20,
                timeout_secs: 1,
                success_threshold: 1,
                failure_threshold: 3,
            },
            readiness: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay_secs: 5,
                period_secs: 10,
                timeout_secs: 1,
                success_threshold: 1,
                failure_threshold: 3,
            },
            startup: ProbeSpec {
                handler: ProbeHandler::Internal,
                initial_delay_secs: 0,
                period_secs: 10,
                timeout_secs: 1,
                success_threshold: 1,
                failure_threshold: 30,
            },
        }
    }
}

/// A single health probe specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeSpec {
    /// Probe handler to evaluate.
    pub handler: ProbeHandler,
    /// Delay before first probe after a state transition.
    pub initial_delay_secs: u64,
    /// Time between probe attempts.
    pub period_secs: u64,
    /// Maximum time for one probe attempt.
    pub timeout_secs: u64,
    /// Consecutive successes required to pass.
    pub success_threshold: u32,
    /// Consecutive failures required to fail.
    pub failure_threshold: u32,
}

/// Health probe handler variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbeHandler {
    /// Internal runtime health check.
    Internal,
    /// HTTP GET health endpoint.
    Http {
        /// Request path.
        path: String,
        /// TCP port.
        port: u16,
    },
    /// TCP connection probe.
    Tcp {
        /// TCP port.
        port: u16,
    },
    /// Custom command probe where exit code zero means healthy.
    Exec {
        /// Command and arguments.
        command: Vec<String>,
    },
}

/// Restart backoff configuration and mutable failure count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartBackoff {
    /// Current failure count since the last successful run.
    pub failure_count: u32,
    /// Base delay in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds.
    pub max_delay_ms: u64,
    /// Successful runtime required before failure count resets.
    pub reset_after_ms: u64,
}

impl RestartBackoff {
    /// Compute delay before the next restart attempt.
    pub fn next_delay(&self) -> Duration {
        let multiplier = 10_u128.saturating_pow(self.failure_count);
        let delay = u128::from(self.base_delay_ms).saturating_mul(multiplier);
        let capped = delay.min(u128::from(self.max_delay_ms));
        Duration::from_millis(u64::try_from(capped).unwrap_or(u64::MAX))
    }

    /// Record a failed run.
    pub const fn record_failure(&mut self) {
        self.failure_count = self.failure_count.saturating_add(1);
    }

    /// Reset the failure count after a successful run.
    pub const fn reset(&mut self) {
        self.failure_count = 0;
    }
}

impl Default for RestartBackoff {
    fn default() -> Self {
        Self {
            failure_count: 0,
            base_delay_ms: 100,
            max_delay_ms: 300_000,
            reset_after_ms: 300_000,
        }
    }
}

/// `GitOps` configuration source for lifecycle-managed agent configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitOpsConfig {
    /// Git repository URL.
    pub repo_url: String,
    /// Branch, tag, or commit SHA.
    pub target_revision: String,
    /// Relative config path within the repository.
    pub path: String,
    /// Poll interval in seconds.
    pub poll_interval_secs: u64,
    /// Automatically apply detected changes.
    pub auto_sync: bool,
    /// Revert manual drift back to the Git state.
    pub self_heal: bool,
    /// Remove config keys absent from desired state.
    pub prune: bool,
    /// Number of historical revisions retained for rollback.
    pub revision_history_limit: usize,
    /// Retry policy for failed reconciliation.
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

/// Retry policy for `GitOps` synchronization failures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitOpsRetryPolicy {
    /// Maximum retry attempts. `-1` means unlimited.
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

/// Result of a `GitOps` drift-detection pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConfigDrift {
    /// Actual state matches the desired state.
    InSync {
        /// Git revision used for comparison.
        revision: String,
    },
    /// Actual state diverges from the desired state.
    Drifted {
        /// Git revision used for comparison.
        revision: String,
        /// Divergent configuration keys.
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

/// Type-state marker: manifest has not been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Unvalidated;

/// Type-state marker: manifest has passed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Validated;

/// Type-state marker: runtime resources have been allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ResourcesAllocated;

/// Type-state marker: the knowledge store has been initialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NeuroInitialized;

/// Type-state marker: model routing has been configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RoutingConfigured;

/// Type-state marker: tool profile has been loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ToolsLoaded;

/// Type-state marker: Mesh registration stage has completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MeshRegistered;

/// Type-state marker: agent is ready to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Ready;

/// Runtime state accumulated during provisioning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentState {
    /// Stable agent identifier.
    pub agent_id: Option<String>,
    /// Allocated resource labels.
    pub resources: Vec<String>,
    /// Whether the Neuro store is initialized.
    pub neuro_initialized: bool,
    /// Whether model routing is configured.
    pub routing_configured: bool,
    /// Loaded tool profile name.
    pub tool_profile: Option<String>,
    /// Whether Mesh registration completed.
    pub mesh_registered: bool,
}

impl AgentState {
    /// Attach an allocated resource label.
    #[must_use]
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resources.push(resource.into());
        self
    }

    /// Mark the Neuro store initialized.
    #[must_use]
    pub const fn with_neuro_initialized(mut self) -> Self {
        self.neuro_initialized = true;
        self
    }

    /// Mark model routing configured.
    #[must_use]
    pub const fn with_routing_configured(mut self) -> Self {
        self.routing_configured = true;
        self
    }

    /// Attach a loaded tool profile.
    #[must_use]
    pub fn with_tool_profile(mut self, profile: impl Into<String>) -> Self {
        self.tool_profile = Some(profile.into());
        self
    }

    /// Mark Mesh registration complete.
    #[must_use]
    pub const fn with_mesh_registered(mut self) -> Self {
        self.mesh_registered = true;
        self
    }
}

/// Agent in a specific provisioning stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent<S> {
    manifest_id: String,
    state: AgentState,
    stage: PhantomData<S>,
}

impl Agent<Unvalidated> {
    /// Create a new provisioning agent from a manifest identifier.
    pub fn new(manifest_id: impl Into<String>) -> Self {
        Self {
            manifest_id: manifest_id.into(),
            state: AgentState::default(),
            stage: PhantomData,
        }
    }

    /// Validate the manifest and advance the type state.
    pub fn validate(self) -> Agent<Validated> {
        self.transition()
    }
}

impl Agent<Validated> {
    /// Record resource allocation and advance the type state.
    pub fn allocate_resources(self, resource: impl Into<String>) -> Agent<ResourcesAllocated> {
        Agent {
            manifest_id: self.manifest_id,
            state: self.state.with_resource(resource),
            stage: PhantomData,
        }
    }
}

impl Agent<ResourcesAllocated> {
    /// Record Neuro initialization and advance the type state.
    pub fn init_neuro(self) -> Agent<NeuroInitialized> {
        Agent {
            manifest_id: self.manifest_id,
            state: self.state.with_neuro_initialized(),
            stage: PhantomData,
        }
    }
}

impl Agent<NeuroInitialized> {
    /// Record routing configuration and advance the type state.
    pub fn configure_routing(self) -> Agent<RoutingConfigured> {
        Agent {
            manifest_id: self.manifest_id,
            state: self.state.with_routing_configured(),
            stage: PhantomData,
        }
    }
}

impl Agent<RoutingConfigured> {
    /// Record tool loading and advance the type state.
    pub fn load_tools(self, profile: impl Into<String>) -> Agent<ToolsLoaded> {
        Agent {
            manifest_id: self.manifest_id,
            state: self.state.with_tool_profile(profile),
            stage: PhantomData,
        }
    }
}

impl Agent<ToolsLoaded> {
    /// Record Mesh registration completion and advance the type state.
    pub fn register_mesh(self, enabled: bool) -> Agent<MeshRegistered> {
        Agent {
            manifest_id: self.manifest_id,
            state: if enabled {
                self.state.with_mesh_registered()
            } else {
                self.state
            },
            stage: PhantomData,
        }
    }
}

impl Agent<MeshRegistered> {
    /// Final transition: agent is ready to run.
    pub fn ready(self) -> Agent<Ready> {
        self.transition()
    }
}

impl Agent<Ready> {
    /// Return the accumulated ready-state snapshot.
    pub const fn state(&self) -> &AgentState {
        &self.state
    }

    /// Return the manifest identifier used to start provisioning.
    pub fn manifest_id(&self) -> &str {
        &self.manifest_id
    }
}

impl<S> Agent<S> {
    fn transition<T>(self) -> Agent<T> {
        Agent {
            manifest_id: self.manifest_id,
            state: self.state,
            stage: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_backoff_caps_delay() {
        let mut backoff = RestartBackoff::default();
        backoff.failure_count = 10;
        assert_eq!(backoff.next_delay(), Duration::from_millis(300_000));
    }

    #[test]
    fn provisioning_type_state_accumulates_state() {
        let agent = Agent::new("manifest-1")
            .validate()
            .allocate_resources("small")
            .init_neuro()
            .configure_routing()
            .load_tools("standard")
            .register_mesh(true)
            .ready();

        assert_eq!(agent.manifest_id(), "manifest-1");
        assert!(agent.state().neuro_initialized);
        assert!(agent.state().routing_configured);
        assert!(agent.state().mesh_registered);
    }
}

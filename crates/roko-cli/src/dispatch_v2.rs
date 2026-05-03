//! Provider-neutral dispatch primitives for the plan runner.
//!
//! This module is intentionally small and side-effect free except for
//! `AgentDispatcherV2::create_agent`: callers can first resolve a model into a
//! concrete runtime, inspect whether that runtime is supported, then either
//! build a subprocess invocation for streaming CLI providers or construct a
//! provider-backed `Agent` through `roko-agent`.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result as AnyhowResult};
use roko_agent::AgentRuntimeEvent;
use roko_agent::StreamChunk;
use roko_agent::provider::{AgentOptions, ProviderSemaphores};
use roko_agent::{Agent, AgentResult, create_agent_for_model};
use roko_core::agent::{ProviderKind, resolve_model};
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::foundation::{FeedbackEvent, FeedbackSink};
use roko_core::{Body, Context, Engram, Kind};
use roko_learn::cascade_router::CascadeRouter;
use roko_learn::feedback_service::FeedbackService;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::learning_helpers::{capture_runtime_model_slugs, record_persisted_provider_health};

/// A single tool execution output captured from a dispatch response.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Tool name (e.g. "Read", "Bash", "Edit"), if available.
    pub tool_name: Option<String>,
    /// The tool's output content (file contents, bash stdout, etc.).
    pub content: String,
}

/// Result of dispatching a prompt to an LLM backend.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// The model's text response.
    pub text: String,
    /// Which model answered.
    pub model: String,
    /// Approximate input tokens.
    pub input_tokens: u64,
    /// Approximate output tokens.
    pub output_tokens: u64,
    /// Tool execution outputs captured from the agent's tool calls.
    pub tool_outputs: Vec<ToolOutput>,
    /// Session ID for conversation resume, when provided by the backend.
    pub session_id: Option<String>,
}

/// Dispatch a prompt through ModelCallService (v2 path).
///
/// Uses the ModelCaller trait that WorkflowEngine uses, preserving routing,
/// budget, cache, gateway event, and feedback behavior.
pub async fn dispatch_via_model_call_service(prompt: &str) -> AnyhowResult<DispatchResult> {
    use crate::learning_helpers::{
        capture_runtime_model_slugs, provider_id_for_model, record_persisted_provider_health,
    };
    use roko_agent::model_call_service::ModelCallService;
    use roko_core::agent::resolve_model;
    use roko_core::config::schema::RokoConfig;
    use roko_core::foundation::{
        ChatMessage, FeedbackSink, MessageRole, ModelCallRequest, ModelCaller, caller,
    };
    use roko_learn::cascade_router::CascadeRouter;
    use roko_learn::feedback_service::FeedbackService;

    let workdir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let config = crate::config::load_layered(&workdir)
        .map(|r| r.config)
        .unwrap_or_default();

    let mut model_config = RokoConfig::default();
    model_config.providers.extend(config.providers.clone());
    model_config.models.extend(config.models.clone());
    model_config.agent.command = Some(config.agent.command.clone());
    model_config.agent.args = Some(config.agent.args.clone());
    model_config.agent.timeout_ms = Some(config.agent.timeout_ms);
    model_config.agent.env = Some(config.agent.env.clone());
    model_config.agent.default_effort = config.agent.effort.clone();
    model_config.agent.bare_mode = config.agent.bare_mode;
    model_config.agent.fallback_model = config.agent.fallback_model.clone();
    model_config.agent.tier_models = config.agent.tier_models.clone();
    if let Some(model) = config.agent.model.clone() {
        model_config.agent.default_model = model;
    }
    let model_key = config
        .agent
        .model
        .clone()
        .unwrap_or_else(|| model_config.agent.default_model.clone());
    let model = resolve_model(&model_config, &model_key).slug;

    let cascade_path = workdir
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let cascade_model_slugs = capture_runtime_model_slugs(&model_config, &model);
    let cascade_router = (!cascade_model_slugs.is_empty()).then(|| {
        Arc::new(CascadeRouter::load_or_new(
            &cascade_path,
            cascade_model_slugs,
        ))
    });

    let feedback_service = FeedbackService::from_roko_dir(&workdir.join(".roko"));
    let feedback_sink: Arc<dyn FeedbackSink> = match &cascade_router {
        Some(router) => Arc::new(feedback_service.with_cascade_router(Arc::clone(router))),
        None => Arc::new(feedback_service),
    };
    let mut service = ModelCallService::new(model.clone())
        .with_config(model_config.clone())
        .with_feedback_sink(feedback_sink);
    if let Some(ref mcp_path) = config.agent.mcp_config {
        service = service.with_mcp_config(mcp_path.clone());
    }

    let request = ModelCallRequest {
        model: model.clone(),
        system: None,
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: prompt.to_string(),
        }],
        max_tokens: None,
        caller: Some(caller::CLI.to_string()),
        ..Default::default()
    };

    let call_result = service.call(request).await;
    if let Some(router) = &cascade_router
        && let Err(err) = router.save(&cascade_path)
    {
        tracing::warn!(
            path = %cascade_path.display(),
            error = %err,
            "failed to persist direct ModelCallService cascade observation"
        );
    }

    let response = match call_result {
        Ok(response) => {
            if let Some(provider) = provider_id_for_model(&model_config, &response.model) {
                record_persisted_provider_health(&workdir, &provider, true)
                    .context("record direct ModelCallService provider success")?;
            }
            response
        }
        Err(err) => {
            if let Some(provider) = provider_id_for_model(&model_config, &model)
                && let Err(health_err) =
                    record_persisted_provider_health(&workdir, &provider, false)
            {
                tracing::warn!(
                    provider = %provider,
                    error = %health_err,
                    "failed to persist direct ModelCallService provider failure"
                );
            }
            return Err(err).context("ModelCallService dispatch failed");
        }
    };

    Ok(DispatchResult {
        text: response.content,
        model: response.model,
        input_tokens: response.usage.input_tokens,
        output_tokens: response.usage.output_tokens,
        tool_outputs: Vec::new(),
        session_id: None,
    })
}

/// Wire protocol emitted by a supported CLI provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliProtocol {
    /// Anthropic Claude CLI `--output-format stream-json`.
    ClaudeStreamJson,
    /// OpenAI Codex CLI `codex exec --json`.
    CodexExecJson,
}

impl CliProtocol {
    /// Stable provider label used in runner events.
    pub const fn event_provider(self) -> &'static str {
        match self {
            Self::ClaudeStreamJson => "claude-cli",
            Self::CodexExecJson => "codex-cli",
        }
    }

    /// Provider kind used by config/model resolution.
    pub const fn provider_kind(self) -> ProviderKind {
        match self {
            Self::ClaudeStreamJson => ProviderKind::ClaudeCli,
            Self::CodexExecJson => ProviderKind::OpenAiCompat,
        }
    }

    /// Whether this CLI supports resuming an existing session through runner config.
    pub const fn supports_resume(self) -> bool {
        matches!(self, Self::ClaudeStreamJson)
    }

    /// Whether this CLI accepts an MCP config path directly.
    pub const fn supports_mcp_config(self) -> bool {
        matches!(self, Self::ClaudeStreamJson)
    }

    /// Whether this CLI has a native system-prompt flag.
    pub const fn supports_system_prompt_flag(self) -> bool {
        matches!(self, Self::ClaudeStreamJson)
    }
}

/// Human-readable CLI provider metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliProviderDescriptor {
    /// Provider registry id, for example `claude_cli`.
    pub provider_id: String,
    /// Config protocol family.
    pub provider_kind: ProviderKind,
    /// CLI wire protocol.
    pub protocol: CliProtocol,
    /// Label emitted in normalized runtime events.
    pub event_provider: String,
}

impl CliProviderDescriptor {
    fn new(provider_id: impl Into<String>, protocol: CliProtocol) -> Self {
        Self {
            provider_id: provider_id.into(),
            provider_kind: protocol.provider_kind(),
            protocol,
            event_provider: protocol.event_provider().to_string(),
        }
    }
}

/// Configured CLI provider plus its executable and static args.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliProviderConfig {
    /// Static provider metadata.
    pub descriptor: CliProviderDescriptor,
    /// Program to execute.
    pub command: PathBuf,
    /// Provider-level extra args from `roko.toml`.
    pub provider_args: Vec<String>,
}

impl CliProviderConfig {
    /// Build a Claude CLI provider.
    pub fn claude(provider_id: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        Self {
            descriptor: CliProviderDescriptor::new(provider_id, CliProtocol::ClaudeStreamJson),
            command: command.into(),
            provider_args: Vec::new(),
        }
    }

    /// Build a Codex CLI provider.
    pub fn codex(provider_id: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        Self {
            descriptor: CliProviderDescriptor::new(provider_id, CliProtocol::CodexExecJson),
            command: command.into(),
            provider_args: Vec::new(),
        }
    }

    /// Preserve runner-v2 compatibility while moving CLI detection out of
    /// `agent_stream`: a configured `codex` executable uses Codex protocol,
    /// everything else uses Claude's stream-json protocol.
    pub fn from_legacy_runner_program(program: impl Into<PathBuf>) -> Self {
        let program = program.into();
        if executable_name(&program).contains("codex") {
            Self::codex("codex_cli", program)
        } else {
            Self::claude("claude_cli", program)
        }
    }

    /// Resolve a CLI provider from an explicit provider registry entry.
    ///
    /// API-backed providers are not errors here because they are handled by the
    /// `AgentResultBridge` runtime, not a subprocess-json runtime.
    pub fn from_provider_config(
        provider_id: impl Into<String>,
        provider: &ProviderConfig,
    ) -> Result<Self, DispatchV2Error> {
        let provider_id = provider_id.into();
        match provider.kind {
            ProviderKind::ClaudeCli => {
                let command = required_command(&provider_id, provider)?;
                let mut config = if executable_name(&command).contains("codex") {
                    Self::codex(provider_id, command)
                } else {
                    Self::claude(provider_id, command)
                };
                config.provider_args = provider.args.clone().unwrap_or_default();
                Ok(config)
            }
            ProviderKind::OpenAiCompat => {
                let command = required_command(&provider_id, provider)?;
                if executable_name(&command).contains("codex") {
                    let mut config = Self::codex(provider_id, command);
                    config.provider_args = provider.args.clone().unwrap_or_default();
                    Ok(config)
                } else {
                    Err(DispatchV2Error::UnsupportedCommand {
                        provider_id,
                        command: command.display().to_string(),
                    })
                }
            }
            kind => Err(DispatchV2Error::UnsupportedCliProvider { provider_id, kind }),
        }
    }
}

/// Trait implemented by provider-specific CLI launchers.
pub trait CliDispatchProvider {
    /// Static description of this provider.
    fn descriptor(&self) -> &CliProviderDescriptor;

    /// Build the exact subprocess invocation for a runner turn.
    fn build_invocation(
        &self,
        request: &CliDispatchRequest,
    ) -> Result<CliInvocation, DispatchV2Error>;
}

impl CliDispatchProvider for CliProviderConfig {
    fn descriptor(&self) -> &CliProviderDescriptor {
        &self.descriptor
    }

    fn build_invocation(
        &self,
        request: &CliDispatchRequest,
    ) -> Result<CliInvocation, DispatchV2Error> {
        request.validate()?;
        match self.descriptor.protocol {
            CliProtocol::ClaudeStreamJson => self.build_claude_invocation(request),
            CliProtocol::CodexExecJson => self.build_codex_invocation(request),
        }
    }
}

impl CliProviderConfig {
    fn build_claude_invocation(
        &self,
        request: &CliDispatchRequest,
    ) -> Result<CliInvocation, DispatchV2Error> {
        let settings_json = roko_agent::claude_cli_agent::build_settings_json();
        let mut args = vec![
            "--print".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--model".to_string(),
            request.model.clone(),
            "--max-turns".to_string(),
            request.max_turns.to_string(),
            "--settings".to_string(),
            settings_json,
        ];
        args.extend(self.provider_args.clone());

        if request.dangerously_skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        if !request.system_prompt.trim().is_empty() {
            args.push("--append-system-prompt".to_string());
            args.push(request.system_prompt.clone());
        }
        if let Some(mcp_config) = &request.mcp_config {
            args.push("--mcp-config".to_string());
            args.push(mcp_config.to_string_lossy().to_string());
        }
        if let Some(session) = &request.resume_session {
            args.push("--resume".to_string());
            args.push(session.clone());
        }

        Ok(CliInvocation::new(
            self,
            request,
            args,
            request.prompt.clone(),
        ))
    }

    fn build_codex_invocation(
        &self,
        request: &CliDispatchRequest,
    ) -> Result<CliInvocation, DispatchV2Error> {
        let mut args = vec!["exec".to_string()];
        args.extend(self.provider_args.clone());
        args.push("--json".to_string());
        args.push("--cd".to_string());
        args.push(request.workdir.to_string_lossy().to_string());
        args.push("--skip-git-repo-check".to_string());
        args.push("--color".to_string());
        args.push("never".to_string());

        if request.dangerously_skip_permissions {
            args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
        } else {
            args.push("--sandbox".to_string());
            args.push("workspace-write".to_string());
        }

        if !request.model.trim().is_empty() && !request.model.starts_with("claude") {
            args.push("--model".to_string());
            args.push(request.model.clone());
        }
        args.push("-".to_string());

        let stdin = if request.system_prompt.trim().is_empty() {
            request.prompt.clone()
        } else {
            format!(
                "{}\n\n---\n\n{}",
                request.system_prompt.trim(),
                request.prompt
            )
        };

        Ok(CliInvocation::new(self, request, args, stdin))
    }
}

/// Provider-neutral request to launch a CLI-backed agent turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliDispatchRequest {
    /// Prompt sent to the provider on stdin.
    pub prompt: String,
    /// System prompt, either passed as a native flag or folded into stdin.
    pub system_prompt: String,
    /// Concrete model slug or model key selected for this turn.
    pub model: String,
    /// Working directory for the agent.
    pub workdir: PathBuf,
    /// Maximum agent turns when the provider supports it.
    pub max_turns: u32,
    /// Whether to bypass provider permission prompts/sandboxing.
    pub dangerously_skip_permissions: bool,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Optional session to resume when the provider supports it.
    pub resume_session: Option<String>,
    /// Extra subprocess environment entries.
    pub env: Vec<(String, String)>,
    /// Agent id used by observers.
    pub agent_id: String,
}

impl CliDispatchRequest {
    fn validate(&self) -> Result<(), DispatchV2Error> {
        if self.prompt.trim().is_empty() {
            return Err(DispatchV2Error::EmptyPrompt);
        }
        if self.model.trim().is_empty() {
            return Err(DispatchV2Error::EmptyModel);
        }
        if !self.workdir.exists() {
            return Err(DispatchV2Error::WorkdirMissing {
                path: self.workdir.clone(),
            });
        }
        Ok(())
    }
}

/// Fully materialized subprocess invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliInvocation {
    /// Program to execute.
    pub program: PathBuf,
    /// Program arguments.
    pub args: Vec<String>,
    /// Current directory for the subprocess.
    pub workdir: PathBuf,
    /// Stdin payload.
    pub stdin: String,
    /// Environment entries to set on the subprocess.
    pub env: Vec<(String, String)>,
    /// CLI wire protocol.
    pub protocol: CliProtocol,
    /// Provider label for normalized runner events.
    pub event_provider: String,
    /// Model selected for this invocation.
    pub model: String,
    /// Agent id associated with this invocation.
    pub agent_id: String,
}

impl CliInvocation {
    fn new(
        provider: &CliProviderConfig,
        request: &CliDispatchRequest,
        args: Vec<String>,
        stdin: String,
    ) -> Self {
        let mut env = request.env.clone();
        upsert_env(&mut env, "CARGO_INCREMENTAL", "0");
        upsert_env(&mut env, "CARGO_BUILD_JOBS", "2");

        Self {
            program: provider.command.clone(),
            args,
            workdir: request.workdir.clone(),
            stdin,
            env,
            protocol: provider.descriptor.protocol,
            event_provider: provider.descriptor.event_provider.clone(),
            model: request.model.clone(),
            agent_id: request.agent_id.clone(),
        }
    }
}

/// Runtime the runner should use for a resolved provider/model pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRuntime {
    /// Spawn a subprocess and decode provider JSON lines.
    Cli(CliProviderConfig),
    /// Construct a `roko-agent::Agent` and bridge its `AgentResult` into
    /// normalized events. This is the API-provider path.
    AgentResultBridge {
        /// Provider family bridged through `Agent::run`.
        provider_kind: ProviderKind,
    },
    /// The provider cannot currently be dispatched by the runner.
    Unsupported(UnsupportedProvider),
}

impl ProviderRuntime {
    /// Whether this resolved target can be dispatched by this layer.
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported(_))
    }

    /// Return the CLI runtime when this is a subprocess-json provider.
    pub fn as_cli(&self) -> Option<&CliProviderConfig> {
        match self {
            Self::Cli(provider) => Some(provider),
            Self::AgentResultBridge { .. } | Self::Unsupported(_) => None,
        }
    }
}

/// Unsupported provider metadata retained for diagnostics and fallback routing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedProvider {
    /// Machine-readable reason.
    pub reason: UnsupportedProviderReason,
    /// Human-readable detail.
    pub detail: String,
}

/// Why a provider/model cannot be dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedProviderReason {
    /// The model references a provider id absent from the effective config.
    MissingProvider,
    /// A CLI provider has no command.
    MissingCommand,
    /// The provider kind has no subprocess-json adapter.
    UnsupportedCliProvider,
    /// The command is not a known supported CLI protocol.
    UnsupportedCommand,
}

/// Fully resolved dispatch target for a model key.
#[derive(Debug, Clone)]
pub struct ProviderDispatchSpec {
    /// User-facing model key requested by the runner.
    pub model_key: String,
    /// Concrete model slug sent to the provider.
    pub model_slug: String,
    /// Provider registry id.
    pub provider_id: String,
    /// Protocol family.
    pub provider_kind: ProviderKind,
    /// Effective model profile, when present.
    pub model_profile: Option<ModelProfile>,
    /// Effective provider config, when present.
    pub provider_config: Option<ProviderConfig>,
    /// Runtime selected by this abstraction.
    pub runtime: ProviderRuntime,
}

impl ProviderDispatchSpec {
    /// Whether this spec can be dispatched.
    pub fn is_supported(&self) -> bool {
        self.runtime.is_supported()
    }
}

/// Provider/model resolver backed by `RokoConfig`.
#[derive(Debug, Clone)]
pub struct ProviderDispatchResolver {
    config: Arc<RokoConfig>,
}

impl ProviderDispatchResolver {
    /// Create a resolver from effective `roko.toml` config.
    pub fn new(config: Arc<RokoConfig>) -> Self {
        Self { config }
    }

    /// Resolve a model key into a dispatchable provider target.
    pub fn resolve(&self, model_key: &str) -> ProviderDispatchSpec {
        let resolved = resolve_model(&self.config, model_key);
        let models = self.config.effective_models();
        let providers = self.config.effective_providers();

        let model_profile = resolved
            .profile
            .clone()
            .or_else(|| models.get(model_key).cloned())
            .or_else(|| {
                models
                    .values()
                    .find(|profile| profile.slug == resolved.slug)
                    .cloned()
            });

        let model_slug = model_profile
            .as_ref()
            .map(|profile| profile.slug.clone())
            .unwrap_or_else(|| resolved.slug.clone());

        let requested_provider_id = model_profile
            .as_ref()
            .map(|profile| profile.provider.clone())
            .unwrap_or_else(|| resolved.provider_kind.label().to_string());

        let provider_match = if model_profile.is_some() {
            providers
                .get(&requested_provider_id)
                .cloned()
                .map(|provider| (requested_provider_id.clone(), provider))
        } else {
            providers
                .get(&requested_provider_id)
                .cloned()
                .map(|provider| (requested_provider_id.clone(), provider))
                .or_else(|| {
                    providers
                        .iter()
                        .find(|(_, provider)| provider.kind == resolved.provider_kind)
                        .map(|(id, provider)| (id.clone(), provider.clone()))
                })
        };

        let (provider_id, provider_config) = match provider_match {
            Some((provider_id, provider)) => (provider_id, Some(provider)),
            None => (requested_provider_id, None),
        };
        let provider_kind = provider_config
            .as_ref()
            .map(|provider| provider.kind)
            .unwrap_or(resolved.provider_kind);
        let runtime = classify_runtime(&provider_id, provider_kind, provider_config.as_ref());

        ProviderDispatchSpec {
            model_key: model_key.to_string(),
            model_slug,
            provider_id,
            provider_kind,
            model_profile,
            provider_config,
            runtime,
        }
    }
}

/// Provider-neutral agent construction facade.
#[derive(Debug, Clone)]
pub struct AgentDispatcherV2 {
    config: Arc<RokoConfig>,
    resolver: ProviderDispatchResolver,
    semaphores: Arc<ProviderSemaphores>,
}

impl AgentDispatcherV2 {
    /// Create a dispatcher from effective `roko.toml` config.
    pub fn new(config: Arc<RokoConfig>) -> Self {
        let providers = config.effective_providers();
        let semaphores = Arc::new(ProviderSemaphores::new(&providers));
        let resolver = ProviderDispatchResolver::new(Arc::clone(&config));
        Self {
            config,
            resolver,
            semaphores,
        }
    }

    /// Create a dispatcher that reuses pre-built semaphores.
    ///
    /// Used by `SharedAgentFactory` to avoid rebuilding the semaphore set
    /// for every task dispatch.
    pub fn with_shared(config: Arc<RokoConfig>, semaphores: Arc<ProviderSemaphores>) -> Self {
        let resolver = ProviderDispatchResolver::new(Arc::clone(&config));
        Self {
            config,
            resolver,
            semaphores,
        }
    }

    /// Resolve a model without launching anything.
    pub fn resolve(&self, model_key: &str) -> ProviderDispatchSpec {
        self.resolver.resolve(model_key)
    }

    /// Create the provider-backed agent for a request.
    ///
    /// This is the generalized path for API providers and provider adapters
    /// that return a single `AgentResult`. CLI subprocess streaming providers
    /// can still use `build_cli_invocation` when the runner needs PID-level
    /// lifecycle control.
    pub fn create_agent(
        &self,
        request: &AgentDispatchRequest,
    ) -> Result<CreatedAgent, DispatchV2Error> {
        request.validate()?;
        let target = self.resolve(&request.model_key);
        if let ProviderRuntime::Unsupported(unsupported) = &target.runtime {
            return Err(DispatchV2Error::UnsupportedResolvedProvider {
                provider_id: target.provider_id.clone(),
                detail: unsupported.detail.clone(),
            });
        }

        let options = self.agent_options(request);
        let agent =
            create_agent_for_model(&self.config, &request.model_key, options).map_err(|err| {
                DispatchV2Error::AgentCreation {
                    model_key: request.model_key.clone(),
                    message: err.to_string(),
                }
            })?;

        Ok(CreatedAgent { target, agent })
    }

    /// Run a provider-factory agent and return provider-neutral events.
    ///
    /// This is not wired into runner v2 yet because runner v2's `Started`
    /// event requires an OS pid. The returned event type carries `pid:
    /// Option<u32>` so the event protocol can evolve without lying about
    /// process ownership.
    pub async fn run_agent_result_bridge(
        &self,
        request: AgentDispatchRequest,
    ) -> Result<AgentResultDispatch, DispatchV2Error> {
        let created = self.create_agent(&request)?;
        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(request.prompt.clone()))
            .build();
        let started = Instant::now();
        let result = created.agent.run(&input, &Context::now()).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        record_agent_dispatch_feedback(
            &self.config,
            &request,
            &created.target,
            &result,
            latency_ms,
        )
        .await;
        let events = dispatch_events_from_result(&request, &created.target, &result);
        Ok(AgentResultDispatch {
            target: created.target,
            result,
            events,
        })
    }

    /// Run a provider-factory agent with streaming events forwarded in real time.
    ///
    /// Emits `Started` immediately, spawns an internal forwarder that converts
    /// [`StreamChunk`]s into [`AgentRuntimeEvent`]s as they arrive, then
    /// emits `TurnCompleted` + `Exited` after the agent finishes.
    pub async fn run_agent_streaming(
        &self,
        request: AgentDispatchRequest,
        event_tx: mpsc::Sender<AgentRuntimeEvent>,
    ) -> Result<AgentResult, DispatchV2Error> {
        let created = self.create_agent(&request)?;

        // Emit Started immediately so the TUI shows the agent is running.
        let _ = event_tx
            .send(AgentRuntimeEvent::Started {
                agent_id: request.agent_id.clone(),
                provider: created.target.provider_id.clone(),
                model: created.target.model_slug.clone(),
                pid: None,
            })
            .await;

        // Set up streaming channel: chunks flow from agent -> forwarder -> event_tx.
        let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel::<StreamChunk>();
        let forwarder_tx = event_tx.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(chunk) = chunk_rx.recv().await {
                let event = agent_event_from_chunk(chunk);
                if forwarder_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(request.prompt.clone()))
            .build();
        let started = Instant::now();
        let result = created
            .agent
            .run_streaming(&input, &Context::now(), chunk_tx)
            .await;
        let latency_ms = started.elapsed().as_millis() as u64;

        // Wait for forwarder to drain remaining chunks.
        let _ = forwarder.await;

        // Emit terminal events.
        if result.usage.total_tokens() > 0 || result.usage.cost_usd > 0.0 {
            let _ = event_tx
                .send(AgentRuntimeEvent::TokenUsage {
                    input_tokens: u64::from(result.usage.input_tokens),
                    output_tokens: u64::from(result.usage.output_tokens),
                    cache_read_tokens: u64::from(result.usage.cache_read_tokens),
                    cache_write_tokens: u64::from(result.usage.cache_create_tokens),
                })
                .await;
        }
        if !result.success {
            let message = result
                .output
                .body
                .as_text()
                .unwrap_or("agent failed without text output")
                .to_string();
            let _ = event_tx.send(AgentRuntimeEvent::Error { message }).await;
        }
        let _ = event_tx
            .send(AgentRuntimeEvent::TurnCompleted {
                session_id: None,
                total_cost_usd: (result.usage.cost_usd > 0.0)
                    .then_some(f64::from(result.usage.cost_usd)),
                num_turns: Some(1),
                is_error: !result.success,
            })
            .await;
        let _ = event_tx
            .send(AgentRuntimeEvent::Exited {
                exit_code: Some(if result.success { 0 } else { 1 }),
            })
            .await;

        record_agent_dispatch_feedback(
            &self.config,
            &request,
            &created.target,
            &result,
            latency_ms,
        )
        .await;

        Ok(result)
    }

    /// Run a provider-factory agent with pre-discovered MCP tools.
    ///
    /// When `mcp_tools` is `Some`, the tools are passed to the provider adapter
    /// so it skips MCP discovery entirely (no `block_on`, no OS thread).
    pub async fn run_agent_result_bridge_with_mcp(
        &self,
        request: AgentDispatchRequest,
        mcp_tools: Option<Arc<Vec<roko_core::tool::ToolDef>>>,
    ) -> Result<AgentResultDispatch, DispatchV2Error> {
        request.validate()?;
        let target = self.resolve(&request.model_key);
        if let ProviderRuntime::Unsupported(unsupported) = &target.runtime {
            return Err(DispatchV2Error::UnsupportedResolvedProvider {
                provider_id: target.provider_id.clone(),
                detail: unsupported.detail.clone(),
            });
        }

        let mut options = self.agent_options(&request);
        if let Some(tools) = mcp_tools {
            options.pre_discovered_mcp_tools = Some(tools);
        }
        let agent =
            create_agent_for_model(&self.config, &request.model_key, options).map_err(|err| {
                DispatchV2Error::AgentCreation {
                    model_key: request.model_key.clone(),
                    message: err.to_string(),
                }
            })?;

        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(request.prompt.clone()))
            .build();
        let started = Instant::now();
        let result = agent.run(&input, &Context::now()).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        record_agent_dispatch_feedback(&self.config, &request, &target, &result, latency_ms).await;
        let events = dispatch_events_from_result(&request, &target, &result);
        Ok(AgentResultDispatch {
            target,
            result,
            events,
        })
    }

    fn agent_options(&self, request: &AgentDispatchRequest) -> AgentOptions {
        AgentOptions {
            command: request.command.clone(),
            timeout_ms: request.timeout_ms,
            system_prompt: (!request.system_prompt.trim().is_empty())
                .then(|| request.system_prompt.clone()),
            cached_content: None,
            tools: request.tools.clone(),
            mcp_config: request.mcp_config.clone(),
            working_dir: Some(request.workdir.clone()),
            provider_semaphores: Some(Arc::clone(&self.semaphores)),
            env: request.env.clone(),
            extra_args: request.extra_args.clone(),
            effort: request.effort.clone(),
            bare_mode: request.bare_mode,
            dangerously_skip_permissions: request.dangerously_skip_permissions,
            name: request.agent_id.clone(),
            ..Default::default()
        }
    }
}

async fn record_agent_dispatch_feedback(
    config: &RokoConfig,
    request: &AgentDispatchRequest,
    target: &ProviderDispatchSpec,
    result: &AgentResult,
    latency_ms: u64,
) {
    let cascade_path = request
        .workdir
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let cascade_model_slugs = capture_runtime_model_slugs(config, &target.model_slug);
    let cascade_router = (!cascade_model_slugs.is_empty()).then(|| {
        Arc::new(CascadeRouter::load_or_new(
            &cascade_path,
            cascade_model_slugs,
        ))
    });
    let mut feedback_service = FeedbackService::from_roko_dir(&request.workdir.join(".roko"));
    if let Some(router) = &cascade_router {
        feedback_service = feedback_service.with_cascade_router(Arc::clone(router));
    }

    let turn_tokens = u64::from(result.usage.input_tokens) + u64::from(result.usage.output_tokens);
    if let Err(error) = feedback_service
        .record(FeedbackEvent::ModelCall {
            run_id: None,
            request_id: Some(format!("dispatch-v2-{}", request.agent_id)),
            prompt_section_ids: Vec::new(),
            knowledge_ids: Vec::new(),
            model: Some(target.model_slug.clone()),
            provider: Some(target.provider_id.clone()),
            token_usage: Some(turn_tokens),
            cost: Some(f64::from(result.usage.cost_usd)),
            role: "dispatch_v2".to_string(),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            cost_usd: f64::from(result.usage.cost_usd),
            latency_ms,
            success: result.success,
        })
        .await
    {
        tracing::warn!(
            provider = %target.provider_id,
            model = %target.model_slug,
            agent_id = %request.agent_id,
            error = %error,
            "failed to record dispatch-v2 feedback"
        );
    }

    if let Err(error) =
        record_persisted_provider_health(&request.workdir, &target.provider_id, result.success)
    {
        tracing::warn!(
            provider = %target.provider_id,
            model = %target.model_slug,
            agent_id = %request.agent_id,
            error = %error,
            "failed to persist dispatch-v2 provider health"
        );
    }

    if let Err(error) = feedback_service.flush_async().await {
        tracing::warn!(
            provider = %target.provider_id,
            model = %target.model_slug,
            agent_id = %request.agent_id,
            error = %error,
            "failed to flush dispatch-v2 feedback"
        );
    }
    if let Some(router) = &cascade_router
        && let Err(error) = router.save(&cascade_path)
    {
        tracing::warn!(
            path = %cascade_path.display(),
            error = %error,
            "failed to persist dispatch-v2 cascade observation"
        );
    }
}

/// Request for provider-factory dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDispatchRequest {
    /// Logical model key to resolve.
    pub model_key: String,
    /// User prompt.
    pub prompt: String,
    /// System prompt.
    pub system_prompt: String,
    /// Working directory.
    pub workdir: PathBuf,
    /// Agent id for diagnostics.
    pub agent_id: String,
    /// Optional command override for legacy providers.
    pub command: Option<String>,
    /// Optional timeout override.
    pub timeout_ms: Option<u64>,
    /// Optional MCP config path.
    pub mcp_config: Option<PathBuf>,
    /// Extra environment entries.
    pub env: Vec<(String, String)>,
    /// Extra provider args.
    pub extra_args: Vec<String>,
    /// Optional effort hint.
    pub effort: Option<String>,
    /// Optional tool allowlist/config payload.
    pub tools: Option<String>,
    /// Whether provider built-in prompts should be disabled.
    pub bare_mode: bool,
    /// Whether provider permission prompts/sandboxing should be bypassed.
    pub dangerously_skip_permissions: bool,
}

impl AgentDispatchRequest {
    fn validate(&self) -> Result<(), DispatchV2Error> {
        if self.prompt.trim().is_empty() {
            return Err(DispatchV2Error::EmptyPrompt);
        }
        if self.model_key.trim().is_empty() {
            return Err(DispatchV2Error::EmptyModel);
        }
        if !self.workdir.exists() {
            return Err(DispatchV2Error::WorkdirMissing {
                path: self.workdir.clone(),
            });
        }
        Ok(())
    }
}

/// Created provider-backed agent plus its resolved target metadata.
pub struct CreatedAgent {
    /// Resolved dispatch target.
    pub target: ProviderDispatchSpec,
    /// Real provider-backed agent.
    pub agent: Box<dyn Agent>,
}

/// Result from running an `AgentResultBridge` dispatch.
pub struct AgentResultDispatch {
    /// Resolved dispatch target.
    pub target: ProviderDispatchSpec,
    /// Raw provider result.
    pub result: AgentResult,
    /// Provider-neutral event projection.
    pub events: Vec<DispatchEvent>,
}

/// Provider-neutral events emitted by dispatch v2.
pub type DispatchEvent = AgentRuntimeEvent;

fn dispatch_events_from_result(
    request: &AgentDispatchRequest,
    target: &ProviderDispatchSpec,
    result: &AgentResult,
) -> Vec<DispatchEvent> {
    let mut events = vec![DispatchEvent::Started {
        agent_id: request.agent_id.clone(),
        provider: target.provider_id.clone(),
        model: target.model_slug.clone(),
        pid: None,
    }];

    for signal in &result.trace {
        if let Ok(text) = signal.body.as_text()
            && !text.trim().is_empty()
        {
            events.push(DispatchEvent::MessageDelta {
                text: text.to_string(),
            });
        }
    }
    if let Ok(text) = result.output.body.as_text()
        && !text.trim().is_empty()
    {
        events.push(DispatchEvent::MessageDelta {
            text: text.to_string(),
        });
    }

    if result.usage.total_tokens() > 0 || result.usage.cost_usd > 0.0 {
        events.push(DispatchEvent::TokenUsage {
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            cache_read_tokens: u64::from(result.usage.cache_read_tokens),
            cache_write_tokens: u64::from(result.usage.cache_create_tokens),
        });
    }

    if !result.success {
        let message = result
            .output
            .body
            .as_text()
            .unwrap_or("agent failed without text output")
            .to_string();
        events.push(DispatchEvent::Error { message });
    }

    events.push(DispatchEvent::TurnCompleted {
        session_id: None,
        total_cost_usd: (result.usage.cost_usd > 0.0).then_some(f64::from(result.usage.cost_usd)),
        num_turns: Some(1),
        is_error: !result.success,
    });
    events.push(DispatchEvent::Exited {
        exit_code: Some(if result.success { 0 } else { 1 }),
    });
    events
}

/// Convert a [`StreamChunk`] into the corresponding [`AgentRuntimeEvent`].
fn agent_event_from_chunk(chunk: StreamChunk) -> AgentRuntimeEvent {
    match chunk {
        StreamChunk::ContentDelta(text) => AgentRuntimeEvent::MessageDelta { text },
        StreamChunk::ReasoningDelta(text) => AgentRuntimeEvent::MessageDelta { text },
        StreamChunk::ToolCallDelta {
            id_delta,
            name_delta,
            ..
        } => AgentRuntimeEvent::ToolCall {
            id: id_delta.unwrap_or_default(),
            name: name_delta.unwrap_or_default(),
        },
        StreamChunk::Usage(usage) => AgentRuntimeEvent::TokenUsage {
            input_tokens: u64::from(usage.input_tokens),
            output_tokens: u64::from(usage.output_tokens),
            cache_read_tokens: u64::from(usage.cache_read_tokens),
            cache_write_tokens: u64::from(usage.cache_create_tokens),
        },
        StreamChunk::Done(_) => AgentRuntimeEvent::TurnCompleted {
            session_id: None,
            total_cost_usd: None,
            num_turns: None,
            is_error: false,
        },
        StreamChunk::Error(message) => AgentRuntimeEvent::Error { message },
    }
}

fn classify_runtime(
    provider_id: &str,
    provider_kind: ProviderKind,
    provider: Option<&ProviderConfig>,
) -> ProviderRuntime {
    let Some(provider) = provider else {
        return ProviderRuntime::Unsupported(UnsupportedProvider {
            reason: UnsupportedProviderReason::MissingProvider,
            detail: format!("model references missing provider `{provider_id}`"),
        });
    };

    match CliProviderConfig::from_provider_config(provider_id.to_string(), provider) {
        Ok(cli) => return ProviderRuntime::Cli(cli),
        Err(DispatchV2Error::MissingCommand { .. }) => {
            if matches!(
                provider_kind,
                ProviderKind::ClaudeCli | ProviderKind::CursorAcp
            ) {
                return ProviderRuntime::Unsupported(UnsupportedProvider {
                    reason: UnsupportedProviderReason::MissingCommand,
                    detail: format!("provider `{provider_id}` requires a command"),
                });
            }
        }
        Err(DispatchV2Error::UnsupportedCommand { command, .. }) => {
            if provider.base_url.is_none() && provider.api_key_env.is_none() {
                return ProviderRuntime::Unsupported(UnsupportedProvider {
                    reason: UnsupportedProviderReason::UnsupportedCommand,
                    detail: format!(
                        "provider `{provider_id}` command `{command}` is not a supported runner CLI"
                    ),
                });
            }
        }
        Err(DispatchV2Error::UnsupportedCliProvider { .. }) => {}
        Err(_) => {}
    }

    match provider_kind {
        ProviderKind::AnthropicApi
        | ProviderKind::OpenAiCompat
        | ProviderKind::PerplexityApi
        | ProviderKind::GeminiApi
        | ProviderKind::CursorAcp
        | ProviderKind::CerebrasApi => ProviderRuntime::AgentResultBridge { provider_kind },
        ProviderKind::ClaudeCli => ProviderRuntime::Unsupported(UnsupportedProvider {
            reason: UnsupportedProviderReason::UnsupportedCliProvider,
            detail: format!("provider `{provider_id}` is not dispatchable as configured"),
        }),
    }
}

fn required_command(
    provider_id: &str,
    provider: &ProviderConfig,
) -> Result<PathBuf, DispatchV2Error> {
    provider
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| DispatchV2Error::MissingCommand {
            provider_id: provider_id.to_string(),
            kind: provider.kind,
        })
}

fn executable_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn upsert_env(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = env.iter_mut().find(|(candidate, _)| candidate == key) {
        *existing = value.to_string();
    } else {
        env.push((key.to_string(), value.to_string()));
    }
}

/// Dispatch v2 error type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchV2Error {
    EmptyPrompt,
    EmptyModel,
    WorkdirMissing {
        path: PathBuf,
    },
    MissingCommand {
        provider_id: String,
        kind: ProviderKind,
    },
    UnsupportedCliProvider {
        provider_id: String,
        kind: ProviderKind,
    },
    UnsupportedCommand {
        provider_id: String,
        command: String,
    },
    UnsupportedResolvedProvider {
        provider_id: String,
        detail: String,
    },
    AgentCreation {
        model_key: String,
        message: String,
    },
}

impl fmt::Display for DispatchV2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPrompt => f.write_str("cannot dispatch an empty prompt"),
            Self::EmptyModel => f.write_str("cannot dispatch without a model"),
            Self::WorkdirMissing { path } => {
                write!(f, "dispatch workdir does not exist: {}", path.display())
            }
            Self::MissingCommand { provider_id, kind } => write!(
                f,
                "provider `{provider_id}` ({kind}) requires a non-empty command"
            ),
            Self::UnsupportedCliProvider { provider_id, kind } => write!(
                f,
                "provider `{provider_id}` ({kind}) has no supported CLI dispatch adapter"
            ),
            Self::UnsupportedCommand {
                provider_id,
                command,
            } => write!(
                f,
                "provider `{provider_id}` command `{command}` is not a supported CLI protocol"
            ),
            Self::UnsupportedResolvedProvider {
                provider_id,
                detail,
            } => write!(f, "provider `{provider_id}` is not dispatchable: {detail}"),
            Self::AgentCreation { model_key, message } => {
                write!(f, "failed to create agent for `{model_key}`: {message}")
            }
        }
    }
}

impl Error for DispatchV2Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::defaults::{
        DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_REQUEST_TIMEOUT_MS, DEFAULT_TTFT_TIMEOUT_MS,
    };
    use tempfile::tempdir;

    fn write_fake_claude_script(tmp: &tempfile::TempDir, body: &str) -> PathBuf {
        let script = tmp.path().join("claude-fake.sh");
        std::fs::write(&script, body).expect("write fake claude script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).expect("chmod");
        }
        script
    }

    #[test]
    fn legacy_runner_program_detects_codex_only_by_executable_name() {
        let codex = CliProviderConfig::from_legacy_runner_program("/opt/bin/codex");
        assert_eq!(codex.descriptor.protocol, CliProtocol::CodexExecJson);

        let claude = CliProviderConfig::from_legacy_runner_program("/tmp/custom-agent");
        assert_eq!(claude.descriptor.protocol, CliProtocol::ClaudeStreamJson);
    }

    #[test]
    fn codex_invocation_folds_system_prompt_into_stdin() {
        let provider = CliProviderConfig::codex("codex_cli", "codex");
        let request = CliDispatchRequest {
            prompt: "implement it".to_string(),
            system_prompt: "system".to_string(),
            model: "gpt-5".to_string(),
            workdir: std::env::current_dir().unwrap(),
            max_turns: 50,
            dangerously_skip_permissions: false,
            mcp_config: None,
            resume_session: None,
            env: Vec::new(),
            agent_id: "p/t".to_string(),
        };

        let invocation = provider.build_invocation(&request).unwrap();
        assert_eq!(invocation.protocol, CliProtocol::CodexExecJson);
        assert!(invocation.args.iter().any(|arg| arg == "--model"));
        assert_eq!(invocation.stdin, "system\n\n---\n\nimplement it");
    }

    #[tokio::test]
    async fn run_agent_result_bridge_records_feedback_and_provider_health() {
        let tmp = tempdir().expect("tempdir");
        let script = write_fake_claude_script(
            &tmp,
            r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"content_block_delta","delta":{"text":"dispatch-ok"}}'
"#,
        );

        let mut config = RokoConfig::default();
        config.providers.clear();
        config.models.clear();
        config.agent.default_model = "dispatch-model".to_string();
        config.providers.insert(
            "dispatch-cli".to_string(),
            ProviderConfig {
                kind: ProviderKind::ClaudeCli,
                base_url: None,
                api_key_env: None,
                command: Some(script.display().to_string()),
                args: None,
                timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
                ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
                connect_timeout_ms: Some(DEFAULT_CONNECT_TIMEOUT_MS),
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "dispatch-model".to_string(),
            ModelProfile {
                provider: "dispatch-cli".to_string(),
                slug: "claude-sonnet-4-6".to_string(),
                ..Default::default()
            },
        );

        let request = AgentDispatchRequest {
            model_key: "dispatch-model".to_string(),
            prompt: "do work".to_string(),
            system_prompt: "system".to_string(),
            workdir: tmp.path().to_path_buf(),
            agent_id: "dispatch-agent".to_string(),
            command: None,
            timeout_ms: Some(5_000),
            mcp_config: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            effort: None,
            tools: None,
            bare_mode: false,
            dangerously_skip_permissions: false,
        };
        let dispatcher = AgentDispatcherV2::new(Arc::new(config));

        let dispatch = dispatcher
            .run_agent_result_bridge(request)
            .await
            .expect("dispatch");

        assert!(dispatch.result.success);
        assert_eq!(
            dispatch.result.output.body.as_text().unwrap_or(""),
            "dispatch-ok"
        );

        let efficiency_path = tmp.path().join(".roko/learn/efficiency.jsonl");
        let efficiency = std::fs::read_to_string(&efficiency_path).expect("read efficiency");
        assert!(efficiency.contains(r#""kind":"model_call""#));
        assert!(efficiency.contains(r#""role":"dispatch_v2""#));
        assert!(efficiency.contains(r#""model":"claude-sonnet-4-6""#));
        assert!(efficiency.contains(r#""provider":"dispatch-cli""#));
        assert!(efficiency.contains(r#""success":true"#));

        let provider_health =
            std::fs::read_to_string(tmp.path().join(".roko/learn/provider-health.json"))
                .expect("read provider health");
        assert!(provider_health.contains("dispatch-cli"));

        let cascade_router =
            std::fs::read_to_string(tmp.path().join(".roko/learn/cascade-router.json"))
                .expect("read cascade router");
        assert!(cascade_router.contains("claude-sonnet-4-6"));
    }
}

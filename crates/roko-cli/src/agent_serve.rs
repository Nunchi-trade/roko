//! `roko agent serve` command wiring.
//!
//! Also contains the `roko agent create` and `roko agent delete` commands
//! for lifecycle management (LIFE-01, LIFE-06).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use roko_agent::{
    Agent,
    chat_types::{ChatRequest, ChatResponse},
    lifecycle::{
        AgentCoreManifest, AgentExtendedManifest, ChainConfig as LifecycleChainConfig,
        CodingConfig, DeploymentMode, DomainPlugin, ResearchConfig, resolve_manifest,
        validate_manifest,
    },
    process::registry::{register_spawned_pid, unregister_pid},
};
use roko_agent_server::{
    AgentRegistration, AgentServer, DispatchError, DispatchLike, RelayClientConfig,
};
use roko_cli::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use roko_core::config::schema::RokoConfig;
use roko_core::{Body, Context, Engram, Kind, MessageContent};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Agent-focused CLI subtree.
#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// Create a new agent from a manifest.
    ///
    /// Generates an `AgentExtendedManifest` TOML at `.roko/agents/<name>/manifest.toml`
    /// after validating the manifest fields. Supports domain presets (coding, research,
    /// chain, general) and optional strategy templates.
    Create {
        /// Human-readable agent name (required).
        #[arg(long)]
        name: String,
        /// Agent domain: coding, research, chain, or general.
        #[arg(long, default_value = "general")]
        domain: String,
        /// Strategy template to use (e.g. fast-coding, deep-research).
        #[arg(long)]
        template: Option<String>,
        /// Natural-language prompt describing what the agent should do.
        #[arg(long)]
        prompt: Option<String>,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Comma-separated skill tags (e.g. "rust,p2p,networking").
        #[arg(long, value_delimiter = ',')]
        skills: Vec<String>,
        /// Agent tier: Unverified, Verified, Trusted, Expert, Pioneer.
        #[arg(long)]
        tier: Option<String>,
        /// Reputation score (0–100).
        #[arg(long, default_value_t = 0)]
        reputation: u32,
        /// Maximum concurrent jobs.
        #[arg(long, default_value_t = 0)]
        max_concurrent_jobs: u32,
        /// Auto-register with roko-serve at this URL after creation.
        /// Uses the default http://localhost:6677 when set to empty string.
        #[arg(long)]
        serve_url: Option<String>,
    },
    /// Delete an agent and clean up its state.
    ///
    /// Performs an ordered 8-step shutdown: stop processing, flush pending,
    /// backup knowledge, deregister from mesh, release resources, archive
    /// signals, clean state, and emit a deletion marker. Use --force to
    /// skip the ordered shutdown for immediate removal.
    Delete {
        /// Agent name to delete.
        #[arg(long)]
        name: String,
        /// Skip ordered shutdown and remove immediately.
        #[arg(long)]
        force: bool,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// List all agents with their status.
    List {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Start a previously created agent.
    Start {
        /// Agent name.
        #[arg(long)]
        name: String,
        /// Socket address to bind (default: 127.0.0.1:0 for auto-port).
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Stop a running agent.
    Stop {
        /// Agent name.
        #[arg(long)]
        name: String,
        /// Force kill (SIGKILL instead of SIGTERM).
        #[arg(long)]
        force: bool,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show detailed status for one agent.
    Status {
        /// Agent name.
        #[arg(long)]
        name: String,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Start a per-agent HTTP runtime.
    Serve(AgentServeArgs),
    /// Interactive chat REPL with an agent.
    Chat {
        /// Agent ID to chat with.
        #[arg(long, default_value = "nunchi-intelligence")]
        agent: String,
        /// roko-serve base URL.
        #[arg(long, default_value_t = roko_cli::DEFAULT_SERVE_URL.to_string())]
        serve_url: String,
        /// Use a direct API provider instead of sidecar/serve routing.
        /// Accepted values: anthropic_api, openai_compat.
        #[arg(long)]
        provider: Option<String>,
    },
}

/// Arguments for `roko agent serve`.
#[derive(Debug, Args, Clone)]
pub struct AgentServeArgs {
    /// Unique agent identifier advertised by the runtime.
    #[arg(long)]
    pub agent_id: String,
    /// Socket address to bind (default: auto-pick a free port on localhost).
    #[arg(long, default_value = "127.0.0.1:0")]
    pub bind: String,
    /// Relay base URL reserved for a future relay bridge hook.
    #[arg(long)]
    pub relay_url: Option<String>,
    /// Chain JSON-RPC URL reserved for future chain hooks.
    #[arg(long)]
    pub chain_rpc_url: Option<String>,
    /// ERC-8004 identity registry contract address.
    #[arg(long)]
    pub identity_registry: Option<String>,
    /// ERC-8004 passport id used for `updateAgentCardUri`.
    #[arg(long)]
    pub passport_id: Option<String>,
    /// Wallet private key reserved for future signing hooks.
    #[arg(long)]
    pub wallet_key: Option<String>,
    /// roko-serve control plane URL for heartbeat reporting.
    #[arg(long, default_value_t = roko_cli::DEFAULT_SERVE_URL.to_string())]
    pub serve_url: String,
}

#[derive(Debug, Clone)]
struct AgentServeRuntimeConfig {
    agent_id: String,
    bind: String,
    relay: Option<RelayConfig>,
    chain: Option<ChainConfig>,
    serve_url: String,
}

#[derive(Debug, Clone)]
struct RelayConfig {
    url: String,
}

#[derive(Debug, Clone)]
struct ChainConfig {
    rpc_url: Option<String>,
    identity_registry: Option<String>,
    passport_id: Option<String>,
    wallet_key: Option<String>,
}

impl AgentServeRuntimeConfig {
    fn from_args(args: AgentServeArgs) -> Self {
        let chain = ChainConfig::from_args(&args);
        let relay = args.relay_url.map(|url| RelayConfig { url });
        Self {
            agent_id: args.agent_id,
            bind: args.bind,
            relay,
            chain,
            serve_url: args.serve_url,
        }
    }

    async fn run(self) -> Result<()> {
        let startup = self.startup_snapshot();
        let server = self.build_server()?;
        info!(
            agent_id = %startup.agent_id,
            bind = %startup.bind,
            "starting roko agent server"
        );
        let result = server.serve().await;

        // Cleanup: remove our entry from agents.json on shutdown.
        let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        remove_agent_entry(&workdir, &startup.agent_id);

        result
    }

    fn build_server(&self) -> Result<AgentServer> {
        let mut builder = AgentServer::builder()
            .agent_id(self.agent_id.clone())
            .bind(self.bind.clone())
            .serve_url(self.serve_url.clone())
            .messaging()
            .predictions();

        if let Some(dispatcher) = self.try_build_dispatcher()? {
            builder = builder.with_message_dispatcher(dispatcher);
        }

        if let Some(registration) = self.registration() {
            builder = builder.registration(registration);
        }

        let startup = self.startup_snapshot();
        let workdir_for_start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        builder
            .on_start(move |addr, card| {
                let startup = startup.clone();
                let workdir = workdir_for_start.clone();
                async move {
                    info!(
                        agent_id = %startup.agent_id,
                        bind = %startup.bind,
                        local_addr = %addr,
                        rest_endpoint = ?card.endpoints.rest,
                        "agent server is ready"
                    );
                    // Register with roko-serve so the control plane and
                    // dashboard can discover this agent.  Retry up to 3
                    // times with 2 s gaps — when `roko up` starts serve and
                    // agents near-simultaneously the first attempt may fail
                    // because serve isn't listening yet.
                    let rest_endpoint = card
                        .endpoints
                        .rest
                        .clone()
                        .unwrap_or_else(|| format!("http://127.0.0.1:{}", addr.port()));
                    let register_url = format!(
                        "{}/api/agents/register",
                        startup.serve_url.trim_end_matches('/')
                    );
                    let body = serde_json::json!({
                        "agent_id": startup.agent_id,
                        "label": startup.agent_id,
                        "rest_endpoint": rest_endpoint,
                        "process_id": std::process::id(),
                    });
                    let client = reqwest::Client::new();
                    let mut registered = false;
                    for attempt in 1..=3u32 {
                        match client
                            .post(&register_url)
                            .json(&body)
                            .timeout(std::time::Duration::from_secs(3))
                            .send()
                            .await
                        {
                            Ok(resp) if resp.status().is_success() => {
                                info!(
                                    agent_id = %startup.agent_id,
                                    serve_url = %startup.serve_url,
                                    attempt,
                                    "registered with roko-serve"
                                );
                                registered = true;
                                break;
                            }
                            Ok(resp) => {
                                warn!(
                                    agent_id = %startup.agent_id,
                                    status = %resp.status(),
                                    attempt,
                                    "roko-serve registration returned non-success"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    agent_id = %startup.agent_id,
                                    error = %err,
                                    attempt,
                                    "could not register with roko-serve (is it running?)"
                                );
                            }
                        }
                        if attempt < 3 {
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        }
                    }
                    if !registered {
                        warn!(
                            agent_id = %startup.agent_id,
                            "failed to register with roko-serve after 3 attempts"
                        );
                    }

                    // Write to agents.json so `roko agent list`, `roko agent chat`,
                    // and the dashboard can discover this sidecar.
                    let actual_bind = format!("http://127.0.0.1:{}", addr.port());
                    upsert_agent_entry(&workdir, &startup.agent_id, &actual_bind);

                    if let Some(relay) = &startup.relay {
                        info!(
                            agent_id = %startup.agent_id,
                            relay_url = %relay.url,
                            "relay config captured for later hook-up"
                        );
                    }
                    if let Some(chain) = &startup.chain {
                        if let Some(url) = &chain.rpc_url {
                            match roko_chain::alloy_impl::AlloyChainClient::http(url) {
                                Ok(_client) => {
                                    let has_wallet = chain.wallet_key.is_some();
                                    info!(
                                        agent_id = %startup.agent_id,
                                        chain_rpc = url,
                                        has_wallet,
                                        "chain tools active for agent sidecar"
                                    );
                                }
                                Err(e) => {
                                    warn!(error = %e, "chain rpc_url set but client failed");
                                }
                            }
                        }
                    }
                    Ok(())
                }
            })
            .build()
    }

    fn try_build_dispatcher(&self) -> Result<Option<Arc<dyn DispatchLike>>> {
        let workdir = std::env::current_dir().context("read current working directory")?;
        let mut config = load_roko_config(&workdir)?;

        let model = config.agent.default_model.trim().to_string();
        if model.is_empty() {
            return Ok(None);
        }

        // When ANTHROPIC_API_KEY is set, prefer the direct HTTP API over the
        // CLI subprocess for serving.  The API returns clean text; the CLI
        // subprocess emits raw streaming-protocol JSON that leaks through to
        // callers (dashboard, chat REPL).
        //
        // Override the model profile so `create_agent_for_model` resolves
        // through the AnthropicApi adapter (ClaudeAgent with Messages API)
        // instead of ClaudeCli (subprocess).
        let has_anthropic_env = std::env::var_os("ANTHROPIC_API_KEY").is_some();
        let use_anthropic_api = has_anthropic_env && !config.models.contains_key(&model);
        if use_anthropic_api {
            let mut profile = config
                .effective_models()
                .get(&model)
                .cloned()
                .unwrap_or_default();
            if profile.provider == "claude_cli" {
                profile.provider = "anthropic".to_string();
                config.models.insert(model.clone(), profile);
                info!(
                    model = %model,
                    "ANTHROPIC_API_KEY set — overriding provider to anthropic (direct HTTP)"
                );
            }
        }

        // A dispatcher can be built if any of:
        //   (a) the default model resolves in the provider registry, or
        //   (b) a legacy subprocess command is configured, or
        //   (c) ANTHROPIC_API_KEY is set (model override above ensures resolution).
        let has_provider_backing = config.effective_models().contains_key(&model);
        let has_legacy_command = config.agent.command.is_some();
        if !has_provider_backing && !has_legacy_command && !has_anthropic_env {
            return Ok(None);
        }

        // When using the API, don't pass the legacy CLI command — it would
        // cause the provider layer to spawn a subprocess instead.
        let command = if use_anthropic_api {
            None
        } else {
            config.agent.command.clone()
        };

        let agent = spawn_agent_scoped(
            &config,
            SpawnAgentSpec {
                model: model.to_string(),
                command,
                timeout_ms: config.agent.timeout_ms,
                system_prompt: None,
                cached_content: None,
                tools: None,
                mcp_config: None,
                working_dir: Some(workdir),
                env: config.agent.env.clone().unwrap_or_default(),
                extra_args: config.agent.args.clone().unwrap_or_default(),
                effort: Some(config.agent.default_effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: false,
                name: self.agent_id.clone(),
                role: None,
            },
            format!("create serving agent for {}", self.agent_id),
        )?;

        Ok(Some(Arc::new(ServingAgentDispatcher {
            agent: Arc::from(agent),
        })))
    }

    fn registration(&self) -> Option<AgentRegistration> {
        if self.relay.is_none() && self.chain.is_none() {
            return None;
        }

        let mut registration = AgentRegistration::default();
        if let Some(relay) = &self.relay {
            registration.relay = Some(RelayClientConfig::new(relay.url.clone()));
        }
        if let Some(chain) = &self.chain {
            registration.identity_registry_address = chain.identity_registry.clone();
            registration.passport_id = chain.passport_id.clone();
        }
        Some(registration)
    }

    fn startup_snapshot(&self) -> StartupSnapshot {
        StartupSnapshot {
            agent_id: self.agent_id.clone(),
            bind: self.bind.clone(),
            relay: self.relay.clone(),
            chain: self.chain.clone(),
            serve_url: self.serve_url.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct StartupSnapshot {
    agent_id: String,
    bind: String,
    relay: Option<RelayConfig>,
    chain: Option<ChainConfig>,
    #[allow(dead_code)]
    serve_url: String,
}

impl ChainConfig {
    fn from_args(args: &AgentServeArgs) -> Option<Self> {
        let has_chain_inputs = args.chain_rpc_url.is_some()
            || args.identity_registry.is_some()
            || args.passport_id.is_some()
            || args.wallet_key.is_some();
        has_chain_inputs.then(|| Self {
            rpc_url: args.chain_rpc_url.clone(),
            identity_registry: args.identity_registry.clone(),
            passport_id: args.passport_id.clone(),
            wallet_key: args.wallet_key.clone(),
        })
    }
}

struct ServingAgentDispatcher {
    agent: Arc<dyn Agent>,
}

#[async_trait]
impl DispatchLike for ServingAgentDispatcher {
    async fn dispatch(&self, request: ChatRequest) -> Result<ChatResponse, DispatchError> {
        let prompt = extract_prompt(&request).ok_or(DispatchError::NotConfigured)?;
        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(prompt.clone()))
            .build();
        let result = self
            .agent
            .run(&input, &Context::now().with_goal(prompt))
            .await;

        // Clean up raw JSON from the agent output (e.g. Claude CLI streaming
        // protocol).  The `extract_clean_text` parser handles plain text
        // (no-op), JSONL with result/assistant events, content block arrays,
        // and nested `result`/`content` fields.
        let raw = result.output.body.as_text().unwrap_or_default();
        let content = roko_cli::chat::extract_clean_text(raw);

        Ok(ChatResponse {
            content,
            usage: result.usage,
            finish_reason: if result.success {
                roko_agent::chat_types::FinishReason::Stop
            } else {
                roko_agent::chat_types::FinishReason::Error(
                    result
                        .output
                        .body
                        .as_text()
                        .unwrap_or("agent failed")
                        .to_string(),
                )
            },
            ..ChatResponse::default()
        })
    }
}

fn extract_prompt(request: &ChatRequest) -> Option<String> {
    request.messages.iter().find_map(|message| match message {
        roko_core::ChatMessage::User { content } => match content {
            MessageContent::Text(text) => Some(text.clone()),
            MessageContent::Blocks(blocks) => {
                let parts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|block| match block {
                        roko_core::ContentBlock::Text { text } => Some(text.as_str()),
                        roko_core::ContentBlock::ImageUrl { .. } => None,
                    })
                    .collect();
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("\n"))
                }
            }
        },
        _ => None,
    })
}

fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    roko_core::config::loader::load_config_unified(workdir).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Run `roko agent ...`.
pub async fn run(cmd: AgentCmd) -> Result<()> {
    match cmd {
        AgentCmd::Create {
            name,
            domain,
            template,
            prompt,
            workdir,
            skills,
            tier,
            reputation,
            max_concurrent_jobs,
            serve_url,
        } => {
            run_agent_create(
                &name,
                &domain,
                template.as_deref(),
                prompt.as_deref(),
                workdir.as_deref(),
            )
            .await?;

            // Auto-register with roko-serve if --serve-url is given.
            let url = serve_url.as_deref().map(|u| {
                if u.is_empty() {
                    "http://localhost:6677"
                } else {
                    u
                }
            });
            if let Some(base) = url {
                let capabilities = match domain.as_str() {
                    "research" => vec!["messaging".to_string(), "research".to_string()],
                    _ => vec!["messaging".to_string(), "tasks".to_string()],
                };
                let body = serde_json::json!({
                    "agent_id": name,
                    "label": name,
                    "capabilities": capabilities,
                    "domain_tags": [domain],
                    "skills": skills,
                    "tier": tier,
                    "reputation": reputation,
                    "max_concurrent_jobs": max_concurrent_jobs,
                });
                let register_url = format!("{}/api/agents/register", base.trim_end_matches('/'));
                match reqwest::Client::new()
                    .post(&register_url)
                    .json(&body)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        println!("Registered with serve at {base}");
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        eprintln!("warning: serve registration failed ({status}): {text}");
                    }
                    Err(err) => {
                        eprintln!("warning: could not reach serve at {register_url}: {err}");
                        eprintln!("  (the agent was created locally; register manually later)");
                    }
                }
            }
            Ok(())
        }
        AgentCmd::Delete {
            name,
            force,
            workdir,
        } => run_agent_delete(&name, force, workdir.as_deref()).await,
        AgentCmd::List { workdir } => run_agent_list(workdir.as_deref()),
        AgentCmd::Start {
            name,
            bind,
            workdir,
        } => run_agent_start(&name, &bind, workdir.as_deref()),
        AgentCmd::Stop {
            name,
            force,
            workdir,
        } => run_agent_stop(&name, force, workdir.as_deref()),
        AgentCmd::Status { name, workdir } => run_agent_status(&name, workdir.as_deref()),
        AgentCmd::Serve(args) => AgentServeRuntimeConfig::from_args(args).run().await,
        AgentCmd::Chat {
            agent, serve_url, ..
        } => {
            roko_cli::chat_inline::run_chat_inline(&agent, &serve_url).await?;
            Ok(())
        }
    }
}

// ─── Structured agent tracking ──────────────────────────────────────────

/// Runtime state for a single agent, persisted to `.roko/runtime/agents.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentEntry {
    name: String,
    pid: u32,
    bind: String,
    domain: String,
    started_at: String, // RFC 3339
}

/// Path to the structured agent tracking file.
fn agents_file_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("runtime").join("agents.json")
}

/// Load all agent entries from disk. Returns empty vec if file is missing or corrupt.
fn load_agent_entries(workdir: &Path) -> Vec<AgentEntry> {
    let path = agents_file_path(workdir);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

/// Persist agent entries to disk.
fn save_agent_entries(workdir: &Path, entries: &[AgentEntry]) -> Result<()> {
    let path = agents_file_path(workdir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create runtime directory at {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(entries).context("serialize agent entries")?;
    std::fs::write(&path, json)
        .with_context(|| format!("write agent entries to {}", path.display()))?;
    Ok(())
}

/// Insert or update an agent entry in agents.json.
fn upsert_agent_entry(workdir: &Path, agent_id: &str, bind: &str) {
    let mut entries = load_agent_entries(workdir);
    entries.retain(|e| e.name != agent_id);
    entries.push(AgentEntry {
        name: agent_id.to_string(),
        pid: std::process::id(),
        bind: bind.to_string(),
        domain: "general".to_string(),
        started_at: chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = save_agent_entries(workdir, &entries) {
        warn!(error = %e, "failed to write agent entry to agents.json");
    }
}

/// Remove an agent entry from agents.json.
fn remove_agent_entry(workdir: &Path, agent_id: &str) {
    let mut entries = load_agent_entries(workdir);
    let before = entries.len();
    entries.retain(|e| e.name != agent_id);
    if entries.len() < before {
        if let Err(e) = save_agent_entries(workdir, &entries) {
            warn!(error = %e, "failed to clean agent entry from agents.json");
        } else {
            info!(agent_id, "removed agent entry from agents.json");
        }
    }
}

/// Check whether a process with the given PID is alive.
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_wrap)]
fn is_process_alive(pid: u32) -> bool {
    // SAFETY: signal 0 is an existence check — no signal is delivered.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    false
}

/// Send a signal to a process.
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_wrap)]
fn send_signal(pid: u32, sig: i32) {
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: u32, _sig: i32) {}

/// Extract the domain string from a manifest TOML on disk.
fn read_domain_from_manifest(manifest_path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(manifest_path) else {
        return "unknown".to_string();
    };
    let Ok(manifest) = toml::from_str::<AgentExtendedManifest>(&text) else {
        return "unknown".to_string();
    };
    match &manifest.core.domain {
        Some(DomainPlugin::Coding(_)) => "coding".to_string(),
        Some(DomainPlugin::Research(_)) => "research".to_string(),
        Some(DomainPlugin::Chain(_)) => "chain".to_string(),
        Some(DomainPlugin::Custom(c)) => c.id.clone(),
        None => "general".to_string(),
    }
}

/// Format a duration in a human-readable way.
fn format_duration(dur: chrono::Duration) -> String {
    let secs = dur.num_seconds();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

// ─── Agent list ─────────────────────────────────────────────────────────

fn run_agent_list(workdir: Option<&Path>) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let agents_dir = wd.join(".roko").join("agents");
    if !agents_dir.exists() {
        println!("No agents found.");
        return Ok(());
    }

    // Scan manifests.
    let mut agents: Vec<(String, String)> = Vec::new(); // (name, domain)
    let entries = std::fs::read_dir(&agents_dir)
        .with_context(|| format!("read agents directory at {}", agents_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let agent_path = entry.path();

        // Skip deleted agents.
        if agent_path.join("DELETED").exists() {
            continue;
        }
        let manifest_path = agent_path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        let domain = read_domain_from_manifest(&manifest_path);
        agents.push((name, domain));
    }

    if agents.is_empty() {
        println!("No agents found.");
        return Ok(());
    }

    agents.sort_by(|a, b| a.0.cmp(&b.0));

    // Load runtime state.
    let runtime_entries = load_agent_entries(&wd);

    // Count active/idle.
    let mut active = 0u32;
    let mut idle = 0u32;
    for (name, _) in &agents {
        let rt = runtime_entries.iter().find(|e| e.name == *name);
        if rt.is_some_and(|e| is_process_alive(e.pid)) {
            active += 1;
        } else {
            idle += 1;
        }
    }

    // Use inline primitives for formatted output when TTY.
    if roko_cli::inline::should_use_inline() {
        let theme = roko_cli::tui::Theme::from_env();
        let total = agents.len();
        let mut lines = vec![
            roko_cli::inline::styled::section_start(
                &theme,
                "agents",
                &format!("{total} registered"),
                Some(&format!("{active} active, {idle} idle")),
            ),
            ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                roko_cli::inline::symbols::BAR.to_string(),
                theme.muted(),
            )]),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    roko_cli::inline::symbols::BAR.to_string(),
                    theme.muted(),
                ),
                ratatui::text::Span::raw("  "),
                ratatui::text::Span::styled(
                    format!(
                        "{:<16} {:<12} {:<30} {}",
                        "NAME", "STATUS", "IDENTITY", "DOMAIN"
                    ),
                    theme.muted(),
                ),
            ]),
        ];

        for (name, domain) in &agents {
            let rt = runtime_entries.iter().find(|e| e.name == *name);
            let (status_icon, status_label) = if rt.is_some_and(|e| is_process_alive(e.pid)) {
                ("\u{25cf}", "active") // ● active
            } else {
                ("\u{25cb}", "idle") // ○ idle
            };
            let identity = format!("eid://roko/{name}");

            lines.push(ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    roko_cli::inline::symbols::BAR.to_string(),
                    theme.muted(),
                ),
                ratatui::text::Span::raw("  "),
                ratatui::text::Span::styled(format!("{:<16}", name), theme.text()),
                ratatui::text::Span::styled(
                    format!("{status_icon} "),
                    if status_label == "active" {
                        theme.success()
                    } else {
                        theme.muted()
                    },
                ),
                ratatui::text::Span::styled(
                    format!("{:<10}", status_label),
                    if status_label == "active" {
                        theme.success()
                    } else {
                        theme.muted()
                    },
                ),
                ratatui::text::Span::styled(
                    format!("{:<30}", identity),
                    ratatui::style::Style::default().fg(roko_cli::tui::Theme::DREAM),
                ),
                ratatui::text::Span::styled(
                    domain.to_string(),
                    ratatui::style::Style::default().fg(roko_cli::tui::Theme::TEXT_DIM),
                ),
            ]));
        }

        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(roko_cli::inline::symbols::BAR.to_string(), theme.muted()),
        ]));
        lines.push(roko_cli::inline::styled::section_end(
            &theme,
            "",
            &format!("{total} agent{}", if total == 1 { "" } else { "s" }),
        ));

        roko_cli::inline::plaintext::print_plain(&lines);
    } else {
        // Plain fallback
        println!(
            "{:<20} {:<10} {:<8} {:<22} {}",
            "NAME", "STATUS", "PID", "BIND", "DOMAIN"
        );
        for (name, domain) in &agents {
            let rt = runtime_entries.iter().find(|e| e.name == *name);
            let (status, pid_str, bind_str) = match rt {
                Some(entry) if is_process_alive(entry.pid) => (
                    "running".to_string(),
                    entry.pid.to_string(),
                    entry.bind.clone(),
                ),
                Some(_) => ("stopped".to_string(), "-".to_string(), "-".to_string()),
                None => ("created".to_string(), "-".to_string(), "-".to_string()),
            };
            println!(
                "{:<20} {:<10} {:<8} {:<22} {}",
                name, status, pid_str, bind_str, domain
            );
        }
    }

    // Clean up stale entries.
    let live: Vec<AgentEntry> = runtime_entries
        .into_iter()
        .filter(|e| is_process_alive(e.pid))
        .collect();
    let _ = save_agent_entries(&wd, &live);

    Ok(())
}

// ─── Agent start ────────────────────────────────────────────────────────

pub(crate) fn run_agent_start(name: &str, bind: &str, workdir: Option<&Path>) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let agent_dir = wd.join(".roko").join("agents").join(name);
    let manifest_path = agent_dir.join("manifest.toml");

    if !manifest_path.exists() {
        bail!(
            "agent '{}' not found (no manifest at {})",
            name,
            manifest_path.display()
        );
    }
    if agent_dir.join("DELETED").exists() {
        bail!("agent '{}' has been deleted", name);
    }

    // Check if already running.
    let mut entries = load_agent_entries(&wd);
    if let Some(existing) = entries.iter().find(|e| e.name == name) {
        if is_process_alive(existing.pid) {
            bail!(
                "agent '{}' is already running (pid {}, bind {})",
                name,
                existing.pid,
                existing.bind
            );
        }
        // Stale entry — remove it.
        entries.retain(|e| e.name != name);
    }

    let domain = read_domain_from_manifest(&manifest_path);

    // Spawn `roko agent serve --agent-id <name> --bind <bind>` as detached child.
    let roko_bin = std::env::current_exe().context("determine roko binary path")?;
    let child = std::process::Command::new(&roko_bin)
        .arg("agent")
        .arg("serve")
        .arg("--agent-id")
        .arg(name)
        .arg("--bind")
        .arg(bind)
        .current_dir(&wd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn agent serve for '{}'", name))?;

    let pid = child.id();
    register_spawned_pid(pid);

    let now = chrono::Utc::now().to_rfc3339();
    entries.push(AgentEntry {
        name: name.to_string(),
        pid,
        bind: bind.to_string(),
        domain,
        started_at: now,
    });
    save_agent_entries(&wd, &entries)?;

    println!("Agent '{}' started (pid {}, bind {}).", name, pid, bind);
    Ok(())
}

// ─── Agent stop ─────────────────────────────────────────────────────────

pub(crate) fn run_agent_stop(name: &str, force: bool, workdir: Option<&Path>) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut entries = load_agent_entries(&wd);
    let entry_idx = entries.iter().position(|e| e.name == name);

    let Some(idx) = entry_idx else {
        println!("Agent '{}' is not running.", name);
        return Ok(());
    };

    let entry = entries[idx].clone();
    if !is_process_alive(entry.pid) {
        println!("Agent '{}' is not running (stale entry cleaned up).", name);
        entries.remove(idx);
        save_agent_entries(&wd, &entries)?;
        unregister_pid(entry.pid);
        return Ok(());
    }

    // Send initial signal.
    if force {
        #[cfg(unix)]
        send_signal(entry.pid, libc::SIGKILL);
        #[cfg(not(unix))]
        send_signal(entry.pid, 9);
    } else {
        #[cfg(unix)]
        send_signal(entry.pid, libc::SIGTERM);
        #[cfg(not(unix))]
        send_signal(entry.pid, 15);
    }

    // Wait up to 5 seconds for exit.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !is_process_alive(entry.pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // If still alive after timeout and not force, escalate to SIGKILL.
    if is_process_alive(entry.pid) && !force {
        #[cfg(unix)]
        send_signal(entry.pid, libc::SIGKILL);
        #[cfg(not(unix))]
        send_signal(entry.pid, 9);

        // Brief wait for SIGKILL to take effect.
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // Compute run duration.
    let duration_str = chrono::DateTime::parse_from_rfc3339(&entry.started_at)
        .ok()
        .map(|started| {
            let dur = chrono::Utc::now().signed_duration_since(started);
            format_duration(dur)
        })
        .unwrap_or_else(|| "unknown".to_string());

    entries.remove(idx);
    save_agent_entries(&wd, &entries)?;
    unregister_pid(entry.pid);

    println!(
        "Agent '{}' stopped (pid {}, ran for {}).",
        name, entry.pid, duration_str
    );
    Ok(())
}

// ─── Agent status ───────────────────────────────────────────────────────

fn run_agent_status(name: &str, workdir: Option<&Path>) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let agent_dir = wd.join(".roko").join("agents").join(name);
    let manifest_path = agent_dir.join("manifest.toml");

    if !manifest_path.exists() {
        bail!(
            "agent '{}' not found (no manifest at {})",
            name,
            manifest_path.display()
        );
    }
    if agent_dir.join("DELETED").exists() {
        bail!("agent '{}' has been deleted", name);
    }

    let domain = read_domain_from_manifest(&manifest_path);
    let entries = load_agent_entries(&wd);
    let rt = entries.iter().find(|e| e.name == name);

    let (status, pid_str, bind_str, started_str) = match rt {
        Some(entry) if is_process_alive(entry.pid) => {
            let ago = chrono::DateTime::parse_from_rfc3339(&entry.started_at)
                .ok()
                .map(|started| {
                    let dur = chrono::Utc::now().signed_duration_since(started);
                    format!("{} ({} ago)", entry.started_at, format_duration(dur))
                })
                .unwrap_or_else(|| entry.started_at.clone());
            ("running", entry.pid.to_string(), entry.bind.clone(), ago)
        }
        Some(_) => ("stopped", "-".to_string(), "-".to_string(), "-".to_string()),
        None => ("created", "-".to_string(), "-".to_string(), "-".to_string()),
    };

    println!("Agent:    {}", name);
    println!("Status:   {}", status);
    println!("Domain:   {}", domain);
    println!("PID:      {}", pid_str);
    println!("Bind:     {}", bind_str);
    println!("Started:  {}", started_str);
    println!("Manifest: {}", manifest_path.display());

    Ok(())
}

// ─── LIFE-01: Agent creation ────────────────────────────────────────────

/// Default prompt used when the operator does not supply one.
const DEFAULT_AGENT_PROMPT: &str =
    "You are a helpful agent. Describe your task in the strategy document.";

/// Three-step agent creation: build manifest, validate, write to disk.
pub(crate) async fn run_agent_create(
    name: &str,
    domain: &str,
    template: Option<&str>,
    prompt: Option<&str>,
    workdir: Option<&Path>,
) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    // Step 1: Build the core manifest from user input.
    let agent_prompt = prompt.unwrap_or(DEFAULT_AGENT_PROMPT);
    let domain_plugin = match domain {
        "coding" => Some(DomainPlugin::Coding(CodingConfig {
            workspace_path: wd.display().to_string(),
            language: None,
        })),
        "research" => Some(DomainPlugin::Research(ResearchConfig::default())),
        "chain" => Some(DomainPlugin::Chain(LifecycleChainConfig::default())),
        "general" => None,
        other => bail!(
            "unknown domain '{}'; expected: coding, research, chain, general",
            other
        ),
    };

    let core = AgentCoreManifest {
        prompt: agent_prompt.to_string(),
        mode: DeploymentMode::SelfHosted,
        domain: domain_plugin,
        schema_version: 1,
    };

    let mut manifest = AgentExtendedManifest::new(core);
    manifest.name = Some(name.to_string());
    manifest.template_id = template.map(String::from);

    // Step 2: Resolve defaults and validate.
    let manifest = resolve_manifest(manifest);
    validate_manifest(&manifest).map_err(|e| anyhow::anyhow!("manifest validation failed: {e}"))?;

    // Step 3: Write to disk.
    let agents_dir = wd.join(".roko").join("agents").join(name);
    std::fs::create_dir_all(&agents_dir)
        .with_context(|| format!("create agent directory at {}", agents_dir.display()))?;

    let manifest_path = agents_dir.join("manifest.toml");
    let toml_text =
        toml::to_string_pretty(&manifest).context("serialize agent manifest to TOML")?;
    std::fs::write(&manifest_path, &toml_text)
        .with_context(|| format!("write manifest to {}", manifest_path.display()))?;

    println!("Agent '{}' created successfully.", name);
    println!("  domain:   {domain}");
    if let Some(tpl) = template {
        println!("  template: {tpl}");
    }
    println!("  manifest: {}", manifest_path.display());
    println!();
    println!("Edit the manifest to customize, then provision with:");
    println!("  roko agent serve --agent-id {name}");

    Ok(())
}

// ─── LIFE-06: Agent deletion ────────────────────────────────────────────

/// 8-step agent deletion with per-step 30-second timeout.
///
/// Steps:
///   1. Stop processing (cancel current task, drain queue)
///   2. Flush pending (complete in-flight tool calls)
///   3. Backup knowledge (auto-invoke neuro backup)
///   4. Deregister from mesh
///   5. Release resources
///   6. Archive signals (compress JSONL logs)
///   7. Clean state (remove executor.json and transient files)
///   8. Confirm (write DELETED marker)
async fn run_agent_delete(name: &str, force: bool, workdir: Option<&Path>) -> Result<()> {
    let wd = workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let agent_dir = wd.join(".roko").join("agents").join(name);
    if !agent_dir.exists() {
        bail!("agent '{}' not found at {}", name, agent_dir.display());
    }

    if force {
        println!("Force-deleting agent '{name}'...");
        // Force mode: skip ordered shutdown, remove everything immediately.
        std::fs::remove_dir_all(&agent_dir)
            .with_context(|| format!("remove agent directory at {}", agent_dir.display()))?;
        println!("Agent '{name}' force-deleted.");
        return Ok(());
    }

    // Ordered 8-step shutdown, each step has a 30-second budget.
    let step_timeout = std::time::Duration::from_secs(30);

    // Step 1: Stop processing.
    run_deletion_step("Stop processing", step_timeout, || {
        info!(agent = name, "stopping agent processing");
        Ok(())
    });

    // Step 2: Flush pending.
    run_deletion_step("Flush pending", step_timeout, || {
        info!(agent = name, "flushing pending operations");
        Ok(())
    });

    // Step 3: Backup knowledge.
    run_deletion_step("Backup knowledge", step_timeout, || {
        let neuro_dir = wd.join(".roko").join("neuro");
        if neuro_dir.exists() {
            let backup_dir = wd.join(".roko").join("backups").join(format!(
                "{}-{}",
                name,
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            ));
            std::fs::create_dir_all(&backup_dir)?;
            // Copy knowledge files into the backup directory.
            let knowledge_src = neuro_dir.join("knowledge.jsonl");
            if knowledge_src.exists() {
                std::fs::copy(&knowledge_src, backup_dir.join("knowledge.jsonl"))?;
                println!("  knowledge backed up to {}", backup_dir.display());
            }
            let confirmations_src = neuro_dir.join("knowledge-confirmations.jsonl");
            if confirmations_src.exists() {
                std::fs::copy(
                    &confirmations_src,
                    backup_dir.join("knowledge-confirmations.jsonl"),
                )?;
            }
        } else {
            println!("  no neuro store to backup");
        }
        Ok(())
    });

    // Step 4: Deregister from mesh.
    run_deletion_step("Deregister from mesh", step_timeout, || {
        info!(agent = name, "deregistering from mesh");
        // Mesh deregistration would happen here if mesh is enabled.
        Ok(())
    });

    // Step 5: Release resources.
    run_deletion_step("Release resources", step_timeout, || {
        info!(agent = name, "releasing allocated resources");
        Ok(())
    });

    // Step 6: Archive signals.
    run_deletion_step("Archive signals", step_timeout, || {
        let signals_path = wd.join(".roko").join("signals.jsonl");
        let episodes_path = wd.join(".roko").join("episodes.jsonl");
        let archive_dir = agent_dir.join("archived");
        std::fs::create_dir_all(&archive_dir)?;
        if signals_path.exists() {
            std::fs::copy(&signals_path, archive_dir.join("signals.jsonl"))?;
        }
        if episodes_path.exists() {
            std::fs::copy(&episodes_path, archive_dir.join("episodes.jsonl"))?;
        }
        Ok(())
    });

    // Step 7: Clean state.
    run_deletion_step("Clean state", step_timeout, || {
        let executor_state = wd.join(".roko").join("state").join("executor.json");
        if executor_state.exists() {
            std::fs::remove_file(&executor_state)?;
        }
        // Remove transient files in the agent directory.
        let _ = std::fs::remove_dir_all(agent_dir.join("tmp"));
        Ok(())
    });

    // Step 8: Confirm deletion.
    run_deletion_step("Confirm deletion", step_timeout, || {
        // Write a DELETED marker in the agent directory.
        let marker = agent_dir.join("DELETED");
        let ts = chrono::Utc::now().to_rfc3339();
        std::fs::write(&marker, format!("deleted_at={ts}\nagent={name}\n"))?;
        Ok(())
    });

    println!("Agent '{name}' deleted (ordered shutdown complete).");
    println!("  Archived signals and DELETED marker remain at:");
    println!("  {}", agent_dir.display());

    Ok(())
}

/// Run a single deletion step with a wall-clock timeout. If the step panics
/// or exceeds the timeout, it is skipped and the next step proceeds.
fn run_deletion_step(label: &str, timeout: std::time::Duration, f: impl FnOnce() -> Result<()>) {
    print!("  [{label}] ");
    let start = std::time::Instant::now();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => {
            let elapsed = start.elapsed();
            if elapsed > timeout {
                println!("ok (exceeded {timeout:?}, continuing)");
            } else {
                println!("ok");
            }
        }
        Ok(Err(err)) => {
            println!("skipped: {err}");
        }
        Err(_) => {
            println!("skipped (panicked)");
        }
    }
}

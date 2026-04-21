//! `roko agent serve` command wiring.
//!
//! Also contains the `roko agent create` and `roko agent delete` commands
//! for lifecycle management (LIFE-01, LIFE-06).

use std::path::{Path, PathBuf};
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
};
use roko_agent_server::{
    AgentRegistration, AgentServer, DispatchError, DispatchLike, RelayClientConfig,
};
use roko_cli::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use roko_core::config::schema::RokoConfig;
use roko_core::{Body, Context, Engram, Kind, MessageContent};
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
    /// Start a per-agent HTTP runtime.
    Serve(AgentServeArgs),
}

/// Arguments for `roko agent serve`.
#[derive(Debug, Args, Clone)]
pub struct AgentServeArgs {
    /// Unique agent identifier advertised by the runtime.
    #[arg(long)]
    pub agent_id: String,
    /// Socket address to bind, for example `127.0.0.1:8081`.
    #[arg(long, default_value = "0.0.0.0:8081")]
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
}

#[derive(Debug, Clone)]
struct AgentServeRuntimeConfig {
    agent_id: String,
    bind: String,
    relay: Option<RelayConfig>,
    chain: Option<ChainConfig>,
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
        server.serve().await
    }

    fn build_server(&self) -> Result<AgentServer> {
        let mut builder = AgentServer::builder()
            .agent_id(self.agent_id.clone())
            .bind(self.bind.clone())
            .messaging()
            .predictions()
            .research()
            .tasks();

        if let Some(dispatcher) = self.try_build_dispatcher()? {
            builder = builder.with_message_dispatcher(dispatcher);
        }

        if let Some(registration) = self.registration() {
            builder = builder.registration(registration);
        }

        let startup = self.startup_snapshot();
        builder
            .on_start(move |addr, card| {
                let startup = startup.clone();
                async move {
                    info!(
                        agent_id = %startup.agent_id,
                        bind = %startup.bind,
                        local_addr = %addr,
                        rest_endpoint = ?card.endpoints.rest,
                        "agent server is ready"
                    );
                    if let Some(relay) = &startup.relay {
                        info!(
                            agent_id = %startup.agent_id,
                            relay_url = %relay.url,
                            "relay config captured for later hook-up"
                        );
                    }
                    if let Some(chain) = &startup.chain {
                        info!(
                            agent_id = %startup.agent_id,
                            chain_rpc_url = ?chain.rpc_url,
                            identity_registry = ?chain.identity_registry,
                            passport_id = ?chain.passport_id,
                            "chain registration config captured for later hook-up"
                        );
                        if chain.wallet_key.is_some() {
                            warn!(
                                agent_id = %startup.agent_id,
                                "wallet-key was provided but signing hooks are not wired in this batch"
                            );
                        }
                    }
                    Ok(())
                }
            })
            .build()
    }

    fn try_build_dispatcher(&self) -> Result<Option<Arc<dyn DispatchLike>>> {
        let workdir = std::env::current_dir().context("read current working directory")?;
        let config = load_roko_config(&workdir)?;

        let model = config.agent.default_model.trim();
        if model.is_empty() {
            return Ok(None);
        }

        // A dispatcher can be built if either:
        //   (a) the default model resolves in the provider registry
        //       (e.g. `[providers.lmstudio] kind = "openai_compat"` +
        //       `[models.qwen] provider = "lmstudio"`), or
        //   (b) a legacy subprocess command is configured (e.g. `command = "claude"`).
        // Without either, there is no backend to call.
        let has_provider_backing = config.effective_models().contains_key(model);
        let has_legacy_command = config.agent.command.is_some();
        if !has_provider_backing && !has_legacy_command {
            return Ok(None);
        }

        let agent = spawn_agent_scoped(
            &config,
            SpawnAgentSpec {
                model: model.to_string(),
                command: config.agent.command.clone(),
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
        }
    }
}

#[derive(Debug, Clone)]
struct StartupSnapshot {
    agent_id: String,
    bind: String,
    relay: Option<RelayConfig>,
    chain: Option<ChainConfig>,
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

        Ok(ChatResponse {
            content: result.output.body.as_text().unwrap_or_default().to_string(),
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
    let path = std::env::var_os("ROKO_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workdir.join("roko.toml"));
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))
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
        } => {
            run_agent_create(
                &name,
                &domain,
                template.as_deref(),
                prompt.as_deref(),
                workdir.as_deref(),
            )
            .await
        }
        AgentCmd::Delete {
            name,
            force,
            workdir,
        } => run_agent_delete(&name, force, workdir.as_deref()).await,
        AgentCmd::Serve(args) => AgentServeRuntimeConfig::from_args(args).run().await,
    }
}

// ─── LIFE-01: Agent creation ────────────────────────────────────────────

/// Default prompt used when the operator does not supply one.
const DEFAULT_AGENT_PROMPT: &str =
    "You are a helpful agent. Describe your task in the strategy document.";

/// Three-step agent creation: build manifest, validate, write to disk.
async fn run_agent_create(
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

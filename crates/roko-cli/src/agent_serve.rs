//! `roko agent serve` command wiring.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use clap::{Args, Subcommand};
use roko_agent::{
    Agent,
    chat_types::{ChatRequest, ChatResponse},
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
        AgentCmd::Serve(args) => AgentServeRuntimeConfig::from_args(args).run().await,
    }
}

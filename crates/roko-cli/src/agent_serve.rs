//! `roko agent serve` command wiring.

use anyhow::Result;
use clap::{Args, Subcommand};
use roko_agent_server::{AgentRegistration, AgentServer};
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

        if let Some(chain) = &self.chain {
            let mut registration = AgentRegistration::default();
            registration.identity_registry_address = chain.identity_registry.clone();
            registration.passport_id = chain.passport_id.clone();
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

/// Run `roko agent ...`.
pub async fn run(cmd: AgentCmd) -> Result<()> {
    match cmd {
        AgentCmd::Serve(args) => AgentServeRuntimeConfig::from_args(args).run().await,
    }
}

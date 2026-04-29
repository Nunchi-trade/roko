//! agent command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_agent(cli: &Cli, cmd: AgentCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);

    let provider: Option<&str> = match &cmd {
        AgentCmd::Chat { provider, .. } => provider.as_deref(),
        _ => None,
    };
    if let Some(provider_name) = provider {
        if let AgentCmd::Chat { agent, .. } = &cmd {
            let resolved = load_layered(&workdir)?;
            let config = resolved.config;
            let mut provider_config = roko_core::config::schema::RokoConfig::default();
            provider_config.providers.extend(config.providers.clone());
            provider_config.models.extend(config.models.clone());
            if let Some(model) = config.agent.model.clone() {
                provider_config.agent.default_model = model;
            }
            provider_config.agent.default_effort = config.agent.effort.clone();
            provider_config.agent.bare_mode = config.agent.bare_mode;
            provider_config.agent.timeout_ms = Some(config.agent.timeout_ms);
            provider_config.agent.fallback_model = config.agent.fallback_model.clone();
            provider_config.agent.tier_models = config.agent.tier_models.clone();
            provider_config.agent.env = Some(config.agent.env.clone());

            roko_cli::chat::run_direct_provider_chat(agent, provider_name, &provider_config)
                .await?;
            return Ok(EXIT_SUCCESS);
        }
    }

    agent_serve::run(cmd).await?;
    Ok(EXIT_SUCCESS)
}

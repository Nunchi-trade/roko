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
            // Pre-flight: check providers before starting chat session.
            {
                let chat_config: roko_core::config::schema::RokoConfig =
                    std::fs::read_to_string(workdir.join("roko.toml"))
                        .ok()
                        .and_then(|s| roko_core::config::schema::RokoConfig::from_toml(&s).ok())
                        .unwrap_or_default();
                let dm = &chat_config.agent.default_model;
                if !dm.trim().is_empty() {
                    crate::commands::util::preflight_provider_for_model(&chat_config, dm)?;
                }
                // Aggregate provider readiness: warn/abort if no providers are usable.
                crate::commands::util::preflight_providers_aggregate(&chat_config)?;
            }
            let resolved = load_resolved_config(&workdir)?;
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

            roko_cli::chat::run_direct_provider_chat(
                agent,
                provider_name,
                &provider_config,
                &workdir,
            )
            .await?;
            return Ok(EXIT_SUCCESS);
        }
    }

    agent_serve::run(cmd).await?;
    Ok(EXIT_SUCCESS)
}

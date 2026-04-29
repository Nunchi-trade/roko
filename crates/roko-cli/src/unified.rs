//! Unified CLI entry point — one command, everything works.
//!
//! `roko` with no args launches inline chat with:
//! - Auto-detected auth (Claude CLI → API key → prompt)
//! - In-process dispatch (no sidecar required)
//! - Optional background `roko serve` for HTTP/dashboard via `serve.auto_start`
//!   (disable with `--no-serve`)

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::task::JoinHandle;
use tracing::info;

use crate::auth_detect::{AuthMethod, detect_auth, print_setup_instructions};
use crate::chat_inline;
use crate::chat_session::ChatAgentSession;
use crate::config::RepoRegistry;
use crate::model_selection::resolve_effective_model;
use crate::serve_runtime::RokoCliRuntime;
use roko_core::agent::ProviderKind;
use roko_core::config::schema::RokoConfig;

/// Main unified entry point: auto-detect auth, launch chat.
///
/// Called when the user runs `roko` with no subcommand and stdin is a TTY.
pub async fn cmd_unified_chat(
    config_path: Option<&std::path::Path>,
    quiet: bool,
    no_serve: bool,
) -> Result<i32> {
    // 1. Auto-detect auth
    let auth = detect_auth();
    if matches!(auth, AuthMethod::NeedsSetup) {
        print_setup_instructions();
        return Ok(1);
    }

    // 2. Resolve working directory
    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // 3. Auto-create .roko/ if missing
    ensure_workspace(&workdir)?;

    // 4. Load config for serve (best-effort)
    let config = load_config_or_defaults(config_path, &workdir)?;

    // 5. Start serve in background only when the resolved config opts in.
    let serve_state = if no_serve {
        None
    } else if load_auto_start_config(&config) {
        spawn_background_serve(&config, &workdir).await
    } else {
        if !quiet {
            eprintln!("Tip: run `roko serve` to start the HTTP control plane");
        }
        None
    };

    if !quiet {
        eprintln!(
            "roko — auth: {}{}",
            auth.label(),
            if serve_state.is_some() {
                ", serve :6677"
            } else {
                ""
            }
        );
    }

    // 6. Launch inline chat with direct dispatch
    let result = chat_inline::run_unified_inline(&auth).await;

    // 7. Graceful shutdown of background serve
    if let Some((state, handle)) = serve_state {
        state.shutdown().await;
        handle.abort();
    }

    match result {
        Ok(()) => Ok(0),
        Err(e) => {
            eprintln!("chat error: {e:#}");
            Ok(1)
        }
    }
}

/// One-shot inline mode: dispatch a bare prompt, print result, exit.
///
/// Called for `roko "fix the bug"` (positional prompt, no subcommand).
///
/// Uses `ChatAgentSession` for full system prompt, tools, MCP, and safety
/// settings. Falls back to raw dispatch if session initialization fails.
pub async fn cmd_oneshot_inline(prompt: &str, quiet: bool) -> Result<i32> {
    let auth = detect_auth();
    if matches!(auth, AuthMethod::NeedsSetup) {
        print_setup_instructions();
        return Ok(1);
    }

    if !quiet {
        eprintln!("roko — auth: {}", auth.label());
    }

    let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = load_config_or_defaults(None, &workdir)?;

    // Build a ChatAgentSession for full tool/system-prompt/MCP support.
    let mut session = match build_oneshot_session(&config, &auth, workdir.clone()) {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!("ChatAgentSession init failed ({e:#}), falling back to dispatch_direct");
            // Fallback: raw dispatch (no system prompt, no tools, no MCP)
            #[cfg(feature = "legacy-orchestrate")]
            {
                let result = {
                    #[allow(deprecated)]
                    {
                        match crate::dispatch_v2::dispatch_via_model_call_service(prompt).await {
                            Ok(r) => r,
                            Err(e2) => {
                                tracing::debug!(
                                    "ModelCallService also failed ({e2:#}), using raw dispatch"
                                );
                                crate::dispatch_direct::dispatch_prompt(&auth, prompt).await?
                            }
                        }
                    }
                };
                for tool_output in &result.tool_outputs {
                    let label = tool_output.tool_name.as_deref().unwrap_or("tool");
                    eprintln!(
                        "[{label}] {}",
                        tool_output.content.lines().next().unwrap_or("")
                    );
                }
                println!("{}", result.text);
                if !quiet {
                    eprintln!(
                        "\n[{} | {} in / {} out tokens]",
                        result.model, result.input_tokens, result.output_tokens,
                    );
                }
                return Ok(0);
            }
            #[cfg(not(feature = "legacy-orchestrate"))]
            {
                return Err(e);
            }
        }
    };

    // Single-turn dispatch via ChatAgentSession.
    let result = match session.send_turn_oneshot(prompt).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            return Ok(1);
        }
    };

    if result.cancelled {
        eprintln!("error: turn cancelled");
        return Ok(1);
    }

    // Show tool summaries on stderr before response text.
    for tc in &result.tool_calls {
        let status = if tc.success { "done" } else { "failed" };
        eprintln!("[{}] {}", tc.name, status);
    }

    // Response text to stdout (pipe-friendly).
    println!("{}", result.text);

    if !quiet {
        eprintln!(
            "\n[{} | {} in / {} out tokens | {:.1}s]",
            result.model,
            result.input_tokens,
            result.output_tokens,
            result.duration.as_secs_f64(),
        );
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// Background serve
// ---------------------------------------------------------------------------

/// Start `roko serve` as a background tokio task.
///
/// Returns the `AppState` (for graceful shutdown) and the task handle.
/// Returns `None` if the server fails to start (non-fatal).
async fn spawn_background_serve(
    config: &crate::config::Config,
    workdir: &std::path::Path,
) -> Option<(Arc<roko_serve::state::AppState>, JoinHandle<Result<()>>)> {
    let runtime = RokoCliRuntime::new(config.clone(), RepoRegistry::default()).into_arc();
    match roko_serve::start_server_background(workdir.to_path_buf(), runtime, None, None).await {
        Ok(pair) => Some(pair),
        Err(e) => {
            tracing::warn!("background serve failed to start: {e:#}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create `.roko/` directory if it doesn't exist.
fn ensure_workspace(workdir: &std::path::Path) -> Result<()> {
    let roko_dir = workdir.join(".roko");
    if !roko_dir.exists() {
        std::fs::create_dir_all(&roko_dir)
            .with_context(|| format!("create {}", roko_dir.display()))?;
        info!("created .roko/ directory");
    }
    Ok(())
}

/// Load config from roko.toml, falling back to defaults if absent.
fn load_config_or_defaults(
    config_path: Option<&std::path::Path>,
    workdir: &std::path::Path,
) -> Result<crate::config::Config> {
    if let Some(p) = config_path {
        return crate::config::Config::from_file(p);
    }

    // Try layered resolution; if it fails (no roko.toml at all), use defaults.
    match crate::config::load_layered(workdir) {
        Ok(resolved) => Ok(resolved.config),
        Err(_) => Ok(crate::config::Config::default()),
    }
}

/// Read the resolved `serve.auto_start` flag from the already-loaded config.
fn load_auto_start_config(config: &crate::config::Config) -> bool {
    config.serve.auto_start
}

fn build_oneshot_session(
    config: &crate::config::Config,
    auth: &AuthMethod,
    workdir: PathBuf,
) -> Result<ChatAgentSession> {
    // One-shot chat currently has a Claude CLI implementation only.
    if !matches!(auth, &AuthMethod::ClaudeCli) {
        return Err(anyhow::anyhow!(
            "ChatAgentSession oneshot currently supports Claude CLI auth, got {}",
            auth.label()
        ));
    }

    let mut model_config = RokoConfig::default();
    model_config.providers.extend(config.providers.clone());
    model_config.models.extend(config.models.clone());
    model_config.agent.default_effort = config.agent.effort.clone();
    model_config.agent.bare_mode = config.agent.bare_mode;
    model_config.agent.timeout_ms = Some(config.agent.timeout_ms);
    model_config.agent.fallback_model = config.agent.fallback_model.clone();
    model_config.agent.tier_models = config.agent.tier_models.clone();
    model_config.agent.env = Some(config.agent.env.clone());
    if let Some(model) = config.agent.model.clone() {
        model_config.agent.default_model = model;
    }

    let selection = resolve_effective_model(None, None, None, None, &model_config)
        .context("resolve oneshot model selection")?;
    if selection.provider_kind != ProviderKind::ClaudeCli.label() {
        return Err(anyhow::anyhow!(
            "ChatAgentSession oneshot currently supports Claude CLI provider, got {}",
            selection.provider_kind
        ));
    }
    ChatAgentSession::new(config, workdir, selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_workspace_creates_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let roko_dir = tmp.path().join(".roko");
        assert!(!roko_dir.exists());
        ensure_workspace(tmp.path()).unwrap();
        assert!(roko_dir.exists());
    }

    #[test]
    fn ensure_workspace_noop_if_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko")).unwrap();
        ensure_workspace(tmp.path()).unwrap(); // should not error
    }

    #[test]
    fn load_config_defaults_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let config = load_config_or_defaults(None, tmp.path()).unwrap();
        // Should get a valid default config without error
        assert!(!config.agent.command.is_empty());
    }

    #[test]
    fn load_auto_start_config_reads_resolved_flag() {
        let mut config = crate::config::Config::default();
        assert!(!load_auto_start_config(&config));
        config.serve.auto_start = true;
        assert!(load_auto_start_config(&config));
    }
}

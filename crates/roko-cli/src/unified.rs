//! Unified CLI entry point — one command, everything works.
//!
//! `roko` with no args launches inline chat with:
//! - Auto-detected auth (Claude CLI → API key → prompt)
//! - In-process dispatch (no sidecar required)
//! - Background `roko serve` for HTTP/dashboard (disable with `--no-serve`)

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::task::JoinHandle;
use tracing::info;

use crate::auth_detect::{AuthMethod, detect_auth, print_setup_instructions};
use crate::chat_inline;
use crate::config::RepoRegistry;
use crate::serve_runtime::RokoCliRuntime;

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

    // 5. Start serve in background (tracing already routed to file by main.rs)
    let serve_state = if no_serve {
        None
    } else {
        spawn_background_serve(&config, &workdir).await
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
pub async fn cmd_oneshot_inline(prompt: &str, quiet: bool) -> Result<i32> {
    let auth = detect_auth();
    if matches!(auth, AuthMethod::NeedsSetup) {
        print_setup_instructions();
        return Ok(1);
    }

    if !quiet {
        eprintln!("roko — auth: {}", auth.label());
    }

    let result = crate::dispatch_direct::dispatch_prompt(&auth, prompt).await?;

    // Show tool outputs before the response text
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
}

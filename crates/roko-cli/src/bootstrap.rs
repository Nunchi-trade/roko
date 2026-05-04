//! Unified startup helper for all Roko entry points.
//!
//! `RokoBootstrap` consolidates the per-entry-point startup logic (workdir
//! canonicalization, workspace check, config load, provider validation) so
//! that `chat`, `plan run`, and `serve` share a consistent bootstrap path.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use roko_core::config::schema::RokoConfig;

/// Options that control which startup checks are performed.
pub struct BootOpts {
    /// Fail if no `.roko/` directory exists under `workdir`.
    pub require_workspace: bool,
    /// Fail if no LLM provider is reachable (checks env vars / config).
    pub require_provider: bool,
    /// Acquire a workspace lock before returning (reserved for future use).
    pub acquire_lock: bool,
}

impl Default for BootOpts {
    fn default() -> Self {
        Self {
            require_workspace: true,
            require_provider: false,
            acquire_lock: false,
        }
    }
}

/// Result of a successful bootstrap.
pub struct RokoBootstrap {
    /// Fully resolved config (unified loader: global merge + env overrides).
    pub config: RokoConfig,
    /// Canonical working directory.
    pub workdir: PathBuf,
    /// Whether `.roko/` exists under `workdir`.
    pub workspace_ready: bool,
}

impl RokoBootstrap {
    /// Run startup checks according to `opts` and return a bootstrapped context.
    ///
    /// # Errors
    ///
    /// - `require_workspace = true` → returns an error when `.roko/` is absent.
    /// - `require_provider = true` → returns an error when no provider is usable.
    pub fn new(workdir: &Path, opts: BootOpts) -> Result<Self> {
        // 1. Canonicalize workdir (best-effort; fall back to the raw path).
        let workdir = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf());

        // 2. Workspace presence check.
        let roko_dir = workdir.join(".roko");
        let workspace_ready = roko_dir.is_dir();
        if opts.require_workspace && !workspace_ready {
            anyhow::bail!(
                "No roko workspace found at {}.\n  hint: run `roko init`",
                workdir.display()
            );
        }

        // 3. Load unified config (global merge + ROKO__* env overrides).
        //    Falls back to built-in defaults on missing/unreadable roko.toml.
        let config = roko_core::config::loader::load_config_unified(&workdir).unwrap_or_default();

        // 4. Validate that at least one provider is usable when required.
        if opts.require_provider {
            validate_provider_available(&config, &workdir)
                .context("provider check failed — run `roko config providers health`")?;
        }

        Ok(Self {
            config,
            workdir,
            workspace_ready,
        })
    }
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Check that at least one LLM provider appears usable.
///
/// Accepts env-var credentials (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
/// `ZAI_API_KEY`) or configured providers in `roko.toml` with a resolvable
/// key, or a working `claude` CLI installation.
fn validate_provider_available(config: &RokoConfig, _workdir: &Path) -> Result<()> {
    // 1. Env-var credentials.
    let has_env_key = std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ZAI_API_KEY").is_ok();
    if has_env_key {
        return Ok(());
    }

    // 2. Configured provider with a resolvable API key.
    let has_config_key = config.providers.values().any(|p| {
        p.api_key_env
            .as_deref()
            .map(|env| std::env::var(env).map(|v| !v.is_empty()).unwrap_or(false))
            .unwrap_or(false)
    });
    if has_config_key {
        return Ok(());
    }

    // 3. CLI-based provider (command on PATH).
    let has_cli_provider = config
        .providers
        .values()
        .any(|p| p.command.as_deref().is_some_and(|cmd| binary_on_path(cmd)));
    if has_cli_provider {
        return Ok(());
    }

    // 4. `claude` CLI available (default provider when nothing else is configured).
    let claude_ok = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if claude_ok {
        return Ok(());
    }

    anyhow::bail!(
        "No LLM provider found.\n\
         Set ANTHROPIC_API_KEY (or OPENAI_API_KEY), configure a provider in roko.toml,\n\
         or install and log in to the `claude` CLI."
    )
}

fn binary_on_path(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

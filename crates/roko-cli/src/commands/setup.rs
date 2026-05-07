//! `roko setup` — interactive workspace bootstrap wizard.
//!
//! Steps:
//! 1. Detect available providers via `detect_auth_from_env()`
//! 2. If `NeedsSetup`: prompt for API key (or print instructions)
//! 3. Auto-select default model based on available provider
//! 4. Run `roko init` if `.roko/` doesn't exist
//! 5. Run `roko doctor` to verify
//! 6. Print "next steps" message

use anyhow::Result;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::Cli;
use roko_cli::auth_detect::{AuthMethod, detect_auth_from_env};
use roko_cli::doctor::{DoctorOptions, run_doctor};

use super::util::cmd_init;

/// Run the interactive setup wizard.
///
/// When `yes` is true, skip all prompts and use the first available provider.
pub(crate) async fn cmd_setup(cli: &Cli, workdir: Option<PathBuf>, yes: bool) -> Result<i32> {
    let workdir =
        workdir.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    println!("roko setup");
    println!("==========\n");

    // ── Step 1: Detect auth ─────────────────────────────────────────────
    println!("[1/5] Detecting available LLM providers...");
    let auth = detect_auth_from_env();

    let auth = match &auth {
        AuthMethod::NeedsSetup => {
            if yes {
                eprintln!("  No provider found. Set an API key env var and re-run.");
                return Ok(1);
            }
            // Interactive: prompt for API key
            prompt_for_api_key()?
        }
        other => {
            println!("  Found: {}", other.label());
            other.clone()
        }
    };

    // ── Step 2: Show selected model ─────────────────────────────────────
    let model = default_model_for_auth(&auth);
    println!("\n[2/5] Default model: {model}");

    // ── Step 3: Init if needed ──────────────────────────────────────────
    let roko_dir = workdir.join(".roko");
    if roko_dir.is_dir() {
        println!("\n[3/5] Workspace already initialized (.roko/ exists)");
    } else {
        println!("\n[3/5] Initializing workspace...");
        cmd_init(Some(workdir.clone()), false, None, false).await?;
        println!("  Created .roko/ and roko.toml");
    }

    // ── Step 4: Doctor ──────────────────────────────────────────────────
    println!("\n[4/5] Running diagnostics...");
    let report = run_doctor(&DoctorOptions {
        workdir: workdir.clone(),
        config_override: cli.config.clone(),
        serve_url: None,
    })
    .await?;

    if report.healthy {
        println!("  All checks passed.");
    } else {
        println!("  Some checks need attention:");
        for check in &report.checks {
            if check.status == roko_cli::doctor::DoctorStatus::Fail {
                println!("    [fail] {}: {}", check.id, check.message);
                if let Some(fix) = &check.fix {
                    println!("           fix: {fix}");
                }
            }
        }
    }

    // ── Step 5: Next steps ──────────────────────────────────────────────
    println!("\n[5/5] Next steps:");
    println!("  roko \"describe your task\"     Run a one-shot task");
    println!("  roko do \"add feature X\"       Plan and execute a feature");
    println!("  roko doctor                   Re-run diagnostics anytime");
    println!("  roko status                   Check workspace health");
    if matches!(auth, AuthMethod::NeedsSetup) {
        println!("\n  (Set an LLM provider key to enable agent dispatch)");
    }

    Ok(0)
}

/// Prompt the user to set up an API key interactively.
fn prompt_for_api_key() -> Result<AuthMethod> {
    println!("  No LLM provider detected.\n");
    println!("  Options:");
    println!("    1. Install Claude CLI: npm install -g @anthropic-ai/claude-cli && claude login");
    println!("    2. Set ANTHROPIC_API_KEY=sk-ant-...");
    println!("    3. Set OPENAI_API_KEY=sk-...");
    println!();
    print!("  Enter API key (or press Enter to skip): ");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let line = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
    let key = line.trim().to_string();

    if key.is_empty() {
        println!("  Skipped. You can set an env var later.");
        return Ok(AuthMethod::NeedsSetup);
    }

    // Guess provider from key prefix
    if key.starts_with("sk-ant-") {
        println!("  Detected Anthropic key. Set in your shell:");
        println!("    export ANTHROPIC_API_KEY={key}");
        Ok(AuthMethod::AnthropicApi { key, model: None })
    } else {
        println!("  Detected OpenAI-compatible key. Set in your shell:");
        println!("    export OPENAI_API_KEY={key}");
        Ok(AuthMethod::OpenAiCompat {
            key,
            base_url: "https://api.openai.com/v1".to_string(),
            model: None,
        })
    }
}

/// Pick a sensible default model based on detected auth.
fn default_model_for_auth(auth: &AuthMethod) -> &'static str {
    match auth {
        AuthMethod::ClaudeCli | AuthMethod::AnthropicApi { .. } => "claude-sonnet-4-6",
        AuthMethod::OpenAiCompat { .. } => "gpt-5.4-mini",
        AuthMethod::NeedsSetup => "claude-sonnet-4-6",
    }
}

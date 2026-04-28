//! `roko demo` subcommand — setup and run demos.
//!
//! Provides quick commands for demo preparation:
//! - `roko demo setup` — build release binary, pre-warm caches
//! - `roko demo serve` — start serve with quiet logs + open browser
//! - `roko demo warm` — populate LLM response cache for determinism

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::inline::plaintext;
use crate::inline::styled;
use crate::inline::symbols;
use crate::tui::Theme;

/// Run the demo setup: build release binary, verify workspace, print instructions.
pub fn cmd_demo_setup(workdir: &Path) -> Result<()> {
    let theme = Theme::dark();

    plaintext::print_plain(&[styled::section_start(&theme, "demo", "setup", None)]);

    // Step 1: Build release binary
    plaintext::print_plain(&[styled::continuation(
        &theme,
        "build",
        "compiling release binary...",
        None,
    )]);

    let build = Command::new("cargo")
        .args(["build", "-p", "roko-cli", "--release"])
        .status()
        .context("run cargo build")?;

    if build.success() {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "",
            &format!("{} release binary ready", symbols::PASS),
            None,
        )]);
    } else {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "",
            &format!("{} build failed", symbols::FAIL),
            None,
        )]);
        return Ok(());
    }

    // Step 2: Check .roko/ exists
    let roko_dir = workdir.join(".roko");
    if roko_dir.exists() {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "workspace",
            &format!("{} .roko/ exists", symbols::PASS),
            None,
        )]);
    } else {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "workspace",
            "initializing .roko/...",
            None,
        )]);
        let roko = std::env::current_exe().unwrap_or_else(|_| "roko".into());
        let _ = Command::new(roko).arg("init").current_dir(workdir).status();
    }

    // Step 3: Check demo pages exist
    let demo_dir = workdir.join("demo").join("demo-web");
    if demo_dir.join("builder.html").exists() {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "pages",
            &format!("{} demo pages found", symbols::PASS),
            None,
        )]);
    } else {
        plaintext::print_plain(&[styled::continuation(
            &theme,
            "pages",
            &format!("{} demo/demo-web/ not found", symbols::WARN),
            None,
        )]);
    }

    plaintext::print_plain(&[styled::section_end(&theme, "", "")]);

    println!();
    println!("  To run the demo:");
    println!("    roko serve");
    println!("    open http://localhost:6677");
    println!();
    println!("  Pages:");
    println!("    /                        Index");
    println!("    /demo/builder.html       Interactive builder");
    println!("    /demo/terminal.html      Multi-terminal");
    println!("    /demo/index.html         Scripted demo");
    println!();

    Ok(())
}

/// Pre-warm the LLM response cache with demo prompts.
pub async fn cmd_demo_warm(workdir: &Path) -> Result<()> {
    let theme = Theme::dark();

    plaintext::print_plain(&[styled::section_start(
        &theme,
        "demo",
        "warming response cache",
        None,
    )]);

    // Note: set ROKO_DEMO_CACHE=1 before running this command to enable file caching.
    // We can't set it here due to Rust 2024 unsafe rules for env vars.

    let prompts = [
        "Build a CLI calculator in Rust",
        "Create a hello world REST API",
        "Fix the failing test in src/auth.rs",
        "Add a --dry-run flag to the plan command",
        "Summarize Q3 fintech earnings",
    ];

    for prompt in &prompts {
        plaintext::print_plain(&[styled::continuation(&theme, "warm", prompt, None)]);
        // Run each prompt through the universal loop
        // This populates both the in-memory and file caches
        let config = crate::config::Config::default();
        match crate::run::run_once(workdir, &config, prompt, None).await {
            Ok(report) => {
                let icon = if report.overall_success() {
                    symbols::PASS
                } else {
                    symbols::FAIL
                };
                plaintext::print_plain(&[styled::continuation(
                    &theme,
                    "",
                    &format!("  {icon} cached"),
                    None,
                )]);
            }
            Err(e) => {
                plaintext::print_plain(&[styled::continuation(
                    &theme,
                    "",
                    &format!("  {} {e}", symbols::FAIL),
                    None,
                )]);
            }
        }
    }

    let cache_dir = workdir.join(".roko").join("demo-cache");
    let count = std::fs::read_dir(&cache_dir)
        .map(|e| e.count())
        .unwrap_or(0);

    plaintext::print_plain(&[styled::section_end(
        &theme,
        "cached",
        &format!("{count} entries in .roko/demo-cache/"),
    )]);

    Ok(())
}

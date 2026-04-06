//! `roko` binary entrypoint.
//!
//! See [`roko_cli`] for the lib-side description. The binary exposes four
//! subcommands — `init`, `run`, `status`, and `replay` — and reads its
//! agent/gate config from `./roko.toml`.

#![allow(clippy::too_many_lines)]

use anyhow::{anyhow, Context as _, Result};
use clap::{Parser, Subcommand};
use roko_cli::{
    config_cmd, load_layered, run_init_wizard, run_once, Config, EditTarget, Source, WizardInputs,
};
use roko_core::{Context, ContentHash, Kind, Query, Substrate};
use roko_fs::FileSubstrate;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Minimum CLI for the Roko universal loop.
#[derive(Debug, Parser)]
#[command(name = "roko", version, about = "Minimal CLI for the Roko universal loop")]
struct Cli {
    /// Override the config file (default: `./roko.toml`).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create `.roko/` and a default `roko.toml` in `path` (default: cwd).
    Init {
        /// Directory to initialize (default: current dir).
        path: Option<PathBuf>,
    },
    /// Seed a prompt and run the universal loop (compose → agent → gate → persist).
    Run {
        /// The user prompt text.
        prompt: String,
        /// Override the working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Print signal counts by kind, most recent episode, and gate pass/fail counts.
    Status {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Walk the lineage DAG rooted at a signal hash and print it.
    Replay {
        /// Signal hash (64 hex chars) to walk.
        hash: String,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Manage global and project config (wizard, show, path, edit, set).
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCmd {
    /// Interactive wizard: detects installed LLM CLIs, writes global config.
    Init {
        /// Skip all confirmation prompts.
        #[arg(long)]
        yes: bool,
        /// Pre-select agent command (skip picker).
        #[arg(long)]
        agent: Option<String>,
        /// Pre-set model name (ollama-only convenience).
        #[arg(long)]
        model: Option<String>,
        /// Pre-set token budget.
        #[arg(long)]
        budget: Option<usize>,
        /// Pre-set role string.
        #[arg(long)]
        role: Option<String>,
        /// Enable default compile+clippy gates.
        #[arg(long)]
        enable_gates: bool,
        /// Write to this path instead of the resolved global path.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Non-interactive mode: skip all prompts, fail if any answer is missing.
        #[arg(long)]
        non_interactive: bool,
    },
    /// Print the effective merged config with per-field source tags.
    Show {
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Print the resolved global + project + env config paths.
    Path {
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Open $EDITOR on the chosen config file.
    Edit {
        /// Open the global config file.
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Open (or create) the project `roko.toml`.
        #[arg(long, conflicts_with = "global")]
        project: bool,
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Set a dotted key (e.g. `agent.command = ollama`) in the chosen layer.
    Set {
        /// Dotted key path.
        key: String,
        /// Value to write.
        value: String,
        /// Write to project config instead of global.
        #[arg(long, conflicts_with = "global")]
        project: bool,
        /// Write to global config (default).
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { path } => cmd_init(path).await,
        Command::Run { prompt, workdir } => cmd_run(cli.config, workdir, prompt).await,
        Command::Status { workdir } => cmd_status(workdir).await,
        Command::Replay { hash, workdir } => cmd_replay(workdir, hash).await,
        Command::Config { cmd } => dispatch_config(cmd),
    }
}

fn dispatch_config(cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Init {
            yes,
            agent,
            model,
            budget,
            role,
            enable_gates,
            path,
            non_interactive,
        } => {
            let mut inputs = WizardInputs {
                agent_command: agent.clone(),
                token_budget: budget,
                role,
                enable_gates: if enable_gates { Some(true) } else { None },
                yes,
                ..Default::default()
            };
            if let (Some("ollama"), Some(m)) = (agent.as_deref(), model.as_ref()) {
                inputs.agent_args = Some(vec!["run".into(), m.clone()]);
            }
            if non_interactive {
                // Require all answers up-front.
                if inputs.agent_command.is_none() {
                    return Err(anyhow!("--non-interactive requires --agent"));
                }
                inputs.token_budget.get_or_insert(8000);
                inputs.role.get_or_insert_with(|| "You are a Roko agent.".into());
                inputs.enable_gates.get_or_insert(false);
                inputs.yes = true;
                if inputs.agent_args.is_none() {
                    inputs.agent_args = Some(vec![]);
                }
            }
            let _ = run_init_wizard(path, &inputs)?;
            Ok(())
        }
        ConfigCmd::Show { workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            config_cmd::cmd_show(&wd)
        }
        ConfigCmd::Path { workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            config_cmd::cmd_path(&wd)
        }
        ConfigCmd::Edit { global, project, workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            let target = edit_target(global, project);
            config_cmd::cmd_edit(&wd, target)
        }
        ConfigCmd::Set { key, value, global: _, project, workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            // Default for `set` is global unless --project is passed.
            let target = if project {
                EditTarget::Project
            } else {
                EditTarget::Global
            };
            config_cmd::cmd_set(&wd, target, &key, &value)
        }
    }
}

const fn edit_target(global: bool, project: bool) -> EditTarget {
    if global {
        EditTarget::Global
    } else if project {
        EditTarget::Project
    } else {
        EditTarget::Auto
    }
}

async fn cmd_init(path: Option<PathBuf>) -> Result<()> {
    let target = path.unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let roko_dir = target.join(".roko");
    tokio::fs::create_dir_all(&roko_dir)
        .await
        .with_context(|| format!("create {}", roko_dir.display()))?;

    // Touch signals.jsonl so FileSubstrate replays cleanly on first open.
    let signals_path = roko_dir.join("signals.jsonl");
    if !signals_path.exists() {
        tokio::fs::write(&signals_path, b"")
            .await
            .with_context(|| format!("create {}", signals_path.display()))?;
    }

    // Write a default roko.toml if missing.
    let config_path = target.join("roko.toml");
    if config_path.exists() {
        println!("{} already exists; leaving untouched.", config_path.display());
    } else {
        let default = Config::default().to_toml()?;
        tokio::fs::write(&config_path, default)
            .await
            .with_context(|| format!("write {}", config_path.display()))?;
        println!("wrote {}", config_path.display());
    }

    println!("initialized roko workspace at {}", target.display());
    Ok(())
}

async fn cmd_run(
    config_path: Option<PathBuf>,
    workdir: Option<PathBuf>,
    prompt: String,
) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    // Explicit --config overrides everything; otherwise use layered loading.
    let config = if let Some(p) = config_path {
        Config::from_file(&p)?
    } else {
        let resolved = load_layered(&workdir)?;
        // First-run hint: no config anywhere AND we're falling back to `cat`.
        let fully_default = resolved.sources.agent_command == Source::Default
            && resolved.sources.prompt_token_budget == Source::Default;
        if fully_default && resolved.config.agent.command == "cat" {
            println!(
                "no config found — using built-in `cat` agent. run `roko config init` to set up a model."
            );
        }
        resolved.config
    };

    println!(
        "running agent `{}` with {} gate(s)",
        config.agent.command,
        config.gates.len()
    );
    let report = run_once(&workdir, &config, &prompt).await?;

    println!("---");
    println!("agent        : {} (success={})", config.agent.command, report.agent_success);
    println!("prompt_id    : {}", report.prompt_id);
    println!("agent_output : {}", report.agent_output_id);
    if report.gate_verdicts.is_empty() {
        println!("gates        : (none configured)");
    } else {
        println!("gates:");
        for (name, ok) in &report.gate_verdicts {
            let marker = if *ok { "PASS" } else { "FAIL" };
            println!("  [{marker}] {name}");
        }
    }
    println!("episode      : {}", report.episode_id);
    println!("signals      : {}", report.total_signals);

    if !report.overall_success() {
        std::process::exit(1);
    }
    Ok(())
}

async fn cmd_status(workdir: Option<PathBuf>) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let ctx = Context::now();

    let all = substrate
        .query(&Query::all(), &ctx)
        .await
        .map_err(|e| anyhow!("query: {e}"))?;

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for sig in &all {
        *counts.entry(sig.kind.to_string()).or_default() += 1;
    }

    println!("signal counts ({} total):", all.len());
    if counts.is_empty() {
        println!("  (empty)");
    } else {
        for (kind, n) in &counts {
            println!("  {kind:<24} {n}");
        }
    }

    // Most recent episode.
    let mut episodes = substrate
        .query(&Query::of_kind(Kind::Episode), &ctx)
        .await
        .map_err(|e| anyhow!("query episodes: {e}"))?;
    episodes.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
    println!();
    match episodes.first() {
        Some(ep) => {
            println!("most recent episode: {} (passed={})", ep.id, ep.tag("passed").unwrap_or("?"));
            println!(
                "  gates passed={} failed={}",
                ep.tag("gates_passed").unwrap_or("0"),
                ep.tag("gates_failed").unwrap_or("0")
            );
        }
        None => println!("most recent episode: (none)"),
    }

    // Gate verdict pass/fail rollup.
    let verdicts = substrate
        .query(&Query::of_kind(Kind::GateVerdict), &ctx)
        .await
        .map_err(|e| anyhow!("query verdicts: {e}"))?;
    let passed = verdicts.iter().filter(|v| v.tag("passed") == Some("true")).count();
    let failed = verdicts.iter().filter(|v| v.tag("passed") == Some("false")).count();
    println!("gate verdicts: {passed} pass / {failed} fail");

    Ok(())
}

async fn cmd_replay(workdir: Option<PathBuf>, hash: String) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let start = ContentHash::from_hex(&hash)
        .ok_or_else(|| anyhow!("invalid hash (expected 64 hex chars): {hash}"))?;

    // Breadth-first walk of the lineage DAG.
    let mut visited = std::collections::HashSet::new();
    let mut queue = vec![(start, 0usize)];
    let mut printed = 0usize;
    while let Some((id, depth)) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        let indent = "  ".repeat(depth);
        if let Some(sig) = substrate.get(&id).await.map_err(|e| anyhow!("get: {e}"))? {
            println!("{indent}{} {}  (author={})", sig.kind, sig.id, sig.provenance.author);
            for parent in &sig.lineage {
                queue.push((*parent, depth + 1));
            }
            printed += 1;
        } else {
            println!("{indent}<missing {id}>");
        }
    }
    if printed == 0 {
        println!("signal {hash} not found in substrate");
        std::process::exit(1);
    }
    Ok(())
}

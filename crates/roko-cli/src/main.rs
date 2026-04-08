//! `roko` binary entrypoint.
//!
//! See [`roko_cli`] for the lib-side description. The binary exposes
//! subcommands (`init`, `run`, `status`, `replay`, `config`, `inject`,
//! `plan`) plus top-level flags for mode selection (`--headless`,
//! `--role`, `--model`, `--effort`, `--json`, `--quiet`, `--resume`,
//! `--repo`, and a positional `[prompt]` for one-shot mode).

#![allow(clippy::too_many_lines)]

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, Subcommand, ValueEnum};
use roko_agent::process::{cleanup_orphaned_agents, reap_orphaned_children};
use roko_cli::{
    Config, DaemonMode, EditTarget, InjectKind, InjectRequest, OneshotMode, PipeMode, Plan,
    PlanSummary, ReplMode, SessionStatus, Source, WizardInputs, config_cmd, load_layered,
    run_init_wizard, run_once,
};
use roko_core::{ContentHash, Context, Kind, Query, Substrate};
use roko_fs::{FileSubstrate, RokoLayout};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing_subscriber::EnvFilter;

// -----------------------------------------------------------------------
// Exit codes
// -----------------------------------------------------------------------

/// Successful execution.
const EXIT_SUCCESS: i32 = 0;
const EXIT_FAILURE: i32 = 1;
/// Agent or gate failure (logical error in the build).
const EXIT_AGENT_FAILURE: i32 = 1;
/// System error (I/O, config, infrastructure).
const EXIT_SYSTEM_ERROR: i32 = 2;

// -----------------------------------------------------------------------
// Effort level
// -----------------------------------------------------------------------

/// Reasoning effort level for the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Effort {
    /// Minimal reasoning — fast, cheap.
    Low,
    /// Balanced reasoning (default).
    Medium,
    /// Thorough reasoning.
    High,
    /// Maximum reasoning — slowest, most expensive.
    Max,
}

impl std::fmt::Display for Effort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "low"),
            Self::Medium => write!(f, "medium"),
            Self::High => write!(f, "high"),
            Self::Max => write!(f, "max"),
        }
    }
}

// -----------------------------------------------------------------------
// CLI structure
// -----------------------------------------------------------------------

/// Minimal CLI for the Roko universal loop.
#[derive(Debug, Parser)]
#[command(
    name = "roko",
    version,
    about = "Minimal CLI for the Roko universal loop"
)]
struct Cli {
    /// Override the config file (default: `./roko.toml`).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Set the agent role / persona.
    #[arg(long, global = true)]
    role: Option<String>,

    /// Set the model name (passed to the agent backend).
    #[arg(long, global = true)]
    model: Option<String>,

    /// Set the repository / working directory root.
    #[arg(long, global = true)]
    repo: Option<PathBuf>,

    /// Resume a previous session by ID.
    #[arg(long, global = true)]
    resume: Option<String>,

    /// Set reasoning effort level.
    #[arg(long, global = true, value_enum)]
    effort: Option<Effort>,

    /// Emit JSON output instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Suppress non-essential output.
    #[arg(long, global = true)]
    quiet: bool,

    /// Run as a headless daemon (background service).
    #[arg(long, global = true)]
    headless: bool,

    /// One-shot mode: execute this prompt and exit.
    #[arg(global = false)]
    prompt: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create `.roko/` and a default `roko.toml` in `path` (default: cwd).
    Init {
        /// Directory to initialize (default: current dir).
        path: Option<PathBuf>,
    },
    /// Seed a prompt and run the universal loop (compose -> agent -> gate -> persist).
    Run {
        /// The user prompt text.
        prompt: String,
        /// Override the working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Print signal counts, most recent episode, and gate pass/fail.
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
    /// Inject a signal into a running session.
    Inject {
        /// Target session ID.
        session: String,
        /// Kind of signal to inject (directive, abort, context).
        #[arg(long, default_value = "directive")]
        kind: String,
        /// Payload text.
        payload: String,
        /// Working directory (to locate the daemon socket).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Manage plans (list, show, create).
    Plan {
        #[command(subcommand)]
        cmd: PlanCmd,
    },
    /// Manage product requirements documents (idea, draft, publish, plan).
    Prd {
        #[command(subcommand)]
        cmd: PrdCmd,
    },
}

#[derive(Debug, Subcommand)]
enum PlanCmd {
    /// List all plans in the workspace.
    List {
        /// Working directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show details of a specific plan.
    Show {
        /// Plan ID.
        plan_id: String,
        /// Working directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Create a new plan.
    Create {
        /// Plan ID.
        plan_id: String,
        /// Plan title.
        #[arg(long)]
        title: String,
        /// Plan description.
        #[arg(long, default_value = "")]
        description: String,
        /// Working directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Run a plan directory through the orchestration loop.
    Run {
        /// Path to the plans directory.
        plans_dir: PathBuf,
        /// Working directory (repo root). Defaults to current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Resume from a saved snapshot.
        #[arg(long)]
        resume: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum PrdCmd {
    /// Capture a quick idea.
    Idea {
        /// The idea text.
        text: Vec<String>,
    },
    /// List all PRDs (published, drafts, ideas).
    List,
    /// Show coverage report across PRDs and plans.
    Status,
    /// Create, edit, or promote draft PRDs.
    Draft {
        #[command(subcommand)]
        cmd: PrdDraftCmd,
    },
    /// Generate implementation plans from a PRD.
    Plan {
        /// PRD slug (filename without .md).
        slug: String,
    },
    /// Scan all PRDs for duplicates, gaps, and inconsistencies.
    Consolidate,
}

#[derive(Debug, Subcommand)]
enum PrdDraftCmd {
    /// Create a new draft PRD (agent-assisted).
    New {
        /// Title for the new PRD.
        title: Vec<String>,
    },
    /// Refine an existing draft.
    Edit {
        /// Draft slug (filename without .md).
        slug: String,
    },
    /// Promote a draft to published.
    Promote {
        /// Draft slug (filename without .md).
        slug: String,
    },
    /// List all drafts.
    List,
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
async fn main() {
    let cli = Cli::parse();
    init_tracing(&cli);
    let code = match dispatch(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            EXIT_SYSTEM_ERROR
        }
    };
    std::process::exit(code);
}

async fn dispatch(mut cli: Cli) -> Result<i32> {
    // If there is an explicit subcommand, handle it.
    if let Some(command) = cli.command.take() {
        return dispatch_subcommand(command, &cli).await;
    }

    // Headless daemon mode.
    if cli.headless {
        return Ok(cmd_headless(&cli));
    }

    // One-shot mode: positional prompt argument.
    if let Some(prompt) = &cli.prompt {
        return cmd_oneshot(&cli, prompt).await;
    }

    // Pipe mode: stdin is not a TTY and no prompt argument.
    if !roko_cli::stdin_is_tty() {
        return cmd_pipe(&cli).await;
    }

    // REPL mode: stdin is a TTY and no prompt argument.
    cmd_repl(&cli)
}

async fn dispatch_subcommand(command: Command, cli: &Cli) -> Result<i32> {
    match command {
        Command::Init { path } => {
            cmd_init(path).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Run { prompt, workdir } => cmd_run(cli, workdir, prompt).await,
        Command::Status { workdir } => {
            cmd_status(cli, workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Replay { hash, workdir } => cmd_replay(workdir, hash).await,
        Command::Config { cmd } => {
            dispatch_config(cmd)?;
            Ok(EXIT_SUCCESS)
        }
        Command::Inject {
            session,
            kind,
            payload,
            workdir,
        } => cmd_inject(cli, session, &kind, payload, workdir),
        Command::Plan { cmd } => cmd_plan(cli, cmd).await,
        Command::Prd { cmd } => cmd_prd(cli, cmd),
    }
}

// -----------------------------------------------------------------------
// Mode handlers
// -----------------------------------------------------------------------

fn cmd_repl(cli: &Cli) -> Result<i32> {
    let session_id = cli
        .resume
        .clone()
        .unwrap_or_else(|| format!("repl-{}", std::process::id()));
    let mut repl = ReplMode::new(session_id);

    let _commands = repl
        .run(&mut std::io::stdin().lock(), &mut std::io::stdout().lock())
        .map_err(|e| anyhow!("repl I/O error: {e}"))?;

    Ok(EXIT_SUCCESS)
}

async fn cmd_oneshot(cli: &Cli, prompt: &str) -> Result<i32> {
    let mode = OneshotMode::new(prompt.to_string())
        .with_json(cli.json)
        .with_quiet(cli.quiet);

    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    let mut config = resolve_config(cli)?;
    apply_resume_session_override(&mut config, cli.resume.clone());

    let report = run_once(&workdir, &config, &mode.prepare().prompt).await?;
    let result = mode.format_result(
        report.overall_success(),
        &format!(
            "episode={} signals={}",
            report.episode_id, report.total_signals
        ),
    );
    if !result.summary.is_empty() {
        println!("{}", result.summary);
    }
    Ok(result.exit_code)
}

async fn cmd_pipe(cli: &Cli) -> Result<i32> {
    let pipe = PipeMode::new().with_json(cli.json).with_quiet(cli.quiet);

    let input = pipe
        .read_input(&mut std::io::stdin().lock())
        .map_err(|e| anyhow!("read stdin: {e}"))?;

    if input.text.is_empty() {
        if !cli.quiet {
            eprintln!("no input received on stdin");
        }
        return Ok(EXIT_SYSTEM_ERROR);
    }

    if input.truncated && !cli.quiet {
        eprintln!(
            "warning: stdin input truncated at {} bytes",
            input.bytes_read
        );
    }

    // Dispatch the piped text as a one-shot prompt.
    cmd_oneshot(cli, &input.text).await
}

fn cmd_headless(cli: &Cli) -> i32 {
    let session_id = cli
        .resume
        .clone()
        .unwrap_or_else(|| format!("daemon-{}", std::process::id()));
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    let mut daemon = DaemonMode::with_workdir(&workdir, session_id);

    daemon.start();
    let status = daemon.status_summary();

    if cli.json {
        println!(
            r#"{{"session":"{}","state":"{}","pid":{},"socket":"{}"}}"#,
            status.session_id,
            status.state,
            status.pid,
            status.socket_path.display(),
        );
    } else if !cli.quiet {
        println!("{status}");
    }

    // In a real implementation this would block on a socket listener.
    // For now, report that the daemon started and exit cleanly.
    daemon.stop();
    daemon.mark_stopped();

    EXIT_SUCCESS
}

// -----------------------------------------------------------------------
// Subcommand handlers
// -----------------------------------------------------------------------

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
                model: model.clone(),
                role,
                enable_gates: if enable_gates { Some(true) } else { None },
                yes,
                ..Default::default()
            };
            if let (Some("ollama"), Some(m)) = (agent.as_deref(), model.as_ref()) {
                inputs.agent_args = Some(vec!["run".into(), m.clone()]);
            }
            if non_interactive {
                if inputs.agent_command.is_none() {
                    return Err(anyhow!("--non-interactive requires --agent"));
                }
                inputs.token_budget.get_or_insert(8000);
                inputs
                    .role
                    .get_or_insert_with(|| "You are a Roko agent.".into());
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
        ConfigCmd::Edit {
            global,
            project,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            let target = edit_target(global, project);
            config_cmd::cmd_edit(&wd, target)
        }
        ConfigCmd::Set {
            key,
            value,
            global: _,
            project,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
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

fn cmd_inject(
    cli: &Cli,
    session: String,
    kind_str: &str,
    payload: String,
    workdir: Option<PathBuf>,
) -> Result<i32> {
    let kind = InjectKind::parse(kind_str).map_err(|e| anyhow!("{e}"))?;
    let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let request = InjectRequest::new(session, kind, payload, wd);

    request.validate().map_err(|e| anyhow!("{e}"))?;

    if cli.json {
        println!(
            r#"{{"status":"queued","kind":"{}","session":"{}","bytes":{}}}"#,
            request.kind,
            request.session_id,
            request.payload.len(),
        );
    } else if !cli.quiet {
        println!("{}", request.summary());
    }

    Ok(EXIT_SUCCESS)
}

async fn cmd_plan(cli: &Cli, cmd: PlanCmd) -> Result<i32> {
    match cmd {
        PlanCmd::List { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let files =
                roko_cli::plan::list_plan_files(&wd).map_err(|e| anyhow!("list plans: {e}"))?;
            let summaries: Vec<PlanSummary> = files
                .iter()
                .map(|p| {
                    let id = p
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    PlanSummary {
                        id,
                        title: "(unloaded)".into(),
                        task_count: 0,
                        completed: false,
                    }
                })
                .collect();

            if cli.json {
                println!("{}", roko_cli::plan::format_plan_list_json(&summaries));
            } else {
                println!("{}", roko_cli::plan::format_plan_list(&summaries));
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Show { plan_id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let plans_dir = roko_cli::plan::plans_dir(&wd);
            let path = plans_dir.join(format!("{plan_id}.toml"));
            if path.exists() {
                println!("plan file: {}", path.display());
            } else {
                let alt = plans_dir.join(format!("{plan_id}.json"));
                if !alt.exists() {
                    eprintln!("plan '{plan_id}' not found");
                    return Ok(EXIT_AGENT_FAILURE);
                }
                println!("plan file: {}", alt.display());
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Create {
            plan_id,
            title,
            description,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let plan = Plan::new(plan_id.clone(), title, description);
            plan.validate()
                .map_err(|errs| anyhow!("plan validation failed: {}", errs.join("; ")))?;

            let plans_dir = roko_cli::plan::plans_dir(&wd);
            std::fs::create_dir_all(&plans_dir).map_err(|e| anyhow!("create plans dir: {e}"))?;
            let path = plans_dir.join(format!("{plan_id}.toml"));

            // Write a minimal plan skeleton.
            let content = format!(
                "# Plan: {}\n# {}\n\n[plan]\nid = \"{}\"\ntitle = \"{}\"\ntasks = []\n",
                plan.title, plan.description, plan.id, plan.title,
            );
            std::fs::write(&path, content).map_err(|e| anyhow!("write plan: {e}"))?;

            if cli.json {
                println!(r#"{{"created":"{}","path":"{}"}}"#, plan_id, path.display());
            } else if !cli.quiet {
                println!("created plan '{}' at {}", plan_id, path.display());
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Run {
            plans_dir,
            workdir,
            resume,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            prepare_runtime_hooks(&wd, cli.quiet);
            let config = load_layered(&wd)?.config;

            let mut runner = if let Some(snap_path) = resume {
                let exec_json = std::fs::read_to_string(&snap_path)
                    .map_err(|e| anyhow!("read snapshot: {e}"))?;
                // Try to load the event log from alongside the executor snapshot.
                let events_path = snap_path.with_file_name("events.json");
                if events_path.exists() {
                    let log_json = std::fs::read_to_string(&events_path)
                        .map_err(|e| anyhow!("read event log: {e}"))?;
                    roko_cli::PlanRunner::from_snapshots(&exec_json, &log_json, &wd, config)?
                } else {
                    roko_cli::PlanRunner::from_snapshot(&exec_json, &wd, config)?
                }
            } else {
                roko_cli::PlanRunner::from_plans_dir(&plans_dir, &wd, config)?
            };
            runner.set_claude_resume_session(cli.resume.clone());

            let report = runner.run_all().await?;

            // State is auto-saved during and after the run.
            let snap_path = wd.join(".roko").join("state").join("executor.json");

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "succeeded": report.all_succeeded(),
                        "total_agent_calls": report.total_agent_calls,
                        "total_gate_runs": report.total_gate_runs,
                        "plans": report.plans.iter().map(|p| serde_json::json!({
                            "plan_id": p.plan_id,
                            "succeeded": p.succeeded,
                            "agent_calls": p.agent_calls,
                        })).collect::<Vec<_>>(),
                    }))
                    .unwrap_or_default()
                );
            } else if !cli.quiet {
                println!("Orchestration complete:");
                for p in &report.plans {
                    let status = if p.succeeded { "✓" } else { "✗" };
                    println!(
                        "  {status} {} — {} agent calls, {} gate results",
                        p.plan_id,
                        p.agent_calls,
                        p.gate_results.len()
                    );
                }
                println!(
                    "\nTotal: {} agent calls, {} gate runs. Overall: {}",
                    report.total_agent_calls,
                    report.total_gate_runs,
                    if report.all_succeeded() {
                        "SUCCESS"
                    } else {
                        "FAILED"
                    }
                );
                println!("Snapshot saved to {}", snap_path.display());
            }

            Ok(if report.all_succeeded() {
                EXIT_SUCCESS
            } else {
                EXIT_FAILURE
            })
        }
    }
}

// -----------------------------------------------------------------------
// Existing subcommand handlers (init, run, status, replay)
// -----------------------------------------------------------------------

fn cmd_prd(_cli: &Cli, cmd: PrdCmd) -> Result<i32> {
    let workdir = std::env::current_dir().context("resolve cwd")?;

    match cmd {
        PrdCmd::Idea { text } => {
            let joined = text.join(" ");
            roko_cli::prd::cmd_idea(&workdir, &joined)?;
            Ok(0)
        }
        PrdCmd::List => {
            roko_cli::prd::cmd_list(&workdir)?;
            Ok(0)
        }
        PrdCmd::Status => {
            roko_cli::prd::cmd_status(&workdir, None)?;
            Ok(0)
        }
        PrdCmd::Draft { cmd: draft_cmd } => match draft_cmd {
            PrdDraftCmd::New { title } => {
                let title = title.join(" ");
                let slug = roko_cli::prd::slugify(&title);
                let drafts = workdir.join(".roko").join("prd").join("drafts");
                roko_cli::prd::ensure_dirs(&workdir)?;
                let target = drafts.join(format!("{slug}.md"));
                if target.exists() {
                    eprintln!("Draft already exists: {}", target.display());
                    eprintln!("Use: roko prd draft edit {slug}");
                    return Ok(1);
                }
                // Write the scaffold, then print agent prompt for the user
                let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, &title);
                let body = format!(
                    "{frontmatter}\
                     # {title}\n\n\
                     ## Overview\n\n\
                     <!-- Describe what this feature/system does and why -->\n\n\
                     ## Requirements\n\n\
                     <!-- REQ-XXX items -->\n\n\
                     ## Acceptance criteria\n\n\
                     <!-- Machine-verifiable checkboxes -->\n\n\
                     ## Design\n\n\
                     <!-- How it should work, reference existing crates -->\n\n\
                     ## References\n\n\
                     <!-- Links to relevant source files -->\n"
                );
                std::fs::write(&target, &body)?;
                println!("📄 Draft created: {}", target.display());
                println!();
                println!("Edit it manually or run an agent:");
                println!("  roko prd draft edit {slug}");
                let prompt = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Fill in the draft PRD at {path}. Read the codebase to understand what exists. \
                         Make requirements specific and acceptance criteria machine-verifiable.",
                        path = target.display()
                    ),
                );
                println!();
                println!("Or use this prompt with `roko run`:");
                println!("  roko run '{}'", prompt.lines().next().unwrap_or(""));
                Ok(0)
            }
            PrdDraftCmd::Edit { slug } => {
                let draft = workdir
                    .join(".roko")
                    .join("prd")
                    .join("drafts")
                    .join(format!("{slug}.md"));
                if !draft.exists() {
                    eprintln!("Draft not found: {}", draft.display());
                    return Ok(1);
                }
                let prompt = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Read and improve the draft PRD at {path}. \
                         Check: are requirements specific? Are acceptance criteria machine-verifiable? \
                         Search the codebase to verify claims. Update the file in place.",
                        path = draft.display()
                    ),
                );
                println!("Refining draft: {slug}");
                println!("Run: roko run '{}'", prompt.lines().next().unwrap_or(""));
                // TODO: once roko run can be called programmatically, do it here
                Ok(0)
            }
            PrdDraftCmd::Promote { slug } => {
                roko_cli::prd::cmd_promote(&workdir, &slug)?;
                Ok(0)
            }
            PrdDraftCmd::List => {
                let drafts = workdir.join(".roko").join("prd").join("drafts");
                roko_cli::prd::ensure_dirs(&workdir)?;
                let files = roko_cli::prd::list_md_files(&drafts);
                if files.is_empty() {
                    println!("No drafts. Create one: roko prd draft new \"title\"");
                } else {
                    for f in &files {
                        println!("  {}", f.file_stem().unwrap_or_default().to_string_lossy());
                    }
                }
                Ok(0)
            }
        },
        PrdCmd::Plan { slug } => {
            println!("Generating plans from PRD: {slug}");
            println!("Run: ./scripts/generate-plans.sh checklist \"{slug}\"");
            // TODO: call agent directly once run_once is refactored to return output
            Ok(0)
        }
        PrdCmd::Consolidate => {
            println!("Run: ./scripts/roko-prd.sh consolidate");
            // TODO: call agent directly
            Ok(0)
        }
    }
}

// Make list_md_files public so main.rs can use it for draft list
// (it's already pub in prd.rs)

async fn cmd_init(path: Option<PathBuf>) -> Result<()> {
    let target = path.unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let roko_dir = target.join(".roko");
    tokio::fs::create_dir_all(&roko_dir)
        .await
        .with_context(|| format!("create {}", roko_dir.display()))?;

    let signals_path = roko_dir.join("signals.jsonl");
    if !signals_path.exists() {
        tokio::fs::write(&signals_path, b"")
            .await
            .with_context(|| format!("create {}", signals_path.display()))?;
    }

    let config_path = target.join("roko.toml");
    if config_path.exists() {
        println!(
            "{} already exists; leaving untouched.",
            config_path.display()
        );
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

async fn cmd_run(cli: &Cli, workdir: Option<PathBuf>, prompt: String) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);
    let mut config = resolve_config_for_workdir(cli, &workdir)?;
    apply_resume_session_override(&mut config, cli.resume.clone());

    if !cli.quiet {
        println!(
            "running agent `{}` with {} gate(s)",
            config.agent.command,
            config.gates.len()
        );
    }
    let report = run_once(&workdir, &config, &prompt).await?;

    if cli.json {
        println!(
            r#"{{"success":{},"episode":"{}","prompt":"{}","agent_output":"{}","signals":{}}}"#,
            report.overall_success(),
            report.episode_id,
            report.prompt_id,
            report.agent_output_id,
            report.total_signals,
        );
    } else if !cli.quiet {
        println!("---");
        println!(
            "agent        : {} (success={})",
            config.agent.command, report.agent_success
        );
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
    }

    if report.overall_success() {
        Ok(EXIT_SUCCESS)
    } else {
        Ok(EXIT_AGENT_FAILURE)
    }
}

async fn cmd_status(cli: &Cli, workdir: Option<PathBuf>) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let ctx = Context::now();

    let all = substrate
        .query(&Query::all(), &ctx)
        .await
        .map_err(|e| anyhow!("query: {e}"))?;

    if cli.json {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for sig in &all {
            *counts.entry(sig.kind.to_string()).or_default() += 1;
        }
        let episode_count = counts.get("episode").copied().unwrap_or(0);
        let status = SessionStatus {
            session_id: cli.resume.clone(),
            workdir: workdir.clone(),
            daemon_running: false,
            signal_count: Some(all.len()),
            episode_count: Some(episode_count),
            last_episode_passed: None,
        };
        println!("{}", status.display_json());
        return Ok(());
    }

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

    let mut episodes = substrate
        .query(&Query::of_kind(Kind::Episode), &ctx)
        .await
        .map_err(|e| anyhow!("query episodes: {e}"))?;
    episodes.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
    println!();
    match episodes.first() {
        Some(ep) => {
            println!(
                "most recent episode: {} (passed={})",
                ep.id,
                ep.tag("passed").unwrap_or("?")
            );
            println!(
                "  gates passed={} failed={}",
                ep.tag("gates_passed").unwrap_or("0"),
                ep.tag("gates_failed").unwrap_or("0")
            );
        }
        None => println!("most recent episode: (none)"),
    }

    let verdicts = substrate
        .query(&Query::of_kind(Kind::GateVerdict), &ctx)
        .await
        .map_err(|e| anyhow!("query verdicts: {e}"))?;
    let passed = verdicts
        .iter()
        .filter(|v| v.tag("passed") == Some("true"))
        .count();
    let failed = verdicts
        .iter()
        .filter(|v| v.tag("passed") == Some("false"))
        .count();
    println!("gate verdicts: {passed} pass / {failed} fail");

    Ok(())
}

async fn cmd_replay(workdir: Option<PathBuf>, hash: String) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let start = ContentHash::from_hex(&hash)
        .ok_or_else(|| anyhow!("invalid hash (expected 64 hex chars): {hash}"))?;

    let mut visited = std::collections::HashSet::new();
    let mut queue = vec![(start, 0usize)];
    let mut printed = 0usize;
    while let Some((id, depth)) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        let indent = "  ".repeat(depth);
        if let Some(sig) = substrate.get(&id).await.map_err(|e| anyhow!("get: {e}"))? {
            println!(
                "{indent}{} {}  (author={})",
                sig.kind, sig.id, sig.provenance.author
            );
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
        return Ok(EXIT_AGENT_FAILURE);
    }
    Ok(EXIT_SUCCESS)
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Resolve the working directory from CLI flags.
fn resolve_workdir(cli: &Cli) -> PathBuf {
    cli.repo.clone().unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve the config, applying CLI overrides for --role, --model, --effort.
fn resolve_config(cli: &Cli) -> Result<Config> {
    let workdir = resolve_workdir(cli);
    resolve_config_for_workdir(cli, &workdir)
}

/// Resolve config from a specific workdir, applying CLI overrides.
fn resolve_config_for_workdir(cli: &Cli, workdir: &Path) -> Result<Config> {
    let mut config = if let Some(p) = &cli.config {
        Config::from_file(p)?
    } else {
        let resolved = load_layered(workdir)?;
        let fully_default = resolved.sources.agent_command == Source::Default
            && resolved.sources.prompt_token_budget == Source::Default;
        if fully_default && resolved.config.agent.command == "cat" && !cli.quiet {
            println!(
                "no config found — using built-in `cat` agent. run `roko config init` to set up a model."
            );
        }
        resolved.config
    };

    // Apply CLI overrides.
    if let Some(role) = &cli.role {
        config.prompt.role.clone_from(role);
    }
    if let Some(model) = &cli.model {
        config.agent.model = Some(model.clone());
        // Non-Claude CLIs often still expect the model as a positional arg.
        if config.agent.command != "claude" && !config.agent.args.contains(model) {
            config.agent.args.insert(0, model.clone());
        }
    }
    if let Some(effort) = &cli.effort {
        config.agent.effort = effort.to_string();
        // Claude handles effort natively; preserve the prompt budget only for
        // the older stdin/stdout backends.
        if config.agent.command != "claude" {
            let budget = match effort {
                Effort::Low => 4_000,
                Effort::Medium => 10_000,
                Effort::High => 32_000,
                Effort::Max => 100_000,
            };
            config.prompt.token_budget = budget;
        }
    }

    Ok(config)
}

fn apply_resume_session_override(config: &mut Config, resume: Option<String>) {
    let Some(session_id) = resume
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    if let Some(existing) = config
        .agent
        .env
        .iter_mut()
        .find(|(key, _)| key.eq_ignore_ascii_case("ROKO_SESSION_ID"))
    {
        existing.1 = session_id;
    } else {
        config
            .agent
            .env
            .push(("ROKO_SESSION_ID".to_string(), session_id));
    }
}

fn init_tracing(cli: &Cli) {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let env_filter = if std::env::var_os("RUST_LOG").is_some() {
            EnvFilter::from_default_env()
        } else if cli.quiet {
            EnvFilter::new("warn")
        } else {
            EnvFilter::new("info")
        };

        if cli.json {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_ansi(false)
                .json()
                .try_init();
        } else {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .try_init();
        }
    });
}

fn prepare_runtime_hooks(workdir: &Path, quiet: bool) {
    if let Err(err) = bootstrap_observability_dirs(workdir) {
        if !quiet {
            eprintln!("warning: observability bootstrap failed: {err}");
        }
    }
    run_process_lifecycle_hooks(workdir, quiet);
}

fn bootstrap_observability_dirs(workdir: &Path) -> std::io::Result<()> {
    let layout = RokoLayout::for_project(workdir);
    std::fs::create_dir_all(layout.root())?;
    for dir in layout.top_level_dirs() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::create_dir_all(layout.root().join("traces"))?;
    std::fs::create_dir_all(layout.root().join("metrics"))?;
    Ok(())
}

fn run_process_lifecycle_hooks(workdir: &Path, quiet: bool) {
    // The process registry currently keys off `std::env::current_dir()`.
    // Avoid cleaning the wrong workspace when `--repo` points elsewhere.
    if !process_registry_matches_workdir(workdir) {
        tracing::debug!(
            workdir = %workdir.display(),
            "skipping process lifecycle hooks; registry is cwd-scoped",
        );
        return;
    }
    cleanup_orphaned_agents();
    let reaped = reap_orphaned_children();
    if reaped > 0 && !quiet {
        eprintln!("reaped {reaped} orphaned agent process(es)");
    }
}

fn process_registry_matches_workdir(workdir: &Path) -> bool {
    let cwd = std::env::current_dir().ok();
    let target = if workdir.is_absolute() {
        Some(workdir.to_path_buf())
    } else {
        cwd.clone().map(|base| base.join(workdir))
    };

    let Some(cwd) = cwd else { return false };
    let Some(target) = target else { return false };

    let lhs = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from("."));
    let rhs = std::fs::canonicalize(&target).unwrap_or(target);
    lhs == rhs
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_no_args() {
        // With no args and no subcommand, cli.prompt and cli.command are None.
        let cli = Cli::try_parse_from(["roko"]).unwrap();
        assert!(cli.command.is_none());
        assert!(cli.prompt.is_none());
        assert!(!cli.json);
        assert!(!cli.quiet);
        assert!(!cli.headless);
    }

    #[test]
    fn cli_parses_global_flags() {
        let cli = Cli::try_parse_from([
            "roko",
            "--role",
            "engineer",
            "--model",
            "gpt-4",
            "--repo",
            "/tmp/proj",
            "--effort",
            "high",
            "--json",
            "--quiet",
            "--headless",
        ])
        .unwrap();
        assert_eq!(cli.role.as_deref(), Some("engineer"));
        assert_eq!(cli.model.as_deref(), Some("gpt-4"));
        assert_eq!(cli.repo, Some(PathBuf::from("/tmp/proj")));
        assert_eq!(cli.effort, Some(Effort::High));
        assert!(cli.json);
        assert!(cli.quiet);
        assert!(cli.headless);
    }

    #[test]
    fn cli_parses_positional_prompt() {
        let cli = Cli::try_parse_from(["roko", "fix the bug"]).unwrap();
        assert_eq!(cli.prompt.as_deref(), Some("fix the bug"));
        assert!(cli.command.is_none());
    }

    #[test]
    fn cli_parses_run_subcommand() {
        let cli = Cli::try_parse_from(["roko", "run", "do something"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Run { .. })));
    }

    #[test]
    fn cli_parses_init_subcommand() {
        let cli = Cli::try_parse_from(["roko", "init", "/tmp/project"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Init { .. })));
    }

    #[test]
    fn cli_parses_status_subcommand() {
        let cli = Cli::try_parse_from(["roko", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Status { .. })));
    }

    #[test]
    fn cli_parses_inject_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "inject",
            "session-1",
            "stop doing that",
            "--kind",
            "directive",
        ])
        .unwrap();
        assert!(matches!(cli.command, Some(Command::Inject { .. })));
    }

    #[test]
    fn cli_parses_plan_list() {
        let cli = Cli::try_parse_from(["roko", "plan", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Plan {
                cmd: PlanCmd::List { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_plan_create() {
        let cli = Cli::try_parse_from(["roko", "plan", "create", "my-plan", "--title", "My Plan"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Plan {
                cmd: PlanCmd::Create { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_resume_flag() {
        let cli = Cli::try_parse_from(["roko", "--resume", "sess-42"]).unwrap();
        assert_eq!(cli.resume.as_deref(), Some("sess-42"));
    }

    #[test]
    fn apply_resume_session_override_adds_env_var() {
        let mut config = Config::default();
        apply_resume_session_override(&mut config, Some("sess-42".to_string()));
        assert_eq!(
            config
                .agent
                .env
                .iter()
                .find(|(key, _)| key == "ROKO_SESSION_ID")
                .map(|(_, value)| value.as_str()),
            Some("sess-42")
        );
    }

    #[test]
    fn apply_resume_session_override_updates_existing_env_var() {
        let mut config = Config::default();
        config
            .agent
            .env
            .push(("ROKO_SESSION_ID".to_string(), "old".to_string()));
        apply_resume_session_override(&mut config, Some("  sess-99  ".to_string()));
        assert_eq!(
            config
                .agent
                .env
                .iter()
                .find(|(key, _)| key == "ROKO_SESSION_ID")
                .map(|(_, value)| value.as_str()),
            Some("sess-99")
        );
    }

    #[test]
    fn effort_display() {
        assert_eq!(Effort::Low.to_string(), "low");
        assert_eq!(Effort::Medium.to_string(), "medium");
        assert_eq!(Effort::High.to_string(), "high");
        assert_eq!(Effort::Max.to_string(), "max");
    }

    #[test]
    fn effort_value_enum_all_variants() {
        // Ensure all four variants parse.
        for name in &["low", "medium", "high", "max"] {
            let cli = Cli::try_parse_from(["roko", "--effort", name]).unwrap();
            assert!(cli.effort.is_some());
        }
    }

    #[test]
    fn exit_code_constants() {
        assert_eq!(EXIT_SUCCESS, 0);
        assert_eq!(EXIT_AGENT_FAILURE, 1);
        assert_eq!(EXIT_SYSTEM_ERROR, 2);
    }

    #[test]
    fn resolve_workdir_uses_repo_flag() {
        let cli = Cli::try_parse_from(["roko", "--repo", "/custom"]).unwrap();
        assert_eq!(resolve_workdir(&cli), PathBuf::from("/custom"));
    }

    #[test]
    fn resolve_workdir_defaults_to_cwd() {
        let cli = Cli::try_parse_from(["roko"]).unwrap();
        assert_eq!(resolve_workdir(&cli), PathBuf::from("."));
    }

    #[test]
    fn cli_parses_config_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "show"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Show { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_replay_subcommand() {
        let cli = Cli::try_parse_from(["roko", "replay", "abcd1234"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Replay { .. })));
    }

    #[test]
    fn bootstrap_observability_dirs_creates_expected_paths() {
        let tmp = tempfile::tempdir().unwrap();
        bootstrap_observability_dirs(tmp.path()).unwrap();
        let roko = tmp.path().join(".roko");
        assert!(roko.join("traces").is_dir());
        assert!(roko.join("metrics").is_dir());
        assert!(roko.join("runtime").is_dir());
        assert!(roko.join("runs").is_dir());
    }

    #[test]
    fn process_registry_matches_workdir_for_current_dir() {
        assert!(process_registry_matches_workdir(Path::new(".")));
    }

    #[test]
    fn process_registry_does_not_match_unrelated_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!process_registry_matches_workdir(tmp.path()));
    }
}

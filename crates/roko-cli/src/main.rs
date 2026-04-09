//! `roko` binary entrypoint.
//!
//! See [`roko_cli`] for the lib-side description. The binary exposes
//! subcommands (`init`, `run`, `status`, `replay`, `config`, `inject`,
//! `plan`) plus top-level flags for mode selection (`--headless`,
//! `--role`, `--model`, `--effort`, `--json`, `--log-format`, `--quiet`,
//! `--resume`, `--repo`, `--no-replan`, and a positional `[prompt]` for
//! one-shot mode).

#![allow(clippy::too_many_lines)]

use anyhow::{Context as _, Result, anyhow};
use clap::{Parser, Subcommand, ValueEnum};
use roko_agent::process::{cleanup_orphaned_agents, reap_orphaned_children};
use roko_cli::tui::App;
use roko_cli::{
    Config, DaemonMode, DashboardScaffold, EditTarget, InjectKind, InjectRequest, OneshotMode,
    PageId, PipeMode, Plan, PlanSummary, ReplMode, SessionStatus, Source, WizardInputs, config_cmd,
    load_layered, run_init_wizard, run_once,
};
use roko_core::{ContentHash, Context, Kind, Query, Substrate};
use roko_core::{Headlines, TaskMetric, compute_headlines};
use roko_fs::{FileSubstrate, FsObservabilitySinks, RokoLayout};
use roko_learn::efficiency::compute_role_profiles;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::prompt_experiment::ExperimentStore;
use roko_learn::runtime_feedback::read_efficiency_events;
use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

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

/// Log output format for tracing subscriber initialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    /// Human-readable text logs.
    Text,
    /// Structured JSON logs.
    Json,
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

    /// Set the tracing log format.
    #[arg(long, global = true, value_enum, default_value_t = LogFormat::Text)]
    log_format: LogFormat,

    /// Suppress non-essential output.
    #[arg(long, global = true)]
    quiet: bool,

    /// Disable all re-planning; gate failures become terminal failures.
    #[arg(long, global = true)]
    no_replan: bool,

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
    /// Manage global and project config (wizard, show, path, edit, set, set-secret).
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
    /// Research topics, enhance documents, analyze execution data.
    Research {
        #[command(subcommand)]
        cmd: ResearchCmd,
    },
    /// Launch the dashboard TUI, with text fallback for non-interactive use.
    Dashboard {
        /// Specific dashboard page slug to render.
        #[arg(long)]
        page: Option<String>,
        /// List all available page slugs.
        #[arg(long)]
        list_pages: bool,
        /// Force text-mode output instead of the interactive terminal UI.
        #[arg(long)]
        text: bool,
        /// Override the working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Start the HTTP API server.
    Serve {
        /// Address to bind to (default: 127.0.0.1).
        #[arg(long)]
        bind: Option<String>,
        /// Port number (default: 9090).
        #[arg(long)]
        port: Option<u16>,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Run as a deployed worker (reads template from env, serves tasks).
    Worker {
        /// Port to listen on (default: 8080, overridden by PORT env).
        #[arg(long, default_value_t = 8080)]
        port: u16,
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
        /// Resume from `.roko/state/executor.json` in the working directory.
        #[arg(long, num_args = 0..=1, default_missing_value = ".roko/state/executor.json")]
        resume: Option<PathBuf>,
    },
    /// Generate implementation plans from a prompt, file, or PRD.
    Generate {
        /// Source: free-text prompt, or path to a file (PRD, requirements, etc).
        source: Vec<String>,
        /// Treat source as a file path to read (instead of inline text).
        #[arg(long)]
        from_file: Option<PathBuf>,
    },
    /// Regenerate an existing plan, filling in missing metadata (tier, verify, context).
    Regenerate {
        /// Path to the plan directory (containing tasks.toml).
        plan_dir: PathBuf,
        /// Preview changes without overwriting.
        #[arg(long)]
        dry_run: bool,
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
enum ResearchCmd {
    /// Deep-dive research on a topic. Produces .roko/research/<slug>.md with citations.
    Topic {
        /// The research topic.
        topic: Vec<String>,
    },
    /// Enhance a PRD with academic citations, diagrams, and research-backed improvements.
    EnhancePrd {
        /// PRD slug (filename without .md).
        slug: String,
    },
    /// Optimize an implementation plan with research-backed task decomposition techniques.
    EnhancePlan {
        /// Plan directory name under plans/.
        plan: String,
    },
    /// Optimize tasks for efficiency, parallelism, and cheapest viable model.
    EnhanceTasks {
        /// Plan directory name under plans/.
        plan: String,
    },
    /// Analyze execution episodes for self-learning insights and bandit weight recommendations.
    Analyze,
    /// List all research artifacts.
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
    /// Store a secret in `~/.roko/.env` as `NAME=VALUE`.
    SetSecret {
        /// Secret name.
        name: String,
        /// Secret value.
        value: String,
    },
    /// Check `${VAR}` references in config and validate referenced secrets.
    CheckSecrets {
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

fn main() {
    let startup_env_redactions = match load_startup_env_files() {
        Ok(values) => values,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(EXIT_SYSTEM_ERROR);
        }
    };

    let cli = Cli::parse();
    let filter = tracing_subscriber::EnvFilter::try_new(
        env::var("ROKO_LOG").unwrap_or_else(|_| "info".to_string()),
    )
    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let scrubber = build_log_scrubber(&startup_env_redactions);
    match cli.log_format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .event_format(RedactingFormat::new(
                    tracing_subscriber::fmt::format().json(),
                    scrubber,
                ))
                .with_env_filter(filter)
                .init();
        }
        LogFormat::Text => {
            tracing_subscriber::fmt()
                .event_format(RedactingFormat::new(
                    tracing_subscriber::fmt::format(),
                    scrubber,
                ))
                .with_env_filter(filter)
                .init();
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");

    let code = match runtime.block_on(dispatch(cli)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            EXIT_SYSTEM_ERROR
        }
    };
    std::process::exit(code);
}

#[derive(Debug)]
struct RedactingFormat<E> {
    inner: E,
    scrubber: roko_core::obs::LogScrubber,
}

impl<E> RedactingFormat<E> {
    fn new(inner: E, scrubber: roko_core::obs::LogScrubber) -> Self {
        Self {
            inner,
            scrubber,
        }
    }
}

fn build_log_scrubber(env_redactions: &[(String, String)]) -> roko_core::obs::LogScrubber {
    let scrubber = roko_core::obs::LogScrubber::new();
    for (name, value) in env_redactions {
        let _ = scrubber.add_literal_value(value, name);
    }
    scrubber
}

impl<S, N, E> FormatEvent<S, N> for RedactingFormat<E>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'writer> FormatFields<'writer> + 'static,
    E: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let mut buffer = String::new();
        let buffer_writer = tracing_subscriber::fmt::format::Writer::new(&mut buffer);
        self.inner.format_event(ctx, buffer_writer, event)?;
        let scrubbed = self.scrubber.scrub(&buffer);
        writer.write_str(&scrubbed)
    }
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
        Command::Plan { cmd } => {
            let result = cmd_plan(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Prd { cmd } => {
            let result = cmd_prd(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Research { cmd } => {
            let result = cmd_research(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Dashboard {
            page,
            list_pages,
            text,
            workdir,
        } => cmd_dashboard(cli, workdir, page, list_pages, text).await,
        Command::Serve {
            bind,
            port,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::serve::run_server(wd, bind, port).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Worker { port } => {
            roko_cli::worker::run_worker(port).await?;
            Ok(EXIT_SUCCESS)
        }
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

async fn cmd_dashboard(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
    text: bool,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);

    let initial_page = page.as_deref().map(|page| {
        parse_dashboard_page(page).ok_or_else(|| {
            anyhow!(
                "unknown dashboard page `{page}`; available pages: {}",
                dashboard_page_slugs().join(", ")
            )
        })
    });
    let initial_page = initial_page.transpose()?;

    if !text && !list_pages && std::io::stdout().is_terminal() {
        if App::new_with_page(&workdir, initial_page).run().is_ok() {
            return Ok(EXIT_SUCCESS);
        }
    }

    let output = render_dashboard_text(cli, Some(workdir), page, list_pages).await?;
    print!("{output}");
    Ok(EXIT_SUCCESS)
}

async fn render_dashboard_text(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
) -> Result<String> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);

    let dashboard = DashboardScaffold::new_in(&workdir);
    if list_pages {
        let mut out = String::new();
        for page in dashboard.pages() {
            let _ = writeln!(out, "{:<16} {}", page.id.slug(), page.title);
        }
        return Ok(out);
    }

    let snapshot = DashboardSnapshot::load(&workdir).await?;

    if let Some(page) = page {
        let Some(page_id) = parse_dashboard_page(&page) else {
            anyhow::bail!(
                "unknown dashboard page `{page}`; available pages: {}",
                dashboard_page_slugs().join(", ")
            );
        };
        Ok(match page_id {
            PageId::Health | PageId::Trends => snapshot
                .render_page_text(page_id)
                .unwrap_or_else(|| dashboard.render_active_page_text()),
            _ => {
                let mut dashboard = dashboard;
                let _ = dashboard.set_active_page(page_id);
                dashboard.render_active_page_text()
            }
        })
    } else {
        Ok(dashboard.render_overview_text())
    }
}

#[cfg(test)]
async fn dashboard_output(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
) -> Result<String> {
    render_dashboard_text(cli, workdir, page, list_pages).await
}

#[derive(Debug, Clone, PartialEq)]
struct DashboardSnapshot {
    episodes: Vec<Episode>,
    task_metrics: Vec<TaskMetric>,
    headlines: Headlines,
}

impl DashboardSnapshot {
    async fn load(workdir: &Path) -> Result<Self> {
        let layout = RokoLayout::for_project(workdir);
        let episodes = EpisodeLogger::read_all_lossy(layout.episodes_path()).await?;
        let task_metrics = load_task_metrics(layout.memory_dir().join("task-metrics.jsonl")).await;
        let headlines = compute_headlines(&task_metrics);
        Ok(Self {
            episodes,
            task_metrics,
            headlines,
        })
    }

    fn render_page_text(&self, page: PageId) -> Option<String> {
        match page {
            PageId::Health => Some(self.render_health_page_text()),
            PageId::Trends => Some(self.render_trends_page_text()),
            _ => None,
        }
    }

    fn render_health_page_text(&self) -> String {
        let summary = self.health_summary();
        render_data_page(
            "Health",
            PageId::Health.slug(),
            "Top-line health gauges derived from the latest snapshot.",
            &[
                format!(
                    "focus: {} episodes, {} success, {} avg cost",
                    summary.episode_count,
                    format_percent(summary.success_rate),
                    format_currency(summary.avg_cost_per_episode)
                ),
                format!("episodes: {}", summary.episode_count),
                format!("success rate: {}", format_percent(summary.success_rate)),
                format!(
                    "avg cost / episode: {}",
                    format_currency(summary.avg_cost_per_episode)
                ),
                format!(
                    "avg wall time: {}",
                    format_duration(summary.avg_wall_time_ms)
                ),
                format!("cache hit rate: {}", format_percent(summary.cache_hit_rate)),
                format!("haiku share: {}", format_percent(summary.haiku_share)),
            ],
        )
    }

    fn render_trends_page_text(&self) -> String {
        let summary = self.health_summary();
        let headlines = self.headlines;
        render_data_page(
            "Trends",
            PageId::Trends.slug(),
            "Time-series learning signals from the current snapshot.",
            &[
                format!(
                    "focus: {} records across {} plans, {} pass rate",
                    headlines.n_records,
                    headlines.n_plans,
                    format_percent(headlines.first_attempt_pass_rate)
                ),
                format!(
                    "first-attempt pass rate: {}",
                    format_percent(headlines.first_attempt_pass_rate)
                ),
                format!(
                    "avg iterations per plan: {:.2}",
                    headlines.avg_iterations_per_plan
                ),
                format!(
                    "avg cost per plan: {}",
                    format_currency(headlines.avg_cost_per_plan)
                ),
                format!(
                    "avg input tokens per spawn: {:.0}",
                    headlines.avg_input_tokens_per_spawn
                ),
                format!("haiku share: {}", format_percent(summary.haiku_share)),
                format!("cache hit rate: {}", format_percent(summary.cache_hit_rate)),
            ],
        )
    }

    #[allow(clippy::cast_precision_loss)]
    fn health_summary(&self) -> DashboardHealthSummary {
        let episode_count = self.episodes.len();
        let success_count = self
            .episodes
            .iter()
            .filter(|episode| episode.success)
            .count();
        let total_cost = self
            .episodes
            .iter()
            .map(|episode| episode.usage.cost_usd)
            .sum::<f64>();
        let total_wall_ms = self
            .episodes
            .iter()
            .map(|episode| episode.usage.wall_ms)
            .sum::<u64>();
        let total_input_tokens = self
            .episodes
            .iter()
            .map(|episode| episode.usage.input_tokens)
            .sum::<u64>();
        let total_cache_read_tokens = self
            .episodes
            .iter()
            .map(|episode| episode.usage.cache_read_tokens)
            .sum::<u64>();
        let avg_cost_per_episode = if episode_count == 0 {
            0.0
        } else {
            total_cost / episode_count as f64
        };
        let avg_wall_time_ms = if episode_count == 0 {
            0.0
        } else {
            total_wall_ms as f64 / episode_count as f64
        };
        let cache_hit_rate = if total_input_tokens == 0 {
            0.0
        } else {
            total_cache_read_tokens as f64 / total_input_tokens as f64
        };
        let haiku_share = if self.task_metrics.is_empty() {
            0.0
        } else {
            self.task_metrics
                .iter()
                .filter(|metric| metric.model.to_ascii_lowercase().contains("haiku"))
                .count() as f64
                / self.task_metrics.len() as f64
        };

        DashboardHealthSummary {
            episode_count,
            success_rate: if episode_count == 0 {
                0.0
            } else {
                success_count as f64 / episode_count as f64
            },
            avg_cost_per_episode,
            avg_wall_time_ms,
            cache_hit_rate,
            haiku_share,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DashboardHealthSummary {
    episode_count: usize,
    success_rate: f64,
    avg_cost_per_episode: f64,
    avg_wall_time_ms: f64,
    cache_hit_rate: f64,
    haiku_share: f64,
}

fn render_data_page(title: &str, slug: &str, intent: &str, lines: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title} ({slug})");
    let _ = writeln!(out, "group: efficiency");
    let _ = writeln!(out, "intent: {intent}");
    let _ = writeln!(
        out,
        "focus: {}",
        lines.first().map_or("n/a", String::as_str)
    );
    let _ = writeln!(out, "widgets ({}):", lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let _ = writeln!(out, "- {}: {}", idx + 1, line);
    }
    out
}

fn format_percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn format_currency(value: f64) -> String {
    format!("${value:.4}")
}

fn format_duration(ms: f64) -> String {
    if ms >= 1000.0 {
        let seconds = ms / 1000.0;
        format!("{seconds:.2}s")
    } else {
        format!("{ms:.0}ms")
    }
}

async fn load_task_metrics(path: PathBuf) -> Vec<TaskMetric> {
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Vec::new();
    };

    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(metric) = TaskMetric::from_jsonl(line) {
            records.push(metric);
        }
    }
    records
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
        ConfigCmd::SetSecret { name, value } => config_cmd::cmd_set_secret(&name, &value),
        ConfigCmd::CheckSecrets { workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            config_cmd::cmd_check_secrets(&wd)
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

            // Create the shared metric registry and register standard metrics.
            let metrics = std::sync::Arc::new(roko_core::obs::MetricRegistry::new());
            roko_core::obs::register_standard_metrics(&metrics);

            let mut runner = if let Some(snap_path) = resume {
                let snap_path = if snap_path.is_relative() {
                    wd.join(snap_path)
                } else {
                    snap_path
                };
                let state_dir = wd.join(".roko").join("state");
                let exec_json = std::fs::read_to_string(&snap_path)
                    .map_err(|e| anyhow!("read snapshot {}: {e}", snap_path.display()))?;
                // Try to load the event log from alongside the executor snapshot.
                let events_path = state_dir.join("events.json");
                if events_path.exists() {
                    let log_json = std::fs::read_to_string(&events_path)
                        .map_err(|e| anyhow!("read event log {}: {e}", events_path.display()))?;
                    roko_cli::PlanRunner::from_snapshots(
                        &exec_json,
                        &log_json,
                        &wd,
                        config,
                        metrics,
                        cli.no_replan,
                    )
                    .await?
                } else {
                    roko_cli::PlanRunner::from_snapshot(
                        &exec_json,
                        &wd,
                        config,
                        metrics,
                        cli.no_replan,
                    )
                    .await?
                }
            } else {
                roko_cli::PlanRunner::from_plans_dir(
                    &plans_dir,
                    &wd,
                    config,
                    metrics,
                    cli.no_replan,
                )
                .await?
            };
            runner.set_claude_resume_session(cli.resume.clone());

            // Use task-driven execution (reads tasks.toml directly) instead of
            // the phase-machine executor which expects enrichment phases.
            let report = runner.run_task_plans(&plans_dir).await?;

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
        PlanCmd::Generate { source, from_file } => {
            use roko_cli::agent_exec::{
                AgentExecOpts, load_gateway_env, model_from_config, run_agent,
            };

            let workdir = std::env::current_dir().context("resolve cwd")?;
            let gw = load_gateway_env(&workdir);
            let model = model_from_config(&workdir);
            let model_ref = model.as_deref();

            // Get the source content: either from a file or inline text
            let source_text = if let Some(ref path) = from_file {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("read {}", path.display()))?;
                eprintln!("📋 Generating plans from file: {}", path.display());
                content
            } else {
                let text = source.join(" ");
                if text.is_empty() {
                    anyhow::bail!("Provide a prompt or --from-file <path>");
                }
                eprintln!("📋 Generating plans from prompt: {text}");
                text
            };

            let source_type = if from_file.is_some() {
                "file"
            } else {
                "prompt"
            };
            let system = roko_cli::plan_generate::build_generation_prompt(
                &workdir,
                &source_text,
                source_type,
            );

            let task_prompt = format!(
                "Read the source below and generate implementation plan directories under plans/. \
                 Search the codebase first to understand what exists. \
                 Create plan.md and tasks.toml files with tier, model_hint, context (read_files with line ranges), \
                 mcp_servers (per-task MCP server names), and verify steps (executable shell commands). \
                 Use the cheapest model tier for each task.\n\n{source_text}"
            );

            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: Some("high"),
                system_prompt: Some(&system),
                resume_session: None,
                env_vars: &gw.vars,
            })
            .await
        }
        PlanCmd::Regenerate { plan_dir, dry_run } => {
            use roko_cli::agent_exec::{
                AgentExecOpts, load_gateway_env, model_from_config, run_agent,
            };

            let workdir = std::env::current_dir().context("resolve cwd")?;
            let tasks_path = plan_dir.join("tasks.toml");
            if !tasks_path.exists() {
                anyhow::bail!("No tasks.toml found in {}", plan_dir.display());
            }

            let existing = std::fs::read_to_string(&tasks_path)
                .with_context(|| format!("read {}", tasks_path.display()))?;

            // Validate the existing plan and show issues.
            if let Ok(tf) = roko_cli::task_parser::TasksFile::parse(&tasks_path) {
                let issues = tf.validate();
                if issues.is_empty() {
                    eprintln!("Plan already has full metadata. Nothing to regenerate.");
                    return Ok(EXIT_SUCCESS);
                }
                eprintln!("Validation issues ({}):", issues.len());
                for issue in &issues {
                    eprintln!("  - {issue}");
                }
            }

            if dry_run {
                eprintln!("\n[dry-run] Would regenerate {}", tasks_path.display());
                let system =
                    roko_cli::plan_generate::build_regeneration_prompt(&workdir, &existing);
                eprintln!("Prompt length: {} chars", system.len());
                return Ok(EXIT_SUCCESS);
            }

            let gw = load_gateway_env(&workdir);
            let model = model_from_config(&workdir);
            let model_ref = model.as_deref();

            let system = roko_cli::plan_generate::build_regeneration_prompt(&workdir, &existing);
            let task_prompt = format!(
                "Regenerate the tasks.toml at {} with full metadata. \
                Read the codebase to fill in description, tier, model_hint, max_loc, \
                context (read_files with line ranges, symbols, anti_patterns), \
                and mcp_servers (per-task MCP server names) \
                and verify steps for each task. \
                Write the updated tasks.toml back to the same file.",
                tasks_path.display()
            );

            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: Some("high"),
                system_prompt: Some(&system),
                resume_session: None,
                env_vars: &gw.vars,
            })
            .await
        }
    }
}

// -----------------------------------------------------------------------
// Existing subcommand handlers (init, run, status, replay)
// -----------------------------------------------------------------------

async fn cmd_research(cli: &Cli, cmd: ResearchCmd) -> Result<i32> {
    use roko_cli::agent_exec::{AgentExecOpts, load_gateway_env, model_from_config, run_agent};
    use roko_cli::research::{ResearchMode, build_research_prompt};

    let workdir = resolve_workdir(cli);
    roko_cli::research::ensure_dirs(&workdir)?;
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();

    match cmd {
        ResearchCmd::Topic { topic } => {
            let topic = topic.join(" ");
            println!("🔬 Researching: {topic}");
            let task_prompt = format!(
                "Research the topic: \"{topic}\". \
                 Save your findings to .roko/research/{slug}.md with full citations. \
                 Read existing docs in .roko/prd/ and .roko/research/ for context on the project.",
                slug = topic.to_lowercase().replace(' ', "-")
            );
            let system = build_research_prompt(&workdir, &topic, "", ResearchMode::Topic);
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
        }
        ResearchCmd::EnhancePrd { slug } => {
            let prd_path = find_prd(&workdir, &slug)?;
            let content = std::fs::read_to_string(&prd_path)
                .with_context(|| format!("read {}", prd_path.display()))?;
            println!("🔬 Enhancing PRD: {slug}");
            let task_prompt = format!(
                "Read the PRD at {path} and enhance it: \
                 (1) Add academic citations [AUTHOR-YEAR] for every design decision. \
                 (2) Add mermaid diagrams with color styling where architecture would be clearer. \
                 (3) Identify improvements from recent research. \
                 (4) Flag claims that contradict recent findings. \
                 Update the file in place. Also save a research summary to .roko/research/enhance-{slug}.md",
                path = prd_path.display()
            );
            let system = build_research_prompt(&workdir, &slug, &content, ResearchMode::EnhancePrd);
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
        }
        ResearchCmd::EnhancePlan { plan } => {
            let plan_dir = workdir.join("plans").join(&plan);
            if !plan_dir.is_dir() {
                anyhow::bail!("Plan directory not found: {}", plan_dir.display());
            }
            println!("🔬 Enhancing plan: {plan}");
            let task_prompt = format!(
                "Read the plan at plans/{plan}/plan.md and plans/{plan}/tasks.toml. \
                 Optimize them using research-backed techniques: \
                 (1) Better task decomposition (cite SWE-bench, Agentless). \
                 (2) More precise context injection per task (exact file:line ranges). \
                 (3) Stronger verification (executable commands, not descriptions). \
                 (4) Cost optimization (assign cheapest model per task tier). \
                 Update the files in place."
            );
            let mut context = String::new();
            for name in ["plan.md", "tasks.toml"] {
                let p = plan_dir.join(name);
                if p.exists() {
                    let c = std::fs::read_to_string(&p).unwrap_or_default();
                    let _ = write!(context, "### {name}\n```\n{c}\n```\n\n");
                }
            }
            let system =
                build_research_prompt(&workdir, &plan, &context, ResearchMode::EnhancePlan);
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
        }
        ResearchCmd::EnhanceTasks { plan } => {
            let tasks_path = workdir.join("plans").join(&plan).join("tasks.toml");
            if !tasks_path.exists() {
                anyhow::bail!("tasks.toml not found: {}", tasks_path.display());
            }
            println!("🔬 Optimizing tasks: {plan}");
            let content = std::fs::read_to_string(&tasks_path)?;
            let task_prompt = format!(
                "Read plans/{plan}/tasks.toml and optimize every task: \
                 (1) Split any task >50 LOC into smaller subtasks. \
                 (2) Add context.read_files with exact line ranges for each task. \
                 (3) Ensure every acceptance criterion is a runnable shell command. \
                 (4) Remove unnecessary dependency edges to increase parallelism. \
                 (5) Assign tier (mechanical/focused/integrative/architectural) and model_hint. \
                 Search the codebase to verify file paths exist. Update tasks.toml in place."
            );
            let system =
                build_research_prompt(&workdir, &plan, &content, ResearchMode::EnhanceTasks);
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
        }
        ResearchCmd::Analyze => {
            let episodes_path = workdir.join(".roko/memory/episodes.jsonl");
            let context = if episodes_path.exists() {
                std::fs::read_to_string(&episodes_path).unwrap_or_default()
            } else {
                String::from("(no episodes yet — run some tasks first)")
            };
            println!("🔬 Analyzing execution data");
            let task_prompt = "Read .roko/memory/episodes.jsonl and analyze: \
                 (1) First-attempt pass rate by task tier and model. \
                 (2) Cost per task — are expensive models used for easy tasks? \
                 (3) Retry patterns — what kinds of tasks fail most? \
                 (4) Recommendations: which bandit weights to adjust. \
                 Save analysis to .roko/research/execution-analysis.md"
                .to_string();
            let system = build_research_prompt(
                &workdir,
                "execution-analysis",
                &context,
                ResearchMode::AnalyzeExecution,
            );
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
        }
        ResearchCmd::List => {
            let files = roko_cli::research::list_research(&workdir)?;
            if files.is_empty() {
                println!("No research artifacts. Run: roko research topic \"your topic\"");
            } else {
                println!("═══ Research Artifacts ═══");
                for f in &files {
                    let name = f.file_stem().unwrap_or_default().to_string_lossy();
                    let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                    println!("  {name:<45} {size:>6} bytes");
                }
            }
            Ok(0)
        }
    }
}

/// Find a PRD by slug in either published or drafts.
fn find_prd(workdir: &Path, slug: &str) -> Result<PathBuf> {
    let published = workdir
        .join(".roko/prd/published")
        .join(format!("{slug}.md"));
    if published.exists() {
        return Ok(published);
    }
    let draft = workdir.join(".roko/prd/drafts").join(format!("{slug}.md"));
    if draft.exists() {
        return Ok(draft);
    }
    anyhow::bail!("PRD not found: {slug} (checked published/ and drafts/)");
}

async fn cmd_prd(cli: &Cli, cmd: PrdCmd) -> Result<i32> {
    use roko_cli::agent_exec::{AgentExecOpts, load_gateway_env, model_from_config, run_agent};

    let workdir = resolve_workdir(cli);
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();

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
                // Write scaffold first so agent can read and fill it
                let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, &title);
                let scaffold = format!(
                    "{frontmatter}# {title}\n\n\
                     ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
                     ## Design\n\n## References\n"
                );
                std::fs::write(&target, &scaffold)?;
                println!("📄 Creating PRD: {title}");

                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Fill in the draft PRD at {path}. \
                         Read the codebase to understand what exists. \
                         Follow the PRD quality standards in your system prompt exactly.",
                        path = target.display()
                    ),
                );
                let task_prompt = format!(
                    "Read the scaffold at {path} and fill it in completely. \
                     Search the codebase with grep to understand what exists. \
                     Include 10+ academic citations, 2+ mermaid diagrams, and \
                     machine-verifiable acceptance criteria. \
                     Write the complete PRD to {path}.",
                    path = target.display()
                );
                run_agent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                })
                .await
            }
            PrdDraftCmd::Edit { slug } => {
                let draft = workdir.join(".roko/prd/drafts").join(format!("{slug}.md"));
                if !draft.exists() {
                    eprintln!("Draft not found: {}", draft.display());
                    return Ok(1);
                }
                println!("📝 Refining draft: {slug}");
                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Read and improve the draft PRD at {path}. \
                         Follow the PRD quality standards in your system prompt.",
                        path = draft.display()
                    ),
                );
                let task_prompt = format!(
                    "Read {path} and improve it: \
                     (1) Are requirements specific and testable? \
                     (2) Are acceptance criteria machine-verifiable shell commands? \
                     (3) Are there 10+ citations with [AUTHOR-YEAR] format? \
                     (4) Are there 2+ mermaid diagrams with color styling? \
                     Search the codebase to verify claims. Update the file in place.",
                    path = draft.display()
                );
                run_agent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                })
                .await
            }
            PrdDraftCmd::Promote { slug } => {
                roko_cli::prd::cmd_promote(&workdir, &slug).await?;
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
            let prd_path = find_prd(&workdir, &slug)?;
            let _generated_plans_root =
                roko_cli::prd::generate_plan_from_prd(&slug, &prd_path).await?;
            Ok(0)
        }
        PrdCmd::Consolidate => {
            println!("🔄 Scanning all PRDs for duplicates, gaps, and inconsistencies...");
            let mut all_context = String::new();
            for dir_name in ["published", "drafts"] {
                let dir = workdir.join(".roko/prd").join(dir_name);
                for path in roko_cli::prd::list_md_files(&dir) {
                    if let Ok(c) = std::fs::read_to_string(&path) {
                        let truncated: String = c.lines().take(50).collect::<Vec<_>>().join("\n");
                        let _ = write!(all_context, "### {}\n{truncated}\n---\n\n", path.display());
                    }
                }
            }
            let ideas =
                std::fs::read_to_string(workdir.join(".roko/prd/ideas.md")).unwrap_or_default();
            let task_prompt = format!(
                "Review ALL existing PRDs and ideas. Report: \
                 (1) DUPLICATES: PRDs covering the same thing (propose merge). \
                 (2) GAPS: Areas with no PRD coverage. \
                 (3) INCONSISTENCIES: Conflicting requirements. \
                 (4) STALE: Requirements already implemented (check the code). \
                 (5) IDEAS TO PROMOTE: Ideas that should become draft PRDs. \
                 After analysis, create new drafts for gaps and update existing PRDs.\n\n\
                 PRDs:\n{all_context}\n\nIdeas:\n{ideas}"
            );
            let system = roko_cli::prd::prd_agent_prompt(&workdir, "Consolidate all PRDs");
            run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
            })
            .await
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
        let default = Config::default_toml_template()?;
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

    // Learning subsystem stats.
    let learn_dir = workdir.join(".roko").join("learn");
    let efficiency_path = learn_dir.join("efficiency.jsonl");
    match read_efficiency_events(&efficiency_path).await {
        Ok(events) if !events.is_empty() => {
            println!();
            println!("efficiency events: {} total", events.len());
            let profiles = compute_role_profiles(&events);
            for p in &profiles {
                println!(
                    "  {:<16} avg_cost=${:.4}  p95_cost=${:.4}  pass_rate={:.0}%  n={}",
                    p.role,
                    p.avg_cost_usd,
                    p.p95_cost_usd,
                    p.pass_rate * 100.0,
                    p.observations,
                );
            }
        }
        _ => {}
    }

    // Experiment store summary.
    let experiments_path = learn_dir.join("experiments.json");
    let exp_store = ExperimentStore::load_or_new(&experiments_path);
    let running = exp_store.running_count();
    let concluded = exp_store.concluded_count();
    if running > 0 || concluded > 0 {
        println!();
        println!("prompt experiments: {running} running, {concluded} concluded");
    }

    // Adaptive threshold summary.
    let thresholds_path = learn_dir.join("gate-thresholds.json");
    let thresholds =
        roko_gate::adaptive_threshold::AdaptiveThresholds::load_or_new(&thresholds_path);
    let rung_count: usize = thresholds.all_rungs().count();
    if rung_count > 0 {
        println!();
        println!("adaptive gate thresholds: {rung_count} rungs tracked");
        for (rung, stats) in thresholds.all_rungs() {
            println!(
                "  rung {rung}: pass_rate={:.0}% retries={} obs={} skip={}",
                stats.ema_pass_rate * 100.0,
                thresholds.suggested_max_retries(*rung),
                stats.total_observations,
                if thresholds.should_skip_rung(*rung) {
                    "yes"
                } else {
                    "no"
                },
            );
        }
    }

    // Health probes — quick snapshot of orchestrator readiness.
    let health_probes = roko_core::obs::health::ProbeRegistry::new();
    health_probes.register(std::sync::Arc::new(
        roko_core::obs::health::AlwaysUpProbe::new("orchestrator"),
    ));
    let (readiness_status, degraded_reasons) = health_probes.readiness();
    println!();
    println!("health: {readiness_status}");
    if !degraded_reasons.is_empty() {
        for reason in &degraded_reasons {
            println!("  {} — {}", reason.component, reason.message);
        }
    }

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
    let sinks = FsObservabilitySinks::for_workdir(workdir);
    std::fs::create_dir_all(sinks.trace_sink.root())?;
    if let Some(parent) = sinks.metrics_sink.path().parent() {
        std::fs::create_dir_all(parent)?;
    }
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

fn parse_dashboard_page(input: &str) -> Option<PageId> {
    let normalized = input.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    Some(match normalized.as_str() {
        "health" => PageId::Health,
        "trends" => PageId::Trends,
        "correlations" => PageId::Correlations,
        "learning" => PageId::Learning,
        "parameters" => PageId::Parameters,
        "experiments" => PageId::Experiments,
        "optimizer" => PageId::Optimizer,
        "agent-status" | "agentstatus" | "agent-activity" | "agentactivity" => PageId::AgentStatus,
        "plan-view" | "planview" => PageId::PlanView,
        "log-view" | "logview" => PageId::LogView,
        "signals" => PageId::Signals,
        "config-view" | "configview" => PageId::ConfigView,
        _ => return None,
    })
}

fn dashboard_page_slugs() -> Vec<&'static str> {
    [
        PageId::Health,
        PageId::Trends,
        PageId::Correlations,
        PageId::Learning,
        PageId::Parameters,
        PageId::Experiments,
        PageId::Optimizer,
        PageId::AgentStatus,
        PageId::PlanView,
        PageId::LogView,
        PageId::Signals,
        PageId::ConfigView,
    ]
    .into_iter()
    .map(PageId::slug)
    .collect()
}

fn load_startup_env_files() -> Result<Vec<(String, String)>> {
    let mut redactions = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        let global_env = PathBuf::from(home).join(".roko").join(".env");
        if global_env.is_file() {
            redactions.extend(load_env_file(&global_env)?);
            dotenvy::from_path_override(&global_env)
                .with_context(|| format!("load {}", global_env.display()))?;
        }
    }

    let local_env = PathBuf::from(".env");
    if local_env.is_file() {
        redactions.extend(load_env_file(&local_env)?);
        dotenvy::from_path_override(&local_env)
            .with_context(|| format!("load {}", local_env.display()))?;
    }

    Ok(redactions)
}

fn load_env_file(path: &Path) -> Result<Vec<(String, String)>> {
    let entries = dotenvy::from_path_iter(path)
        .with_context(|| format!("inspect {}", path.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(entries)
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use roko_core::ConfigHash;
    use tempfile::tempdir;
    use tokio::fs;

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
    fn cli_parses_plan_resume_flag() {
        let cli = Cli::try_parse_from(["roko", "plan", "run", "plans", "--resume"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Plan {
                cmd: PlanCmd::Run {
                    resume: Some(_),
                    ..
                }
            })
        ));
    }

    #[test]
    fn cli_parses_resume_flag() {
        let cli = Cli::try_parse_from(["roko", "--resume", "sess-42"]).unwrap();
        assert_eq!(cli.resume.as_deref(), Some("sess-42"));
    }

    #[test]
    fn cli_parses_no_replan_flag() {
        let cli = Cli::try_parse_from(["roko", "--no-replan"]).unwrap();
        assert!(cli.no_replan);
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
    fn cli_parses_check_secrets_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "check-secrets"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::CheckSecrets { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_set_secret_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "set-secret", "TOKEN", "value"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::SetSecret { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_replay_subcommand() {
        let cli = Cli::try_parse_from(["roko", "replay", "abcd1234"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Replay { .. })));
    }

    #[test]
    fn cli_parses_dashboard_subcommand() {
        let cli = Cli::try_parse_from(["roko", "dashboard", "--page", "plan-view", "--list-pages"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Dashboard {
                page: Some(_),
                list_pages: true,
                text: false,
                ..
            })
        ));
    }

    #[test]
    fn cli_parses_dashboard_text_flag() {
        let cli = Cli::try_parse_from(["roko", "dashboard", "--text"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Dashboard { text: true, .. })
        ));
    }

    #[test]
    fn parse_dashboard_page_accepts_known_slugs() {
        assert_eq!(parse_dashboard_page("health"), Some(PageId::Health));
        assert_eq!(
            parse_dashboard_page("agent status"),
            Some(PageId::AgentStatus)
        );
        assert_eq!(
            parse_dashboard_page("agent activity"),
            Some(PageId::AgentStatus)
        );
        assert_eq!(parse_dashboard_page("plan_view"), Some(PageId::PlanView));
        assert_eq!(parse_dashboard_page("learning"), Some(PageId::Learning));
    }

    #[test]
    fn parse_dashboard_page_rejects_unknown_slugs() {
        assert_eq!(parse_dashboard_page("unknown"), None);
    }

    async fn seed_dashboard_snapshot(workdir: &Path) {
        let memory_dir = workdir.join(".roko").join("memory");
        fs::create_dir_all(&memory_dir).await.unwrap();

        let mut ep1 = Episode::new("agent-a", "task-a");
        ep1.success = true;
        ep1.usage.cost_usd = 1.25;
        ep1.usage.wall_ms = 125;
        ep1.usage.input_tokens = 100;
        ep1.usage.cache_read_tokens = 25;

        let mut ep2 = Episode::new("agent-b", "task-b");
        ep2.success = false;
        ep2.usage.cost_usd = 2.75;
        ep2.usage.wall_ms = 225;
        ep2.usage.input_tokens = 200;
        ep2.usage.cache_read_tokens = 50;

        let episodes_path = memory_dir.join("episodes.jsonl");
        let episodes = [
            serde_json::to_string(&ep1).unwrap(),
            serde_json::to_string(&ep2).unwrap(),
        ]
        .join("\n")
            + "\n";
        fs::write(&episodes_path, episodes).await.unwrap();

        let config_hash = ConfigHash::from("abcd1234".to_string());
        let mut metric1 = TaskMetric::new(config_hash.clone(), "plan-a", "task-a");
        metric1.model = "claude-haiku".to_string();
        metric1.gate_passed = true;
        metric1.cost_usd = 1.0;
        metric1.input_tokens = 100;
        metric1.iteration = 1;

        let mut metric2 = TaskMetric::new(config_hash, "plan-b", "task-b");
        metric2.model = "claude-sonnet".to_string();
        metric2.gate_passed = false;
        metric2.cost_usd = 3.0;
        metric2.input_tokens = 200;
        metric2.iteration = 1;

        let task_metrics_path = memory_dir.join("task-metrics.jsonl");
        let task_metrics =
            [metric1.to_jsonl().unwrap(), metric2.to_jsonl().unwrap()].join("\n") + "\n";
        fs::write(&task_metrics_path, task_metrics).await.unwrap();
    }

    #[tokio::test]
    async fn dashboard_output_renders_snapshot_for_health_and_falls_back_for_other_pages() {
        let dir = tempdir().unwrap();
        seed_dashboard_snapshot(dir.path()).await;

        let cli = Cli::try_parse_from(["roko", "--quiet"]).unwrap();
        let health = dashboard_output(
            &cli,
            Some(dir.path().to_path_buf()),
            Some("health".to_string()),
            false,
        )
        .await
        .unwrap();
        assert!(health.contains("Health (health)"));
        assert!(health.contains("episodes: 2"));
        assert!(health.contains("success rate: 50.0%"));
        assert!(health.contains("avg cost / episode: $2.0000"));
        assert!(health.contains("cache hit rate: 25.0%"));

        let trends = dashboard_output(
            &cli,
            Some(dir.path().to_path_buf()),
            Some("trends".to_string()),
            false,
        )
        .await
        .unwrap();
        assert!(trends.contains("Trends (trends)"));
        assert!(trends.contains("first-attempt pass rate: 50.0%"));
        assert!(trends.contains("avg iterations per plan: 1.00"));
        assert!(trends.contains("avg cost per plan: $2.0000"));
        assert!(trends.contains("haiku share: 50.0%"));

        let fallback = dashboard_output(
            &cli,
            Some(dir.path().to_path_buf()),
            Some("plan-view".to_string()),
            false,
        )
        .await
        .unwrap();
        assert!(fallback.contains("Plan View (plan-view)"));
        assert!(fallback.contains("widgets (2):"));
        assert!(fallback.contains("DAG [dag]"));
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

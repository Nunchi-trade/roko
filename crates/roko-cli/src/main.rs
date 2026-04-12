//! `roko` binary entrypoint.
//!
//! See [`roko_cli`] for the lib-side description. The binary exposes
//! subcommands (`init`, `run`, `status`, `replay`, `dream`, `config`, `inject`,
//! `plan`, `research`, `neuro`, `subscription`, `event-sources`, `experiment`) plus top-level flags for mode selection (`--headless`,
//! `--role`, `--model`, `--effort`, `--json`, `--log-format`, `--quiet`,
//! `--resume`, `--repo`, `--no-replan`, and a positional `[prompt]` for
//! one-shot mode).

#![allow(clippy::too_many_lines)]

mod commands;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use commands::experiment::{ExperimentCmd, dispatch_experiment};
use octocrab::Octocrab;
use octocrab::models::hooks::{Config as HookConfig, ContentType, Hook};
use octocrab::models::webhook_events::WebhookEventType;
use roko_agent::process::{cleanup_orphaned_agents, reap_orphaned_children};
use roko_agent::translate::BackendResponse;
use roko_cli::serve_runtime::RokoCliRuntime;
use roko_cli::tui::App;
use roko_cli::{
    Config, DaemonMode, DashboardScaffold, EditTarget, InjectKind, InjectRequest, OneshotMode,
    PageId, PipeMode, Plan, PlanSummary, ReplMode, RepoRegistry, SessionStatus, Source,
    WizardInputs, config_cmd, load_layered, run_init_wizard, run_once,
};
use roko_core::agent::{AgentRole, ProviderKind};
use roko_core::config::ServeDeployWebhookConfig;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::{ContentHash, Context, Kind, Query, Substrate};
use roko_core::{Headlines, TaskMetric, compute_headlines};
use roko_dreams::{DreamAgentConfig, DreamEngine, DreamLoopConfig, DreamRunner};
use roko_fs::{FileSubstrate, FsObservabilitySinks, RokoLayout};
use roko_learn::cascade_router::{CascadeRouteExplanation, CascadeRouter};
use roko_learn::cfactor::{CFactor, trend_arrow as cfactor_trend_arrow};
use roko_learn::cost_table::CostTable;
use roko_learn::efficiency::compute_role_profiles;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::latency::{LatencyRegistry, LatencyStats};
use roko_learn::model_router::{RoutingContext, normalized_cost};
use roko_learn::prompt_experiment::ExperimentStore;
use roko_learn::provider_health::{CircuitState, ProviderHealth};
use roko_learn::runtime_feedback::{read_efficiency_events, refresh_cfactor_snapshot};
use roko_neuro::{DEFAULT_GC_MIN_CONFIDENCE, KnowledgeStore};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt::Write as _;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
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
        /// Generate cloud-ready defaults for deployment.
        #[arg(long)]
        cloud: bool,
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
        /// Compute and persist the latest C-Factor snapshot.
        #[arg(long)]
        cfactor: bool,
    },
    /// Walk the lineage DAG rooted at a signal hash and print it.
    Replay {
        /// Signal hash (64 hex chars) to walk.
        hash: String,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Manage dream replay, report, and scheduling.
    Dream {
        #[command(subcommand)]
        cmd: DreamCmd,
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
    /// Manage plans (list, show, create, validate).
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
    /// Search durable knowledge and memory entries.
    Neuro {
        #[command(subcommand)]
        cmd: NeuroCmd,
    },
    /// Manage event subscriptions.
    Subscription {
        #[command(subcommand)]
        cmd: SubscriptionCmd,
    },
    /// Inspect configured event sources.
    EventSources {
        #[command(subcommand)]
        cmd: EventSourcesCmd,
    },
    /// Inspect configured LLM providers.
    Provider {
        #[command(subcommand)]
        cmd: ProviderCmd,
    },
    /// Inspect configured models and their capabilities.
    Model {
        #[command(subcommand)]
        cmd: ModelCmd,
    },
    /// Manage model experiments.
    Experiment {
        #[command(subcommand)]
        cmd: ExperimentCmd,
    },
    /// Manage cloud deployment targets.
    Deploy {
        #[command(subcommand)]
        cmd: DeployCmd,
    },
    /// Manage daemon mode.
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
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
enum DaemonCmd {
    Start {
        #[arg(long)]
        foreground: bool,
        #[arg(long, default_value = "9090")]
        port: u16,
    },
    Stop,
    Status,
    Logs {
        #[arg(long, short = 'f')]
        follow: bool,
        #[arg(long, short = 'n', default_value = "50")]
        lines: usize,
    },
    Reload,
    // SIGHUP equivalent — re-scan subscriptions/templates without restart
    Restart {
        #[arg(long, default_value = "9090")]
        port: u16,
    },
    Install,
    // macOS launchd plist generation
    Uninstall, // remove launchd plist
}

#[derive(Debug, Subcommand)]
enum DreamCmd {
    /// Run a dream consolidation cycle immediately.
    Run {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show the latest dream report without running a new cycle.
    Report {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show when the next dream should fire.
    Schedule {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
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
    /// Validate a plan directory for modern `tasks.toml` fields.
    Validate {
        /// Path to the plan directory containing `tasks.toml`.
        plan_dir: PathBuf,
    },
    /// Run a plan directory through the orchestration loop.
    Run {
        /// Path to the plans directory.
        plans_dir: PathBuf,
        /// Working directory (repo root). Defaults to current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Resume from `.roko/state/executor.json` in the working directory.
        #[arg(long = "resume-plan", num_args = 0..=1, default_missing_value = ".roko/state/executor.json")]
        resume_plan: Option<PathBuf>,
    },
    /// Generate implementation plans from a prompt, file, or PRD.
    Generate {
        /// Source: free-text prompt, or path to a file (PRD, requirements, etc).
        source: Vec<String>,
        /// Treat source as a file path to read (instead of inline text).
        #[arg(long)]
        from_file: Option<PathBuf>,
    },
    /// Regenerate an existing plan from its source PRD / plan extract.
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
        /// Preview generation without writing tasks.toml files.
        #[arg(long)]
        dry_run: bool,
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
        /// Execute the generated plan immediately after promotion.
        #[arg(long)]
        auto_execute: bool,
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
        /// Use Perplexity deep research (async, 1-10 min).
        #[arg(long, help = "Use Perplexity deep research (async, 1-10 min)")]
        deep: bool,
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
    /// Direct web search using Perplexity's pure search API. Returns raw results without synthesis.
    Search {
        /// The search query.
        query: Vec<String>,
        /// Restrict results to these domains (comma-separated, e.g. "docs.rs,github.com").
        #[arg(long, value_delimiter = ',')]
        domains: Vec<String>,
        /// Recency filter: day, week, month, year.
        #[arg(long)]
        recency: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum NeuroCmd {
    /// Query the durable knowledge store for a topic.
    Query {
        /// Topic to search for.
        topic: Vec<String>,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show aggregate statistics for the durable knowledge store.
    Stats {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Run garbage collection on the durable knowledge store.
    Gc {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum SubscriptionCmd {
    /// List all subscriptions.
    List,
    /// Create a new subscription.
    Add {
        /// Agent template name to invoke.
        #[arg(long)]
        template: String,
        /// Signal trigger glob to match.
        #[arg(long)]
        trigger: String,
    },
    /// Delete a subscription.
    Remove {
        /// Subscription ID.
        id: String,
    },
    /// Enable a subscription.
    Enable {
        /// Subscription ID.
        id: String,
    },
    /// Disable a subscription.
    Disable {
        /// Subscription ID.
        id: String,
    },
}

#[derive(Debug, Subcommand)]
enum EventSourcesCmd {
    /// List configured cron schedules and file watchers.
    List {
        /// Directory containing `roko.toml` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum ProviderCmd {
    /// List configured providers and their current connection status.
    List {
        /// Directory containing `roko.toml` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show persisted provider circuit-breaker health and latency.
    Health {
        /// Directory containing `.roko/` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Send a minimal request to verify provider connectivity.
    Test {
        /// Provider name from `[providers.*]`.
        provider: String,
        /// Directory containing `roko.toml` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum ModelCmd {
    /// List configured models and their capabilities.
    List {
        /// Directory containing `roko.toml` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show the current routing decision and optionally explain why it won.
    Route {
        /// Model key or slug to explain.
        model: String,
        /// Show the full routing trace instead of only the final decision.
        #[arg(long)]
        explain: bool,
        /// Complexity tier (`mechanical`, `focused`, `integrative`, `architectural`).
        #[arg(long)]
        complexity: Option<String>,
        /// Directory containing `roko.toml` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum DeployCmd {
    /// Deploy the current workspace to Railway via the public GraphQL API.
    Railway {
        /// Working directory / repository root (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Generate `fly.toml` and deploy the current workspace with Fly.io.
    Fly {
        /// Working directory / repository root (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Build the local Docker image and tag it for the configured registry.
    Docker {
        /// Working directory / repository root (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Registry namespace to tag the image under.
        #[arg(long)]
        registry: Option<String>,
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
    /// Validate `roko.toml` syntax, schema, and semantic references.
    Validate {
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Migrate a legacy project `roko.toml` into explicit provider/model tables.
    Migrate {
        /// Directory to resolve project config from (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Print the proposed migration without writing changes.
        #[arg(long)]
        dry_run: bool,
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

    // ROKO_LOG_RAW=1 disables secret redaction (useful for debugging).
    let raw_logs = env::var("ROKO_LOG_RAW")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if raw_logs {
        match cli.log_format {
            LogFormat::Json => {
                tracing_subscriber::fmt()
                    .json()
                    .with_env_filter(filter)
                    .init();
            }
            LogFormat::Text => {
                tracing_subscriber::fmt().with_env_filter(filter).init();
            }
        }
    } else {
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
        Self { inner, scrubber }
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
        Command::Init { path, cloud } => {
            cmd_init(path, cloud).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Run { prompt, workdir } => cmd_run(cli, workdir, prompt).await,
        Command::Status { workdir, cfactor } => {
            cmd_status(cli, workdir, cfactor).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Replay { hash, workdir } => cmd_replay(workdir, hash).await,
        Command::Dream { cmd } => cmd_dream(cli, cmd).await,
        Command::Config { cmd } => {
            dispatch_config(cmd).await?;
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
        Command::Neuro { cmd } => cmd_neuro(cli, cmd).await,
        Command::Subscription { cmd } => {
            let result = cmd_subscription(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::EventSources { cmd } => {
            let result = cmd_event_sources(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Provider { cmd } => cmd_provider(cli, cmd).await,
        Command::Model { cmd } => cmd_model(cli, cmd).await,
        Command::Experiment { cmd } => dispatch_experiment(cli, cmd),
        Command::Deploy { cmd } => cmd_deploy(cli, cmd).await,
        Command::Daemon { cmd } => cmd_daemon(cli, cmd).await,
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
            let config = resolve_config_for_workdir(cli, &wd)?;
            let repo_registry = RepoRegistry::load(&config, &wd).unwrap_or_default();
            let runtime = RokoCliRuntime::new(config, repo_registry).into_arc();
            roko_serve::run_server(wd, runtime, bind, port).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Worker { port } => {
            roko_cli::worker::run_worker(port).await?;
            Ok(EXIT_SUCCESS)
        }
    }
}

async fn cmd_daemon(_cli: &Cli, cmd: DaemonCmd) -> Result<i32> {
    match cmd {
        DaemonCmd::Start { foreground, port } => {
            roko_cli::daemon::daemon_start(foreground, port).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Stop => {
            roko_cli::daemon::daemon_stop().await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Status => {
            roko_cli::daemon::daemon_status().await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Logs { follow, lines } => {
            roko_cli::daemon::daemon_logs(follow, lines).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Reload => {
            roko_cli::daemon::daemon_reload().await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Restart { port } => {
            roko_cli::daemon::daemon_restart(port).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Install => {
            roko_cli::daemon::daemon_install()?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Uninstall => {
            roko_cli::daemon::daemon_uninstall()?;
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

async fn cmd_event_sources(cli: &Cli, cmd: EventSourcesCmd) -> Result<i32> {
    match cmd {
        EventSourcesCmd::List { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::event_sources::cmd_list(&wd, cli.json)?;
            Ok(EXIT_SUCCESS)
        }
    }
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
        // Use the Mori-style interactive TUI with 60fps event loop.
        if App::new_with_page(&workdir, initial_page)
            .run()
            .is_ok()
        {
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
    cfactor_history: Vec<CFactor>,
    cfactor: Option<CFactor>,
}

impl DashboardSnapshot {
    async fn load(workdir: &Path) -> Result<Self> {
        let layout = RokoLayout::for_project(workdir);
        let episodes = EpisodeLogger::read_all_lossy(layout.episodes_path()).await?;
        let task_metrics = load_task_metrics(layout.memory_dir().join("task-metrics.jsonl")).await;
        let cfactor_history =
            load_cfactor_history(workdir.join(".roko").join("learn").join("c-factor.jsonl")).await;
        let cfactor = cfactor_history.last().cloned();
        let headlines = compute_headlines(&task_metrics);
        Ok(Self {
            episodes,
            task_metrics,
            headlines,
            cfactor_history,
            cfactor,
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
        let cfactor = self.cfactor.as_ref().cloned().unwrap_or_default();
        let trend =
            cfactor_trend_arrow(&self.cfactor_history, Duration::from_secs(7 * 24 * 60 * 60));
        render_data_page(
            "Health",
            PageId::Health.slug(),
            "Top-line health gauges derived from the latest snapshot.",
            &[
                format!(
                    "focus: current C-Factor {:.2} {}, {} episodes",
                    cfactor.overall, trend, summary.episode_count
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
                format!("current c-factor: {:.2} {}", cfactor.overall, trend),
                format!(
                    "gate pass rate: {}",
                    format_percent(cfactor.components.gate_pass_rate)
                ),
                format!(
                    "cost efficiency: {}",
                    format_percent(cfactor.components.cost_efficiency)
                ),
                format!("speed: {}", format_percent(cfactor.components.speed)),
                format!(
                    "information flow rate: {}",
                    format_percent(cfactor.components.information_flow_rate)
                ),
                format!(
                    "first-try rate: {}",
                    format_percent(cfactor.components.first_try_rate)
                ),
                format!(
                    "knowledge growth: {}",
                    format_percent(cfactor.components.knowledge_growth)
                ),
                format!(
                    "knowledge integration rate: {}",
                    format_percent(cfactor.components.knowledge_integration_rate)
                ),
                format!(
                    "convergence velocity: {}",
                    format_percent(cfactor.components.convergence_velocity)
                ),
                format!(
                    "turn-taking equality: {}",
                    format_percent(cfactor.components.turn_taking_equality)
                ),
                format!(
                    "social sensitivity: {}",
                    format_percent(cfactor.components.social_sensitivity)
                ),
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

async fn load_cfactor_history(path: PathBuf) -> Vec<CFactor> {
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Vec::new();
    };

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<CFactor>(line).ok())
        .collect()
}

// -----------------------------------------------------------------------
// Subcommand handlers
// -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderListRow {
    provider: String,
    kind: String,
    base_url: String,
    status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelListRow {
    model: String,
    provider: String,
    slug: String,
    context: String,
    tools: String,
    thinking: String,
    vision: String,
    cost: String,
}

const PROVIDER_FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderHealthRow {
    provider: String,
    state: String,
    fails: String,
    cooldown: String,
    latency_p50: String,
    error_rate: String,
    last_check: String,
}

#[derive(Debug, Deserialize, Default)]
struct ProviderHealthSnapshot {
    #[serde(default)]
    providers: HashMap<String, ProviderHealth>,
}

#[derive(Debug, Deserialize, Default)]
struct LatencyStatsSnapshot {
    #[serde(default)]
    entries: Vec<LatencyStatsEntry>,
}

#[derive(Debug, Deserialize)]
struct LatencyStatsEntry {
    provider: String,
    stats: LatencyStats,
}

#[derive(Debug, Default)]
struct ProviderLatencySummary {
    recent_latencies: Vec<f64>,
    weighted_latency_ms: f64,
    observations: u64,
}

impl ProviderLatencySummary {
    fn record(&mut self, stats: &LatencyStats) {
        self.recent_latencies
            .extend(stats.recent_latencies.iter().copied());
        self.weighted_latency_ms += stats.total_latency_ema_ms * stats.observations as f64;
        self.observations = self.observations.saturating_add(stats.observations);
    }

    fn p50_ms(&self) -> Option<f64> {
        if !self.recent_latencies.is_empty() {
            let mut latencies = self.recent_latencies.clone();
            latencies.sort_by(|a, b| a.total_cmp(b));
            let idx = ((latencies.len() as f64) * 0.50).floor() as usize;
            let idx = idx.min(latencies.len().saturating_sub(1));
            return latencies.get(idx).copied();
        }

        if self.observations > 0 {
            return Some(self.weighted_latency_ms / self.observations as f64);
        }

        None
    }
}

async fn cmd_provider(cli: &Cli, cmd: ProviderCmd) -> Result<i32> {
    match cmd {
        ProviderCmd::List { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_provider_list(&wd).await?;
            Ok(EXIT_SUCCESS)
        }
        ProviderCmd::Health { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_provider_health(&wd)?;
            Ok(EXIT_SUCCESS)
        }
        ProviderCmd::Test { provider, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_provider_test(&wd, &provider).await?;
            Ok(EXIT_SUCCESS)
        }
    }
}

async fn cmd_model(cli: &Cli, cmd: ModelCmd) -> Result<i32> {
    match cmd {
        ModelCmd::List { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_model_list(&wd)?;
            Ok(EXIT_SUCCESS)
        }
        ModelCmd::Route {
            model,
            explain,
            complexity,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_model_route(
                &wd,
                cli.role.as_deref(),
                &model,
                explain,
                complexity.as_deref(),
            )?;
            Ok(EXIT_SUCCESS)
        }
    }
}

async fn cmd_provider_list(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let providers = configured_providers(&config);
    if providers.is_empty() {
        println!("no providers configured");
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_secs(2))
        .build()
        .context("build provider probe client")?;

    let mut provider_names = providers.keys().cloned().collect::<Vec<_>>();
    provider_names.sort_unstable();

    let mut rows = Vec::with_capacity(provider_names.len());
    for provider_name in provider_names {
        let provider = providers
            .get(&provider_name)
            .expect("provider name collected from provider registry");
        rows.push(inspect_provider(&client, &provider_name, provider).await);
    }

    print!("{}", format_provider_rows(&rows));
    Ok(())
}

fn cmd_model_list(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let models = configured_models(&config);
    if models.is_empty() {
        println!("no models configured");
        return Ok(());
    }

    let mut model_names = models.keys().cloned().collect::<Vec<_>>();
    model_names.sort_unstable();

    let rows = model_names
        .into_iter()
        .map(|model_name| {
            let profile = models
                .get(&model_name)
                .expect("model name collected from model registry");
            build_model_list_row(&model_name, profile)
        })
        .collect::<Vec<_>>();

    print!("{}", format_model_rows(&rows));
    Ok(())
}

fn cmd_model_route(
    workdir: &Path,
    role_arg: Option<&str>,
    requested_model: &str,
    explain: bool,
    complexity_arg: Option<&str>,
) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let models = configured_models(&config);
    if models.is_empty() {
        println!("no models configured");
        return Ok(());
    }

    let mut model_slugs = models
        .values()
        .map(|profile| profile.slug.clone())
        .collect::<Vec<_>>();
    model_slugs.sort();
    model_slugs.dedup();

    let role = parse_agent_role(role_arg)?;
    let complexity = parse_route_complexity(complexity_arg)?;
    let aliases = model_aliases_by_slug(&models);
    let requested_slug = resolve_requested_model_slug(requested_model, &models)
        .unwrap_or_else(|| requested_model.to_string());
    let context = RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: complexity.band,
        iteration: 1,
        role,
        crate_familiarity: 0.0,
        has_prior_failure: false,
        affect_confidence: 0.5,
        thinking_level: None,
        previous_model: Some(requested_slug.clone()),
        plan_context_tokens: None,
    };

    let router = CascadeRouter::load_or_new(&cascade_router_path(workdir), model_slugs.clone());
    let provider_health = load_provider_health_snapshot(&provider_health_path(workdir))?;
    let latency_registry = LatencyRegistry::load_or_new(&latency_stats_path(workdir));
    let model_providers = model_provider_map(&models, &model_slugs);
    let available_candidates = available_model_candidates(
        &model_slugs,
        &model_providers,
        &provider_health,
        unix_ms_now(),
    );
    let explanation = router.explain_route(
        &context,
        (!available_candidates.is_empty()).then_some(available_candidates.as_slice()),
    );

    if !explain {
        let selected_name = display_model_name(&aliases, &explanation.selected_slug);
        let provider = model_providers
            .get(&explanation.selected_slug)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        println!("{selected_name} via {provider}");
        return Ok(());
    }

    let confidence = router.confidence_snapshot();
    let cost_table = CostTable::from_config(&models).with_defaults();
    print!(
        "{}",
        format_model_route_explanation(
            requested_model,
            &requested_slug,
            &aliases,
            &explanation,
            &confidence,
            &model_providers,
            &provider_health,
            &latency_registry,
            &cost_table,
        )
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RouteComplexity {
    band: TaskComplexityBand,
    tier_label: &'static str,
}

fn parse_agent_role(input: Option<&str>) -> Result<AgentRole> {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Ok(AgentRole::Implementer);
    };

    let normalized = normalize_route_token(input);
    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS)
        .find(|role| {
            normalize_route_token(role.label()) == normalized
                || normalize_route_token(role.short()) == normalized
        })
        .ok_or_else(|| anyhow!("unknown role '{input}'"))
}

fn parse_route_complexity(input: Option<&str>) -> Result<RouteComplexity> {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "focused",
        });
    };

    match normalize_route_token(input).as_str() {
        "mechanical" | "fast" | "low" => Ok(RouteComplexity {
            band: TaskComplexityBand::Fast,
            tier_label: "mechanical",
        }),
        "focused" | "standard" | "medium" => Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "focused",
        }),
        "integrative" => Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "integrative",
        }),
        "architectural" | "complex" | "premium" | "high" => Ok(RouteComplexity {
            band: TaskComplexityBand::Complex,
            tier_label: "architectural",
        }),
        _ => bail!("unknown complexity '{input}'"),
    }
}

fn normalize_route_token(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn resolve_requested_model_slug(
    requested_model: &str,
    models: &HashMap<String, ModelProfile>,
) -> Option<String> {
    if let Some(profile) = models.get(requested_model) {
        return Some(profile.slug.clone());
    }

    let normalized = normalize_route_token(requested_model);
    let mut entries = models.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));

    for (model_key, profile) in entries {
        if normalize_route_token(model_key) == normalized
            || normalize_route_token(&profile.slug) == normalized
        {
            return Some(profile.slug.clone());
        }
    }

    None
}

fn model_aliases_by_slug(models: &HashMap<String, ModelProfile>) -> HashMap<String, String> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for (model_key, profile) in models {
        grouped
            .entry(profile.slug.clone())
            .or_default()
            .push(model_key.clone());
    }

    let mut aliases = HashMap::new();
    for (slug, mut keys) in grouped {
        keys.sort();
        let alias = if keys.len() == 1 {
            keys[0].clone()
        } else {
            slug.clone()
        };
        aliases.insert(slug, alias);
    }
    aliases
}

fn display_model_name(aliases: &HashMap<String, String>, slug: &str) -> String {
    aliases
        .get(slug)
        .cloned()
        .unwrap_or_else(|| slug.to_string())
}

fn model_provider_map(
    models: &HashMap<String, ModelProfile>,
    model_slugs: &[String],
) -> HashMap<String, String> {
    let mut entries = models.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));

    let mut providers = HashMap::new();
    for slug in model_slugs {
        if let Some((_, profile)) = entries.iter().find(|(_, profile)| profile.slug == *slug) {
            providers.insert(slug.clone(), profile.provider.clone());
        }
    }
    providers
}

fn available_model_candidates(
    model_slugs: &[String],
    model_providers: &HashMap<String, String>,
    provider_health: &HashMap<String, ProviderHealth>,
    now_ms: i64,
) -> Vec<String> {
    model_slugs
        .iter()
        .filter(|slug| {
            model_providers
                .get(slug.as_str())
                .map(|provider| provider_is_available(provider_health.get(provider), now_ms))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn provider_is_available(health: Option<&ProviderHealth>, now_ms: i64) -> bool {
    health
        .map(|snapshot| effective_circuit_state(snapshot, now_ms) != CircuitState::Open)
        .unwrap_or(true)
}

fn format_model_route_explanation(
    requested_model: &str,
    requested_slug: &str,
    aliases: &HashMap<String, String>,
    explanation: &CascadeRouteExplanation,
    confidence: &HashMap<String, (u64, u64)>,
    model_providers: &HashMap<String, String>,
    provider_health: &HashMap<String, ProviderHealth>,
    latency_registry: &LatencyRegistry,
    cost_table: &CostTable,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Routing decision for '{requested_model}':");
    let _ = writeln!(
        out,
        "  Stage: {} ({} observations)",
        format_route_stage(explanation.stage),
        explanation.observations
    );
    if let Some(alpha) = explanation.alpha {
        let _ = writeln!(out, "  Alpha: {alpha:.3} ({})", describe_alpha(alpha));
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  Candidate Scores:");

    let candidate_names = explanation
        .candidates
        .iter()
        .map(|candidate| display_model_name(aliases, &candidate.slug))
        .collect::<Vec<_>>();
    let name_width = candidate_names
        .iter()
        .map(String::len)
        .max()
        .unwrap_or("Model".len())
        .max("Model".len());

    for (candidate, name) in explanation.candidates.iter().zip(candidate_names.iter()) {
        let (trials, successes) = confidence.get(&candidate.slug).copied().unwrap_or((0, 0));
        let pass_rate = if trials > 0 {
            successes as f64 / trials as f64
        } else {
            0.0
        };
        let provider = model_providers.get(&candidate.slug).map(String::as_str);
        let cost = normalized_cost(&candidate.slug, cost_table);
        let latency = provider
            .and_then(|provider| {
                normalized_latency_for_model(&candidate.slug, provider, latency_registry)
            })
            .unwrap_or(0.0);
        let selected_marker = if candidate.selected {
            "  <- selected"
        } else {
            ""
        };

        let _ = writeln!(
            out,
            "    {:<name_width$}  {:>5.3}  (pass: {:>3.0}%, cost: {:.2}, latency: {:.2}){}",
            name,
            candidate.score,
            pass_rate * 100.0,
            cost,
            latency,
            selected_marker,
            name_width = name_width,
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  Provider Health:");
    let mut providers = explanation
        .candidates
        .iter()
        .filter_map(|candidate| model_providers.get(&candidate.slug))
        .cloned()
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        let _ = writeln!(out, "    none");
    } else {
        let now_ms = unix_ms_now();
        for provider in providers {
            let status = format_provider_health_note(provider_health.get(&provider), now_ms);
            let _ = writeln!(out, "    {provider}: {status}");
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  Cache Affinity:");
    let previous_name = display_model_name(aliases, requested_slug);
    let affinity_note = match explanation.stage {
        roko_learn::cascade_router::CascadeStage::Confidence
            if explanation
                .candidates
                .iter()
                .any(|candidate| candidate.slug == requested_slug) =>
        {
            "(+0.15 bonus applied)"
        }
        roko_learn::cascade_router::CascadeStage::Ucb
            if explanation
                .candidates
                .iter()
                .any(|candidate| candidate.slug == requested_slug) =>
        {
            "(affinity feature active)"
        }
        _ => "(no matching candidate bonus)",
    };
    let _ = writeln!(out, "    Previous model: {previous_name} {affinity_note}");

    let _ = writeln!(out);
    let _ = writeln!(out, "  Pareto Status:");
    let selected_name = display_model_name(aliases, &explanation.selected_slug);
    let pareto_status = if explanation
        .pareto_frontier
        .iter()
        .any(|slug| slug == &explanation.selected_slug)
    {
        "ON frontier (not dominated)"
    } else {
        "OFF frontier (dominated)"
    };
    let _ = writeln!(out, "    {selected_name}: {pareto_status}");

    let selected_provider = model_providers
        .get(&explanation.selected_slug)
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  Final: {} via {}",
        display_model_name(aliases, &explanation.selected_slug),
        selected_provider
    );
    out
}

fn format_route_stage(stage: roko_learn::cascade_router::CascadeStage) -> &'static str {
    match stage {
        roko_learn::cascade_router::CascadeStage::Static => "Static",
        roko_learn::cascade_router::CascadeStage::Confidence => "Confidence",
        roko_learn::cascade_router::CascadeStage::Ucb => "UCB",
    }
}

fn describe_alpha(alpha: f64) -> &'static str {
    if alpha <= 0.10 {
        "mostly exploitation"
    } else if alpha <= 0.25 {
        "balanced exploration"
    } else {
        "exploration-heavy"
    }
}

fn normalized_latency_for_model(
    model_slug: &str,
    provider: &str,
    latency_registry: &LatencyRegistry,
) -> Option<f64> {
    let stats = latency_registry.get(model_slug, provider)?;
    let sla_ms = default_latency_sla_for_slug(model_slug) as f64;
    (sla_ms > 0.0).then(|| (stats.total_latency_ema_ms / sla_ms).min(1.0))
}

fn default_latency_sla_for_slug(slug: &str) -> u64 {
    if slug.contains("haiku") {
        10_000
    } else if slug.contains("opus") || slug.contains("premium") {
        120_000
    } else {
        30_000
    }
}

fn format_provider_health_note(health: Option<&ProviderHealth>, now_ms: i64) -> String {
    let Some(health) = health else {
        return "CLOSED (healthy)".to_string();
    };

    match effective_circuit_state(health, now_ms) {
        CircuitState::Closed => "CLOSED (healthy)".to_string(),
        CircuitState::HalfOpen => "HALF-OPEN (probe allowed)".to_string(),
        CircuitState::Open => {
            let cooldown = format_cooldown(Some(health), CircuitState::Open, now_ms);
            if cooldown == "—" {
                "OPEN (cooldown active)".to_string()
            } else {
                format!("OPEN ({cooldown})")
            }
        }
    }
}

fn cascade_router_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("cascade-router.json")
}

fn cmd_provider_health(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let configured = configured_providers(&config);
    let health_path = provider_health_path(workdir);
    let latency_path = latency_stats_path(workdir);
    let provider_health = load_provider_health_snapshot(&health_path)?;
    let latency_stats = load_latency_stats_by_provider(&latency_path)?;

    let mut provider_names = BTreeSet::new();
    provider_names.extend(configured.keys().cloned());
    provider_names.extend(provider_health.keys().cloned());
    provider_names.extend(latency_stats.keys().cloned());

    if provider_names.is_empty() {
        println!("no provider health recorded");
        return Ok(());
    }

    let now_ms = unix_ms_now();
    let health_file_ms = file_modified_ms(&health_path);
    let latency_file_ms = file_modified_ms(&latency_path);
    let rows = provider_names
        .into_iter()
        .map(|provider| {
            build_provider_health_row(
                &provider,
                provider_health.get(&provider),
                latency_stats.get(&provider),
                now_ms,
                health_file_ms,
                latency_file_ms,
            )
        })
        .collect::<Vec<_>>();

    print!("{}", format_provider_health_rows(&rows));
    Ok(())
}

async fn cmd_provider_test(workdir: &Path, provider_name: &str) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let providers = configured_providers(&config);
    let provider = providers
        .get(provider_name)
        .ok_or_else(|| anyhow!("provider '{provider_name}' is not configured"))?;
    let model = select_provider_test_model(&config, provider_name)
        .ok_or_else(|| anyhow!("provider '{provider_name}' has no configured models"))?;

    match provider.kind {
        ProviderKind::OpenAiCompat => {
            run_openai_compat_provider_test(provider_name, provider, &model.1).await
        }
        other => bail!("provider '{provider_name}' uses unsupported kind '{other}'"),
    }
}

fn configured_providers(config: &RokoConfig) -> std::collections::HashMap<String, ProviderConfig> {
    if !config.providers.is_empty() {
        return config.providers.clone();
    }

    if config.agent.command.is_some()
        || config.agent.args.is_some()
        || config.agent.timeout_ms.is_some()
        || config
            .agent
            .env
            .as_ref()
            .is_some_and(|entries| !entries.is_empty())
    {
        return config.effective_providers();
    }

    std::collections::HashMap::new()
}

fn configured_models(config: &RokoConfig) -> std::collections::HashMap<String, ModelProfile> {
    config.effective_models()
}

fn select_provider_test_model(
    config: &RokoConfig,
    provider_name: &str,
) -> Option<(String, ModelProfile)> {
    let models = configured_models(config);
    let default_model = config.agent.default_model.trim();
    if let Some(profile) = models.get(default_model)
        && profile.provider == provider_name
    {
        return Some((default_model.to_string(), profile.clone()));
    }

    let mut candidates = models
        .into_iter()
        .filter(|(_, profile)| profile.provider == provider_name)
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.into_iter().next()
}

async fn run_openai_compat_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: &ModelProfile,
) -> Result<()> {
    let endpoint = openai_compat_test_endpoint(provider);
    let api_key_env = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|env_name| !env_name.is_empty());
    let api_key = provider
        .resolve_api_key()
        .filter(|value| !value.trim().is_empty());
    let body = json!({
        "model": model.slug,
        "messages": [{
            "role": "user",
            "content": "Say hello"
        }],
        "max_tokens": 10
    });
    let body_text = serde_json::to_string(&body).context("serialize provider test body")?;

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!("  Endpoint: {endpoint}");
    match (api_key_env, api_key.as_ref()) {
        (Some(env_name), Some(_)) => println!("  API Key:  set ({env_name})"),
        (Some(env_name), None) => {
            println!("  API Key:  missing ({env_name})");
            bail!("missing API key: env var {env_name} not set");
        }
        (None, _) => println!("  API Key:  not required"),
    }
    println!("  Model:    {}", model.slug);
    println!();
    println!("  Sending: {body_text}");

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_millis(
            provider.timeout_ms.unwrap_or(120_000),
        ))
        .build()
        .context("build provider test client")?;

    let mut request = client
        .post(&endpoint)
        .header("content-type", "application/json");
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    if let Some(extra_headers) = provider.extra_headers.as_ref() {
        let mut entries = extra_headers.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
        for (name, value) in entries {
            request = request.header(name.as_str(), value.as_str());
        }
    }

    let started = Instant::now();
    let response = request
        .json(&body)
        .send()
        .await
        .with_context(|| format!("send provider test request to {endpoint}"))?;
    let elapsed = started.elapsed();
    let status = response.status();
    let status_line = status.to_string();
    let response_text = response
        .text()
        .await
        .context("read provider test response body")?;

    if !status.is_success() {
        println!(
            "  Response: {} ({})",
            status_line,
            format_provider_test_duration(elapsed)
        );
        println!("  Error:    {response_text}");
        bail!("provider '{provider_name}' test failed");
    }

    let response_json: Value = serde_json::from_str(&response_text)
        .with_context(|| format!("parse provider test response from {endpoint}"))?;
    let backend_response = BackendResponse::Json(response_json);
    let content = backend_response.extract_text();
    let usage = backend_response.extract_usage();
    let cost = estimate_provider_test_cost(model, &usage);

    println!(
        "  Response: {} ({})",
        status_line,
        format_provider_test_duration(elapsed)
    );
    println!(
        "  Content:  {}",
        serde_json::to_string(&content).context("format provider test content")?
    );
    println!(
        "  Tokens:   input={}, output={}",
        usage.input_tokens, usage.output_tokens
    );
    match cost {
        Some(cost) => println!("  Cost:     ${cost:.6}"),
        None => println!("  Cost:     n/a"),
    }
    println!();
    println!("  ✓ Provider '{provider_name}' is working");
    Ok(())
}

fn openai_compat_test_endpoint(provider: &ProviderConfig) -> String {
    format!(
        "{}/chat/completions",
        provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/')
    )
}

fn estimate_provider_test_cost(model: &ModelProfile, usage: &roko_agent::Usage) -> Option<f64> {
    let mut cost = 0.0;
    let mut priced = false;

    if let Some(rate) = model.cost_input_per_m {
        cost += f64::from(usage.input_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_output_per_m {
        cost += f64::from(usage.output_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_cache_read_per_m {
        cost += f64::from(usage.cache_read_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_cache_write_per_m {
        cost += f64::from(usage.cache_create_tokens) * rate / 1_000_000.0;
        priced = true;
    }

    priced.then_some(cost)
}

fn format_provider_test_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.1}s", duration.as_secs_f64())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

async fn inspect_provider(
    client: &reqwest::Client,
    provider_name: &str,
    provider: &ProviderConfig,
) -> ProviderListRow {
    match provider.kind {
        ProviderKind::ClaudeCli => inspect_cli_provider(provider_name, provider),
        _ => inspect_http_provider(client, provider_name, provider).await,
    }
}

fn inspect_cli_provider(provider_name: &str, provider: &ProviderConfig) -> ProviderListRow {
    let command = provider
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty());
    let status = match command {
        Some(command) if command_available(command) => "ok (cli found)".to_string(),
        Some(_) => "warn (cli missing)".to_string(),
        None => "warn (command missing)".to_string(),
    };

    ProviderListRow {
        provider: provider_name.to_string(),
        kind: provider.kind.to_string(),
        base_url: format!("(cli: {})", command.unwrap_or("<missing>")),
        status,
    }
}

async fn inspect_http_provider(
    client: &reqwest::Client,
    provider_name: &str,
    provider: &ProviderConfig,
) -> ProviderListRow {
    let base_url = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|base_url| !base_url.is_empty());
    let mut issues = Vec::new();

    if let Some(env_name) = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|env_name| !env_name.is_empty())
    {
        let has_key = std::env::var(env_name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_key {
            issues.push("key missing".to_string());
        }
    }

    match base_url {
        Some(base_url) => {
            if let Some(issue) = probe_base_url(client, base_url).await {
                issues.push(issue);
            }
        }
        None => issues.push("base URL missing".to_string()),
    }

    let status = if issues.is_empty() {
        if provider
            .api_key_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|env_name| !env_name.is_empty())
        {
            "ok (key set)".to_string()
        } else {
            "ok (reachable)".to_string()
        }
    } else {
        format!("warn ({})", issues.join(", "))
    };

    ProviderListRow {
        provider: provider_name.to_string(),
        kind: provider.kind.to_string(),
        base_url: base_url.unwrap_or("(missing)").to_string(),
        status,
    }
}

async fn probe_base_url(client: &reqwest::Client, base_url: &str) -> Option<String> {
    match client.head(base_url).send().await {
        Ok(_) => None,
        Err(err) if err.is_builder() => Some("invalid base URL".to_string()),
        Err(_) => Some("unreachable".to_string()),
    }
}

fn command_available(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    let command_path = Path::new(command);
    if command_path.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return executable_file(command_path);
    }

    roko_cli::config::command_on_path(command)
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn format_provider_rows(rows: &[ProviderListRow]) -> String {
    let mut widths = [
        "Provider".len(),
        "Kind".len(),
        "Base URL".len(),
        "Status".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.provider.len());
        widths[1] = widths[1].max(row.kind.len());
        widths[2] = widths[2].max(row.base_url.len());
        widths[3] = widths[3].max(row.status.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<provider_w$}  {:<kind_w$}  {:<base_w$}  {:<status_w$}",
        "Provider",
        "Kind",
        "Base URL",
        "Status",
        provider_w = widths[0],
        kind_w = widths[1],
        base_w = widths[2],
        status_w = widths[3],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<provider_w$}  {:<kind_w$}  {:<base_w$}  {:<status_w$}",
            row.provider,
            row.kind,
            row.base_url,
            row.status,
            provider_w = widths[0],
            kind_w = widths[1],
            base_w = widths[2],
            status_w = widths[3],
        );
    }

    out
}

fn build_model_list_row(model_name: &str, profile: &ModelProfile) -> ModelListRow {
    ModelListRow {
        model: model_name.to_string(),
        provider: profile.provider.clone(),
        slug: profile.slug.clone(),
        context: format_context_window(profile.context_window),
        tools: format_bool_capability(profile.supports_tools).to_string(),
        thinking: format_bool_capability(profile.supports_thinking).to_string(),
        vision: format_bool_capability(profile.supports_vision).to_string(),
        cost: format_model_cost(profile),
    }
}

fn format_model_rows(rows: &[ModelListRow]) -> String {
    let mut widths = [
        "Model".len(),
        "Provider".len(),
        "Slug".len(),
        "Context".len(),
        "Tools".len(),
        "Thinking".len(),
        "Vision".len(),
        "Cost (in/out)".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.model.len());
        widths[1] = widths[1].max(row.provider.len());
        widths[2] = widths[2].max(row.slug.len());
        widths[3] = widths[3].max(row.context.len());
        widths[4] = widths[4].max(row.tools.len());
        widths[5] = widths[5].max(row.thinking.len());
        widths[6] = widths[6].max(row.vision.len());
        widths[7] = widths[7].max(row.cost.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<model_w$}  {:<provider_w$}  {:<slug_w$}  {:<context_w$}  {:<tools_w$}  {:<thinking_w$}  {:<vision_w$}  {:<cost_w$}",
        "Model",
        "Provider",
        "Slug",
        "Context",
        "Tools",
        "Thinking",
        "Vision",
        "Cost (in/out)",
        model_w = widths[0],
        provider_w = widths[1],
        slug_w = widths[2],
        context_w = widths[3],
        tools_w = widths[4],
        thinking_w = widths[5],
        vision_w = widths[6],
        cost_w = widths[7],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<model_w$}  {:<provider_w$}  {:<slug_w$}  {:<context_w$}  {:<tools_w$}  {:<thinking_w$}  {:<vision_w$}  {:<cost_w$}",
            row.model,
            row.provider,
            row.slug,
            row.context,
            row.tools,
            row.thinking,
            row.vision,
            row.cost,
            model_w = widths[0],
            provider_w = widths[1],
            slug_w = widths[2],
            context_w = widths[3],
            tools_w = widths[4],
            thinking_w = widths[5],
            vision_w = widths[6],
            cost_w = widths[7],
        );
    }

    out
}

fn format_context_window(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        let whole_thousands = tokens / 1_000;
        if tokens % 1_000 == 0 {
            format!("{whole_thousands}K")
        } else {
            let value = tokens as f64 / 1_000.0;
            format!("{value:.1}K")
        }
    } else {
        tokens.to_string()
    }
}

fn format_bool_capability(value: bool) -> &'static str {
    if value { "✓" } else { "✗" }
}

fn format_model_cost(profile: &ModelProfile) -> String {
    match (profile.cost_input_per_m, profile.cost_output_per_m) {
        (Some(input), Some(output)) => format!("${input:.2}/${output:.2}"),
        (Some(input), None) => format!("${input:.2}/—"),
        (None, Some(output)) => format!("—/${output:.2}"),
        (None, None) => "—".to_string(),
    }
}

fn provider_health_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("provider-health.json")
}

fn latency_stats_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("latency-stats.json")
}

fn load_provider_health_snapshot(path: &Path) -> Result<HashMap<String, ProviderHealth>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let snapshot: ProviderHealthSnapshot =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(snapshot.providers)
}

fn load_latency_stats_by_provider(path: &Path) -> Result<HashMap<String, ProviderLatencySummary>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let snapshot: LatencyStatsSnapshot =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;

    let mut providers = HashMap::new();
    for entry in snapshot.entries {
        providers
            .entry(entry.provider)
            .or_insert_with(ProviderLatencySummary::default)
            .record(&entry.stats);
    }

    Ok(providers)
}

fn build_provider_health_row(
    provider: &str,
    health: Option<&ProviderHealth>,
    latency: Option<&ProviderLatencySummary>,
    now_ms: i64,
    health_file_ms: Option<i64>,
    latency_file_ms: Option<i64>,
) -> ProviderHealthRow {
    let state = health
        .map(|snapshot| effective_circuit_state(snapshot, now_ms))
        .unwrap_or(CircuitState::Closed);
    let fails = health
        .map(|snapshot| {
            format!(
                "{}/{}",
                snapshot.consecutive_failures, PROVIDER_FAILURE_THRESHOLD
            )
        })
        .unwrap_or_else(|| format!("0/{PROVIDER_FAILURE_THRESHOLD}"));
    let cooldown = format_cooldown(health, state, now_ms);
    let latency_p50 = latency
        .and_then(ProviderLatencySummary::p50_ms)
        .map(format_latency_p50)
        .unwrap_or_else(|| "—".to_string());
    let error_rate = health
        .filter(|snapshot| snapshot.total_requests > 0)
        .map(|snapshot| {
            format!(
                "{:.1}%",
                (snapshot.total_failures as f64 * 100.0) / snapshot.total_requests as f64
            )
        })
        .unwrap_or_else(|| "—".to_string());

    let mut last_check_ms = health.and_then(|snapshot| snapshot.last_failure_at);
    if health.is_some() {
        last_check_ms = max_timestamp(last_check_ms, health_file_ms);
    }
    if latency.is_some() {
        last_check_ms = max_timestamp(last_check_ms, latency_file_ms);
    }

    ProviderHealthRow {
        provider: provider.to_string(),
        state: format_circuit_state(state).to_string(),
        fails,
        cooldown,
        latency_p50,
        error_rate,
        last_check: last_check_ms
            .map(|timestamp_ms| format_timestamp_age(timestamp_ms, now_ms))
            .unwrap_or_else(|| "—".to_string()),
    }
}

fn effective_circuit_state(health: &ProviderHealth, now_ms: i64) -> CircuitState {
    match health.state {
        CircuitState::Open if health.cooldown_until.is_some_and(|until| now_ms >= until) => {
            CircuitState::HalfOpen
        }
        state => state,
    }
}

fn format_circuit_state(state: CircuitState) -> &'static str {
    match state {
        CircuitState::Closed => "CLOSED",
        CircuitState::Open => "OPEN",
        CircuitState::HalfOpen => "HALF-OPEN",
    }
}

fn format_cooldown(health: Option<&ProviderHealth>, state: CircuitState, now_ms: i64) -> String {
    let Some(health) = health else {
        return "—".to_string();
    };

    if state != CircuitState::Open {
        return "—".to_string();
    }

    health
        .cooldown_until
        .map(|until| until.saturating_sub(now_ms))
        .filter(|remaining_ms| *remaining_ms > 0)
        .map(format_remaining_ms)
        .unwrap_or_else(|| "—".to_string())
}

fn format_provider_health_rows(rows: &[ProviderHealthRow]) -> String {
    let mut widths = [
        "Provider".len(),
        "State".len(),
        "Fails".len(),
        "Cooldown".len(),
        "Latency p50".len(),
        "Error Rate".len(),
        "Last Check".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.provider.len());
        widths[1] = widths[1].max(row.state.len());
        widths[2] = widths[2].max(row.fails.len());
        widths[3] = widths[3].max(row.cooldown.len());
        widths[4] = widths[4].max(row.latency_p50.len());
        widths[5] = widths[5].max(row.error_rate.len());
        widths[6] = widths[6].max(row.last_check.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<provider_w$}  {:<state_w$}  {:<fails_w$}  {:<cooldown_w$}  {:<latency_w$}  {:<error_w$}  {:<last_w$}",
        "Provider",
        "State",
        "Fails",
        "Cooldown",
        "Latency p50",
        "Error Rate",
        "Last Check",
        provider_w = widths[0],
        state_w = widths[1],
        fails_w = widths[2],
        cooldown_w = widths[3],
        latency_w = widths[4],
        error_w = widths[5],
        last_w = widths[6],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<provider_w$}  {:<state_w$}  {:<fails_w$}  {:<cooldown_w$}  {:<latency_w$}  {:<error_w$}  {:<last_w$}",
            row.provider,
            row.state,
            row.fails,
            row.cooldown,
            row.latency_p50,
            row.error_rate,
            row.last_check,
            provider_w = widths[0],
            state_w = widths[1],
            fails_w = widths[2],
            cooldown_w = widths[3],
            latency_w = widths[4],
            error_w = widths[5],
            last_w = widths[6],
        );
    }

    out
}

fn format_latency_p50(ms: f64) -> String {
    if ms >= 500.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        format!("{ms:.0}ms")
    }
}

fn format_remaining_ms(ms: i64) -> String {
    let secs = (ms.max(0) + 999) / 1000;
    format!("{} left", format_compact_duration(secs))
}

fn format_timestamp_age(timestamp_ms: i64, now_ms: i64) -> String {
    let secs = now_ms.saturating_sub(timestamp_ms).max(0) / 1000;
    format!("{} ago", format_compact_duration(secs))
}

fn format_compact_duration(secs: i64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    system_time_to_ms(modified)
}

fn system_time_to_ms(timestamp: SystemTime) -> Option<i64> {
    timestamp
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

fn unix_ms_now() -> i64 {
    system_time_to_ms(SystemTime::now()).unwrap_or(0)
}

fn max_timestamp(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

async fn dispatch_config(cmd: ConfigCmd) -> Result<()> {
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
        ConfigCmd::Validate { workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            config_cmd::cmd_validate(&wd).await
        }
        ConfigCmd::Migrate { workdir, dry_run } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            config_cmd::cmd_migrate(&wd, dry_run)
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

fn plan_has_old_tasks_format(workdir: &Path, plan_path: &Path) -> bool {
    let Some(plan_stem) = plan_path.file_stem() else {
        return false;
    };

    let tasks_path = roko_cli::plan::plans_dir(workdir)
        .join(plan_stem)
        .join("tasks.toml");
    if !tasks_path.is_file() {
        return false;
    }

    matches!(
        roko_cli::task_parser::TasksFile::validate_modern_fields(&tasks_path),
        Ok(issues) if !issues.is_empty()
    )
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
                        old_format: plan_has_old_tasks_format(&wd, p),
                        last_error: None,
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
        PlanCmd::Validate { plan_dir } => {
            let tasks_path = plan_dir.join("tasks.toml");
            if !tasks_path.is_file() {
                anyhow::bail!("No tasks.toml found in {}", plan_dir.display());
            }

            let issues = roko_cli::task_parser::TasksFile::validate_modern_fields(&tasks_path)?;
            if issues.is_empty() {
                if !cli.quiet {
                    println!("modern task fields present in {}", tasks_path.display());
                }
                return Ok(EXIT_SUCCESS);
            }

            eprintln!("❌ {} is missing modern fields:", tasks_path.display());
            for issue in &issues {
                eprintln!("  - {issue}");
            }
            Ok(EXIT_AGENT_FAILURE)
        }
        PlanCmd::Run {
            plans_dir,
            workdir,
            resume_plan,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            prepare_runtime_hooks(&wd, cli.quiet);
            let config = load_layered(&wd)?.config;

            // Create the shared metric registry and register standard metrics.
            let metrics = std::sync::Arc::new(roko_core::obs::MetricRegistry::new());
            roko_core::obs::register_standard_metrics(&metrics);

            let mut runner = if let Some(snap_path) = resume_plan {
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
                        "fleet_cfactor": report.fleet_cfactor,
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
                if let Some(fleet) = &report.fleet_cfactor {
                    println!(
                        "fleet c-factor: {:.3} | plans={} | agents={} | turns={}",
                        fleet.overall, fleet.plan_count, fleet.agent_count, fleet.observation_count
                    );
                    println!(
                        "  multi_agent={:.3} pass={:.3} cost={:.3} speed={:.3} turn={:.3}",
                        fleet.components.multi_agent_coverage,
                        fleet.components.pass_rate,
                        fleet.components.cost_efficiency,
                        fleet.components.speed,
                        fleet.components.turn_taking_equality
                    );
                }
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
            let existing_tasks = roko_cli::task_parser::TasksFile::parse(&tasks_path).ok();
            let source_path = find_plan_source_document(&plan_dir)?;
            let source_content = std::fs::read_to_string(&source_path)
                .with_context(|| format!("read {}", source_path.display()))?;

            if dry_run {
                let system = roko_cli::plan_generate::build_generation_prompt(
                    &workdir,
                    &source_content,
                    "prd",
                );
                let task_prompt = format!(
                    "Regenerate the plan at {} from the source PRD above. \
                     Rewrite tasks.toml in place with full modern metadata: tier, model_hint, \
                     max_loc, files, allowed_tools, denied_tools, mcp_servers, depends_on, \
                     [task.context], and [[task.verify]]. Preserve the status of any task that \
                     is already marked done in the existing file. Do not create new plan \
                     directories.\n\n## Existing tasks.toml\n\n```toml\n{existing}\n```",
                    tasks_path.display(),
                    existing = existing,
                );
                eprintln!(
                    "\n[dry-run] Would regenerate {} from {}",
                    tasks_path.display(),
                    source_path.display()
                );
                eprintln!("Prompt length: {} chars", system.len() + task_prompt.len());
                return Ok(EXIT_SUCCESS);
            }

            let gw = load_gateway_env(&workdir);
            let model = model_from_config(&workdir);
            let model_ref = model.as_deref();

            let system =
                roko_cli::plan_generate::build_generation_prompt(&workdir, &source_content, "prd");
            let task_prompt = format!(
                "Regenerate the plan at {} from the source PRD above. \
                 Rewrite tasks.toml in place with full modern metadata: tier, model_hint, \
                 max_loc, files, allowed_tools, denied_tools, mcp_servers, depends_on, \
                 [task.context], and [[task.verify]]. Preserve the status of any task that \
                 is already marked done in the existing file. Do not create new plan \
                 directories.\n\n## Existing tasks.toml\n\n```toml\n{existing}\n```",
                tasks_path.display(),
                existing = existing,
            );

            let exit_code = match run_agent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: Some("high"),
                system_prompt: Some(&system),
                resume_session: None,
                env_vars: &gw.vars,
            })
            .await
            {
                Ok(code) => code,
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            };

            if exit_code != 0 {
                std::fs::write(&tasks_path, &existing)
                    .with_context(|| format!("restore {}", tasks_path.display()))?;
                anyhow::bail!("plan regeneration agent failed with exit code {exit_code}");
            }

            let regenerated = match roko_cli::task_parser::TasksFile::parse(&tasks_path) {
                Ok(tasks) => tasks,
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            };

            let merged =
                preserve_completed_task_status(existing_tasks.as_ref(), regenerated, &plan_dir);
            let rendered =
                toml::to_string_pretty(&merged).context("serialize regenerated tasks.toml")?;
            if let Err(err) = std::fs::write(&tasks_path, rendered) {
                std::fs::write(&tasks_path, &existing)
                    .with_context(|| format!("restore {}", tasks_path.display()))?;
                return Err(err.into());
            }

            match roko_cli::task_parser::TasksFile::validate_modern_fields(&tasks_path) {
                Ok(issues) if !issues.is_empty() => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    anyhow::bail!(
                        "regenerated tasks.toml is missing modern fields: {}",
                        issues
                            .into_iter()
                            .map(|issue| format!("{}: {:?}", issue.task_id, issue.missing_fields))
                            .collect::<Vec<_>>()
                            .join("; ")
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            }

            Ok(EXIT_SUCCESS)
        }
    }
}

// -----------------------------------------------------------------------
// Existing subcommand handlers (init, run, status, replay)
// -----------------------------------------------------------------------

async fn cmd_research(cli: &Cli, cmd: ResearchCmd) -> Result<i32> {
    use roko_cli::agent_exec::{AgentExecOpts, load_gateway_env, model_from_config, run_agent};
    use roko_cli::research::{
        ResearchMode, build_research_prompt, build_research_prompt_gemini,
        build_research_prompt_perplexity, grounding_to_citations, save_research_with_grounding,
    };

    let workdir = resolve_workdir(cli);
    roko_cli::research::ensure_dirs(&workdir)?;
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let config = load_roko_config(&workdir).unwrap_or_default();

    match cmd {
        ResearchCmd::Topic { topic, deep } => {
            let topic = topic.join(" ");
            println!("🔬 Researching: {topic}");

            // --deep: use PerplexityDeepResearchAgent (sonar-deep-research, async polling)
            if deep {
                use roko_agent::agent::Agent as _;
                use roko_agent::perplexity::PerplexityDeepResearchAgent;
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

                let api_key =
                    std::env::var("PERPLEXITY_API_KEY").context("PERPLEXITY_API_KEY not set")?;
                let model_slug = config
                    .perplexity
                    .default_research_model
                    .clone()
                    .unwrap_or_else(|| "sonar-deep-research".to_string());

                let (combined_prompt, _) = build_research_prompt_perplexity(
                    &workdir,
                    &topic,
                    "",
                    ResearchMode::Topic,
                    &config.perplexity,
                );

                let agent = PerplexityDeepResearchAgent::new(
                    api_key,
                    "https://api.perplexity.ai",
                    &model_slug,
                    format!("perplexity:{model_slug}"),
                );
                println!("⏳ Deep research submitted ({model_slug}). This takes 1-10 min...");

                let input = roko_core::Signal::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();

                let mut handle =
                    tokio::spawn(async move { agent.run(&input, &Context::now()).await });
                let poll_started = std::time::Instant::now();
                let result = loop {
                    tokio::select! {
                        r = &mut handle => break r.context("agent task panicked")?,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                            let elapsed = poll_started.elapsed().as_secs();
                            println!("  ⏳ Still researching... ({elapsed}s elapsed)");
                        }
                    }
                };

                if !result.success {
                    let err_text = result.output.body.as_text().unwrap_or("unknown error");
                    anyhow::bail!("Deep research failed: {err_text}");
                }

                let content = result
                    .output
                    .body
                    .as_text()
                    .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                    .to_string();

                let citations: Vec<String> = result
                    .output
                    .tag("pplx_meta")
                    .and_then(|meta_json| {
                        serde_json::from_str::<PerplexityMetadata>(meta_json)
                            .ok()
                            .map(|m| m.citations)
                    })
                    .unwrap_or_default();

                let mut output = content;
                if !citations.is_empty() {
                    output.push_str("\n\n## Sources\n\n");
                    for (i, url) in citations.iter().enumerate() {
                        let _ = writeln!(output, "{}. {url}", i + 1);
                    }
                }

                let slug = topic.to_lowercase().replace(' ', "-");
                let out_path = workdir
                    .join(".roko/research")
                    .join(format!("{slug}-deep.md"));
                std::fs::write(&out_path, &output)
                    .with_context(|| format!("write {}", out_path.display()))?;
                println!("📄 Saved: {}", out_path.display());
                if !citations.is_empty() {
                    println!("📚 {} citations", citations.len());
                }
                return Ok(0);
            }

            // If Perplexity is configured, use PerplexityChatAgent for search-grounded research.
            if let Some(model_slug) = config.gemini.grounding_model.clone() {
                use roko_agent::agent::Agent as _;
                use roko_agent::gemini::{GeminiMetadata, GeminiNativeAgent};
                use roko_agent::provider::AgentOptions;
                use roko_core::Body;
                use roko_core::config::schema::ModelProfile;

                let (combined_prompt, enable_grounding) = build_research_prompt_gemini(
                    &workdir,
                    &topic,
                    ResearchMode::Topic,
                    &config.gemini,
                );
                if enable_grounding {
                    let configured_profile = config.models.get(&model_slug).cloned();
                    let provider = configured_profile
                        .as_ref()
                        .and_then(|profile| config.providers.get(&profile.provider))
                        .or_else(|| config.providers.get("gemini"));
                    let api_key_env = provider
                        .and_then(|provider| provider.api_key_env.clone())
                        .unwrap_or_else(|| "GEMINI_API_KEY".to_string());
                    let api_key = std::env::var(&api_key_env)
                        .with_context(|| format!("{api_key_env} not set"))?;
                    let base_url = provider
                        .and_then(|provider| provider.base_url.clone())
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
                    let timeout_ms = provider
                        .and_then(|provider| provider.timeout_ms)
                        .unwrap_or(300_000);

                    let mut model_profile = configured_profile.unwrap_or_else(|| ModelProfile {
                        provider: "gemini".to_string(),
                        slug: model_slug.clone(),
                        context_window: 1_048_576,
                        max_output: Some(65_536),
                        supports_tools: true,
                        supports_thinking: true,
                        supports_vision: false,
                        supports_web_search: false,
                        supports_mcp_tools: false,
                        supports_partial: false,
                        supports_grounding: true,
                        supports_code_execution: false,
                        supports_caching: false,
                        provider_routing: None,
                        tool_format: "gemini_native".to_string(),
                        cost_input_per_m: None,
                        cost_output_per_m: None,
                        cost_input_per_m_high: None,
                        cost_output_per_m_high: None,
                        cost_cache_read_per_m: None,
                        cost_cache_write_per_m: None,
                        thinking_level: Some(config.gemini.thinking_level.clone()),
                        max_tools: None,
                        tokenizer_ratio: None,
                        supports_search: false,
                        supports_citations: false,
                        supports_async: false,
                        is_embedding_model: false,
                        search_context_size: None,
                        cost_per_request: None,
                    });
                    model_profile.supports_grounding = true;
                    model_profile.tool_format = "gemini_native".to_string();
                    if model_profile.thinking_level.is_none() {
                        model_profile.thinking_level = Some(config.gemini.thinking_level.clone());
                    }

                    let agent = GeminiNativeAgent::new(
                        api_key,
                        base_url,
                        model_profile,
                        &AgentOptions {
                            timeout_ms: Some(timeout_ms),
                            effort: Some(config.gemini.thinking_level.clone()),
                            name: format!("gemini:{model_slug}"),
                            ..Default::default()
                        },
                    );

                    let input = roko_core::Signal::builder(Kind::Prompt)
                        .body(Body::text(&combined_prompt))
                        .build();
                    let result = agent.run(&input, &Context::now()).await;

                    if !result.success {
                        let err_text = result.output.body.as_text().unwrap_or("unknown error");
                        anyhow::bail!("Gemini research failed: {err_text}");
                    }

                    let content = result
                        .output
                        .body
                        .as_text()
                        .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                        .to_string();

                    let grounding = result
                        .output
                        .tag("gemini_meta")
                        .and_then(|meta_json| {
                            serde_json::from_str::<GeminiMetadata>(meta_json).ok()
                        })
                        .and_then(|metadata| metadata.grounding_metadata);

                    let out_path = if let Some(grounding) = &grounding {
                        save_research_with_grounding(&workdir, &topic, &content, grounding)?
                    } else {
                        let slug = topic.to_lowercase().replace(' ', "-");
                        let out_path = workdir.join(".roko/research").join(format!("{slug}.md"));
                        std::fs::write(&out_path, &content)
                            .with_context(|| format!("write {}", out_path.display()))?;
                        out_path
                    };

                    println!("📄 Saved: {}", out_path.display());
                    if let Some(grounding) = &grounding {
                        let citations = grounding_to_citations(grounding);
                        if !citations.is_empty() {
                            println!("📚 {} citations", citations.len());
                        }
                    }
                    return Ok(0);
                }
            }

            if let Some(model_slug) = config.perplexity.default_search_model.clone() {
                use roko_agent::agent::Agent as _;
                use roko_agent::perplexity::PerplexityChatAgent;
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

                let api_key =
                    std::env::var("PERPLEXITY_API_KEY").context("PERPLEXITY_API_KEY not set")?;
                let (combined_prompt, search_opts) = build_research_prompt_perplexity(
                    &workdir,
                    &topic,
                    "",
                    ResearchMode::Topic,
                    &config.perplexity,
                );
                let agent = PerplexityChatAgent::new(
                    api_key,
                    "https://api.perplexity.ai",
                    &model_slug,
                    format!("perplexity:{model_slug}"),
                    300_000,
                )
                .with_search_options(search_opts);

                let input = roko_core::Signal::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();
                let result = agent.run(&input, &Context::now()).await;

                if !result.success {
                    let err_text = result.output.body.as_text().unwrap_or("unknown error");
                    anyhow::bail!("Perplexity research failed: {err_text}");
                }

                let content = result
                    .output
                    .body
                    .as_text()
                    .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                    .to_string();

                let citations: Vec<String> = result
                    .output
                    .tag("pplx_meta")
                    .and_then(|meta_json| {
                        serde_json::from_str::<PerplexityMetadata>(meta_json)
                            .ok()
                            .map(|m| m.citations)
                    })
                    .unwrap_or_default();

                let mut output = content;
                if !citations.is_empty() {
                    output.push_str("\n\n## Sources\n\n");
                    for (i, url) in citations.iter().enumerate() {
                        let _ = writeln!(output, "{}. {url}", i + 1);
                    }
                }

                let slug = topic.to_lowercase().replace(' ', "-");
                let out_path = workdir.join(".roko/research").join(format!("{slug}.md"));
                std::fs::write(&out_path, &output)
                    .with_context(|| format!("write {}", out_path.display()))?;
                println!("📄 Saved: {}", out_path.display());
                if !citations.is_empty() {
                    println!("📚 {} citations", citations.len());
                }
                return Ok(0);
            }

            // Claude CLI fallback
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
        ResearchCmd::Search {
            query,
            domains,
            recency,
        } => {
            use roko_agent::perplexity::search::{PerplexitySearchClient, SearchQuery};

            let query_str = query.join(" ");
            if query_str.trim().is_empty() {
                anyhow::bail!("provide a search query");
            }

            let api_key =
                std::env::var("PERPLEXITY_API_KEY").context("PERPLEXITY_API_KEY not set")?;

            let date_range = recency.as_deref().map(|r| {
                let now = chrono::Local::now();
                let after = match r {
                    "day" => now - chrono::Duration::days(1),
                    "week" => now - chrono::Duration::weeks(1),
                    "month" => now - chrono::Duration::days(30),
                    "year" => now - chrono::Duration::days(365),
                    _ => now - chrono::Duration::days(30),
                };
                (
                    after.format("%Y-%m-%d").to_string(),
                    now.format("%Y-%m-%d").to_string(),
                )
            });

            let search_query = SearchQuery {
                query: query_str.clone(),
                domain_filter: if domains.is_empty() {
                    None
                } else {
                    Some(domains)
                },
                date_range,
                ..Default::default()
            };

            println!("🔍 Searching: {query_str}");

            let client = PerplexitySearchClient::new(api_key);
            let responses = client
                .search_batch(&[search_query])
                .await
                .map_err(|e| anyhow::anyhow!("search error: {e}"))?;

            let results: Vec<_> = responses.into_iter().flat_map(|r| r.results).collect();

            if results.is_empty() {
                println!("No results found.");
            } else {
                println!("\n═══ Results ═══\n");
                for (i, r) in results.iter().enumerate() {
                    println!("{}. {}", i + 1, r.title);
                    println!("   {}", r.url);
                    if let Some(date) = &r.date {
                        println!("   Published: {date}");
                    }
                    let snippet = if r.content.len() > 300 {
                        format!("{}…", &r.content[..300])
                    } else {
                        r.content.clone()
                    };
                    println!("   {snippet}");
                    println!();
                }
            }

            Ok(0)
        }
    }
}

async fn cmd_neuro(cli: &Cli, cmd: NeuroCmd) -> Result<i32> {
    match cmd {
        NeuroCmd::Query { topic, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let topic = topic.join(" ");
            let topic = topic.trim().to_string();
            if topic.is_empty() {
                anyhow::bail!("provide a topic to query");
            }

            let store = KnowledgeStore::for_workdir(&wd);
            let entries = store.query(&topic, 10).with_context(|| {
                format!(
                    "query knowledge store at {} for topic '{topic}'",
                    store.path().display()
                )
            })?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "topic": topic,
                    "count": entries.len(),
                    "entries": entries,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!(
                "Knowledge matches for '{topic}' in {}:",
                store.path().display()
            );
            if entries.is_empty() {
                println!("  (no matches)");
                return Ok(EXIT_SUCCESS);
            }

            for (idx, entry) in entries.iter().enumerate() {
                println!(
                    "{}. [{}] confidence {:.2} {}",
                    idx + 1,
                    format!("{:?}", entry.kind).to_lowercase(),
                    entry.confidence.clamp(0.0, 1.0),
                    entry.content.trim()
                );
                if !entry.tags.is_empty() {
                    println!("   tags: {}", entry.tags.join(", "));
                }
                if !entry.source_episodes.is_empty() {
                    println!("   sources: {}", entry.source_episodes.join(", "));
                }
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Stats { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);
            let stats = store.stats().with_context(|| {
                format!("read knowledge store stats from {}", store.path().display())
            })?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "path": store.path(),
                    "stats": stats,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Knowledge stats for {}:", store.path().display());
            println!("  total entries: {}", stats.total_entries);
            println!(
                "  average confidence: {}",
                stats
                    .average_confidence
                    .map(|confidence| format!("{confidence:.3}"))
                    .unwrap_or_else(|| "n/a".to_owned())
            );
            println!("  entries by kind:");
            if stats.kind_counts.is_empty() {
                println!("    (empty)");
            } else {
                for (kind, count) in &stats.kind_counts {
                    println!("    {kind:<16} {count}");
                }
            }

            match stats.oldest_entry.as_ref() {
                Some(entry) => {
                    println!(
                        "  oldest entry: {} [{}] confidence {:.3} created {}",
                        entry.id,
                        format!("{:?}", entry.kind).to_lowercase(),
                        entry.confidence.clamp(0.0, 1.0),
                        entry.created_at
                    );
                }
                None => println!("  oldest entry: (none)"),
            }

            match stats.newest_entry.as_ref() {
                Some(entry) => {
                    println!(
                        "  newest entry: {} [{}] confidence {:.3} created {}",
                        entry.id,
                        format!("{:?}", entry.kind).to_lowercase(),
                        entry.confidence.clamp(0.0, 1.0),
                        entry.created_at
                    );
                }
                None => println!("  newest entry: (none)"),
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Gc { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);
            let before = store.stats().with_context(|| {
                format!("read knowledge store stats from {}", store.path().display())
            })?;
            store.gc(DEFAULT_GC_MIN_CONFIDENCE).with_context(|| {
                format!(
                    "garbage collect knowledge store at {}",
                    store.path().display()
                )
            })?;
            let after = store.stats().with_context(|| {
                format!(
                    "read knowledge store stats from {} after gc",
                    store.path().display()
                )
            })?;
            let removed = before.total_entries.saturating_sub(after.total_entries);

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "path": store.path(),
                    "threshold": DEFAULT_GC_MIN_CONFIDENCE,
                    "before": before.total_entries,
                    "after": after.total_entries,
                    "removed": removed,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Knowledge GC for {}:", store.path().display());
            println!("  threshold: {:.3}", DEFAULT_GC_MIN_CONFIDENCE);
            println!("  before: {}", before.total_entries);
            println!("  after: {}", after.total_entries);
            println!("  removed entries: {}", removed);

            Ok(EXIT_SUCCESS)
        }
    }
}

async fn cmd_subscription(cli: &Cli, cmd: SubscriptionCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    match cmd {
        SubscriptionCmd::List => {
            roko_cli::subscriptions::cmd_list(&workdir, cli.json)?;
            Ok(EXIT_SUCCESS)
        }
        SubscriptionCmd::Add { template, trigger } => {
            roko_cli::subscriptions::cmd_add(&workdir, &template, &trigger)?;
            Ok(EXIT_SUCCESS)
        }
        SubscriptionCmd::Remove { id } => {
            roko_cli::subscriptions::cmd_remove(&workdir, &id)?;
            Ok(EXIT_SUCCESS)
        }
        SubscriptionCmd::Enable { id } => {
            roko_cli::subscriptions::cmd_set_enabled(&workdir, &id, true)?;
            Ok(EXIT_SUCCESS)
        }
        SubscriptionCmd::Disable { id } => {
            roko_cli::subscriptions::cmd_set_enabled(&workdir, &id, false)?;
            Ok(EXIT_SUCCESS)
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

fn find_plan_source_document(plan_dir: &Path) -> Result<PathBuf> {
    for candidate in ["source-prd.md", "prd-extract.md", "plan.md"] {
        let path = plan_dir.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "no source PRD found in {} (looked for source-prd.md, prd-extract.md, and plan.md)",
        plan_dir.display()
    )
}

fn normalize_task_title(title: &str) -> String {
    title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn preserve_completed_task_status(
    old_tasks: Option<&roko_cli::task_parser::TasksFile>,
    mut regenerated: roko_cli::task_parser::TasksFile,
    plan_dir: &Path,
) -> roko_cli::task_parser::TasksFile {
    if let Some(old_tasks) = old_tasks {
        let completed: Vec<&roko_cli::task_parser::TaskDef> = old_tasks
            .tasks
            .iter()
            .filter(|task| task.status.eq_ignore_ascii_case("done"))
            .collect();

        for task in &mut regenerated.tasks {
            let normalized = normalize_task_title(&task.title);
            if completed.iter().any(|old| {
                old.id == task.id
                    || normalize_task_title(&old.title) == normalized
                    || normalize_task_title(&old.title).contains(&normalized)
                    || normalized.contains(&normalize_task_title(&old.title))
            }) {
                task.status = "done".to_string();
            }
        }

        regenerated.meta.iteration = old_tasks.meta.iteration.saturating_add(1);
        if regenerated.meta.plan.trim().is_empty() {
            regenerated.meta.plan = old_tasks.meta.plan.clone();
        }
    }

    if regenerated.meta.plan.trim().is_empty() {
        regenerated.meta.plan = plan_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown-plan".to_string());
    }

    regenerated.meta.total = regenerated.tasks.len() as u32;
    regenerated.meta.done = regenerated
        .tasks
        .iter()
        .filter(|task| task.status.eq_ignore_ascii_case("done"))
        .count() as u32;
    regenerated.meta.status =
        if regenerated.meta.total > 0 && regenerated.meta.done == regenerated.meta.total {
            "complete".to_string()
        } else {
            "ready".to_string()
        };

    regenerated
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
            PrdDraftCmd::Promote { slug, auto_execute } => {
                roko_cli::prd::cmd_promote(&workdir, &slug, auto_execute).await?;
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
        PrdCmd::Plan { slug, dry_run } => {
            let prd_path = find_prd(&workdir, &slug)?;
            let _generated_plans_root =
                roko_cli::prd::generate_plan_from_prd(&slug, &prd_path, dry_run).await?;
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

async fn cmd_init(path: Option<PathBuf>, cloud: bool) -> Result<()> {
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
        let default = Config::default_toml_template(cloud)?;
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

async fn cmd_status(cli: &Cli, workdir: Option<PathBuf>, cfactor: bool) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let ctx = Context::now();

    let all = substrate
        .query(&Query::all(), &ctx)
        .await
        .map_err(|e| anyhow!("query: {e}"))?;

    let cfactor_snapshot = if cfactor {
        Some(
            refresh_cfactor_snapshot(workdir.join(".roko").join("learn"))
                .await
                .map_err(|e| anyhow!("refresh c-factor snapshot: {e}"))?,
        )
    } else {
        None
    };
    let cfactor_history = if cfactor_snapshot.is_some() {
        load_cfactor_history(workdir.join(".roko").join("learn").join("c-factor.jsonl")).await
    } else {
        Vec::new()
    };
    let cfactor_trend = if cfactor_snapshot.is_some() {
        cfactor_trend_arrow(&cfactor_history, Duration::from_secs(7 * 24 * 60 * 60))
    } else {
        "→"
    };

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
            cfactor: cfactor_snapshot,
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

    if let Some(cfactor) = cfactor_snapshot {
        println!();
        println!(
            "c-factor: {:.3} | trend={} | episodes={} | computed={}",
            cfactor.overall, cfactor_trend, cfactor.episode_count, cfactor.computed_at
        );
        println!(
            "  gate={:.3} cost={:.3} speed={:.3} flow={:.3} first_try={:.3} knowledge={:.3} integration={:.3} convergence={:.3} turn={:.3} social={:.3}",
            cfactor.components.gate_pass_rate,
            cfactor.components.cost_efficiency,
            cfactor.components.speed,
            cfactor.components.information_flow_rate,
            cfactor.components.first_try_rate,
            cfactor.components.knowledge_growth,
            cfactor.components.knowledge_integration_rate,
            cfactor.components.convergence_velocity,
            cfactor.components.turn_taking_equality,
            cfactor.components.social_sensitivity
        );
        if !cfactor.agent_contributions.is_empty() {
            println!(
                "  agent contributions: {}",
                cfactor.top_agent_contribution_lines(3).join(", ")
            );
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

async fn cmd_dream(cli: &Cli, cmd: DreamCmd) -> Result<i32> {
    match cmd {
        DreamCmd::Run { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            prepare_runtime_hooks(&workdir, cli.quiet);

            let mut runner = build_dream_runner(cli, &workdir)?;
            let report = match runner.consolidate() {
                Ok(report) => report,
                Err(e) => {
                    // Appraise dream failure into the daimon affect state.
                    use roko_daimon::{AffectEngine as _, AffectEvent, DaimonState};
                    let daimon_path = workdir.join(".roko").join("daimon").join("affect.json");
                    let mut daimon = DaimonState::load_or_new(&daimon_path);
                    let _ = daimon.appraise(AffectEvent::DreamFailure {
                        task_type: "consolidation".to_string(),
                        failure_count: 1,
                    });
                    return Err(e);
                }
            };
            let cfactor_snapshot = refresh_cfactor_snapshot(workdir.join(".roko").join("learn"))
                .await
                .map_err(|e| anyhow!("refresh c-factor snapshot: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if !cli.quiet {
                println!(
                    "dream cycle completed: {} episodes, {} clusters, {} knowledge entries, {} playbooks",
                    report.processed_episodes,
                    report.clusters.len(),
                    report.knowledge_entries_written,
                    report.playbooks_created
                );
                if let Some(processed_through) = report.processed_through {
                    println!("processed through: {processed_through}");
                }
                println!(
                    "report saved under: {}",
                    workdir.join(".roko").join("dreams").display()
                );
                println!("c-factor: {:.3}", cfactor_snapshot.overall);
            }

            Ok(EXIT_SUCCESS)
        }
        DreamCmd::Report { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &workdir)?;
            let report = runner.latest_report()?.ok_or_else(|| {
                anyhow!(
                    "no dream report found in {}",
                    workdir.join(".roko").join("dreams").display()
                )
            })?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "dream report: {} episodes, {} clusters, {} knowledge entries, {} playbooks",
                    report.processed_episodes,
                    report.clusters.len(),
                    report.knowledge_entries_written,
                    report.playbooks_created
                );
                println!("started: {}", report.started_at);
                println!("completed: {}", report.completed_at);
                if let Some(processed_through) = report.processed_through {
                    println!("processed through: {processed_through}");
                }
            }
            Ok(EXIT_SUCCESS)
        }
        DreamCmd::Schedule { workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &workdir)?;
            let schedule = runner.schedule_next();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "next_fire_seconds": schedule.map(|duration| duration.as_secs())
                    })
                );
            } else if let Some(duration) = schedule {
                println!("next dream in {:?}", duration);
            } else {
                println!("no dream scheduled");
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

fn build_dream_runner(cli: &Cli, workdir: &Path) -> Result<DreamRunner> {
    let cli_config = resolve_config_for_workdir(cli, workdir)?;
    Ok(DreamRunner::new(
        workdir.to_path_buf(),
        DreamLoopConfig {
            auto_dream: cli_config.dreams.auto_dream,
            idle_threshold_mins: cli_config.dreams.idle_threshold_mins,
            min_episodes_for_dream: cli_config.dreams.min_episodes_for_dream,
            agent: DreamAgentConfig {
                command: cli_config.agent.command.clone(),
                args: cli_config.agent.args.clone(),
                model: cli_config.agent.model.clone(),
                bare_mode: cli_config.agent.bare_mode,
                effort: cli_config.agent.effort.clone(),
                fallback_model: cli_config.agent.fallback_model.clone(),
                timeout_ms: cli_config.agent.timeout_ms,
                env: cli_config.agent.env.clone(),
            },
        },
    ))
}

async fn cmd_deploy(cli: &Cli, cmd: DeployCmd) -> Result<i32> {
    match cmd {
        DeployCmd::Railway { workdir } => cmd_deploy_railway(cli, workdir).await,
        DeployCmd::Fly { workdir } => cmd_deploy_fly(cli, workdir).await,
        DeployCmd::Docker { workdir, registry } => cmd_deploy_docker(cli, workdir, registry).await,
    }
}

async fn cmd_deploy_fly(cli: &Cli, workdir: Option<PathBuf>) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));

    write_fly_toml(&workdir)?;
    run_command_status(&workdir, "flyctl", &["deploy", "--remote-only"])?;

    Ok(EXIT_SUCCESS)
}

async fn cmd_deploy_docker(
    cli: &Cli,
    workdir: Option<PathBuf>,
    registry: Option<String>,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let registry = resolve_docker_registry(&workdir, registry)?;
    let tagged_image = format!("{registry}/roko:latest");

    run_command_status(&workdir, "docker", &["build", "-t", "roko", "."])?;
    run_command_status(&workdir, "docker", &["tag", "roko:latest", &tagged_image])?;

    Ok(EXIT_SUCCESS)
}

async fn cmd_deploy_railway(cli: &Cli, workdir: Option<PathBuf>) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    run_release_build(&workdir).await?;

    let config = load_roko_config(&workdir)?;
    let deploy_webhooks = match load_layered(&workdir) {
        Ok(resolved) => resolved.config.serve.deploy.webhooks,
        Err(err) => {
            warn!(
                error = %err,
                "failed to load configured deploy webhooks; skipping GitHub webhook registration"
            );
            Vec::new()
        }
    };
    let deploy_config = &config.deploy;
    let token = deploy_config
        .railway_api_token
        .as_deref()
        .ok_or_else(|| anyhow!("deploy.railway_api_token is required for Railway deployment"))?;
    let repo_slug = git_remote_slug(&workdir)?;
    let branch =
        git_current_branch(&workdir).unwrap_or_else(|_| config.project.fresh_base_branch.clone());
    let env_vars = collect_railway_env_vars();
    let backend = roko_serve::deploy::railway_api::RailwayApiBackend::new(
        token.to_string(),
        deploy_config.project_id.clone(),
        deploy_config.environment_id.clone(),
    );

    let deployment = backend
        .deploy_roko_app(&roko_serve::deploy::railway_api::RailwayDeploySpec {
            project_name: config.project.name.clone(),
            project_id: deploy_config.project_id.clone(),
            environment_id: deploy_config.environment_id.clone(),
            service_name: "roko".to_string(),
            repo_slug,
            branch,
            dockerfile_path: "docker/roko.Dockerfile".to_string(),
            root_directory: ".".to_string(),
            healthcheck_path: "/api/health".to_string(),
            volume_mount_path: "/workspace/.roko".to_string(),
            region: deploy_config.default_region.clone(),
            env_vars,
        })
        .await?;

    let url = deployment
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("Railway deployment finished without a public URL"))?;

    register_deployment_github_webhooks(&deploy_webhooks, url, &config.webhooks.github.secret)
        .await?;

    println!("{url}");
    Ok(EXIT_SUCCESS)
}

fn write_fly_toml(workdir: &Path) -> Result<PathBuf> {
    let path = workdir.join("fly.toml");
    std::fs::write(&path, FLY_TOML_TEMPLATE)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))
}

fn resolve_docker_registry(workdir: &Path, registry: Option<String>) -> Result<String> {
    if let Some(registry) = registry {
        let registry = registry.trim().trim_end_matches('/');
        if registry.is_empty() {
            bail!("deploy.docker.registry cannot be empty");
        }
        return Ok(registry.to_string());
    }

    let config = load_roko_config(workdir)?;
    let worker_image =
        config.deploy.worker_image.as_deref().ok_or_else(|| {
            anyhow!("deploy.docker.registry is required or set deploy.worker_image")
        })?;

    let registry = worker_image
        .rsplit_once('/')
        .map(|(registry, _)| registry)
        .filter(|registry| !registry.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("unable to derive Docker registry from deploy.worker_image: {worker_image}")
        })?;

    Ok(registry.trim().trim_end_matches('/').to_string())
}

async fn run_release_build(workdir: &Path) -> Result<()> {
    let workdir = workdir.to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("cargo")
            .args(["build", "--release", "-p", "roko-cli"])
            .current_dir(&workdir)
            .output()
    })
    .await
    .context("join cargo build task")?
    .context("run cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "cargo build --release -p roko-cli failed: {}",
            stderr.trim()
        );
    }

    Ok(())
}

async fn register_deployment_github_webhooks(
    webhooks: &[ServeDeployWebhookConfig],
    webhook_url: &str,
    secret: &str,
) -> Result<()> {
    if webhooks.is_empty() {
        return Ok(());
    }

    if secret.trim().is_empty() {
        warn!("github webhook secret is not configured; skipping webhook registration");
        return Ok(());
    }

    let token = match env::var("GITHUB_TOKEN").or_else(|_| env::var("GH_TOKEN")) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            warn!("github token is not configured; skipping webhook registration");
            return Ok(());
        }
    };

    let github = Octocrab::builder()
        .personal_token(token)
        .build()
        .context("build GitHub client")?;

    let mut registered = 0usize;
    for webhook in webhooks {
        if webhook.provider != "github" {
            warn!(
                provider = %webhook.provider,
                owner = %webhook.owner,
                repo = %webhook.repo,
                "skipping non-GitHub deploy webhook registration"
            );
            continue;
        }

        if webhook.owner.trim().is_empty() || webhook.repo.trim().is_empty() {
            warn!(
                provider = %webhook.provider,
                owner = %webhook.owner,
                repo = %webhook.repo,
                "skipping deploy webhook with empty repository coordinates"
            );
            continue;
        }

        match register_github_webhook(
            &github,
            webhook.owner.trim(),
            webhook.repo.trim(),
            webhook_url,
            secret,
        )
        .await
        {
            Ok(()) => {
                registered += 1;
            }
            Err(err) => {
                warn!(
                    owner = %webhook.owner,
                    repo = %webhook.repo,
                    error = %err,
                    "failed to register GitHub webhook"
                );
            }
        }
    }

    if registered > 0 {
        info!(count = registered, "registered GitHub webhooks");
    }

    Ok(())
}

async fn register_github_webhook(
    github: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    webhook_url: &str,
    secret: &str,
) -> Result<()> {
    let webhook_endpoint = format!("{}/webhooks/github", webhook_url.trim_end_matches('/'));

    let existing_hooks: Vec<Hook> = github
        .get(format!("/repos/{owner}/{repo}/hooks"), None::<&()>)
        .await
        .with_context(|| format!("list GitHub webhooks for {owner}/{repo}"))?;

    if existing_hooks
        .iter()
        .any(|hook| hook.name == "web" && hook.config.url == webhook_endpoint)
    {
        info!(owner = %owner, repo = %repo, "GitHub webhook already registered");
        return Ok(());
    }

    let hook = Hook {
        name: "web".to_string(),
        active: true,
        events: vec![
            WebhookEventType::Push,
            WebhookEventType::PullRequest,
            WebhookEventType::Issues,
            WebhookEventType::IssueComment,
            WebhookEventType::PullRequestReview,
            WebhookEventType::CheckRun,
        ],
        config: HookConfig {
            url: webhook_endpoint,
            content_type: Some(ContentType::Json),
            insecure_ssl: None,
            secret: Some(secret.to_string()),
        },
        ..Hook::default()
    };

    github
        .repos(owner, repo)
        .create_hook(hook)
        .await
        .with_context(|| format!("create GitHub webhook for {owner}/{repo}"))?;

    Ok(())
}

fn git_remote_slug(workdir: &Path) -> Result<String> {
    let remote = run_command_output(workdir, "git", &["remote", "get-url", "origin"])?;
    let remote = remote.trim();
    let slug = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .ok_or_else(|| anyhow!("origin remote is not a GitHub URL: {remote}"))?
        .trim_end_matches(".git")
        .to_string();

    if slug.split('/').count() != 2 {
        return Err(anyhow!(
            "invalid GitHub repo slug derived from origin: {slug}"
        ));
    }

    Ok(slug)
}

fn git_current_branch(workdir: &Path) -> Result<String> {
    let branch = run_command_output(workdir, "git", &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        bail!("unable to determine current git branch");
    }
    Ok(branch.to_string())
}

fn collect_railway_env_vars() -> std::collections::HashMap<String, String> {
    const NAMES: &[&str] = &[
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "SLACK_TOKEN",
        "SLACK_BOT_TOKEN",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "ROKO_SERVER_AUTH_TOKEN",
    ];

    let mut vars = std::collections::HashMap::new();
    for name in NAMES {
        if let Ok(value) = env::var(name) {
            if !value.trim().is_empty() {
                vars.insert((*name).to_string(), value);
            }
        }
    }
    vars
}

fn run_command_status(workdir: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(workdir)
        .status()
        .with_context(|| format!("run {program} {}", args.join(" ")))?;

    if !status.success() {
        bail!("{program} {} failed with status {status}", args.join(" "));
    }

    Ok(())
}

fn run_command_output(workdir: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(workdir)
        .output()
        .with_context(|| format!("run {program} {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program} {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

const FLY_TOML_TEMPLATE: &str = r#"app = "roko-agent"
primary_region = "iad"

[build]
dockerfile = "Dockerfile"

[http_service]
internal_port = 3000
force_https = true

[[http_service.checks]]
interval = "30s"
timeout = "5s"
path = "/api/health"
method = "GET"

[mounts]
source = "roko_data"
destination = "/data/.roko"
"#;

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
    let (mut config, repo_base) = if let Some(p) = &cli.config {
        (Config::from_file(p)?, p.parent().unwrap_or(workdir))
    } else {
        let resolved = load_layered(workdir)?;
        let fully_default = resolved.sources.agent_command == Source::Default
            && resolved.sources.prompt_token_budget == Source::Default;
        if fully_default && resolved.config.agent.command == "cat" && !cli.quiet {
            println!(
                "no config found — using built-in `cat` agent. run `roko config init` to set up a model."
            );
        }
        (resolved.config, workdir)
    };

    // Validate and load any configured additional repos even when bypassing
    // layered config resolution via `--config`.
    let _repo_registry = RepoRegistry::load(&config, repo_base)?;

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
        "provider-health" | "providerhealth" => PageId::ProviderHealth,
        "model-comparison" | "modelcomparison" => PageId::ModelComparison,
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
        PageId::ProviderHealth,
        PageId::ModelComparison,
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

    // 1. Global: ~/.roko/.env — lower priority, does NOT override existing env vars.
    if let Some(home) = env::var_os("HOME") {
        let global_env = PathBuf::from(home).join(".roko").join(".env");
        if global_env.is_file() {
            redactions.extend(load_env_file(&global_env)?);
            dotenvy::from_path(&global_env)
                .with_context(|| format!("load {}", global_env.display()))?;
        }
    }

    // 2. Project-local: {workdir}/.roko/.env — higher priority, overrides existing vars.
    //    At this point the CLI hasn't parsed yet, so workdir == cwd.
    let local_env = PathBuf::from(".roko").join(".env");
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
        match cli.command {
            Some(Command::Init { path, cloud }) => {
                assert_eq!(path, Some(PathBuf::from("/tmp/project")));
                assert!(!cloud);
            }
            other => panic!("expected init command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_init_cloud_flag() {
        let cli = Cli::try_parse_from(["roko", "init", "--cloud"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Init { cloud: true, .. })
        ));
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
        let cli = Cli::try_parse_from(["roko", "plan", "run", "plans", "--resume-plan"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Plan {
                cmd: PlanCmd::Run {
                    resume_plan: Some(_),
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
    fn cli_parses_deploy_railway_subcommand() {
        let cli = Cli::try_parse_from(["roko", "deploy", "railway"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Deploy {
                cmd: DeployCmd::Railway { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_deploy_fly_subcommand() {
        let cli = Cli::try_parse_from(["roko", "deploy", "fly"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Deploy {
                cmd: DeployCmd::Fly { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_deploy_docker_subcommand() {
        let cli = Cli::try_parse_from(["roko", "deploy", "docker"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Deploy {
                cmd: DeployCmd::Docker { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_neuro_query_subcommand() {
        let cli = Cli::try_parse_from(["roko", "neuro", "query", "rust async"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Neuro {
                cmd: NeuroCmd::Query { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_neuro_stats_subcommand() {
        let cli = Cli::try_parse_from(["roko", "neuro", "stats"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Neuro {
                cmd: NeuroCmd::Stats { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_neuro_gc_subcommand() {
        let cli = Cli::try_parse_from(["roko", "neuro", "gc"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Neuro {
                cmd: NeuroCmd::Gc { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_experiment_model_create_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "experiment",
            "model",
            "create",
            "--id",
            "glm-vs-kimi-impl",
            "--role",
            "implementer",
            "--variant",
            "glm-5-1:glm-5.1:zai",
            "--variant",
            "kimi-k2-5:kimi-k2.5:moonshot",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Experiment {
                cmd: ExperimentCmd::Model { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_provider_list_subcommand() {
        let cli = Cli::try_parse_from(["roko", "provider", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Provider {
                cmd: ProviderCmd::List { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_provider_health_subcommand() {
        let cli = Cli::try_parse_from(["roko", "provider", "health"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Provider {
                cmd: ProviderCmd::Health { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_provider_test_subcommand() {
        let cli = Cli::try_parse_from(["roko", "provider", "test", "zai"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Provider {
                cmd: ProviderCmd::Test { provider, .. }
            }) if provider == "zai"
        ));
    }

    #[test]
    fn cli_parses_model_list_subcommand() {
        let cli = Cli::try_parse_from(["roko", "model", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Model {
                cmd: ModelCmd::List { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_model_route_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "model",
            "route",
            "glm-5-1",
            "--explain",
            "--role",
            "implementer",
            "--complexity",
            "integrative",
        ])
        .unwrap();
        assert_eq!(cli.role.as_deref(), Some("implementer"));
        assert!(matches!(
            cli.command,
            Some(Command::Model {
                cmd: ModelCmd::Route {
                    model,
                    explain: true,
                    complexity: Some(complexity),
                    ..
                }
            }) if model == "glm-5-1" && complexity == "integrative"
        ));
    }

    #[test]
    fn select_provider_test_model_prefers_default_model() {
        let mut config = RokoConfig::default();
        config.agent.default_model = "glm-5-1".to_string();
        config.models.insert(
            "glm-5-1".to_string(),
            ModelProfile {
                provider: "zai".to_string(),
                slug: "glm-5.1".to_string(),
                context_window: 200_000,
                max_output: Some(131_072),
                supports_tools: true,
                supports_thinking: true,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: Some(1.40),
                cost_output_per_m: Some(4.40),
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );
        config.models.insert(
            "glm-5-1-alt".to_string(),
            ModelProfile {
                provider: "zai".to_string(),
                slug: "glm-5.1-air".to_string(),
                context_window: 128_000,
                max_output: Some(8_192),
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: Some(1.0),
                cost_output_per_m: Some(2.0),
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        let selected = select_provider_test_model(&config, "zai").expect("selected model");
        assert_eq!(selected.0, "glm-5-1");
        assert_eq!(selected.1.slug, "glm-5.1");
    }

    #[test]
    fn format_provider_rows_renders_headers_and_rows() {
        let output = format_provider_rows(&[ProviderListRow {
            provider: "anthropic".to_string(),
            kind: "claude_cli".to_string(),
            base_url: "(cli: claude)".to_string(),
            status: "ok (cli found)".to_string(),
        }]);

        assert!(output.contains("Provider"));
        assert!(output.contains("Base URL"));
        assert!(output.contains("anthropic"));
        assert!(output.contains("ok (cli found)"));
    }

    #[test]
    fn format_model_rows_renders_headers_and_rows() {
        let output = format_model_rows(&[ModelListRow {
            model: "glm-5-1".to_string(),
            provider: "zai".to_string(),
            slug: "glm-5.1".to_string(),
            context: "200K".to_string(),
            tools: "✓".to_string(),
            thinking: "✓".to_string(),
            vision: "✗".to_string(),
            cost: "$1.40/$4.40".to_string(),
        }]);

        assert!(output.contains("Model"));
        assert!(output.contains("Cost (in/out)"));
        assert!(output.contains("glm-5-1"));
        assert!(output.contains("$1.40/$4.40"));
    }

    #[test]
    fn build_model_list_row_formats_capabilities_and_costs() {
        let row = build_model_list_row(
            "kimi-k2-5",
            &ModelProfile {
                provider: "moonshot".to_string(),
                slug: "kimi-k2.5".to_string(),
                context_window: 256_000,
                max_output: Some(128_000),
                supports_tools: true,
                supports_thinking: true,
                supports_vision: true,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: Some(0.60),
                cost_output_per_m: Some(3.00),
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        assert_eq!(row.model, "kimi-k2-5");
        assert_eq!(row.provider, "moonshot");
        assert_eq!(row.slug, "kimi-k2.5");
        assert_eq!(row.context, "256K");
        assert_eq!(row.tools, "✓");
        assert_eq!(row.thinking, "✓");
        assert_eq!(row.vision, "✓");
        assert_eq!(row.cost, "$0.60/$3.00");
    }

    #[test]
    fn build_provider_health_row_formats_state_latency_and_error_rate() {
        let health = ProviderHealth {
            provider_id: "zai".to_string(),
            state: CircuitState::Open,
            consecutive_failures: 3,
            total_requests: 20,
            total_failures: 3,
            last_failure_at: Some(90_000),
            cooldown_until: Some(108_000),
            failure_window: std::collections::VecDeque::new(),
        };
        let latency = ProviderLatencySummary {
            recent_latencies: vec![800.0, 1_200.0, 600.0],
            weighted_latency_ms: 0.0,
            observations: 0,
        };

        let row = build_provider_health_row(
            "zai",
            Some(&health),
            Some(&latency),
            100_000,
            Some(95_000),
            Some(99_000),
        );

        assert_eq!(row.provider, "zai");
        assert_eq!(row.state, "OPEN");
        assert_eq!(row.fails, "3/3");
        assert_eq!(row.cooldown, "8s left");
        assert_eq!(row.latency_p50, "0.8s");
        assert_eq!(row.error_rate, "15.0%");
        assert_eq!(row.last_check, "1s ago");
    }

    #[test]
    fn format_provider_health_rows_renders_headers_and_rows() {
        let output = format_provider_health_rows(&[ProviderHealthRow {
            provider: "openrouter".to_string(),
            state: "CLOSED".to_string(),
            fails: "0/3".to_string(),
            cooldown: "—".to_string(),
            latency_p50: "0.8s".to_string(),
            error_rate: "0.0%".to_string(),
            last_check: "5m ago".to_string(),
        }]);

        assert!(output.contains("Provider"));
        assert!(output.contains("Latency p50"));
        assert!(output.contains("Error Rate"));
        assert!(output.contains("openrouter"));
        assert!(output.contains("0.8s"));
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
        assert_eq!(
            parse_dashboard_page("provider health"),
            Some(PageId::ProviderHealth)
        );
        assert_eq!(
            parse_dashboard_page("model comparison"),
            Some(PageId::ModelComparison)
        );
    }

    #[test]
    fn parse_dashboard_page_rejects_unknown_slugs() {
        assert_eq!(parse_dashboard_page("unknown"), None);
    }

    async fn seed_dashboard_snapshot(workdir: &Path) {
        let memory_dir = workdir.join(".roko").join("memory");
        fs::create_dir_all(&memory_dir).await.unwrap();
        let learn_dir = workdir.join(".roko").join("learn");
        fs::create_dir_all(&learn_dir).await.unwrap();

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

        let cfactor_path = learn_dir.join("c-factor.jsonl");
        let mut cf1 = CFactor::default();
        cf1.overall = 0.48;
        cf1.computed_at = chrono::Utc::now() - chrono::Duration::days(6);

        let mut cf2 = CFactor::default();
        cf2.overall = 0.53;
        cf2.computed_at = chrono::Utc::now() - chrono::Duration::days(3);

        let mut cf3 = CFactor::default();
        cf3.overall = 0.67;
        cf3.components = roko_learn::cfactor::CFactorComponents {
            gate_pass_rate: 0.82,
            cost_efficiency: 0.76,
            speed: 0.71,
            information_flow_rate: 0.89,
            first_try_rate: 0.64,
            knowledge_growth: 0.18,
            knowledge_integration_rate: 0.57,
            task_diversity_coverage: 0.73,
            convergence_velocity: 0.66,
            turn_taking_equality: 0.74,
            social_sensitivity: 0.68,
        };
        cf3.computed_at = chrono::Utc::now();

        let cfactor_history = [
            serde_json::to_string(&cf1).unwrap(),
            serde_json::to_string(&cf2).unwrap(),
            serde_json::to_string(&cf3).unwrap(),
        ]
        .join("\n")
            + "\n";
        fs::write(&cfactor_path, cfactor_history).await.unwrap();

        let provider_health_path = learn_dir.join("provider-health.json");
        let provider_health = serde_json::json!({
            "providers": {
                "anthropic": {
                    "provider_id": "anthropic",
                    "state": "Closed",
                    "consecutive_failures": 0,
                    "total_requests": 12,
                    "total_failures": 1,
                    "last_failure_at": null,
                    "cooldown_until": null,
                    "failure_window": []
                },
                "zai": {
                    "provider_id": "zai",
                    "state": "HalfOpen",
                    "consecutive_failures": 3,
                    "total_requests": 8,
                    "total_failures": 2,
                    "last_failure_at": 1710000000000i64,
                    "cooldown_until": 1710000005000i64,
                    "failure_window": []
                }
            }
        });
        fs::write(
            &provider_health_path,
            serde_json::to_string_pretty(&provider_health).unwrap(),
        )
        .await
        .unwrap();

        let latency_stats_path = learn_dir.join("latency-stats.json");
        let latency_stats = serde_json::json!({
            "entries": [
                {
                    "provider": "anthropic",
                    "stats": {
                        "model_slug": "claude-opus-4-6",
                        "provider_id": "anthropic",
                        "ttft_ema_ms": 0.0,
                        "total_latency_ema_ms": 0.0,
                        "tokens_per_second_ema": 0.0,
                        "observations": 3,
                        "recent_latencies": [800.0, 1200.0, 600.0]
                    }
                }
            ]
        });
        fs::write(
            &latency_stats_path,
            serde_json::to_string_pretty(&latency_stats).unwrap(),
        )
        .await
        .unwrap();

        let cascade_router_path = learn_dir.join("cascade-router.json");
        let cascade_router = serde_json::json!({
            "model_slugs": ["kimi-k2.5", "glm-5.1", "claude-sonnet-4-6", "claude-opus-4-6"],
            "confidence_stats": {
                "kimi-k2.5": { "trials": 145, "successes": 113 },
                "glm-5.1": { "trials": 203, "successes": 166 },
                "claude-sonnet-4-6": { "trials": 312, "successes": 250 },
                "claude-opus-4-6": { "trials": 47, "successes": 44 }
            }
        });
        fs::write(
            &cascade_router_path,
            serde_json::to_string_pretty(&cascade_router).unwrap(),
        )
        .await
        .unwrap();
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
        assert!(health.contains("current c-factor: 0.67 ↑"));
        assert!(health.contains("gate pass rate: 82.0%"));
        assert!(health.contains("information flow rate: 89.0%"));
        assert!(health.contains("knowledge growth: 18.0%"));

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

        let provider_health = dashboard_output(
            &cli,
            Some(dir.path().to_path_buf()),
            Some("provider-health".to_string()),
            false,
        )
        .await
        .unwrap();
        assert!(provider_health.contains("Provider Health (provider-health)"));
        assert!(provider_health.contains("anthropic"));
        assert!(provider_health.contains("● CLOSED"));
        assert!(provider_health.contains("p50: 0.8s"));
        assert!(provider_health.contains("summary: 20 requests, 3 failures"));

        let model_comparison = dashboard_output(
            &cli,
            Some(dir.path().to_path_buf()),
            Some("model-comparison".to_string()),
            false,
        )
        .await
        .unwrap();
        assert!(model_comparison.contains("Model Comparison (model-comparison)"));
        assert!(model_comparison.contains("Pareto frontier:"));
        assert!(model_comparison.contains("claude-sonnet-4-6 dominated by glm-5.1"));

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

    #[test]
    fn redacting_format_scrubs_api_keys() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        // Capture output into a shared buffer.
        #[derive(Clone)]
        struct BufWriter(Arc<Mutex<Vec<u8>>>);

        impl std::io::Write for BufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
            type Writer = BufWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = BufWriter(Arc::clone(&buffer));

        let scrubber = build_log_scrubber(&[]);
        let fmt_layer = tracing_subscriber::fmt::layer()
            .event_format(RedactingFormat::new(
                tracing_subscriber::fmt::format(),
                scrubber,
            ))
            .with_writer(writer)
            .with_ansi(false);

        let subscriber = tracing_subscriber::registry().with(fmt_layer);

        // Use `with_default` so the subscriber is scoped to this test — does
        // not conflict with the global subscriber from other tests.
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                "connecting with key sk-ant-api03-AAABBBCCCDDDEEEFFFGGGHHHIIIJJJ and token ghp_ABCDEFGHIJKLMNOPqrstuvwxyz1234567890"
            );
            tracing::warn!("Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature in header");
            tracing::info!("ANTHROPIC_API_KEY=sk-ant-secret-value-99999 leaked");
        });

        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();

        // API keys must be scrubbed.
        assert!(
            !output.contains("sk-ant-api03-AAABBBCCC"),
            "Anthropic key should be scrubbed, got: {output}"
        );
        assert!(
            !output.contains("ghp_ABCDEFGHIJKLMNOP"),
            "GitHub PAT should be scrubbed, got: {output}"
        );
        assert!(
            !output.contains("eyJhbGciOiJIUzI1NiJ9"),
            "Bearer token should be scrubbed, got: {output}"
        );
        assert!(
            !output.contains("sk-ant-secret-value"),
            "env-var key value should be scrubbed, got: {output}"
        );

        // Redaction markers must be present.
        assert!(
            output.contains("[REDACTED"),
            "redaction markers should appear, got: {output}"
        );

        // Non-secret context text must survive.
        assert!(
            output.contains("connecting with key"),
            "context text should survive, got: {output}"
        );
    }

    #[test]
    fn build_log_scrubber_adds_env_redactions() {
        let scrubber =
            build_log_scrubber(&[("MY_TOKEN".to_string(), "super-secret-42".to_string())]);
        let output = scrubber.scrub("leaked super-secret-42 in logs");
        assert!(
            !output.contains("super-secret-42"),
            "env redaction should scrub literal value, got: {output}"
        );
        assert!(
            output.contains("[REDACTED:MY_TOKEN]"),
            "should use named redaction, got: {output}"
        );
    }
}

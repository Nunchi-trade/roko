//! `roko` binary entrypoint.
//!
//! See [`roko_cli`] for the lib-side description. The binary exposes
//! subcommands (`init`, `run`, `status`, `replay`, `dream`, `config`, `inject`,
//! `plan`, `research`, `neuro`, `subscription`, `event-sources`, `experiment`) plus top-level flags for mode selection (`--headless`,
//! `--role`, `--model`, `--effort`, `--json`, `--log-format`, `--quiet`,
//! `--resume`, `--repo`, `--no-replan`, and a positional `[prompt]` for
//! one-shot mode).

#![allow(clippy::too_many_lines)]
#![cfg_attr(
    clippy,
    allow(
        clippy::all,
        clippy::pedantic,
        clippy::nursery,
        clippy::restriction,
        missing_docs
    )
)]

mod agent_serve;
mod commands;
mod plan_validate;

use roko_cli::auth;

use agent_serve::AgentCmd;
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use commands::experiment::{ExperimentCmd, dispatch_experiment};
use octocrab::Octocrab;
use octocrab::models::hooks::{Config as HookConfig, ContentType, Hook};
use octocrab::models::webhook_events::WebhookEventType;
use roko_agent::process::{cleanup_orphaned_agents, reap_orphaned_children};
use roko_agent::translate::BackendResponse;
use roko_cli::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use roko_cli::serve_runtime::RokoCliRuntime;
use roko_cli::tui::App;
use roko_cli::{
    Config, DashboardScaffold, EditTarget, InjectKind, InjectRequest, OneshotMode, PageId,
    PipeMode, Plan, ReplMode, RepoRegistry, SessionStatus, Source, WizardInputs, config_cmd,
    load_layered, run_init_wizard, run_once,
};
use roko_core::agent::{AgentRole, ProviderKind};
use roko_core::config::ServeDeployWebhookConfig;
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::{ContentHash, Context, DaimonPolicy, Kind, Query, Substrate};
use roko_core::{Headlines, TaskMetric, compute_headlines};
use roko_dreams::{DreamAgentConfig, DreamEngine, DreamLoopConfig, DreamRunner};
use roko_fs::{FileSubstrate, FsObservabilitySinks, RokoLayout};
use roko_learn::cascade_router::{CascadeRouteExplanation, CascadeRouter};
use roko_learn::cfactor::{CFactor, trend_arrow as cfactor_trend_arrow};
use roko_learn::cost_table::CostTable;
use roko_learn::costs_log::CostsLog;
use roko_learn::efficiency::compute_role_profiles;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::latency::{LatencyRegistry, LatencyStats};
use roko_learn::model_router::{RoutingContext, normalized_cost};
use roko_learn::prompt_experiment::ExperimentStore;
use roko_learn::provider_health::{CircuitState, ProviderHealth};
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};
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
// Color mode
// -----------------------------------------------------------------------

/// Controls ANSI color output.
///
/// Respects the `NO_COLOR` (https://no-color.org/), `CLICOLOR`, and
/// `CLICOLOR_FORCE` conventions when set to `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Detect from terminal and environment (default).
    Auto,
    /// Always emit ANSI colors.
    Always,
    /// Never emit ANSI colors.
    Never,
}

impl ColorMode {
    /// Resolve the effective color decision, consulting env vars when `Auto`.
    ///
    /// Precedence (highest first):
    /// 1. `--color always|never` (not Auto)
    /// 2. `NO_COLOR` set and non-empty  -> off
    /// 3. `CLICOLOR_FORCE` set and != "0" -> on
    /// 4. `CLICOLOR=0`                   -> off
    /// 5. stdout is a TTY               -> on
    /// 6. otherwise                      -> off
    fn should_color(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                if env::var("NO_COLOR").map_or(false, |v| !v.is_empty()) {
                    return false;
                }
                if env::var("CLICOLOR_FORCE").map_or(false, |v| v != "0") {
                    return true;
                }
                if env::var("CLICOLOR").map_or(false, |v| v == "0") {
                    return false;
                }
                std::io::stdout().is_terminal()
            }
        }
    }
}

// -----------------------------------------------------------------------
// Enhanced version string
// -----------------------------------------------------------------------

fn long_version() -> &'static str {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let version = env!("CARGO_PKG_VERSION");
        let git_hash = env!("ROKO_GIT_HASH");
        let rustc = env!("ROKO_RUSTC_VERSION");
        let target = env!("ROKO_TARGET");
        format!("{version} ({rustc}, {target}, git {git_hash})")
    })
}

// -----------------------------------------------------------------------
// CLI structure
// -----------------------------------------------------------------------

/// Minimal CLI for the Roko universal loop.
#[derive(Debug, Parser)]
#[command(
    name = "roko",
    version,
    long_version = long_version(),
    about = "Minimal CLI for the Roko universal loop",
    after_long_help = "\
COMMAND GROUPS:
  Core workflow:     init, run, status, doctor
  Planning:          plan, prd
  Agents:            agent (create, start, stop, chat, serve)
  Research:          research
  Knowledge:         knowledge (query, dream, custody, archive)
  Learning:          learn (router, experiments, efficiency, tune)
  Jobs:              job
  Benchmarks:        bench
  Configuration:     config (providers, models, subscriptions, plugins, secrets)
  Code intelligence: index
  Server:            up, serve, daemon, deploy, worker
  Interactive:       dashboard
  Utilities:         replay, inject, completions, new, explain"
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

    /// Control color output: auto (default), always, never.
    ///
    /// Respects NO_COLOR, CLICOLOR, and CLICOLOR_FORCE env vars in auto mode.
    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,

    /// Print elapsed time after command execution.
    ///
    /// Also enabled by setting ROKO_TIMING=1 in the environment.
    #[arg(long, global = true)]
    timing: bool,

    /// One-shot mode: execute this prompt and exit.
    #[arg(global = false)]
    prompt: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    // ── Core workflow ────────────────────────────────────────────────
    /// Create `.roko/` and a default `roko.toml` in `path` (default: cwd).
    #[command(after_help = "\
Examples:
  roko init                         Initialize in the current directory
  roko init /path/to/project        Initialize in a specific directory
  roko init --cloud                 Initialize with cloud-ready defaults
  roko init --profile rust          Initialize with Rust project profile")]
    Init {
        /// Directory to initialize (default: current dir).
        path: Option<PathBuf>,
        /// Generate cloud-ready defaults for deployment.
        #[arg(long)]
        cloud: bool,
        /// Project profile to use (e.g. rust, typescript, go, python, general).
        #[arg(long)]
        profile: Option<String>,
    },
    /// Seed a prompt and run the universal loop (compose -> agent -> gate -> persist).
    #[command(after_help = "\
Examples:
  roko run \"Fix the login bug\"      Single prompt through the universal loop
  roko run \"Add tests for auth\"     Generate and execute a plan
  roko run \"Refactor db layer\" --role architect   Run with a specific role")]
    Run {
        /// The user prompt text.
        prompt: String,
        /// Override the working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Print signal counts, most recent episode, and gate pass/fail.
    #[command(after_help = "\
Examples:
  roko status                       Show workspace health summary
  roko status --json                Output status as JSON for scripting
  roko status --cfactor             Compute and show C-Factor metrics")]
    Status {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Compute and persist the latest C-Factor snapshot.
        #[arg(long)]
        cfactor: bool,
        /// Print the CLI/TUI/backend surface inventory instead of session status.
        #[arg(long)]
        surfaces: bool,
    },
    /// Diagnose self-hosted workspace bootstrap state.
    Doctor {
        /// Directory containing `roko.toml` and `.roko/` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// roko-serve base URL or explicit health endpoint to probe.
        #[arg(long)]
        serve_url: Option<String>,
    },

    // ── Planning & PRDs ─────────────────────────────────────────────
    /// Manage plans (list, show, create, validate, run, generate).
    Plan {
        #[command(subcommand)]
        cmd: PlanCmd,
    },
    /// Manage product requirements documents (idea, draft, publish, plan).
    Prd {
        #[command(subcommand)]
        cmd: PrdCmd,
    },

    // ── Agents ──────────────────────────────────────────────────────
    /// Manage standalone agent runtimes and chat.
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },

    // ── Research ────────────────────────────────────────────────────
    /// Research topics, enhance documents, analyze execution data.
    Research {
        #[command(subcommand)]
        cmd: ResearchCmd,
    },

    // ── Knowledge (neuro + dreams + custody + archive) ──────────────
    /// Durable knowledge store, dream consolidation, custody chain, and archival.
    Knowledge {
        #[command(subcommand)]
        cmd: KnowledgeCmd,
    },

    // ── Learning & feedback ─────────────────────────────────────────
    /// Inspect learning state: cascade router, experiments, efficiency, episodes, tuning.
    Learn {
        #[command(subcommand)]
        cmd: LearnCmd,
    },

    // ── Jobs ────────────────────────────────────────────────────────
    /// Manage marketplace jobs (list, create, match, show, execute, cancel).
    Job {
        #[command(subcommand)]
        cmd: JobCmd,
    },

    /// Run benchmark evaluations and write learning telemetry.
    Bench {
        #[command(subcommand)]
        cmd: BenchCmd,
    },

    // ── Configuration (providers, models, subscriptions, etc.) ──────
    /// Manage global and project config, providers, models, subscriptions, plugins.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },

    // ── Code intelligence ───────────────────────────────────────────
    /// Code intelligence: build, search, and inspect the workspace index.
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },

    // ── Server & deployment ─────────────────────────────────────────
    /// Start roko serve + all configured [[agents]] in one command.
    #[command(after_help = "\
Examples:
  roko up                           Start serve + all agents from roko.toml
  roko up --workdir /path/to/proj   Start from a specific project directory")]
    Up {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Start the HTTP API server.
    Serve {
        /// Address to bind to (default: 127.0.0.1).
        #[arg(long)]
        bind: Option<String>,
        /// Port number (default: 6677).
        #[arg(long)]
        port: Option<u16>,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Run the interactive TUI dashboard embedded in the server process.
        /// The TUI reads live state directly from the server's StateHub
        /// (zero-copy, no file polling).
        #[arg(long)]
        tui: bool,
    },
    /// Manage daemon mode (start, stop, status, logs, install).
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Deploy to cloud targets (Railway, Fly.io, Docker).
    Deploy {
        #[command(subcommand)]
        cmd: DeployCmd,
    },
    /// Run as a deployed worker (reads template from env, serves tasks).
    Worker {
        /// Port to listen on (default: 8080, overridden by PORT env).
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },

    // ── Interactive ─────────────────────────────────────────────────
    /// Launch the dashboard TUI.
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
        /// Use high-contrast color scheme for accessibility (WCAG 2.1 AA).
        #[arg(long)]
        high_contrast: bool,
        /// Disable animations for reduced-motion accessibility.
        #[arg(long)]
        reduced_motion: bool,
    },

    // ── Authentication ────────────────────────────────────────────────
    /// Authenticate with a roko-serve instance.
    #[command(after_help = "\
Examples:
  roko login                              Login via browser (Privy)
  roko login --api-key                    Login with an API key (prompts)
  roko login --api-key --check            Validate stored API key credential
  roko login https://my-server.com        Login to a remote server")]
    Login {
        /// URL of the roko-serve instance (default: http://localhost:6677).
        #[arg(default_value = "http://localhost:6677")]
        url: String,
        /// Login with an API key instead of browser auth.
        #[arg(long)]
        api_key: bool,
        /// Non-interactive: validate stored credential only.
        #[arg(long, requires = "api_key")]
        check: bool,
        /// URL of the dashboard for browser auth (default: http://localhost:5173).
        #[arg(
            long,
            env = "NUNCHI_DASHBOARD_URL",
            default_value = "http://localhost:5173"
        )]
        dashboard_url: String,
    },
    /// Remove stored credentials.
    Logout,
    /// Show current authentication status.
    Whoami,

    // ── Vision loop ───────────────────────────────────────────────────
    /// Iterative vision-guided UI refinement loop.
    VisionLoop {
        /// Source file to iterate on (e.g. src/pages/Home.tsx).
        target_file: PathBuf,
        /// What the UI should look/feel like.
        #[arg(long)]
        goal: String,
        /// URL to screenshot (e.g. http://localhost:5173).
        #[arg(long)]
        url: String,
        /// Maximum iterations (default: 10).
        #[arg(long, default_value_t = 10)]
        max_iter: u32,
        /// Score threshold (1-10) for early stopping (default: 9.0).
        #[arg(long, default_value_t = 9.0)]
        target_score: f64,
        /// Consecutive target hits before stopping (default: 2).
        #[arg(long, default_value_t = 2)]
        consecutive_target: u32,
        /// Score drop from peak that triggers rollback (default: 3.0).
        #[arg(long, default_value_t = 3.0)]
        regression_threshold: f64,
        /// Vision model key from roko.toml (auto-detected if omitted).
        #[arg(long)]
        model: Option<String>,
        /// Viewport width in pixels (default: 1280).
        #[arg(long, default_value_t = 1280)]
        viewport_width: u32,
        /// Viewport height in pixels (default: 720).
        #[arg(long, default_value_t = 720)]
        viewport_height: u32,
        /// Milliseconds to wait after writing (HMR settle time, default: 2000).
        #[arg(long, default_value_t = 2000)]
        wait_ms: u64,
    },

    // ── Utilities ───────────────────────────────────────────────────
    /// Walk the lineage DAG rooted at a signal hash and print it.
    Replay {
        /// Engram hash (64 hex chars) to walk.
        hash: String,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Show forensic detail: timestamps, full hashes, metadata.
        #[arg(long)]
        forensic: bool,
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
    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Generate boilerplate for a Synapse trait or domain profile.
    ///
    /// Types: gate, scorer, router, policy, substrate, composer, domain, template, event-source.
    New {
        /// Type of scaffold to generate (e.g. gate, scorer, router).
        #[arg(value_name = "TYPE")]
        type_name: String,
        /// Name for the generated component (e.g. my-custom-gate).
        name: String,
        /// Output directory (default: current directory).
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Explain a roko concept with progressive disclosure (3 depth levels).
    Explain {
        /// Topic to explain (e.g. gates, routing, cognitive, neuro, daimon, dreams, engram, cfactor).
        topic: String,
        /// Disclosure depth: 1 = summary, 2 = how it works, 3 = internals.
        #[arg(long, default_value_t = 1)]
        depth: u8,
    },
}

// -----------------------------------------------------------------------
// Knowledge: neuro + dreams + custody + archive
// -----------------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum KnowledgeCmd {
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
    /// Backup the knowledge store to a directory with optional genomic bottleneck.
    Backup {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Directory to write the backup files into.
        destination: PathBuf,
        /// Overwrite existing backup files in the destination directory.
        #[arg(long)]
        force: bool,
        /// Genomic bottleneck: export only the top N entries by confidence.
        #[arg(long)]
        top_n: Option<usize>,
    },
    /// Restore the knowledge store from a backup with confidence decay.
    Restore {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Directory created by `roko knowledge backup`.
        source: PathBuf,
        /// Overwrite existing local neuro store files.
        #[arg(long)]
        force: bool,
        /// Filter by knowledge types (comma-separated).
        #[arg(long)]
        types: Option<String>,
        /// Only restore entries with confidence >= this threshold (0.0 to 1.0).
        #[arg(long)]
        min_confidence: Option<f64>,
        /// Generation hop count for confidence decay (default: 1).
        #[arg(long, default_value_t = 1)]
        generation: u32,
    },
    /// Sync knowledge with a peer agent via the Mesh protocol.
    Sync {
        /// Peer agent identifier to sync with.
        peer: String,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Direction: send, receive, or both (default: both).
        #[arg(long, default_value = "both")]
        direction: String,
        /// Maximum engrams to send in this sync cycle.
        #[arg(long, default_value_t = 100)]
        max_send: usize,
    },
    /// Dream consolidation, reports, and journal.
    Dream {
        #[command(subcommand)]
        cmd: KnowledgeDreamCmd,
    },
    /// Custody audit chain (list, show, verify).
    Custody {
        #[command(subcommand)]
        cmd: KnowledgeCustodyCmd,
    },
    /// Move old engrams to cold storage (compressed monthly archives).
    Archive {
        /// Only archive engrams older than this duration (e.g. "30d", "7d").
        #[arg(long, default_value = "30d")]
        older_than: String,
        /// Maximum number of engrams to archive per batch.
        #[arg(long, default_value_t = 500)]
        batch_size: usize,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Print what would be archived without doing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum KnowledgeDreamCmd {
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
    /// Display recent dream journal entries.
    Journal {
        /// Number of recent entries to display (default: 10).
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Display recent dream archive entries.
    Archive {
        /// Number of recent entries to display (default: 10).
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum KnowledgeCustodyCmd {
    /// List recent custody records.
    List {
        /// Maximum number of records to display.
        #[arg(long)]
        limit: Option<usize>,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show full details of a custody record by index.
    Show {
        /// Record index (0-based).
        index: usize,
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Verify integrity of the custody chain.
    Verify {
        /// Directory containing `.roko/` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

// -----------------------------------------------------------------------
// Learn: learning state + tuning
// -----------------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum LearnCmd {
    /// Show all learning state (router, experiments, efficiency, episodes).
    All {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show cascade router state.
    Router {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show experiment state.
    Experiments {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show efficiency metrics.
    Efficiency {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show episode summary.
    Episodes {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Tune adaptive thresholds and model routing parameters.
    Tune {
        /// Subsystem to tune: gates, routing, budget.
        #[arg(default_value = "gates")]
        subsystem: String,
        /// Display current values without modifying.
        #[arg(long)]
        dry_run: bool,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum BenchCmd {
    /// Run a native SWE-bench-style proxy batch.
    #[command(after_help = "\
Examples:
  roko bench swe --batch-size 2 --agent-mode gold
  roko bench swe --dataset ./swe-smoke.jsonl --predictions ./predictions.jsonl --agent-mode prediction-file
  roko bench swe --agent-mode command --agent-command './my-agent.sh'")]
    Swe {
        /// Local JSONL dataset. If omitted, a built-in two-task smoke dataset is generated.
        #[arg(long)]
        dataset: Option<PathBuf>,
        /// Number of instances to run.
        #[arg(long, default_value_t = 2)]
        batch_size: usize,
        /// Offset into the dataset.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// Agent adapter to use.
        #[arg(long, value_enum, default_value_t = roko_cli::bench::SweAgentMode::Gold)]
        agent_mode: roko_cli::bench::SweAgentMode,
        /// Predictions JSONL path for --agent-mode prediction-file.
        #[arg(long)]
        predictions: Option<PathBuf>,
        /// Command for --agent-mode command. Receives instance JSON on stdin, prints a unified diff.
        #[arg(long)]
        agent_command: Option<String>,
        /// Scores JSONL output path.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Write SWE-bench-style predictions JSONL.
        #[arg(long)]
        export_predictions: Option<PathBuf>,
        /// Disable learning episode, efficiency, and C-factor writes.
        #[arg(long)]
        no_learning: bool,
        /// Keep per-instance benchmark workdirs for debugging.
        #[arg(long)]
        keep_workdirs: bool,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

// -----------------------------------------------------------------------
// Plugins (now nested under config)
// -----------------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum PluginCmd {
    /// List available and installed plugins.
    List {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Install a plugin from a local path or registry.
    Install {
        /// Path to the plugin manifest (plugin.toml) or directory.
        source: String,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Remove an installed plugin by name.
    Remove {
        /// Name of the plugin to remove.
        name: String,
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Audit installed plugins and report capabilities.
    Audit {
        /// Working directory (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum IndexCmd {
    /// Build a code index for the workspace (or specified directory).
    Build {
        /// Directory to index (default: cwd / --repo).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Drop existing index data and rebuild from source files.
    Rebuild {
        /// Directory to index (default: cwd / --repo).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Search the code index.
    Search {
        /// Search query text.
        query: String,
        /// Restrict to a symbol kind (function, struct, enum, trait, const, type, module, impl).
        #[arg(long)]
        kind: Option<String>,
        /// Search strategy: keyword, structural, hybrid.
        #[arg(long, default_value = "keyword")]
        strategy: String,
        /// Maximum number of results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Directory to index (default: cwd / --repo).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show index statistics.
    Stats {
        /// Directory to index (default: cwd / --repo).
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCmd {
    Start {
        #[arg(long)]
        foreground: bool,
        #[arg(long, default_value_t = roko_cli::DEFAULT_SERVE_PORT)]
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
        #[arg(long, default_value_t = roko_cli::DEFAULT_SERVE_PORT)]
        port: u16,
    },
    Install,
    // macOS launchd plist generation
    Uninstall, // remove launchd plist
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

// (CustodyCmd, DreamCmd, DreamsCmd moved into KnowledgeCmd above)

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
    /// Lint every `tasks.toml` under a plans directory without executing it.
    Validate {
        /// Plans root directory.
        #[arg(default_value = "plans/")]
        dir: PathBuf,
        /// Fail on warnings, not only errors.
        #[arg(long)]
        strict: bool,
        /// Output machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Run a plan directory through the orchestration loop.
    #[command(after_help = "\
Examples:
  roko plan run plans/              Run all plans in the plans/ directory
  roko plan run plans/my-plan       Run a specific plan
  roko plan run plans/ --approval   Run with interactive TUI approval
  roko plan run plans/ --dry-run    Preview without executing
  roko plan run plans/ --resume-plan .roko/state/executor.json   Resume from snapshot")]
    Run {
        /// Path to the plans directory.
        plans_dir: PathBuf,
        /// Working directory (repo root). Defaults to current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Resume from `.roko/state/executor.json` in the working directory.
        #[arg(long = "resume-plan", visible_alias = "resume-state", num_args = 0..=1, default_missing_value = ".roko/state/executor.json")]
        resume_plan: Option<PathBuf>,
        /// Launch the connected approval TUI while the plan runs.
        #[arg(long)]
        approval: bool,
        /// Maximum retry attempts per task (overrides per-task and config values).
        #[arg(long)]
        max_retries: Option<u32>,
        /// Parse and display the plan without executing. Shows tasks, dependencies, and estimates.
        #[arg(long)]
        dry_run: bool,
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
        /// Plan directory name under .roko/plans/.
        plan: String,
    },
    /// Optimize tasks for efficiency, parallelism, and cheapest viable model.
    EnhanceTasks {
        /// Plan directory name under .roko/plans/.
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
enum JobCmd {
    /// List all marketplace jobs.
    List {
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Filter by status (open, assigned, in_progress, completed, failed, cancelled).
        #[arg(long)]
        status: Option<String>,
    },
    /// Create a new marketplace job.
    Create {
        /// Job title.
        title: String,
        /// Job type: research, coding_task, chain_monitor, chain_analysis.
        #[arg(long, default_value = "research")]
        r#type: String,
        /// Job description.
        #[arg(long, default_value = "")]
        description: String,
        /// Priority: low, medium, high, critical.
        #[arg(long, default_value = "medium")]
        priority: String,
        /// Auto-execute the job when the runner picks it up.
        #[arg(long)]
        auto_execute: bool,
        /// Associated plan ID.
        #[arg(long)]
        plan_id: Option<String>,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Match a proposed job against registered agents via roko-serve.
    Match {
        /// Job title.
        title: String,
        /// roko-serve base URL.
        #[arg(long, default_value = "http://localhost:6677")]
        serve_url: String,
        /// Job description.
        #[arg(long, default_value = "")]
        description: String,
        /// Primary implementation language, also treated as a required skill.
        #[arg(long)]
        language: Option<String>,
        /// Minimum agent tier: Unverified, Verified, Trusted, Expert, Pioneer.
        #[arg(long)]
        min_tier: Option<String>,
        /// Reward string, e.g. "2500 KORAI".
        #[arg(long, default_value = "")]
        reward: String,
        /// Required skills, comma-separated.
        #[arg(long, value_delimiter = ',')]
        skills: Vec<String>,
        /// Working directory (default: cwd / --repo), used for auth config.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Show details for a specific job.
    Show {
        /// Job ID.
        id: String,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Execute a job (locally or via roko-serve).
    Execute {
        /// Job ID.
        id: String,
        /// roko-serve base URL. If set, POST to /api/jobs/{id}/execute.
        #[arg(long)]
        serve_url: Option<String>,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Cancel a job.
    Cancel {
        /// Job ID.
        id: String,
        /// Working directory (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

// Internal enum used by cmd_neuro — mirrors the old top-level NeuroCmd.
// KnowledgeCmd dispatches to this.
#[derive(Debug)]
enum NeuroCmd {
    Query {
        topic: Vec<String>,
        workdir: Option<PathBuf>,
    },
    Stats {
        workdir: Option<PathBuf>,
    },
    Gc {
        workdir: Option<PathBuf>,
    },
    Backup {
        workdir: Option<PathBuf>,
        destination: PathBuf,
        force: bool,
        top_n: Option<usize>,
    },
    Restore {
        workdir: Option<PathBuf>,
        source: PathBuf,
        force: bool,
        types: Option<String>,
        min_confidence: Option<f64>,
        generation: u32,
    },
    Sync {
        peer: String,
        workdir: Option<PathBuf>,
        direction: String,
        max_send: usize,
    },
}

// Internal enum used by cmd_dream — mirrors the old top-level DreamCmd.
#[derive(Debug)]
enum DreamCmdLegacy {
    Run { workdir: Option<PathBuf> },
    Report { workdir: Option<PathBuf> },
    Schedule { workdir: Option<PathBuf> },
}

// EventSourcesCmdLegacy, ProviderCmdLegacy, ModelCmdLegacy removed — dispatch goes direct

#[derive(Debug, Subcommand)]
enum DeployCmd {
    /// Deploy the current workspace to Railway via the public GraphQL API.
    ///
    /// Creates a Railway project with roko-serve as the control plane.
    /// Use --with-mirage to also deploy the chain relay, and --workers to
    /// deploy agent workers from the template registry.
    Railway {
        /// Working directory / repository root (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Also deploy the mirage chain relay service.
        #[arg(long)]
        with_mirage: bool,
        /// Deploy worker services for these template names (comma-separated).
        #[arg(long, value_delimiter = ',')]
        workers: Vec<String>,
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
    // ── Core config management ──────────────────────────────────────
    /// Interactive wizard: detects installed LLM CLIs, writes global config.
    #[command(alias = "wizard")]
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

    // ── Providers ───────────────────────────────────────────────────
    /// Inspect configured LLM providers.
    Providers {
        #[command(subcommand)]
        cmd: ConfigProviderCmd,
    },
    // ── Models ──────────────────────────────────────────────────────
    /// Inspect configured models and routing.
    Models {
        #[command(subcommand)]
        cmd: ConfigModelCmd,
    },
    // ── Subscriptions ───────────────────────────────────────────────
    /// Manage event subscriptions.
    Subscriptions {
        #[command(subcommand)]
        cmd: ConfigSubscriptionCmd,
    },
    // ── Event sources ───────────────────────────────────────────────
    /// Inspect configured event sources (cron, file watchers).
    Events {
        /// Directory containing `roko.toml` (default: cwd).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    // ── Experiments ─────────────────────────────────────────────────
    /// Manage model A/B experiments.
    Experiments {
        #[command(subcommand)]
        cmd: ExperimentCmd,
    },
    // ── Plugins ─────────────────────────────────────────────────────
    /// Manage plugins (list, install, remove, audit).
    Plugins {
        #[command(subcommand)]
        cmd: PluginCmd,
    },
    // ── Secrets ─────────────────────────────────────────────────────
    /// Manage profile-aware secrets (set, get, list, rotate).
    Secrets {
        #[command(subcommand)]
        cmd: roko_cli::SecretsCmd,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigProviderCmd {
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
        /// Provider name from `[providers.*]`.  Omit when using `--all`.
        provider: Option<String>,
        /// Test every configured provider and print a summary table.
        #[arg(long)]
        all: bool,
        /// Directory containing `roko.toml` (default: cwd / --repo).
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigModelCmd {
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
enum ConfigSubscriptionCmd {
    /// List all subscriptions.
    List,
    /// Create a new subscription.
    Add {
        /// Agent template name to invoke.
        #[arg(long)]
        template: String,
        /// Engram trigger glob to match.
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

fn main() {
    let startup_env_redactions = match load_startup_env_files() {
        Ok(values) => values,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(EXIT_SYSTEM_ERROR);
        }
    };

    let mut cli = Cli::parse();
    apply_env_overrides(&mut cli);

    // ── TUI mode detection ─────────────────────────────────────────
    let tui_mode = matches!(&cli.command, Some(Command::Serve { tui: true, .. }));

    // ── Color mode ──────────────────────────────────────────────────
    let use_color = cli.color.should_color();

    // ── Timing mode ─────────────────────────────────────────────────
    let timing_enabled = cli.timing
        || env::var("ROKO_TIMING")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    let started_at = Instant::now();

    // In TUI mode, route ALL tracing output to a file instead of stderr.
    // This must be done here, before the global subscriber is set, to
    // prevent serve background tasks from writing over the ratatui screen.
    let filter = if tui_mode {
        // Suppress noisy subsystems in TUI mode.
        tracing_subscriber::EnvFilter::try_new(
            "roko=info,roko_neuro=error,roko_agent=warn,hyper=error,tower=error",
        )
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roko=info"))
    } else {
        tracing_subscriber::EnvFilter::try_new(tracing_log_directive())
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roko=info"))
    };

    // ROKO_LOG_RAW=1 disables secret redaction (useful for debugging).
    let raw_logs = env::var("ROKO_LOG_RAW")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let ansi_logs = use_color;
    // In TUI mode, write all tracing to a file so it doesn't corrupt the
    // ratatui rendering. This sets the global subscriber to a file-backed
    // writer before any background tasks are spawned.
    if tui_mode {
        let workdir = match &cli.command {
            Some(Command::Serve { workdir, .. }) => {
                workdir.clone().unwrap_or_else(|| resolve_workdir(&cli))
            }
            _ => resolve_workdir(&cli),
        };
        let log_path = workdir.join(".roko").join("serve-tui.log");
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(log_file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
        {
            tracing_subscriber::fmt()
                .with_target(true)
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(log_file))
                .with_env_filter(filter)
                .init();
        } else {
            // Fallback: suppress everything if we can't open the log.
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_env_filter(tracing_subscriber::EnvFilter::new("error"))
                .init();
        }
    } else if raw_logs {
        match cli.log_format {
            LogFormat::Json => {
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_ansi(ansi_logs)
                    .json()
                    .with_env_filter(filter)
                    .init();
            }
            LogFormat::Text => {
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_ansi(ansi_logs)
                    .with_env_filter(filter)
                    .init();
            }
        }
    } else {
        let scrubber = build_log_scrubber(&startup_env_redactions);
        match cli.log_format {
            LogFormat::Json => {
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_ansi(ansi_logs)
                    .event_format(RedactingFormat::new(
                        tracing_subscriber::fmt::format().json(),
                        scrubber,
                    ))
                    .with_env_filter(filter)
                    .init();
            }
            LogFormat::Text => {
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_ansi(ansi_logs)
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
        Ok(code) => {
            if timing_enabled {
                print_timing(started_at);
            }
            code
        }
        Err(e) => {
            if timing_enabled {
                print_timing(started_at);
            }
            let msg = format_error_with_hint(&e);
            eprintln!("error: {msg}");
            EXIT_SYSTEM_ERROR
        }
    };
    std::process::exit(code);
}

// -----------------------------------------------------------------------
// Timing helper
// -----------------------------------------------------------------------

fn print_timing(started_at: Instant) {
    let elapsed = started_at.elapsed();
    let secs = elapsed.as_secs_f64();
    if secs < 60.0 {
        eprintln!("Completed in {secs:.1}s");
    } else {
        let mins = (secs / 60.0).floor() as u64;
        let rem = secs - (mins as f64 * 60.0);
        eprintln!("Completed in {mins}m {rem:.1}s");
    }
}

// -----------------------------------------------------------------------
// Contextual error suggestions
// -----------------------------------------------------------------------

/// Format an error with a helpful hint when the message matches a known pattern.
fn format_error_with_hint(err: &anyhow::Error) -> String {
    let msg = format!("{err:#}");
    match error_hint(&msg) {
        Some(h) => format!("{msg}\n\nhint: {h}"),
        None => msg,
    }
}

/// Return an optional hint string based on common error patterns.
fn error_hint(msg: &str) -> Option<&'static str> {
    let lower = msg.to_lowercase();

    if lower.contains("no .roko directory")
        || lower.contains(".roko/")
            && (lower.contains("not found") || lower.contains("no such file"))
        || lower.contains("roko.toml")
            && (lower.contains("not found") || lower.contains("no such file"))
    {
        return Some("run `roko init` to create a workspace in the current directory");
    }

    if lower.contains("agent not found") || lower.contains("unknown agent") {
        return Some("run `roko agent list` to see available agents");
    }

    if lower.contains("plan not found")
        || lower.contains("plans directory does not exist")
        || lower.contains("no plans found")
    {
        return Some(
            "run `roko plan list` to see available plans, or `roko plan create` to make one",
        );
    }

    if lower.contains("connection refused")
        || lower.contains("connect error")
        || lower.contains("failed to connect")
    {
        return Some("is the server running? Start it with `roko serve`");
    }

    if lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("auth")
            && (lower.contains("failed") || lower.contains("invalid") || lower.contains("denied"))
    {
        return Some(
            "check your API key: set ROKO_API_KEY or run `roko config set-secret ROKO_API_KEY <key>`",
        );
    }

    if lower.contains("prd not found") || lower.contains("no prd") {
        return Some("run `roko prd list` to see available PRDs, or `roko prd idea` to create one");
    }

    None
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

fn tracing_log_directive() -> String {
    tracing_log_directive_from(env::var("RUST_LOG").ok(), env::var("ROKO_LOG").ok())
}

fn tracing_log_directive_from(rust_log: Option<String>, roko_log: Option<String>) -> String {
    rust_log
        .or(roko_log)
        .unwrap_or_else(|| "roko=info".to_string())
}


async fn dispatch(mut cli: Cli) -> Result<i32> {
    if let Some(command) = cli.command.take() {
        return dispatch_subcommand(command, &cli).await;
    }
    if cli.headless {
        return commands::util::cmd_headless(&cli).await;
    }
    if let Some(prompt) = &cli.prompt {
        return commands::util::cmd_oneshot(&cli, prompt).await;
    }
    if !roko_cli::stdin_is_tty() {
        return commands::util::cmd_pipe(&cli).await;
    }
    commands::util::cmd_repl(&cli)
}

async fn dispatch_subcommand(command: Command, cli: &Cli) -> Result<i32> {
    match command {
        Command::Init { path, cloud, profile } => {
            commands::util::cmd_init(path, cloud, profile).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Run { prompt, workdir } => commands::util::cmd_run(cli, workdir, prompt).await,
        Command::Status { workdir, cfactor, surfaces } => {
            commands::util::cmd_status(cli, workdir, cfactor, surfaces).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Doctor { workdir, serve_url } => commands::util::cmd_doctor(cli, workdir, serve_url).await,
        Command::Plan { cmd } => {
            let result = commands::plan::cmd_plan(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Prd { cmd } => {
            let result = commands::prd::cmd_prd(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Agent { cmd } => commands::agent::cmd_agent(cli, cmd).await,
        Command::Research { cmd } => {
            let result = commands::research::cmd_research(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }
        Command::Knowledge { cmd } => commands::knowledge::dispatch_knowledge(cli, cmd).await,
        Command::Learn { cmd } => commands::learn::dispatch_learn(cli, cmd).await,
        Command::Job { cmd } => commands::job::cmd_job(cli, cmd).await,
        Command::Bench { cmd } => commands::bench::cmd_bench(cli, cmd).await,
        Command::Config { cmd } => {
            match cmd {
                ConfigCmd::Experiments { cmd: exp_cmd } => {
                    return dispatch_experiment(cli, exp_cmd);
                }
                ConfigCmd::Plugins { cmd: plugin_cmd } => {
                    return commands::config_cmd::cmd_plugin(cli, plugin_cmd).await;
                }
                ConfigCmd::Secrets { cmd: secrets_cmd } => {
                    let workdir = resolve_workdir(cli);
                    roko_cli::secrets::dispatch_secrets(&secrets_cmd, &workdir)?;
                    return Ok(EXIT_SUCCESS);
                }
                other => {
                    commands::config_cmd::dispatch_config(cli, other).await?;
                }
            }
            Ok(EXIT_SUCCESS)
        }
        Command::Index { cmd } => commands::util::cmd_index(cli, cmd),
        Command::Up { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            commands::server::cmd_up(cli, wd).await
        }
        Command::Serve { bind, port, workdir, tui } => {
            let wd = workdir.clone().unwrap_or_else(|| resolve_workdir(cli));
            let config = resolve_config_for_workdir(cli, &wd)?;
            let repo_registry = RepoRegistry::load(&config, &wd).unwrap_or_default();
            let runtime = RokoCliRuntime::new(config, repo_registry).into_arc();
            if tui {
                let (state, server_handle) =
                    roko_serve::start_server_background(wd.clone(), runtime, bind, port).await?;
                let hub = state.state_hub.clone();
                let tui_result = commands::dashboard::cmd_dashboard(cli, Some(wd), None, false, false, Some(hub)).await;
                state.cancel.cancel();
                match server_handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => eprintln!("server error on shutdown: {e}"),
                    Err(e) => eprintln!("server task panicked: {e}"),
                }
                tui_result
            } else {
                roko_serve::run_server(wd, runtime, bind, port).await?;
                Ok(EXIT_SUCCESS)
            }
        }
        Command::Daemon { cmd } => commands::server::cmd_daemon(cli, cmd).await,
        Command::Deploy { cmd } => commands::server::cmd_deploy(cli, cmd).await,
        Command::Worker { port } => {
            roko_cli::worker::run_worker(port).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Dashboard { page, list_pages, text, workdir, high_contrast, reduced_motion } => {
            #[allow(unsafe_code)]
            if high_contrast {
                unsafe { std::env::set_var("ROKO_HIGH_CONTRAST", "1") };
            }
            #[allow(unsafe_code)]
            if reduced_motion {
                unsafe { std::env::set_var("ROKO_REDUCED_MOTION", "1") };
            }
            commands::dashboard::cmd_dashboard(cli, workdir, page, list_pages, text, None).await
        }
        // ── Vision loop ───────────────────────────────────────────
        Command::VisionLoop {
            target_file, goal, url, max_iter, target_score,
            consecutive_target, regression_threshold, model,
            viewport_width, viewport_height, wait_ms,
        } => {
            let config = roko_cli::vision_loop::VisionLoopConfig {
                target_file, goal, url,
                max_iterations: max_iter, target_score, consecutive_target,
                regression_threshold, model_key: model,
                viewport_width, viewport_height, wait_ms,
            };
            let result = roko_cli::vision_loop::cmd_vision_loop(config).await?;
            println!("Vision loop complete: {}", result.stop_reason);
            println!(
                "  iterations: {}, best score: {:.1} (iteration {})",
                result.iterations_completed, result.best_score, result.best_iteration
            );
            println!("  run ID: {}", result.run_id);
            Ok(EXIT_SUCCESS)
        }
        Command::Replay { hash, workdir, forensic } => commands::util::cmd_replay(workdir, hash, forensic).await,
        Command::Inject { session, kind, payload, workdir } => commands::util::cmd_inject(cli, session, &kind, payload, workdir),
        Command::Completions { shell } => {
            commands::util::print_completions(shell);
            Ok(EXIT_SUCCESS)
        }
        Command::New { type_name, name, output } => {
            let output_dir = output.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            match roko_cli::scaffold::scaffold(&type_name, &name, &output_dir) {
                Ok(files) => {
                    println!("scaffolded `{type_name}` as `{name}` ({} file{})", files.len(), if files.len() == 1 { "" } else { "s" });
                    for f in &files { println!("  {}", f.display()); }
                    Ok(EXIT_SUCCESS)
                }
                Err(e) => { eprintln!("error: {e}"); Ok(EXIT_SYSTEM_ERROR) }
            }
        }
        Command::Explain { topic, depth } => {
            commands::util::cmd_explain(&topic, depth);
            Ok(EXIT_SUCCESS)
        }
        Command::Login { url, api_key, check, dashboard_url } => commands::auth::cmd_login(&url, api_key, check, &dashboard_url).await,
        Command::Logout => commands::auth::cmd_logout(),
        Command::Whoami => commands::auth::cmd_whoami().await,
    }
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

pub(crate) fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let path = workdir.join("roko.toml");
    let mut config = if path.exists() {
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))?
    } else {
        RokoConfig::default()
    };

    roko_cli::config::merge_global_providers(&mut config);
    Ok(config)
}

/// Resolve the working directory from CLI flags.
///
/// Detects when the user is running from inside a `.roko/` directory, which
/// would cause a nested `.roko/.roko/` and silent data loss.
fn resolve_workdir(cli: &Cli) -> PathBuf {
    let dir = cli.repo.clone().unwrap_or_else(|| PathBuf::from("."));

    // Detect if we're running from inside a .roko/ directory and auto-correct
    // to the project root to avoid nested .roko/.roko/ data dirs.
    if let Ok(abs) = dir.canonicalize() {
        for ancestor in abs.ancestors() {
            if ancestor.file_name().and_then(|n| n.to_str()) == Some(".roko") {
                let project_root = ancestor.parent().unwrap_or(ancestor).to_path_buf();
                eprintln!(
                    "\x1b[33m\u{26a0} Auto-correcting: running from inside .roko/, using project root: {}\x1b[0m",
                    project_root.display()
                );
                return project_root;
            }
        }
    }

    dir
}

/// Apply environment variable fallbacks to CLI flags.
///
/// When a CLI flag was not explicitly provided, its corresponding `ROKO_*`
/// environment variable is consulted. This runs once immediately after
/// `Cli::parse()` so every downstream consumer sees the resolved value.
///
/// | Env var          | CLI flag       | Behaviour                                  |
/// |------------------|----------------|---------------------------------------------|
/// | `ROKO_MODEL`     | `--model`      | Override when `--model` not given            |
/// | `ROKO_EFFORT`    | `--effort`     | Override when `--effort` not given            |
/// | `ROKO_ROLE`      | `--role`       | Override when `--role` not given              |
/// | `ROKO_QUIET`     | `--quiet`      | Enable quiet if "1" or "true"                |
/// | `ROKO_LOG_FORMAT` | `--log-format` | Override when default "text" is in effect     |
fn apply_env_overrides(cli: &mut Cli) {
    if cli.model.is_none() {
        if let Ok(val) = env::var("ROKO_MODEL") {
            if !val.is_empty() {
                cli.model = Some(val);
            }
        }
    }

    if cli.effort.is_none() {
        if let Ok(val) = env::var("ROKO_EFFORT") {
            match val.to_ascii_lowercase().as_str() {
                "low" => cli.effort = Some(Effort::Low),
                "medium" => cli.effort = Some(Effort::Medium),
                "high" => cli.effort = Some(Effort::High),
                "max" => cli.effort = Some(Effort::Max),
                _ => {
                    eprintln!(
                        "warning: ROKO_EFFORT={val:?} is not valid (expected low/medium/high/max), ignoring"
                    );
                }
            }
        }
    }

    if cli.role.is_none() {
        if let Ok(val) = env::var("ROKO_ROLE") {
            if !val.is_empty() {
                cli.role = Some(val);
            }
        }
    }

    if !cli.quiet {
        if let Ok(val) = env::var("ROKO_QUIET") {
            if val == "1" || val.eq_ignore_ascii_case("true") {
                cli.quiet = true;
            }
        }
    }

    // log_format has a clap default of Text; override only when the user
    // did not pass `--log-format` explicitly (we detect this by checking if
    // the env var is set — the clap default means we can't distinguish
    // "user typed --log-format text" from "default", but the env var path
    // is still useful when the default is in effect).
    if cli.log_format == LogFormat::Text {
        if let Ok(val) = env::var("ROKO_LOG_FORMAT") {
            match val.to_ascii_lowercase().as_str() {
                "json" => cli.log_format = LogFormat::Json,
                "text" => {} // already the default
                _ => {
                    eprintln!(
                        "warning: ROKO_LOG_FORMAT={val:?} is not valid (expected text/json), ignoring"
                    );
                }
            }
        }
    }
}

/// Ask the user to confirm a destructive operation.
///
/// Returns `true` (proceed) immediately when:
/// - `quiet` mode is active,
/// - stdin is not a TTY (CI / pipes), or
/// - the user types `y` or `Y`.
///
/// Returns `false` otherwise, meaning the operation should be skipped.
fn confirm_destructive(message: &str, quiet: bool) -> bool {
    if quiet || !std::io::stdin().is_terminal() {
        return true;
    }
    eprint!("{message} [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_ok() {
        input.trim().eq_ignore_ascii_case("y")
    } else {
        false
    }
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
            eprintln!(
                "warning: no config found — agent command is \"cat\" (test-only, echoes prompts back). \
                 Run `roko init` or set [agent] command in roko.toml."
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
    // Only create .roko/ if the user has expressed intent (roko.toml exists or .roko/ already exists).
    if !workdir.join("roko.toml").exists() && !workdir.join(".roko").exists() {
        return Ok(());
    }
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
    use commands::config_cmd::{
        ProviderListRow, ProviderHealthRow, ProviderLatencySummary,
        ModelListRow, PROVIDER_FAILURE_THRESHOLD,
        format_provider_rows, format_provider_health_rows, format_model_rows,
        build_model_list_row, build_provider_health_row,
        select_provider_test_model,
    };
    use commands::knowledge::{
        NEURO_KNOWLEDGE_FILE, NEURO_CONFIRMATIONS_FILE,
        backup_neuro_store, restore_neuro_store,
    };
    use commands::util::persist_capture_episode;
    use commands::dashboard::dashboard_output;
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
            Some(Command::Init {
                path,
                cloud,
                profile,
            }) => {
                assert_eq!(path, Some(PathBuf::from("/tmp/project")));
                assert!(!cloud);
                assert!(profile.is_none());
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
    fn cli_parses_doctor_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "doctor",
            "--workdir",
            "/tmp/project",
            "--serve-url",
            "http://localhost:9090",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                workdir: Some(_),
                serve_url: Some(_),
            })
        ));
    }

    #[test]
    fn cli_parses_agent_serve_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "agent",
            "serve",
            "--agent-id",
            "demo-1",
            "--bind",
            "127.0.0.1:7777",
            "--relay-url",
            "https://relay.example",
            "--chain-rpc-url",
            "https://rpc.example",
            "--identity-registry",
            "0x1234",
            "--passport-id",
            "7",
            "--wallet-key",
            "0xdeadbeef",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Agent {
                cmd: AgentCmd::Serve(agent_serve::AgentServeArgs {
                    agent_id,
                    bind,
                    relay_url: Some(_),
                    chain_rpc_url: Some(_),
                    identity_registry: Some(_),
                    passport_id: Some(_),
                    wallet_key: Some(_),
                    ..
                }),
            }) if agent_id == "demo-1" && bind == "127.0.0.1:7777"
        ));
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
    fn cli_parses_plan_resume_flag_documented_alias() {
        let cli = Cli::try_parse_from(["roko", "plan", "run", "plans", "--resume-state"]).unwrap();
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

    #[tokio::test]
    async fn persist_capture_episode_records_memory_episode() {
        let dir = tempdir().unwrap();
        let workdir = dir.path();

        persist_capture_episode(
            workdir,
            "claude",
            Some("claude-sonnet-4-6"),
            "prd-draft-new",
            "prd:draft:new:demo",
            "draft a PRD",
            "# demo prd",
            true,
            321,
            Some("resume-123"),
        )
        .await
        .unwrap();

        let episodes_path = workdir.join(".roko").join("memory").join("episodes.jsonl");
        let episodes = EpisodeLogger::read_all_lossy(&episodes_path).await.unwrap();
        assert_eq!(episodes.len(), 1);

        let episode = &episodes[0];
        assert_eq!(episode.agent_id, "claude");
        assert_eq!(episode.task_id, "prd:draft:new:demo");
        assert_eq!(episode.kind, "agent_turn");
        assert_eq!(episode.model, "claude-sonnet-4-6");
        assert!(episode.success);
        assert_eq!(
            episode.extra.get("task_kind"),
            Some(&serde_json::json!("prd-draft-new"))
        );
        assert_eq!(
            episode.extra.get("provider"),
            Some(&serde_json::json!("anthropic"))
        );
        assert_eq!(
            episode.extra.get("role"),
            Some(&serde_json::json!("Strategist"))
        );
        assert_eq!(
            episode.extra.get("task_category"),
            Some(&serde_json::json!("docs"))
        );
        assert_eq!(
            episode.extra.get("complexity_band"),
            Some(&serde_json::json!("standard"))
        );
        assert_eq!(
            episode.extra.get("plan_id"),
            Some(&serde_json::json!("demo"))
        );
        assert_eq!(
            episode.extra.get("session_id"),
            Some(&serde_json::json!("resume-123"))
        );
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
    fn cli_parses_config_wizard_alias() {
        let cli = Cli::try_parse_from(["roko", "config", "wizard"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Init { .. }
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
    fn cli_parses_config_secrets_subcommand() {
        let cli =
            Cli::try_parse_from(["roko", "config", "secrets", "get", "anthropic.api_key"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Secrets { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_replay_subcommand() {
        let cli = Cli::try_parse_from(["roko", "replay", "abcd1234"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Replay { .. })));
    }

    #[test]
    fn cli_parses_completions_subcommand() {
        let cli = Cli::try_parse_from(["roko", "completions", "zsh"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Completions {
                shell: CompletionShell::Zsh
            })
        ));
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
    fn cli_parses_knowledge_query_subcommand() {
        let cli = Cli::try_parse_from(["roko", "knowledge", "query", "rust async"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Knowledge {
                cmd: KnowledgeCmd::Query { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_knowledge_stats_subcommand() {
        let cli = Cli::try_parse_from(["roko", "knowledge", "stats"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Knowledge {
                cmd: KnowledgeCmd::Stats { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_knowledge_gc_subcommand() {
        let cli = Cli::try_parse_from(["roko", "knowledge", "gc"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Knowledge {
                cmd: KnowledgeCmd::Gc { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_knowledge_backup_subcommand() {
        let cli =
            Cli::try_parse_from(["roko", "knowledge", "backup", "/tmp/neuro-backup"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Knowledge {
                cmd: KnowledgeCmd::Backup { destination, force, .. }
            }) if destination == PathBuf::from("/tmp/neuro-backup") && !force
        ));
    }

    #[test]
    fn cli_parses_knowledge_restore_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "knowledge",
            "restore",
            "/tmp/neuro-backup",
            "--force",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Knowledge {
                cmd: KnowledgeCmd::Restore { source, force, .. }
            }) if source == PathBuf::from("/tmp/neuro-backup") && force
        ));
    }

    #[test]
    fn cli_parses_config_experiments_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "config",
            "experiments",
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
            Some(Command::Config {
                cmd: ConfigCmd::Experiments { .. }
            })
        ));
    }

    #[test]
    fn cli_parses_config_providers_list_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "providers", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Providers {
                    cmd: ConfigProviderCmd::List { .. }
                }
            })
        ));
    }

    #[test]
    fn cli_parses_config_providers_health_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "providers", "health"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Providers {
                    cmd: ConfigProviderCmd::Health { .. }
                }
            })
        ));
    }

    #[test]
    fn cli_parses_config_providers_test_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "providers", "test", "zai"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Providers {
                    cmd: ConfigProviderCmd::Test { provider: Some(ref p), all: false, .. }
                }
            }) if p == "zai"
        ));
    }

    #[test]
    fn cli_parses_config_providers_test_all() {
        let cli = Cli::try_parse_from(["roko", "config", "providers", "test", "--all"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Providers {
                    cmd: ConfigProviderCmd::Test {
                        provider: None,
                        all: true,
                        ..
                    }
                }
            })
        ));
    }

    #[test]
    fn cli_parses_config_models_list_subcommand() {
        let cli = Cli::try_parse_from(["roko", "config", "models", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Models {
                    cmd: ConfigModelCmd::List { .. }
                }
            })
        ));
    }

    #[test]
    fn cli_parses_config_models_route_subcommand() {
        let cli = Cli::try_parse_from([
            "roko",
            "config",
            "models",
            "route",
            "glm-5-1",
            "--explain",
            "--complexity",
            "integrative",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                cmd: ConfigCmd::Models { cmd: ConfigModelCmd::Route { model, explain: true, complexity: Some(complexity), .. } }
            }) if model == "glm-5-1" && complexity == "integrative"
        ));
    }

    #[test]
    fn cli_agent_chat_defaults_to_canonical_serve_url() {
        let cli = Cli::try_parse_from(["roko", "agent", "chat"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Agent { .. })));
    }

    #[test]
    fn cli_daemon_start_defaults_to_canonical_port() {
        let cli = Cli::try_parse_from(["roko", "daemon", "start"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Daemon {
                cmd: DaemonCmd::Start { port, .. }
            }) if port == roko_cli::DEFAULT_SERVE_PORT
        ));
    }

    #[test]
    fn cli_daemon_restart_defaults_to_canonical_port() {
        let cli = Cli::try_parse_from(["roko", "daemon", "restart"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Daemon {
                cmd: DaemonCmd::Restart { port }
            }) if port == roko_cli::DEFAULT_SERVE_PORT
        ));
    }

    #[test]
    fn cli_parses_prd_draft_new_instead_of_top_level_new() {
        let cli = Cli::try_parse_from(["roko", "prd", "draft", "new", "Ship", "it"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Prd {
                cmd: PrdCmd::Draft {
                    cmd: PrdDraftCmd::New { title }
                }
            }) if title == vec!["Ship".to_string(), "it".to_string()]
        ));
    }

    #[test]
    fn cli_new_requires_type_and_name() {
        // `roko new` is a subcommand requiring <TYPE> <NAME>.
        assert!(Cli::try_parse_from(["roko", "new"]).is_err());
        assert!(Cli::try_parse_from(["roko", "new", "gate", "MyGate"]).is_ok());
    }

    #[test]
    fn cli_explain_requires_topic() {
        // `roko explain` is a subcommand requiring <TOPIC>.
        assert!(Cli::try_parse_from(["roko", "explain"]).is_err());
        assert!(Cli::try_parse_from(["roko", "explain", "gates"]).is_ok());
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
            hdc_diversity: 0.73,
            convergence_velocity: 0.66,
            turn_taking_equality: 0.74,
            social_perceptiveness: 0.68,
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
        // The guard requires roko.toml or .roko/ to exist before creating dirs.
        std::fs::write(tmp.path().join("roko.toml"), b"").unwrap();
        bootstrap_observability_dirs(tmp.path()).unwrap();
        let roko = tmp.path().join(".roko");
        assert!(roko.join("traces").is_dir());
        assert!(roko.join("metrics").is_dir());
        assert!(roko.join("runtime").is_dir());
        assert!(roko.join("runs").is_dir());
    }

    #[test]
    fn bootstrap_observability_dirs_skips_without_intent() {
        let tmp = tempfile::tempdir().unwrap();
        // No roko.toml or .roko/ — guard should skip creation.
        bootstrap_observability_dirs(tmp.path()).unwrap();
        assert!(!tmp.path().join(".roko").exists());
    }

    #[test]
    fn backup_neuro_store_copies_live_files_into_snapshot_dir() {
        let workdir = tempdir().unwrap();
        let backup_dir = tempdir().unwrap();
        let neuro_dir = workdir.path().join(".roko").join("neuro");
        std::fs::create_dir_all(&neuro_dir).unwrap();
        std::fs::write(neuro_dir.join(NEURO_KNOWLEDGE_FILE), b"{\"id\":\"k1\"}\n").unwrap();
        std::fs::write(
            neuro_dir.join(NEURO_CONFIRMATIONS_FILE),
            b"{\"id\":\"c1\"}\n",
        )
        .unwrap();

        let report = backup_neuro_store(workdir.path(), backup_dir.path(), false, None).unwrap();

        assert_eq!(
            std::fs::read(report.snapshot.knowledge).unwrap(),
            b"{\"id\":\"k1\"}\n"
        );
        assert_eq!(
            std::fs::read(report.snapshot.confirmations).unwrap(),
            b"{\"id\":\"c1\"}\n"
        );
        assert!(report.confirmations_present);
    }

    #[test]
    fn restore_neuro_store_requires_force_for_existing_target_and_removes_stale_optional_file() {
        let workdir = tempdir().unwrap();
        let backup_dir = tempdir().unwrap();
        let neuro_dir = workdir.path().join(".roko").join("neuro");
        std::fs::create_dir_all(&neuro_dir).unwrap();
        std::fs::write(
            neuro_dir.join(NEURO_KNOWLEDGE_FILE),
            b"{\"id\":\"old\",\"content\":\"old data\",\"confidence\":0.5}\n",
        )
        .unwrap();
        std::fs::write(neuro_dir.join(NEURO_CONFIRMATIONS_FILE), b"stale\n").unwrap();
        std::fs::write(
            backup_dir.path().join(NEURO_KNOWLEDGE_FILE),
            b"{\"id\":\"new\",\"content\":\"new data\",\"confidence\":0.9}\n",
        )
        .unwrap();

        let err = restore_neuro_store(workdir.path(), backup_dir.path(), false, 1, None, None)
            .unwrap_err();
        assert!(err.to_string().contains("Re-run with --force"));

        let report =
            restore_neuro_store(workdir.path(), backup_dir.path(), true, 1, None, None).unwrap();
        let restored = std::fs::read_to_string(&report.live.knowledge).unwrap();
        assert!(
            restored.contains("\"new\""),
            "restored store should contain the new entry"
        );
        // The backup has no confirmations file, so the report should note it as absent.
        assert!(!report.confirmations_present);
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

    #[test]
    fn tracing_log_directive_prefers_rust_log() {
        let directive = tracing_log_directive_from(Some("roko=debug".into()), Some("info".into()));
        assert_eq!(directive, "roko=debug");
    }

    #[test]
    fn tracing_log_directive_falls_back_to_roko_log_and_default() {
        let directive = tracing_log_directive_from(None, Some("roko=trace".into()));
        assert_eq!(directive, "roko=trace");

        let default_directive = tracing_log_directive_from(None, None);
        assert_eq!(default_directive, "roko=info");
    }
}

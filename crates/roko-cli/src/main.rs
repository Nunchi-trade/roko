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
use roko_cli::tui::{App, ApprovalChannel};
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
use std::sync::Arc;
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
  Development:     run, plan, prd, research, chat
  Agent:           agent, inject
  Monitoring:      status, dashboard, serve, doctor, learn, replay, explain
  Configuration:   init, config, secret, tune
  Knowledge:       neuro, dream, dreams, experiment
  Infrastructure:  provider, model, subscription, event-sources, daemon, worker, deploy, custody
  Tooling:         index, new, plugin, archive, update, completions"
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

    /// Manage marketplace jobs (list, create, show, execute, cancel).
    Job {
        #[command(subcommand)]
        cmd: JobCmd,
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
    Query { topic: Vec<String>, workdir: Option<PathBuf> },
    Stats { workdir: Option<PathBuf> },
    Gc { workdir: Option<PathBuf> },
    Backup { workdir: Option<PathBuf>, destination: PathBuf, force: bool, top_n: Option<usize> },
    Restore { workdir: Option<PathBuf>, source: PathBuf, force: bool, types: Option<String>, min_confidence: Option<f64>, generation: u32 },
    Sync { peer: String, workdir: Option<PathBuf>, direction: String, max_send: usize },
}

// Internal enum used by cmd_dream — mirrors the old top-level DreamCmd.
#[derive(Debug)]
enum DreamCmdLegacy {
    Run { workdir: Option<PathBuf> },
    Report { workdir: Option<PathBuf> },
    Schedule { workdir: Option<PathBuf> },
}

// Internal enum used by cmd_subscription — mirrors the old top-level SubscriptionCmd.
#[derive(Debug)]
enum SubscriptionCmdLegacy {
    List,
    Add { template: String, trigger: String },
    Remove { id: String },
    Enable { id: String },
    Disable { id: String },
}

// Internal enum used by cmd_event_sources
#[derive(Debug)]
enum EventSourcesCmdLegacy {
    List { workdir: Option<PathBuf> },
}

// Internal enum used by cmd_provider/cmd_model
#[derive(Debug)]
enum ProviderCmdLegacy {
    List { workdir: Option<PathBuf> },
    Health { workdir: Option<PathBuf> },
    Test { provider: String, workdir: Option<PathBuf> },
}

#[derive(Debug)]
enum ModelCmdLegacy {
    List { workdir: Option<PathBuf> },
    Route { model: String, explain: bool, complexity: Option<String>, workdir: Option<PathBuf> },
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
    // ── Core config management ──────────────────────────────────────
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
        /// Provider name from `[providers.*]`.
        provider: String,
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

    // ── Color mode ──────────────────────────────────────────────────
    let use_color = cli.color.should_color();

    // ── Timing mode ─────────────────────────────────────────────────
    let timing_enabled = cli.timing
        || env::var("ROKO_TIMING")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    let started_at = Instant::now();

    let filter = tracing_subscriber::EnvFilter::try_new(tracing_log_directive())
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("roko=info"));

    // ROKO_LOG_RAW=1 disables secret redaction (useful for debugging).
    let raw_logs = env::var("ROKO_LOG_RAW")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let ansi_logs = use_color;
    if raw_logs {
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
    // If there is an explicit subcommand, handle it.
    if let Some(command) = cli.command.take() {
        return dispatch_subcommand(command, &cli).await;
    }

    // Headless daemon mode.
    if cli.headless {
        return cmd_headless(&cli).await;
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
        // ── Core workflow ────────────────────────────────────────────
        Command::Init {
            path,
            cloud,
            profile,
        } => {
            cmd_init(path, cloud, profile).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Run { prompt, workdir } => cmd_run(cli, workdir, prompt).await,
        Command::Status { workdir, cfactor } => {
            cmd_status(cli, workdir, cfactor).await?;
            Ok(EXIT_SUCCESS)
        }
        Command::Doctor { workdir, serve_url } => cmd_doctor(cli, workdir, serve_url).await,

        // ── Planning & PRDs ─────────────────────────────────────────
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

        // ── Agents ──────────────────────────────────────────────────
        Command::Agent { cmd } => cmd_agent(cli, cmd).await,

        // ── Research ────────────────────────────────────────────────
        Command::Research { cmd } => {
            let result = cmd_research(cli, cmd).await;
            let _ = roko_cli::index::rebuild_all(&std::env::current_dir().unwrap_or_default());
            result
        }

        // ── Knowledge ───────────────────────────────────────────────
        Command::Knowledge { cmd } => dispatch_knowledge(cli, cmd).await,

        // ── Learning ────────────────────────────────────────────────
        Command::Learn { cmd } => dispatch_learn(cli, cmd).await,

        // ── Jobs ────────────────────────────────────────────────────
        Command::Job { cmd } => cmd_job(cli, cmd).await,

        // ── Config ──────────────────────────────────────────────────
        Command::Config { cmd } => {
            // Experiments need &Cli for resolve_workdir, so intercept here.
            if let ConfigCmd::Experiments { cmd: exp_cmd } = cmd {
                return dispatch_experiment(cli, exp_cmd);
            }
            dispatch_config(cmd).await?;
            Ok(EXIT_SUCCESS)
        }

        // ── Code intelligence ───────────────────────────────────────
        Command::Index { cmd } => cmd_index(cli, cmd),

        // ── Server & deployment ─────────────────────────────────────
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
        Command::Daemon { cmd } => cmd_daemon(cli, cmd).await,
        Command::Deploy { cmd } => cmd_deploy(cli, cmd).await,
        Command::Worker { port } => {
            roko_cli::worker::run_worker(port).await?;
            Ok(EXIT_SUCCESS)
        }

        // ── Interactive ─────────────────────────────────────────────
        Command::Dashboard {
            page,
            list_pages,
            text,
            workdir,
            high_contrast,
            reduced_motion,
        } => {
            #[allow(unsafe_code)]
            if high_contrast {
                unsafe { std::env::set_var("ROKO_HIGH_CONTRAST", "1") };
            }
            #[allow(unsafe_code)]
            if reduced_motion {
                unsafe { std::env::set_var("ROKO_REDUCED_MOTION", "1") };
            }
            cmd_dashboard(cli, workdir, page, list_pages, text, None).await
        }

        // ── Utilities ───────────────────────────────────────────────
        Command::Replay {
            hash,
            workdir,
            forensic,
        } => cmd_replay(workdir, hash, forensic).await,
        Command::Inject {
            session,
            kind,
            payload,
            workdir,
        } => cmd_inject(cli, session, &kind, payload, workdir),
        Command::Completions { shell } => {
            print_completions(shell);
            Ok(EXIT_SUCCESS)
        }
        Command::New {
            type_name,
            name,
            output,
        } => {
            let output_dir = output.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            match roko_cli::scaffold::scaffold(&type_name, &name, &output_dir) {
                Ok(files) => {
                    println!(
                        "scaffolded `{type_name}` as `{name}` ({} file{})",
                        files.len(),
                        if files.len() == 1 { "" } else { "s" }
                    );
                    for f in &files {
                        println!("  {}", f.display());
                    }
                    Ok(EXIT_SUCCESS)
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    Ok(EXIT_SYSTEM_ERROR)
                }
            }
        }
        Command::Explain { topic, depth } => {
            cmd_explain(&topic, depth);
            Ok(EXIT_SUCCESS)
        }
    }
}

// -----------------------------------------------------------------------
// Knowledge dispatch (neuro + dreams + custody + archive)
// -----------------------------------------------------------------------

async fn dispatch_knowledge(cli: &Cli, cmd: KnowledgeCmd) -> Result<i32> {
    match cmd {
        KnowledgeCmd::Query { topic, workdir } => {
            cmd_neuro(cli, NeuroCmd::Query { topic, workdir }).await
        }
        KnowledgeCmd::Stats { workdir } => {
            cmd_neuro(cli, NeuroCmd::Stats { workdir }).await
        }
        KnowledgeCmd::Gc { workdir } => {
            cmd_neuro(cli, NeuroCmd::Gc { workdir }).await
        }
        KnowledgeCmd::Backup { workdir, destination, force, top_n } => {
            cmd_neuro(cli, NeuroCmd::Backup { workdir, destination, force, top_n }).await
        }
        KnowledgeCmd::Restore { workdir, source, force, types, min_confidence, generation } => {
            cmd_neuro(cli, NeuroCmd::Restore { workdir, source, force, types, min_confidence, generation }).await
        }
        KnowledgeCmd::Sync { peer, workdir, direction, max_send } => {
            cmd_neuro(cli, NeuroCmd::Sync { peer, workdir, direction, max_send }).await
        }
        KnowledgeCmd::Dream { cmd } => dispatch_knowledge_dream(cli, cmd).await,
        KnowledgeCmd::Custody { cmd } => {
            dispatch_knowledge_custody(cli, cmd)?;
            Ok(EXIT_SUCCESS)
        }
        KnowledgeCmd::Archive { older_than, batch_size, workdir, dry_run } => {
            cmd_archive(cli, workdir, &older_than, batch_size, dry_run).await
        }
    }
}

async fn dispatch_knowledge_dream(cli: &Cli, cmd: KnowledgeDreamCmd) -> Result<i32> {
    match cmd {
        KnowledgeDreamCmd::Run { workdir } => {
            cmd_dream(cli, DreamCmdLegacy::Run { workdir }).await
        }
        KnowledgeDreamCmd::Report { workdir } => {
            cmd_dream(cli, DreamCmdLegacy::Report { workdir }).await
        }
        KnowledgeDreamCmd::Schedule { workdir } => {
            cmd_dream(cli, DreamCmdLegacy::Schedule { workdir }).await
        }
        KnowledgeDreamCmd::Journal { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &wd)?;
            let entries = runner.journal(limit)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("no dream journal entries");
            } else {
                for entry in &entries {
                    println!("{}", serde_json::to_string_pretty(entry).unwrap_or_default());
                }
            }
            Ok(EXIT_SUCCESS)
        }
        KnowledgeDreamCmd::Archive { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let runner = build_dream_runner(cli, &wd)?;
            let entries = runner.archive(limit)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("no dream archive entries");
            } else {
                for entry in &entries {
                    println!("{}", serde_json::to_string_pretty(entry).unwrap_or_default());
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

fn dispatch_knowledge_custody(cli: &Cli, cmd: KnowledgeCustodyCmd) -> Result<()> {
    match cmd {
        KnowledgeCustodyCmd::List { limit, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_list(&wd, limit)?;
        }
        KnowledgeCustodyCmd::Show { index, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_show(&wd, index)?;
        }
        KnowledgeCustodyCmd::Verify { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::custody::cmd_custody_verify(&wd)?;
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------
// Learn dispatch (learning state + tuning)
// -----------------------------------------------------------------------

async fn dispatch_learn(cli: &Cli, cmd: LearnCmd) -> Result<i32> {
    match cmd {
        LearnCmd::All { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "all").await
        }
        LearnCmd::Router { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "router").await
        }
        LearnCmd::Experiments { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "experiments").await
        }
        LearnCmd::Efficiency { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "efficiency").await
        }
        LearnCmd::Episodes { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "episodes").await
        }
        LearnCmd::Tune { subsystem, dry_run, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_tune(&wd, &subsystem, dry_run).await
        }
    }
}

async fn cmd_doctor(cli: &Cli, workdir: Option<PathBuf>, serve_url: Option<String>) -> Result<i32> {
    let report = roko_cli::doctor::run_doctor(&roko_cli::doctor::DoctorOptions {
        workdir: workdir.unwrap_or_else(|| resolve_workdir(cli)),
        config_override: cli.config.clone(),
        serve_url,
    })
    .await?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render_human());
    }

    Ok(report.exit_code())
}

async fn cmd_archive(
    cli: &Cli,
    workdir: Option<PathBuf>,
    older_than: &str,
    batch_size: usize,
    dry_run: bool,
) -> Result<i32> {
    let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let roko_dir = wd.join(".roko");
    if !roko_dir.exists() {
        bail!("no .roko/ directory found in {}", wd.display());
    }

    // Parse duration string (e.g. "30d", "7d", "24h").
    let max_age_ms = parse_duration_to_ms(older_than)
        .ok_or_else(|| anyhow!("invalid duration: {older_than} (expected e.g. '30d' or '7d')"))?;

    let cutoff_ms = chrono::Utc::now().timestamp_millis() - max_age_ms;

    // Open the hot substrate.
    let hot = roko_fs::FileSubstrate::open(&roko_dir).await?;

    // Query for old engrams.
    use roko_core::{Context, Query, Substrate};
    let ctx = Context::now();
    let query = Query::all().until(cutoff_ms).limit(batch_size);
    let candidates = hot.query(&query, &ctx).await?;

    if candidates.is_empty() {
        println!("no engrams older than {older_than} found");
        return Ok(EXIT_SUCCESS);
    }

    println!(
        "found {} engram(s) older than {older_than}{}",
        candidates.len(),
        if dry_run { " (dry run)" } else { "" }
    );

    if dry_run {
        for e in &candidates {
            let age_days = (chrono::Utc::now().timestamp_millis() - e.created_at_ms) / 86_400_000;
            println!("  {:?} | {} | {}d old", e.kind, &e.id, age_days);
        }
        return Ok(EXIT_SUCCESS);
    }

    // Confirm destructive operation (skipped in quiet / non-TTY mode).
    let prompt_msg = format!(
        "Archive {} engram(s) older than {older_than}?",
        candidates.len()
    );
    if !confirm_destructive(&prompt_msg, cli.quiet) {
        println!("aborted");
        return Ok(EXIT_SUCCESS);
    }

    // Open cold substrate and archive.
    let cold_dir = roko_dir.join("cold");
    let cold = roko_fs::ArchiveColdSubstrate::open(&cold_dir).await?;

    use roko_core::ColdSubstrate;
    let archived = cold.archive_batch(candidates.clone()).await?;

    // Prune archived engrams from hot storage.
    // Use prune with a weight threshold of f32::MAX to force-remove everything
    // below cutoff — but prune uses weight, not time. Instead we just log
    // that archival succeeded; hot-side cleanup happens via the normal prune path
    // on the next dream cycle.
    println!("archived {archived} engram(s) to {}", cold_dir.display());

    Ok(EXIT_SUCCESS)
}

/// Parse a human duration string like "30d" or "7d" or "24h" to milliseconds.
fn parse_duration_to_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "d" => Some(num * 24 * 3600 * 1000),
        "h" => Some(num * 3600 * 1000),
        "m" => Some(num * 60 * 1000),
        "s" => Some(num * 1000),
        _ => None,
    }
}

async fn cmd_daemon(cli: &Cli, cmd: DaemonCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    match cmd {
        DaemonCmd::Start { foreground, port } => {
            prepare_runtime_hooks(&workdir, cli.quiet);
            roko_cli::daemon::daemon_start(&workdir, foreground, port).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Stop => {
            roko_cli::daemon::daemon_stop(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Status => {
            roko_cli::daemon::daemon_status(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Logs { follow, lines } => {
            roko_cli::daemon::daemon_logs(&workdir, follow, lines).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Reload => {
            roko_cli::daemon::daemon_reload(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Restart { port } => {
            prepare_runtime_hooks(&workdir, cli.quiet);
            roko_cli::daemon::daemon_restart(&workdir, port).await?;
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

async fn cmd_agent(cli: &Cli, cmd: AgentCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    agent_serve::run(cmd).await?;
    Ok(EXIT_SUCCESS)
}

async fn cmd_plugin(cli: &Cli, cmd: PluginCmd) -> Result<i32> {
    let workdir = match &cmd {
        PluginCmd::List { workdir } => workdir.clone(),
        PluginCmd::Install { workdir, .. } => workdir.clone(),
        PluginCmd::Remove { workdir, .. } => workdir.clone(),
        PluginCmd::Audit { workdir } => workdir.clone(),
    }
    .unwrap_or_else(|| resolve_workdir(cli));

    match cmd {
        PluginCmd::List { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();

            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {} // Directory doesn't exist — fine
                }
            }

            if all_plugins.is_empty() {
                println!("no plugins found");
                println!(
                    "  search paths: {}, {}",
                    plugins_dir.display(),
                    roko_plugins.display()
                );
                println!("  install a plugin with: roko plugin install <path>");
            } else {
                println!("installed plugins ({}):", all_plugins.len());
                for plugin in &all_plugins {
                    let m = &plugin.manifest.plugin;
                    let desc = m.description.as_deref().unwrap_or("no description");
                    println!("  {} v{} — {}", m.name, m.version, desc);
                    if !plugin.manifest.prompts.is_empty() {
                        println!(
                            "    prompts: {}",
                            plugin
                                .manifest
                                .prompts
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    if !plugin.manifest.tools.is_empty() {
                        println!(
                            "    tools: {}",
                            plugin
                                .manifest
                                .tools
                                .iter()
                                .map(|t| t.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    if !plugin.manifest.profiles.is_empty() {
                        println!(
                            "    profiles: {}",
                            plugin
                                .manifest
                                .profiles
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Install { source, .. } => {
            let source_path = std::path::Path::new(&source);

            // Find the manifest file.
            let manifest_path = if source_path.is_file() {
                source_path.to_path_buf()
            } else if source_path.is_dir() {
                let candidate = source_path.join("plugin.toml");
                if candidate.exists() {
                    candidate
                } else {
                    eprintln!("error: no plugin.toml found in {}", source_path.display());
                    return Ok(EXIT_SYSTEM_ERROR);
                }
            } else {
                eprintln!("error: source path does not exist: {source}");
                return Ok(EXIT_SYSTEM_ERROR);
            };

            // Load and validate the manifest.
            let manifest = match roko_plugin::manifest::load_manifest(&manifest_path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: failed to load plugin manifest: {e}");
                    return Ok(EXIT_SYSTEM_ERROR);
                }
            };

            // Copy to .roko/plugins/<name>/
            let install_dir = workdir
                .join(".roko")
                .join("plugins")
                .join(&manifest.plugin.name);
            std::fs::create_dir_all(&install_dir)?;

            // Copy the manifest.
            let dest_manifest = install_dir.join("plugin.toml");
            std::fs::copy(&manifest_path, &dest_manifest)?;

            // Copy the containing directory's files if source is a directory.
            if source_path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(source_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path != manifest_path {
                            let dest = install_dir.join(entry.file_name());
                            std::fs::copy(&path, &dest)?;
                        }
                    }
                }
            }

            println!(
                "installed plugin `{}` v{} to {}",
                manifest.plugin.name,
                manifest.plugin.version,
                install_dir.display()
            );
            println!(
                "  {} prompt(s), {} profile(s), {} tool(s), {} trigger(s)",
                manifest.prompts.len(),
                manifest.profiles.len(),
                manifest.tools.len(),
                manifest.triggers.len(),
            );
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Remove { name, .. } => {
            let install_dir = workdir.join(".roko").join("plugins").join(&name);
            if !install_dir.exists() {
                eprintln!("error: plugin `{name}` is not installed");
                eprintln!("  expected at: {}", install_dir.display());
                return Ok(EXIT_SYSTEM_ERROR);
            }
            std::fs::remove_dir_all(&install_dir)?;
            println!("removed plugin `{name}` from {}", install_dir.display());
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Audit { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();

            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {}
                }
            }

            if all_plugins.is_empty() {
                println!("no plugins to audit");
            } else {
                println!("plugin audit ({} plugins):", all_plugins.len());
                for plugin in &all_plugins {
                    let m = &plugin.manifest;
                    println!("\n  {} v{}", m.plugin.name, m.plugin.version);
                    println!("    location: {}", plugin.base_dir.display());

                    // Tier capabilities
                    let mut tiers = Vec::new();
                    if !m.prompts.is_empty() {
                        tiers.push(format!("T1:prompts({})", m.prompts.len()));
                    }
                    if !m.profiles.is_empty() {
                        tiers.push(format!("T2:profiles({})", m.profiles.len()));
                    }
                    if !m.tools.is_empty() {
                        tiers.push(format!("T3:tools({})", m.tools.len()));
                    }
                    println!(
                        "    capabilities: {}",
                        if tiers.is_empty() {
                            "none".to_string()
                        } else {
                            tiers.join(", ")
                        }
                    );

                    // Tools with their commands (security audit)
                    for tool in &m.tools {
                        println!(
                            "    tool `{}`: `{}` (timeout: {}ms)",
                            tool.name, tool.command, tool.timeout_ms
                        );
                    }

                    // Triggers
                    for trigger in &m.triggers {
                        match trigger {
                            roko_plugin::manifest::TriggerDef::Cron { expression, .. } => {
                                println!("    trigger: cron({expression})");
                            }
                            roko_plugin::manifest::TriggerDef::FileWatch { paths, .. } => {
                                println!("    trigger: file_watch({})", paths.join(", "));
                            }
                            roko_plugin::manifest::TriggerDef::Webhook { path, .. } => {
                                println!("    trigger: webhook({path})");
                            }
                        }
                    }

                    // Dependencies
                    for dep in &m.dependencies {
                        println!(
                            "    requires: {} {}",
                            dep.name,
                            dep.version.as_deref().unwrap_or("*")
                        );
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

// -----------------------------------------------------------------------
// Mode handlers
// -----------------------------------------------------------------------

fn cmd_explain(topic: &str, depth: u8) {
    use roko_cli::explain;
    let depth = depth.clamp(1, 3);
    if topic == "topics" || topic == "list" {
        println!("available topics:");
        for name in explain::topic_names() {
            let entry = explain::find_topic(name).unwrap();
            println!("  {:<12} {}", name, entry.title);
        }
        return;
    }
    match explain::find_topic(topic) {
        Some(entry) => print!("{}", explain::render_topic(entry, depth)),
        None => {
            eprintln!("unknown topic: {topic}");
            eprintln!("available topics: {}", explain::topic_names().join(", "));
            eprintln!("run `roko explain topics` to see all topics with descriptions");
        }
    }
}

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

async fn cmd_headless(cli: &Cli) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    roko_cli::daemon::daemon_start(&workdir, false, roko_cli::DEFAULT_SERVE_PORT).await?;
    Ok(EXIT_SUCCESS)
}

async fn cmd_dashboard(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
    text: bool,
    state_hub: Option<roko_core::SharedStateHub>,
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
        let app = if let Some(state_hub) = state_hub.as_ref() {
            App::new_connected_with_page(&workdir, initial_page, state_hub)
        } else {
            App::new_with_page(&workdir, initial_page)
        };
        if app.run().is_ok() {
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
                    format_percent(cfactor.components.social_perceptiveness)
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
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: DaimonPolicy::default(),
        thinking_level: None,
        temperament: Some(config.agent.temperament_for_role(role.label())),
        previous_model: Some(requested_slug.clone()),
        plan_context_tokens: None,
        tier_thresholds: None,
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
        // ── Providers ───────────────────────────────────────────────
        ConfigCmd::Providers { cmd } => {
            match cmd {
                ConfigProviderCmd::List { workdir } => {
                    let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
                    cmd_provider_list(&wd).await?;
                    Ok(())
                }
                ConfigProviderCmd::Health { workdir } => {
                    let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
                    cmd_provider_health(&wd)?;
                    Ok(())
                }
                ConfigProviderCmd::Test { provider, workdir } => {
                    let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
                    cmd_provider_test(&wd, &provider).await?;
                    Ok(())
                }
            }
        }
        // ── Models ──────────────────────────────────────────────────
        ConfigCmd::Models { cmd } => {
            match cmd {
                ConfigModelCmd::List { workdir } => {
                    let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
                    cmd_model_list(&wd)?;
                    Ok(())
                }
                ConfigModelCmd::Route { model, explain, complexity, workdir } => {
                    let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
                    cmd_model_route(&wd, &model, explain, complexity.as_deref())?;
                    Ok(())
                }
            }
        }
        // ── Subscriptions ───────────────────────────────────────────
        ConfigCmd::Subscriptions { cmd } => {
            dispatch_config_subscriptions(cmd).await?;
            Ok(())
        }
        // ── Event sources ───────────────────────────────────────────
        ConfigCmd::Events { workdir } => {
            let wd = workdir.unwrap_or_else(|| PathBuf::from("."));
            roko_cli::event_sources::cmd_list(&wd, false)?;
            Ok(())
        }
        // ── Experiments (intercepted in dispatch_subcommand) ────────
        ConfigCmd::Experiments { .. } => {
            unreachable!("experiments dispatched in dispatch_subcommand")
        }
        // ── Plugins ─────────────────────────────────────────────────
        ConfigCmd::Plugins { cmd } => {
            dispatch_config_plugins(cmd).await?;
            Ok(())
        }
        // ── Secrets ─────────────────────────────────────────────────
        ConfigCmd::Secrets { cmd } => {
            let wd = PathBuf::from(".");
            roko_cli::secrets::dispatch_secrets(&cmd, &wd)?;
            Ok(())
        }
    }
}

// dispatch_config_subscriptions is handled inline via ConfigCmd::Subscriptions

async fn dispatch_config_plugins(cmd: PluginCmd) -> Result<()> {
    let workdir = match &cmd {
        PluginCmd::List { workdir } => workdir.clone(),
        PluginCmd::Install { workdir, .. } => workdir.clone(),
        PluginCmd::Remove { workdir, .. } => workdir.clone(),
        PluginCmd::Audit { workdir } => workdir.clone(),
    }
    .unwrap_or_else(|| PathBuf::from("."));

    match cmd {
        PluginCmd::List { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();
            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {}
                }
            }
            if all_plugins.is_empty() {
                println!("no plugins found");
                println!(
                    "  search paths: {}, {}",
                    plugins_dir.display(),
                    roko_plugins.display()
                );
                println!("  install a plugin with: roko config plugins install <path>");
            } else {
                println!("installed plugins ({}):", all_plugins.len());
                for plugin in &all_plugins {
                    println!("  {} v{}", plugin.name, plugin.version);
                    if let Some(desc) = &plugin.description {
                        println!("    {desc}");
                    }
                }
            }
        }
        PluginCmd::Install { source, .. } => {
            roko_plugin::manifest::install_plugin(&workdir, &source)?;
            println!("installed plugin from {source}");
        }
        PluginCmd::Remove { name, .. } => {
            roko_plugin::manifest::remove_plugin(&workdir, &name)?;
            println!("removed plugin {name}");
        }
        PluginCmd::Audit { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();
            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {}
                }
            }
            if all_plugins.is_empty() {
                println!("no plugins to audit");
            } else {
                for plugin in &all_plugins {
                    println!("{} v{}", plugin.name, plugin.version);
                    println!("  capabilities: {:?}", plugin.capabilities);
                }
            }
        }
    }
    Ok(())
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
            let summaries =
                roko_cli::plan::summarize_discovered_plans(&wd).map_err(|e| anyhow!("{e}"))?;

            if cli.json {
                println!("{}", roko_cli::plan::format_plan_list_json(&summaries));
            } else {
                println!("{}", roko_cli::plan::format_plan_list(&summaries));
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Show { plan_id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let Some(plan_info) =
                roko_cli::plan::discover_plan_by_id(&wd, &plan_id).map_err(|e| anyhow!("{e}"))?
            else {
                eprintln!("plan '{plan_id}' not found");
                return Ok(EXIT_AGENT_FAILURE);
            };
            let summary = roko_cli::plan::summarize_plan_info(&plan_info);
            let tasks_path = roko_cli::plan::tasks_path(&plan_info);
            let stable_id = roko_cli::plan::stable_plan_id(&plan_info);

            if cli.json {
                let payload = json!({
                    "plan_id": stable_id,
                    "base": plan_info.base,
                    "title": summary.title,
                    "plan_path": plan_info.path,
                    "tasks_path": tasks_path,
                    "task_count": summary.task_count,
                    "frontmatter": plan_info.frontmatter,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("plan: {stable_id}");
                println!("base: {}", plan_info.base);
                println!("title: {}", summary.title);
                println!("plan file: {}", plan_info.path.display());
                println!(
                    "tasks file: {}",
                    tasks_path
                        .as_deref()
                        .filter(|path| path.is_file())
                        .map_or_else(|| "(none)".to_string(), |path| path.display().to_string())
                );
                println!("task count: {}", summary.task_count);
                if let Some(frontmatter) = plan_info.frontmatter.as_ref() {
                    if !frontmatter.depends_on.is_empty() {
                        println!("depends_on: {}", frontmatter.depends_on.join(", "));
                    }
                    if !frontmatter.parallel_with.is_empty() {
                        println!("parallel_with: {}", frontmatter.parallel_with.join(", "));
                    }
                    if let Some(priority) = frontmatter.priority {
                        println!("priority: {priority}");
                    }
                    if !frontmatter.tags.is_empty() {
                        println!("tags: {}", frontmatter.tags.join(", "));
                    }
                    if let Some(milestone) = frontmatter.milestone.as_deref() {
                        println!("milestone: {milestone}");
                    }
                }
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
            let plan_dir = plans_dir.join(&plan_id);
            let legacy_plan = plans_dir.join(format!("{plan_id}.md"));
            if plan_dir.exists() || legacy_plan.exists() {
                bail!("plan '{plan_id}' already exists");
            }
            std::fs::create_dir_all(&plan_dir).map_err(|e| anyhow!("create plan dir: {e}"))?;
            let plan_md_path = plan_dir.join("plan.md");
            let tasks_path = plan_dir.join("tasks.toml");

            let yaml_plan_id = serde_json::to_string(&plan.id)?;
            let plan_md = format!(
                "---\nplan: {yaml_plan_id}\n---\n# {}\n\n{}\n",
                plan.title,
                if plan.description.is_empty() {
                    "Describe the plan here.".to_string()
                } else {
                    plan.description.clone()
                }
            );
            let tasks_toml = format!(
                "[meta]\nplan = {:?}\nmax_parallel = 1\n\n# Add [[task]] entries below.\n",
                plan.id
            );
            std::fs::write(&plan_md_path, plan_md)
                .map_err(|e| anyhow!("write {}: {e}", plan_md_path.display()))?;
            std::fs::write(&tasks_path, tasks_toml)
                .map_err(|e| anyhow!("write {}: {e}", tasks_path.display()))?;

            if cli.json {
                let payload = json!({
                    "created": plan_id,
                    "plan_dir": plan_dir,
                    "plan_path": plan_md_path,
                    "tasks_path": tasks_path,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if !cli.quiet {
                println!("created plan '{}' at {}", plan_id, plan_dir.display());
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Validate { dir, strict, json } => {
            cmd_plan_validate(&dir, strict, json || cli.json)
        }
        PlanCmd::Run {
            plans_dir,
            workdir,
            resume_plan,
            approval,
            max_retries,
            dry_run,
        } => {
            // ── Dry-run mode: parse plans + show summary without executing ──
            if dry_run {
                return cmd_plan_dry_run(&plans_dir, cli).await;
            }

            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            prepare_runtime_hooks(&wd, cli.quiet);
            let config = load_layered(&wd)?.config;
            let state_hub = roko_core::shared_state_hub();

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
                let snapshot = roko_cli::snapshot_migrate::load_executor_snapshot(&exec_json)
                    .map_err(|e| anyhow!("bad snapshot {}: {e}", snap_path.display()))?;
                let discovered_plans = roko_orchestrator::discover_plans(&plans_dir)
                    .map_err(|e| anyhow!("plan discovery failed: {e}"))?;
                roko_cli::snapshot_reconcile::reconcile_snapshot_vs_plans(
                    &snapshot,
                    &discovered_plans,
                    &snap_path,
                    &plans_dir,
                )?;
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
            runner.set_state_hub(state_hub.sender());
            if let Some(retries) = max_retries {
                runner.set_max_retries_override(retries);
            }

            if approval {
                if !std::io::stdout().is_terminal() {
                    anyhow::bail!("approval mode requires an interactive terminal");
                }

                let approval_channel = ApprovalChannel::new(16);
                let state_hub_for_tui = state_hub.clone();
                let workdir_for_tui = wd.clone();
                let approval_rx = approval_channel.rx;
                let process_supervisor: Arc<_> = runner.supervisor_handle();

                std::thread::Builder::new()
                    .name("roko-plan-approval-tui".to_string())
                    .spawn(move || {
                        let mut app = App::new_connected_with_page(
                            &workdir_for_tui,
                            None,
                            &state_hub_for_tui,
                        );
                        app.set_process_supervisor(process_supervisor);
                        app.approval_rx = Some(approval_rx);
                        if let Err(err) = app.run() {
                            tracing::error!(error = %err, "approval TUI exited with error");
                        }
                    })
                    .context("spawn approval TUI thread")?;

                runner.set_approval_tx(Some(approval_channel.tx));
            }

            let report = runner.run(&plans_dir).await?;

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
            use roko_cli::agent_config::{load_gateway_env, model_from_config};
            use roko_cli::agent_exec::{AgentExecEpisode, AgentExecOpts, run_agent_logged};

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
            let task_id = from_file
                .as_ref()
                .and_then(|path| path.file_stem())
                .and_then(|stem| stem.to_str())
                .map(|stem| format!("plan:generate:{stem}"))
                .unwrap_or_else(|| "plan:generate:prompt".to_string());
            let system = roko_cli::plan_generate::build_generation_prompt(
                &workdir,
                &source_text,
                source_type,
            );

            let task_prompt = format!(
                "Read the source below and generate implementation plan directories under .roko/plans/. \
                 Search the codebase first to understand what exists. \
                 Create plan.md and tasks.toml files with tier, model_hint, context (read_files with line ranges), \
                 mcp_servers (per-task MCP server names), and verify steps (executable shell commands). \
                 Use the cheapest model tier for each task.\n\n{source_text}"
            );

            run_agent_logged(
                AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: Some("high"),
                    system_prompt: Some(&system),
                    resume_session: None,
                    env_vars: &gw.vars,
                    role: Some("strategist"),
                },
                AgentExecEpisode {
                    task_kind: "plan-generate",
                    task_id: &task_id,
                },
            )
            .await
        }
        PlanCmd::Regenerate { plan_dir, dry_run } => {
            use roko_cli::agent_config::{load_gateway_env, model_from_config};
            use roko_cli::agent_exec::{AgentExecEpisode, AgentExecOpts, run_agent_logged};

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
            let plan_name = plan_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            let task_id = format!("plan:regenerate:{plan_name}");

            let exit_code = match run_agent_logged(
                AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: Some("high"),
                    system_prompt: Some(&system),
                    resume_session: None,
                    env_vars: &gw.vars,
                    role: Some("strategist"),
                },
                AgentExecEpisode {
                    task_kind: "plan-regenerate",
                    task_id: &task_id,
                },
            )
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
// Dry-run for `plan run`
// -----------------------------------------------------------------------

/// Parse and display a plan directory without executing anything.
async fn cmd_plan_dry_run(plans_dir: &Path, cli: &Cli) -> Result<i32> {
    let plans = roko_orchestrator::discover_plans(plans_dir)
        .map_err(|e| anyhow!("plan discovery failed: {e}"))?;

    if plans.is_empty() {
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "plans": [],
                    "total_plans": 0,
                    "total_tasks": 0,
                }))?
            );
        } else {
            println!("No plans found in {}", plans_dir.display());
        }
        return Ok(EXIT_SUCCESS);
    }

    // For each plan, try to load and count tasks.
    let mut plan_summaries: Vec<serde_json::Value> = Vec::new();
    let mut total_tasks: usize = 0;
    let mut total_estimated_minutes: u32 = 0;

    for plan in &plans {
        // Try loading the tasks.toml adjacent to the plan file.
        let tasks_path = plan
            .path
            .parent()
            .map(|p| p.join("tasks.toml"))
            .filter(|p| p.exists());

        let (task_count, task_details) = if let Some(ref tp) = tasks_path {
            match roko_cli::task_parser::TasksFile::parse(tp) {
                Ok(tf) => {
                    let details: Vec<serde_json::Value> = tf
                        .tasks
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "title": t.title,
                                "status": t.status,
                                "tier": t.tier,
                                "depends_on": t.depends_on,
                                "files": t.files.len(),
                            })
                        })
                        .collect();
                    (tf.tasks.len(), details)
                }
                Err(_) => (0, vec![]),
            }
        } else {
            // New-layout plans might have tasks.toml at plans_dir/plan_name/tasks.toml
            let dir_tasks = plans_dir.join(&plan.base).join("tasks.toml");
            if dir_tasks.exists() {
                match roko_cli::task_parser::TasksFile::parse(&dir_tasks) {
                    Ok(tf) => {
                        let details: Vec<serde_json::Value> = tf
                            .tasks
                            .iter()
                            .map(|t| {
                                json!({
                                    "id": t.id,
                                    "title": t.title,
                                    "status": t.status,
                                    "tier": t.tier,
                                    "depends_on": t.depends_on,
                                    "files": t.files.len(),
                                })
                            })
                            .collect();
                        (tf.tasks.len(), details)
                    }
                    Err(_) => (0, vec![]),
                }
            } else {
                (0, vec![])
            }
        };

        total_tasks += task_count;
        if let Some(ref fm) = plan.frontmatter {
            if let Some(mins) = fm.estimated_minutes {
                total_estimated_minutes += mins;
            }
        }

        plan_summaries.push(json!({
            "plan": plan.base,
            "num": plan.num,
            "task_count": task_count,
            "estimated_minutes": plan.frontmatter.as_ref().and_then(|f| f.estimated_minutes),
            "parallel_width": plan.frontmatter.as_ref().and_then(|f| f.estimated_parallel_width),
            "priority": plan.frontmatter.as_ref().and_then(|f| f.priority),
            "tags": plan.frontmatter.as_ref().map(|f| &f.tags),
            "tasks": task_details,
        }));
    }

    if cli.json {
        let payload = json!({
            "dry_run": true,
            "plans_dir": plans_dir,
            "total_plans": plans.len(),
            "total_tasks": total_tasks,
            "total_estimated_minutes": total_estimated_minutes,
            "plans": plan_summaries,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!(
            "Dry run: {} plan(s), {} task(s) in {}\n",
            plans.len(),
            total_tasks,
            plans_dir.display()
        );

        for (i, plan) in plans.iter().enumerate() {
            let est = plan
                .frontmatter
                .as_ref()
                .and_then(|f| f.estimated_minutes)
                .map(|m| format!(" (~{m} min)"))
                .unwrap_or_default();
            let priority = plan
                .frontmatter
                .as_ref()
                .and_then(|f| f.priority)
                .map(|p| format!(" [priority={p}]"))
                .unwrap_or_default();
            println!("  {}. {}{}{}", i + 1, plan.base, est, priority);

            // Print task list if available.
            if let Some(tasks) = plan_summaries[i].get("tasks").and_then(|v| v.as_array()) {
                for t in tasks {
                    let tid = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let status = t
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    let tier = t.get("tier").and_then(|v| v.as_str()).unwrap_or("?");
                    let deps = t
                        .get("depends_on")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            let ids: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                            if ids.is_empty() {
                                String::new()
                            } else {
                                format!(" (after {})", ids.join(", "))
                            }
                        })
                        .unwrap_or_default();
                    println!("     {tid}: {title} [{tier}, {status}]{deps}");
                }
            }
        }

        if total_estimated_minutes > 0 {
            println!("\nEstimated total: ~{total_estimated_minutes} min");
        }
        println!("\nNo tasks were executed. Remove --dry-run to run the plan.");
    }

    Ok(EXIT_SUCCESS)
}

fn cmd_plan_validate(dir: &Path, strict: bool, json_output: bool) -> Result<i32> {
    let current_dir =
        std::env::current_dir().context("resolve current directory for plan validation")?;
    let config_path = current_dir.join("roko.toml");
    let models = if config_path.is_file() {
        let config_text = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        let config: RokoConfig = toml::from_str(&config_text)
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("parse {}", config_path.display()))?;
        Some(configured_models(&config))
    } else {
        None
    };

    let report = plan_validate::validate_plans_dir(dir, models.as_ref())?;
    if json_output {
        println!("{}", plan_validate::render_json(&report)?);
    } else {
        println!("{}", plan_validate::render_text(&report));
    }
    Ok(report.exit_code(strict))
}

// -----------------------------------------------------------------------
// Existing subcommand handlers (init, run, status, replay)
// -----------------------------------------------------------------------

fn with_research_provider_model(
    config: &RokoConfig,
    provider_key: &str,
    provider_config: ProviderConfig,
    model_profile: ModelProfile,
) -> RokoConfig {
    let mut routing_config = config.clone();
    routing_config
        .providers
        .entry(provider_key.to_string())
        .or_insert(provider_config);
    routing_config
        .models
        .entry(model_profile.slug.clone())
        .or_insert(model_profile);
    routing_config
}

fn with_perplexity_research_model(
    config: &RokoConfig,
    model_slug: &str,
    supports_async: bool,
) -> (RokoConfig, u64) {
    let configured_profile = config.models.get(model_slug).cloned();
    let provider_key = configured_profile
        .as_ref()
        .map(|profile| profile.provider.clone())
        .unwrap_or_else(|| "perplexity".to_string());
    let configured_provider = config
        .providers
        .get(&provider_key)
        .cloned()
        .or_else(|| config.providers.get("perplexity").cloned());
    let timeout_ms = configured_provider
        .as_ref()
        .and_then(|provider| provider.timeout_ms)
        .unwrap_or(300_000);

    let mut model_profile = configured_profile.unwrap_or_else(|| ModelProfile {
        provider: provider_key.clone(),
        slug: model_slug.to_string(),
        context_window: 127_072,
        max_output: Some(8_192),
        supports_tools: false,
        supports_thinking: false,
        supports_vision: false,
        supports_web_search: true,
        supports_mcp_tools: false,
        supports_partial: false,
        supports_grounding: false,
        supports_code_execution: false,
        supports_caching: false,
        provider_routing: None,
        tool_format: "openai_json".to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_input_per_m_high: None,
        cost_output_per_m_high: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        thinking_level: None,
        max_tools: None,
        tokenizer_ratio: None,
        supports_search: true,
        supports_citations: true,
        supports_async,
        is_embedding_model: false,
        search_context_size: None,
        cost_per_request: None,
    });
    model_profile.supports_search = true;
    model_profile.supports_citations = true;
    model_profile.supports_async |= supports_async;

    let routing_config = with_research_provider_model(
        config,
        &provider_key,
        configured_provider.unwrap_or(ProviderConfig {
            kind: ProviderKind::PerplexityApi,
            base_url: Some("https://api.perplexity.ai".to_string()),
            api_key_env: Some("PERPLEXITY_API_KEY".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(timeout_ms),
            ttft_timeout_ms: Some(15_000),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        }),
        model_profile,
    );

    (routing_config, timeout_ms)
}

async fn cmd_research(cli: &Cli, cmd: ResearchCmd) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env, model_from_config};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};
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
    let agent_command = command_from_config(&workdir).unwrap_or_else(|| "claude".to_string());
    let config = load_roko_config(&workdir).unwrap_or_default();

    match cmd {
        ResearchCmd::Topic { topic, deep } => {
            let topic = topic.join(" ");
            println!("🔬 Researching: {topic}");

            // --deep: use PerplexityDeepResearchAgent (sonar-deep-research, async polling)
            if deep {
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

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

                let (routing_config, timeout_ms) =
                    with_perplexity_research_model(&config, &model_slug, true);
                let agent = spawn_agent_scoped(
                    &routing_config,
                    SpawnAgentSpec {
                        model: model_slug.clone(),
                        command: None,
                        timeout_ms: Some(timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(workdir.clone()),
                        env: Vec::new(),
                        extra_args: Vec::new(),
                        effort: None,
                        bare_mode: false,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                        role: Some("researcher".to_string()),
                    },
                    format!("create Perplexity deep research agent for model {model_slug}"),
                )?;
                println!("⏳ Deep research submitted ({model_slug}). This takes 1-10 min...");

                let input = roko_core::Engram::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();

                let started = Instant::now();
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
                    let output = result.output.body.as_text().unwrap_or_default().to_string();
                    let _ = persist_capture_episode(
                        &workdir,
                        "perplexity",
                        Some(&model_slug),
                        "research-topic-deep",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &output,
                        false,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
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
                let _ = persist_capture_episode(
                    &workdir,
                    "perplexity",
                    Some(&model_slug),
                    "research-topic-deep",
                    &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                    &combined_prompt,
                    &output,
                    true,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                return Ok(0);
            }

            // If Perplexity is configured, use PerplexityChatAgent for search-grounded research.
            if let Some(model_slug) = config.gemini.grounding_model.clone() {
                use roko_agent::gemini::GeminiMetadata;
                use roko_core::Body;

                let (combined_prompt, enable_grounding) = build_research_prompt_gemini(
                    &workdir,
                    &topic,
                    ResearchMode::Topic,
                    &config.gemini,
                );
                if enable_grounding {
                    let configured_profile = config.models.get(&model_slug).cloned();
                    let provider_key = configured_profile
                        .as_ref()
                        .map(|profile| profile.provider.clone())
                        .unwrap_or_else(|| "gemini".to_string());
                    let configured_provider = config
                        .providers
                        .get(&provider_key)
                        .cloned()
                        .or_else(|| config.providers.get("gemini").cloned());
                    let base_url = configured_provider
                        .as_ref()
                        .and_then(|provider| provider.base_url.clone())
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
                    let timeout_ms = configured_provider
                        .as_ref()
                        .and_then(|provider| provider.timeout_ms)
                        .unwrap_or(300_000);

                    let mut model_profile = configured_profile.unwrap_or_else(|| ModelProfile {
                        provider: provider_key.clone(),
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

                    let routing_config = with_research_provider_model(
                        &config,
                        &provider_key,
                        configured_provider.unwrap_or(ProviderConfig {
                            kind: ProviderKind::GeminiApi,
                            base_url: Some(base_url),
                            api_key_env: Some("GEMINI_API_KEY".to_string()),
                            command: None,
                            args: None,
                            timeout_ms: Some(timeout_ms),
                            ttft_timeout_ms: Some(15_000),
                            connect_timeout_ms: Some(5_000),
                            extra_headers: None,
                            max_concurrent: None,
                        }),
                        model_profile,
                    );
                    let agent = spawn_agent_scoped(
                        &routing_config,
                        SpawnAgentSpec {
                            model: model_slug.clone(),
                            command: None,
                            timeout_ms: Some(timeout_ms),
                            system_prompt: None,
                            cached_content: None,
                            tools: None,
                            mcp_config: None,
                            effort: Some(config.gemini.thinking_level.clone()),
                            name: format!("gemini:{model_slug}"),
                            working_dir: Some(workdir.clone()),
                            env: Vec::new(),
                            extra_args: Vec::new(),
                            bare_mode: false,
                            dangerously_skip_permissions: false,
                            role: Some("researcher".to_string()),
                        },
                        format!("create Gemini research agent for model {model_slug}"),
                    )?;

                    let input = roko_core::Engram::builder(Kind::Prompt)
                        .body(Body::text(&combined_prompt))
                        .build();
                    let started = Instant::now();
                    let result = agent.run(&input, &Context::now()).await;

                    if !result.success {
                        let err_text = result.output.body.as_text().unwrap_or("unknown error");
                        let output = result.output.body.as_text().unwrap_or_default().to_string();
                        let _ = persist_capture_episode(
                            &workdir,
                            "gemini",
                            Some(&model_slug),
                            "research-topic-gemini",
                            &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                            &combined_prompt,
                            &output,
                            false,
                            started.elapsed().as_millis() as u64,
                            resume_session,
                        )
                        .await;
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
                    let _ = persist_capture_episode(
                        &workdir,
                        "gemini",
                        Some(&model_slug),
                        "research-topic-gemini",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &content,
                        true,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
                    return Ok(0);
                }
            }

            if let Some(model_slug) = config.perplexity.default_search_model.clone() {
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

                let (combined_prompt, search_opts) = build_research_prompt_perplexity(
                    &workdir,
                    &topic,
                    "",
                    ResearchMode::Topic,
                    &config.perplexity,
                );
                let (routing_config, timeout_ms) =
                    with_perplexity_research_model(&config, &model_slug, false);
                let agent = spawn_agent_scoped(
                    &routing_config,
                    SpawnAgentSpec {
                        model: model_slug.clone(),
                        command: None,
                        timeout_ms: Some(timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(workdir.clone()),
                        env: Vec::new(),
                        extra_args: vec![format!(
                            "{}{}",
                            roko_agent::provider::PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX,
                            serde_json::to_string(&search_opts)
                                .expect("Perplexity search options must serialize"),
                        )],
                        effort: None,
                        bare_mode: false,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                        role: Some("researcher".to_string()),
                    },
                    format!("create Perplexity research agent for model {model_slug}"),
                )?;

                let input = roko_core::Engram::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();
                let started = Instant::now();
                let result = agent.run(&input, &Context::now()).await;

                if !result.success {
                    let err_text = result.output.body.as_text().unwrap_or("unknown error");
                    let output = result.output.body.as_text().unwrap_or_default().to_string();
                    let _ = persist_capture_episode(
                        &workdir,
                        "perplexity",
                        Some(&model_slug),
                        "research-topic-perplexity",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &output,
                        false,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
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
                let _ = persist_capture_episode(
                    &workdir,
                    "perplexity",
                    Some(&model_slug),
                    "research-topic-perplexity",
                    &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                    &combined_prompt,
                    &output,
                    true,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
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
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-topic-claude",
                &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
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
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-prd",
                &format!("research:enhance-prd:{slug}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::EnhancePlan { plan } => {
            let plan_dir = roko_cli::plan::plans_dir(&workdir).join(&plan);
            if !plan_dir.is_dir() {
                anyhow::bail!("Plan directory not found: {}", plan_dir.display());
            }
            println!("🔬 Enhancing plan: {plan}");
            let task_prompt = format!(
                "Read the plan at .roko/plans/{plan}/plan.md and .roko/plans/{plan}/tasks.toml. \
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
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-plan",
                &format!("research:enhance-plan:{plan}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::EnhanceTasks { plan } => {
            let tasks_path = roko_cli::plan::plans_dir(&workdir)
                .join(&plan)
                .join("tasks.toml");
            if !tasks_path.exists() {
                anyhow::bail!("tasks.toml not found: {}", tasks_path.display());
            }
            println!("🔬 Optimizing tasks: {plan}");
            let content = std::fs::read_to_string(&tasks_path)?;
            let task_prompt = format!(
                "Read .roko/plans/{plan}/tasks.toml and optimize every task: \
                 (1) Split any task >50 LOC into smaller subtasks. \
                 (2) Add context.read_files with exact line ranges for each task. \
                 (3) Ensure every acceptance criterion is a runnable shell command. \
                 (4) Remove unnecessary dependency edges to increase parallelism. \
                 (5) Assign tier (mechanical/focused/integrative/architectural) and model_hint. \
                 Search the codebase to verify file paths exist. Update tasks.toml in place."
            );
            let system =
                build_research_prompt(&workdir, &plan, &content, ResearchMode::EnhanceTasks);
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-tasks",
                &format!("research:enhance-tasks:{plan}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
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
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-analyze",
                "research:analyze:execution",
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
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

async fn cmd_job(cli: &Cli, cmd: JobCmd) -> Result<i32> {
    let jobs_dir = |wd: &Path| wd.join(".roko").join("jobs");

    match cmd {
        JobCmd::List { workdir, status } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let dir = jobs_dir(&wd);
            if !dir.is_dir() {
                println!(
                    "No jobs found (directory does not exist: {})",
                    dir.display()
                );
                return Ok(EXIT_SUCCESS);
            }
            let mut entries: Vec<_> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext == "json")
                })
                .collect();
            entries.sort_by_key(|e| e.file_name());

            let mut count = 0usize;
            for entry in &entries {
                let data = std::fs::read_to_string(entry.path())?;
                let job: roko_core::MarketplaceJob =
                    serde_json::from_str(&data).unwrap_or_default();
                let effective_status = if !job.status.is_empty() {
                    &job.status
                } else if !job.state.is_empty() {
                    &job.state
                } else {
                    "unknown"
                };
                if let Some(ref filter) = status {
                    if !effective_status.eq_ignore_ascii_case(filter) {
                        continue;
                    }
                }
                let icon = match effective_status {
                    "open" | "pending" => "\u{25cb}",
                    "assigned" => "\u{25d4}",
                    "in_progress" | "active" | "running" => "\u{25b6}",
                    "submitted" => "\u{25d1}",
                    "completed" | "done" => "\u{2713}",
                    "failed" | "cancelled" => "\u{2717}",
                    _ => "\u{00b7}",
                };
                println!(
                    "{icon} [{:>12}] {:>10}  {}  {}",
                    job.job_type,
                    effective_status,
                    &job.id[..job.id.len().min(8)],
                    job.title
                );
                count += 1;
            }
            if count == 0 {
                println!("No jobs found.");
            } else {
                println!("\n{count} job(s)");
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Create {
            title,
            r#type,
            description,
            priority,
            auto_execute,
            plan_id,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let dir = jobs_dir(&wd);
            std::fs::create_dir_all(&dir)?;
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let job = roko_core::MarketplaceJob {
                id: id.clone(),
                title: title.trim().to_string(),
                description: description.trim().to_string(),
                job_type: r#type.trim().to_string(),
                status: "open".to_string(),
                priority: priority.trim().to_string(),
                auto_execute,
                plan_id: plan_id.unwrap_or_default(),
                created_at: now.clone(),
                updated_at: now,
                ..Default::default()
            };
            let path = dir.join(format!("{id}.json"));
            let rendered = serde_json::to_string_pretty(&job)?;
            std::fs::write(&path, &rendered)?;
            println!("Created job: {id}");
            println!("  title:    {}", job.title);
            println!("  type:     {}", job.job_type);
            println!("  priority: {}", job.priority);
            println!("  auto_execute: {}", job.auto_execute);
            println!("  path:     {}", path.display());
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Show { id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let path = jobs_dir(&wd).join(format!("{id}.json"));
            if !path.exists() {
                bail!("job '{id}' not found at {}", path.display());
            }
            let data = std::fs::read_to_string(&path)?;
            let job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;
            let effective_status = if !job.status.is_empty() {
                &job.status
            } else if !job.state.is_empty() {
                &job.state
            } else {
                "unknown"
            };
            println!("id:           {}", job.id);
            println!("title:        {}", job.title);
            println!("type:         {}", job.job_type);
            println!("status:       {effective_status}");
            println!("priority:     {}", job.priority);
            println!("posted_by:    {}", job.posted_by);
            println!("assigned_to:  {}", job.assigned_to);
            println!("auto_execute: {}", job.auto_execute);
            println!("plan_id:      {}", job.plan_id);
            println!("created_at:   {}", job.created_at);
            println!("updated_at:   {}", job.updated_at);
            if !job.tags.is_empty() {
                println!("tags:         {}", job.tags.join(", "));
            }
            if !job.description.is_empty() {
                println!("\n--- description ---\n{}", job.description);
            }
            if let Some(ref sub) = job.submission {
                println!(
                    "\n--- submission ---\n{}",
                    serde_json::to_string_pretty(sub).unwrap_or_default()
                );
            }
            if let Some(ref eval) = job.evaluation {
                println!(
                    "\n--- evaluation ---\n{}",
                    serde_json::to_string_pretty(eval).unwrap_or_default()
                );
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Execute {
            id,
            serve_url,
            workdir,
        } => {
            if let Some(url) = serve_url {
                // Delegate to roko-serve
                let default_wd = resolve_workdir(cli);
                let wd = workdir.as_deref().unwrap_or(&default_wd);
                let auth_cfg = load_layered(wd)
                    .map(|r| r.config.serve.auth)
                    .unwrap_or_default();
                let headers = match auth::resolve_api_key(&auth_cfg, None) {
                    Some(resolved) => auth::auth_headers(&resolved.key),
                    None => reqwest::header::HeaderMap::new(),
                };
                let client = reqwest::Client::new();
                let resp = client
                    .post(format!("{url}/api/jobs/{id}/execute"))
                    .headers(headers)
                    .send()
                    .await?;
                let status = resp.status();
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                if status.is_success() {
                    println!("Job '{id}' execution started via serve.");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                } else {
                    eprintln!(
                        "Failed to execute job '{id}': {} {}",
                        status,
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                    return Ok(EXIT_FAILURE);
                }
            } else {
                // Local inline execution — load config and use run_once
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                let path = jobs_dir(&wd).join(format!("{id}.json"));
                if !path.exists() {
                    bail!("job '{id}' not found at {}", path.display());
                }
                let data = std::fs::read_to_string(&path)?;
                let mut job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;
                println!("Executing job '{id}' locally...");

                // Transition to in_progress
                job.status = "in_progress".to_string();
                job.updated_at = chrono::Utc::now().to_rfc3339();
                std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;

                // Build prompt based on job type
                let prompt = match job.job_type.as_str() {
                    "research" => format!(
                        "Research the following topic and produce a detailed report with citations:\n\n{}",
                        job.description
                    ),
                    "coding_task" | "coding" => {
                        if !job.plan_id.is_empty() {
                            format!("Execute plan '{}' in the current workspace", job.plan_id)
                        } else {
                            job.description.clone()
                        }
                    }
                    _ => job.description.clone(),
                };

                let config = resolve_config_for_workdir(cli, &wd)?;
                let result = run_once(&wd, &config, &prompt).await;
                match result {
                    Ok(report) => {
                        job.status = "completed".to_string();
                        job.submission = Some(serde_json::json!({
                            "result_summary": if report.overall_success() { "success" } else { "completed with failures" },
                            "completed_at": chrono::Utc::now().to_rfc3339(),
                        }));
                        job.updated_at = chrono::Utc::now().to_rfc3339();
                        std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
                        println!("Job '{id}' completed successfully.");
                    }
                    Err(e) => {
                        job.status = "failed".to_string();
                        job.updated_at = chrono::Utc::now().to_rfc3339();
                        std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
                        eprintln!("Job '{id}' failed: {e}");
                        return Ok(EXIT_AGENT_FAILURE);
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Cancel { id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let path = jobs_dir(&wd).join(format!("{id}.json"));
            if !path.exists() {
                bail!("job '{id}' not found at {}", path.display());
            }
            let data = std::fs::read_to_string(&path)?;
            let mut job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;
            let effective_status = if !job.status.is_empty() {
                &job.status
            } else {
                "unknown"
            };
            if matches!(effective_status, "completed" | "failed" | "cancelled") {
                bail!("cannot cancel job '{id}': status '{effective_status}' is terminal");
            }
            job.status = "cancelled".to_string();
            job.updated_at = chrono::Utc::now().to_rfc3339();
            std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
            println!("Job '{id}' cancelled.");
            Ok(EXIT_SUCCESS)
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
            println!("  anti-knowledge: {}", stats.anti_knowledge_count);
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
                    println!("    {kind:<20} {count}");
                }
            }
            println!("  entries by tier:");
            if stats.tier_counts.is_empty() {
                println!("    (empty)");
            } else {
                for (tier, count) in &stats.tier_counts {
                    println!("    {tier:<20} {count}");
                }
            }
            if !stats.source_counts.is_empty() {
                println!("  entries by source:");
                for (source, count) in &stats.source_counts {
                    println!("    {source:<20} {count}");
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
        NeuroCmd::Backup {
            workdir,
            destination,
            force,
            top_n,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let report = backup_neuro_store(&wd, &destination, force, top_n)?;

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "backup_dir": destination,
                    "knowledge_store": report.live.knowledge,
                    "knowledge_backup": report.snapshot.knowledge,
                    "confirmations_store": report.live.confirmations,
                    "confirmations_backup": report.snapshot.confirmations,
                    "confirmations_present": report.confirmations_present,
                    "top_n": top_n,
                    "entries_exported": report.entries_exported,
                    "force": force,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Neuro backup written to {}:", destination.display());
            println!("  knowledge: {}", report.snapshot.knowledge.display());
            if let Some(n) = top_n {
                println!("  genomic bottleneck: top {n} entries by confidence");
            }
            println!("  entries exported: {}", report.entries_exported);
            if report.confirmations_present {
                println!(
                    "  confirmations: {}",
                    report.snapshot.confirmations.display()
                );
            } else {
                println!("  confirmations: (none)");
            }

            // Write manifest.json alongside the backup files.
            let manifest = serde_json::json!({
                "version": 1,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "entry_count": report.entries_exported,
                "top_n": top_n,
                "source_path": report.live.knowledge,
            });
            let manifest_path = destination.join("manifest.json");
            std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
                .with_context(|| format!("write manifest to {}", manifest_path.display()))?;
            println!("  manifest: {}", manifest_path.display());

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Restore {
            workdir,
            source,
            force,
            types,
            min_confidence,
            generation,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));

            // Parse type filters if provided.
            let type_filters: Option<Vec<String>> = types.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            let report = restore_neuro_store(
                &wd,
                &source,
                force,
                generation,
                min_confidence,
                type_filters.as_deref(),
            )?;

            let confidence_decay = 0.85_f64.powi(generation as i32);

            if cli.json {
                let payload = serde_json::json!({
                    "workdir": wd,
                    "backup_dir": source,
                    "knowledge_store": report.live.knowledge,
                    "knowledge_backup": report.snapshot.knowledge,
                    "confirmations_store": report.live.confirmations,
                    "confirmations_backup": report.snapshot.confirmations,
                    "confirmations_present": report.confirmations_present,
                    "generation": generation,
                    "confidence_decay": confidence_decay,
                    "entries_restored": report.entries_restored,
                    "entries_filtered": report.entries_filtered,
                    "force": force,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Neuro backup restored from {}:", source.display());
            println!("  knowledge: {}", report.live.knowledge.display());
            println!("  generation: {generation} (confidence decay: {confidence_decay:.4})");
            println!("  entries restored: {}", report.entries_restored);
            println!("  entries filtered: {}", report.entries_filtered);
            println!("  tier: all restored entries set to Transient (quarantine)");
            if report.confirmations_present {
                println!("  confirmations: {}", report.live.confirmations.display());
            } else {
                println!("  confirmations: (none)");
            }

            Ok(EXIT_SUCCESS)
        }
        NeuroCmd::Sync {
            peer,
            workdir,
            direction,
            max_send,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let store = KnowledgeStore::for_workdir(&wd);

            // Load the version vector from persistent state (or create empty).
            let vv_path = wd.join(".roko").join("neuro").join("version-vectors.json");
            let mut version_vectors: HashMap<String, u64> = if vv_path.exists() {
                let text = std::fs::read_to_string(&vv_path)
                    .with_context(|| format!("read version vectors from {}", vv_path.display()))?;
                serde_json::from_str(&text).unwrap_or_default()
            } else {
                HashMap::new()
            };

            let peer_seq = version_vectors.get(&peer).copied().unwrap_or(0);
            let entries = store
                .read_all()
                .with_context(|| format!("read knowledge store from {}", store.path().display()))?;

            let should_send = direction == "send" || direction == "both";
            let should_receive = direction == "receive" || direction == "both";

            let mut sent_count = 0_usize;
            let mut received_count = 0_usize;

            if should_send {
                // Build delta: entries newer than peer's last-seen sequence.
                // Use entry index as a proxy sequence number for local ordering.
                let delta: Vec<_> = entries
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| (*idx as u64) > peer_seq)
                    .take(max_send)
                    .collect();
                sent_count = delta.len();

                // Write delta to an outbox file for the peer.
                if !delta.is_empty() {
                    let outbox_dir = wd.join(".roko").join("mesh").join("outbox");
                    std::fs::create_dir_all(&outbox_dir)?;
                    let delta_path = outbox_dir.join(format!("delta-{peer}.jsonl"));
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(&delta_path)?;
                    for (_, entry) in &delta {
                        let line = serde_json::to_string(entry)?;
                        use std::io::Write;
                        writeln!(f, "{line}")?;
                    }
                    println!("  outbox: {}", delta_path.display());
                }
            }

            if should_receive {
                // Check inbox for incoming deltas from the peer.
                let inbox_dir = wd.join(".roko").join("mesh").join("inbox");
                let inbox_path = inbox_dir.join(format!("delta-{peer}.jsonl"));
                if inbox_path.exists() {
                    let text = std::fs::read_to_string(&inbox_path)?;
                    let mut imported = Vec::new();
                    for line in text.lines() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if let Ok(mut entry) =
                            serde_json::from_str::<roko_neuro::KnowledgeEntry>(line)
                        {
                            // Apply received confidence discount (0.7x).
                            entry.confidence *= 0.7;
                            entry.tier = roko_neuro::KnowledgeTier::Transient;
                            entry.source = Some(format!("mesh:{peer}"));
                            imported.push(entry);
                        }
                    }
                    received_count = imported.len();
                    if !imported.is_empty() {
                        store.ingest(imported).with_context(|| {
                            format!("import mesh entries from {}", inbox_path.display())
                        })?;
                    }
                    // Clean up processed inbox file.
                    let _ = std::fs::remove_file(&inbox_path);
                }
            }

            // Update version vector for this peer.
            let new_seq = entries.len() as u64;
            version_vectors.insert(peer.clone(), new_seq);
            if let Some(parent) = vv_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&vv_path, serde_json::to_string_pretty(&version_vectors)?)?;

            if cli.json {
                let payload = serde_json::json!({
                    "peer": peer,
                    "direction": direction,
                    "sent": sent_count,
                    "received": received_count,
                    "local_seq": new_seq,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(EXIT_SUCCESS);
            }

            println!("Mesh sync with peer '{peer}':");
            println!("  direction: {direction}");
            println!("  sent: {sent_count} engrams");
            println!("  received: {received_count} engrams (0.7x confidence discount)");
            println!("  local sequence: {new_seq}");

            Ok(EXIT_SUCCESS)
        }
    }
}

const NEURO_KNOWLEDGE_FILE: &str = "knowledge.jsonl";
const NEURO_CONFIRMATIONS_FILE: &str = "knowledge-confirmations.jsonl";

#[derive(Debug, Clone, PartialEq, Eq)]
struct NeuroFileSet {
    knowledge: PathBuf,
    confirmations: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NeuroTransferReport {
    live: NeuroFileSet,
    snapshot: NeuroFileSet,
    confirmations_present: bool,
    /// Number of entries exported (only relevant for backup with --top-n).
    entries_exported: usize,
    /// Number of entries restored (only relevant for restore).
    entries_restored: usize,
    /// Number of entries filtered out during restore.
    entries_filtered: usize,
}

fn backup_neuro_store(
    workdir: &Path,
    destination: &Path,
    force: bool,
    top_n: Option<usize>,
) -> Result<NeuroTransferReport> {
    let live = neuro_live_files(workdir);
    let snapshot = neuro_snapshot_files(destination);

    if let Some(n) = top_n {
        // Genomic bottleneck: export only the top N entries by confidence.
        let store = KnowledgeStore::for_workdir(workdir);
        let mut entries = store
            .read_all()
            .with_context(|| format!("read knowledge store from {}", store.path().display()))?;
        // Sort by confidence descending.
        entries.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(n);

        // Write entries to the snapshot location using export.
        let filter = roko_neuro::knowledge_store::ExportFilter::default();
        ensure_neuro_directory(
            snapshot
                .knowledge
                .parent()
                .ok_or_else(|| anyhow!("resolve backup directory"))?,
            "backup",
        )?;

        // Re-add just the top N entries through a temporary store.
        let temp_store = KnowledgeStore::new(snapshot.knowledge.clone());
        let _count = entries.len();
        if !entries.is_empty() {
            temp_store.ingest(entries)?;
        }

        // Also export using the JSONL export format for maximum compatibility.
        let exported_count = temp_store.export(&snapshot.knowledge, &filter)?;

        let confirmations_present = sync_optional_neuro_file(
            &live.confirmations,
            &snapshot.confirmations,
            force,
            "backup",
        )?;

        return Ok(NeuroTransferReport {
            live,
            snapshot,
            confirmations_present,
            entries_exported: exported_count,
            entries_restored: 0,
            entries_filtered: 0,
        });
    }

    let confirmations_present = sync_neuro_store_files(&live, &snapshot, force, "backup")?;

    // Count entries in the exported file.
    let entries_exported = if snapshot.knowledge.exists() {
        let store = KnowledgeStore::new(snapshot.knowledge.clone());
        store.read_all().map(|e| e.len()).unwrap_or(0)
    } else {
        0
    };

    Ok(NeuroTransferReport {
        live,
        snapshot,
        confirmations_present,
        entries_exported,
        entries_restored: 0,
        entries_filtered: 0,
    })
}

fn restore_neuro_store(
    workdir: &Path,
    source: &Path,
    force: bool,
    generation: u32,
    min_confidence: Option<f64>,
    type_filters: Option<&[String]>,
) -> Result<NeuroTransferReport> {
    let live = neuro_live_files(workdir);
    let snapshot = neuro_snapshot_files(source);

    // Apply confidence decay and filtering during restore.
    let confidence_multiplier = 0.85_f64.powi(generation as i32);

    // Read the source backup entries.
    let source_store = KnowledgeStore::new(snapshot.knowledge.clone());
    let source_entries = source_store
        .read_all()
        .with_context(|| format!("read backup entries from {}", snapshot.knowledge.display()))?;

    let total_source = source_entries.len();

    // Apply filters: type filter and min confidence.
    let filtered: Vec<_> = source_entries
        .into_iter()
        .filter(|entry| {
            if let Some(types) = type_filters {
                let kind_str = format!("{:?}", entry.kind).to_lowercase();
                types.iter().any(|t| kind_str.contains(&t.to_lowercase()))
            } else {
                true
            }
        })
        .filter(|entry| {
            if let Some(min) = min_confidence {
                entry.confidence >= min
            } else {
                true
            }
        })
        .map(|mut entry| {
            // Apply 0.85^N confidence decay.
            entry.confidence = (entry.confidence * confidence_multiplier).clamp(0.0, 1.0);
            // Reset to Transient tier (quarantine).
            entry.tier = roko_neuro::KnowledgeTier::Transient;
            // Mark source as restore with generation info.
            entry.source = Some(format!("restore:gen{generation}"));
            entry
        })
        .collect();

    let entries_restored = filtered.len();
    let entries_filtered = total_source.saturating_sub(entries_restored);

    // Write filtered entries to the live store.
    let dest_store = KnowledgeStore::for_workdir(workdir);
    if let Some(parent) = dest_store.path().parent() {
        std::fs::create_dir_all(parent)?;
    }

    // If force is not set and the live store exists, check before overwriting.
    if dest_store.path().exists() && !force {
        let existing = dest_store.read_all().unwrap_or_default();
        if !existing.is_empty() {
            bail!(
                "restore would modify existing knowledge store at {}. Re-run with --force to proceed.",
                dest_store.path().display()
            );
        }
    }

    if !filtered.is_empty() {
        dest_store.ingest(filtered)?;
    }

    // Copy confirmations if present.
    let confirmations_present = if snapshot.confirmations.exists() {
        sync_optional_neuro_file(
            &snapshot.confirmations,
            &live.confirmations,
            force,
            "restore",
        )?
    } else {
        false
    };

    Ok(NeuroTransferReport {
        live,
        snapshot,
        confirmations_present,
        entries_exported: 0,
        entries_restored,
        entries_filtered,
    })
}

fn neuro_live_files(workdir: &Path) -> NeuroFileSet {
    let store = KnowledgeStore::for_workdir(workdir);
    NeuroFileSet {
        knowledge: store.path().to_path_buf(),
        confirmations: store.confirmations_path().to_path_buf(),
    }
}

fn neuro_snapshot_files(root: &Path) -> NeuroFileSet {
    NeuroFileSet {
        knowledge: root.join(NEURO_KNOWLEDGE_FILE),
        confirmations: root.join(NEURO_CONFIRMATIONS_FILE),
    }
}

fn sync_neuro_store_files(
    source: &NeuroFileSet,
    destination: &NeuroFileSet,
    force: bool,
    operation: &str,
) -> Result<bool> {
    let destination_root = destination
        .knowledge
        .parent()
        .ok_or_else(|| anyhow!("resolve {operation} destination directory"))?;
    ensure_neuro_directory(destination_root, operation)?;

    copy_neuro_file(&source.knowledge, &destination.knowledge, force, operation)?;
    sync_optional_neuro_file(
        &source.confirmations,
        &destination.confirmations,
        force,
        operation,
    )
}

fn ensure_neuro_directory(path: &Path, operation: &str) -> Result<()> {
    if path.exists() && !path.is_dir() {
        bail!(
            "{operation} target must be a directory, found file at {}",
            path.display()
        );
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("create {operation} directory {}", path.display()))?;
    Ok(())
}

fn copy_neuro_file(source: &Path, destination: &Path, force: bool, operation: &str) -> Result<()> {
    if !source.exists() {
        bail!("{operation} source file not found: {}", source.display());
    }
    if destination.exists() && !force {
        bail!(
            "{operation} would overwrite {}. Re-run with --force to replace it.",
            destination.display()
        );
    }
    std::fs::copy(source, destination).with_context(|| {
        format!(
            "{operation} {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn sync_optional_neuro_file(
    source: &Path,
    destination: &Path,
    force: bool,
    operation: &str,
) -> Result<bool> {
    if source.exists() {
        copy_neuro_file(source, destination, force, operation)?;
        return Ok(true);
    }

    if destination.exists() {
        if !force {
            bail!(
                "{operation} would leave stale optional file at {}. Re-run with --force to replace it.",
                destination.display()
            );
        }
        std::fs::remove_file(destination).with_context(|| {
            format!(
                "{operation} remove stale optional file {}",
                destination.display()
            )
        })?;
    }

    Ok(false)
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
    if let Some(path) = roko_cli::workspace_paths::find_prd_path(workdir, slug) {
        return Ok(path);
    }
    anyhow::bail!("PRD not found: {slug} (checked published/ and drafts/)");
}

fn resolved_capture_model(agent_command: &str, model: Option<&str>) -> String {
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        return model.to_string();
    }
    if agent_command.eq_ignore_ascii_case("claude") {
        "claude-opus-4-6".to_string()
    } else {
        "unknown-model".to_string()
    }
}

fn capture_provider(agent_command: &str, resolved_model: &str) -> String {
    let command = agent_command.trim();
    let model = resolved_model.to_ascii_lowercase();
    if command.eq_ignore_ascii_case("claude") || model.starts_with("claude") {
        "anthropic".to_string()
    } else if command.eq_ignore_ascii_case("codex")
        || command.eq_ignore_ascii_case("openai")
        || model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        "openai".to_string()
    } else if command.eq_ignore_ascii_case("ollama") || model.starts_with("ollama/") {
        "ollama".to_string()
    } else {
        command.to_string()
    }
}

fn capture_role(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "Researcher"
    } else {
        "Strategist"
    }
}

fn capture_task_category(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "research"
    } else if task_kind.starts_with("prd-plan") {
        "scaffolding"
    } else {
        "docs"
    }
}

fn capture_complexity_band(task_kind: &str) -> &'static str {
    if task_kind == "research-analyze" {
        "standard"
    } else if task_kind.starts_with("research-") {
        "deep"
    } else {
        "standard"
    }
}

fn capture_plan_id(task_id: &str) -> Option<&str> {
    task_id
        .rsplit(':')
        .next()
        .filter(|segment| !segment.is_empty())
}

fn build_capture_episode(
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> (Episode, String) {
    let resolved_model = resolved_capture_model(agent_command, model);
    let provider = capture_provider(agent_command, &resolved_model);
    let role = capture_role(task_kind);
    let task_category = capture_task_category(task_kind);
    let complexity_band = capture_complexity_band(task_kind);
    let mut episode = Episode::new(agent_command.to_string(), task_id.to_string());
    episode.kind = "agent_turn".to_string();
    episode.trigger_kind = task_kind.to_string();
    episode.agent_template = role.to_string();
    episode.episode_id = episode.id.clone();
    episode.model = resolved_model.clone();
    episode.input_signal_hash = ContentHash::of(prompt.as_bytes()).to_hex();
    episode.output_signal_hash = ContentHash::of(output.as_bytes()).to_hex();
    episode.duration_secs = wall_time_ms as f64 / 1000.0;
    episode.usage.wall_ms = wall_time_ms;
    episode.success = success;
    episode.turns = 1;
    if !success {
        episode.failure_reason = Some("agent returned non-zero exit code".to_string());
    }
    episode
        .extra
        .insert("role".to_string(), serde_json::json!(role));
    episode
        .extra
        .insert("command".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("backend".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("task_kind".to_string(), serde_json::json!(task_kind));
    episode
        .extra
        .insert("task_id".to_string(), serde_json::json!(task_id));
    episode
        .extra
        .insert("model".to_string(), serde_json::json!(resolved_model));
    episode
        .extra
        .insert("provider".to_string(), serde_json::json!(provider.clone()));
    episode.extra.insert(
        "task_category".to_string(),
        serde_json::json!(task_category),
    );
    episode.extra.insert(
        "complexity_band".to_string(),
        serde_json::json!(complexity_band),
    );
    if let Some(plan_id) = capture_plan_id(task_id) {
        episode
            .extra
            .insert("plan_id".to_string(), serde_json::json!(plan_id));
    }
    if let Some(session_id) = resume_session.filter(|value| !value.trim().is_empty()) {
        episode
            .extra
            .insert("session_id".to_string(), serde_json::json!(session_id));
    }
    episode.extra.insert(
        "prompt_chars".to_string(),
        serde_json::json!(prompt.chars().count()),
    );
    episode.extra.insert(
        "output_chars".to_string(),
        serde_json::json!(output.chars().count()),
    );
    episode
        .extra
        .insert("success".to_string(), serde_json::json!(success));
    (episode, provider)
}

async fn persist_capture_episode(
    workdir: &Path,
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> Result<()> {
    let (episode, provider) = build_capture_episode(
        agent_command,
        model,
        task_kind,
        task_id,
        prompt,
        output,
        success,
        wall_time_ms,
        resume_session,
    );

    let mut runtime = LearningRuntime::open_under(workdir.join(".roko").join("memory"))
        .await
        .map_err(|e| anyhow!("open learning runtime: {e}"))?;
    let distillation_workdir = workdir.to_path_buf();
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode);
    });

    let mut completed = CompletedRunInput::from_episode(episode);
    completed.provider = Some(provider);
    runtime
        .record_completed_run(completed)
        .await
        .map_err(|e| anyhow!("record learning feedback: {e}"))?;
    Ok(())
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
    use roko_cli::agent_config::{command_from_config, load_gateway_env, model_from_config};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};

    let workdir = resolve_workdir(cli);
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let agent_command = command_from_config(&workdir).unwrap_or_else(|| "claude".to_string());

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
                let drafts = roko_cli::workspace_paths::drafts_dir(&workdir);
                roko_cli::prd::ensure_dirs(&workdir)?;
                let target = drafts.join(format!("{slug}.md"));
                // If the draft exists and has real content (not just scaffold),
                // point the user to `edit` instead. But if it's only the
                // skeleton left by a failed `new` run, overwrite it.
                if target.exists() {
                    let existing = std::fs::read_to_string(&target).unwrap_or_default();
                    let is_skeleton = existing
                        .lines()
                        .filter(|l| {
                            !l.starts_with("---")
                                && !l.starts_with('#')
                                && !l.starts_with("##")
                                && !l.trim().is_empty()
                        })
                        .count()
                        == 0;
                    if !is_skeleton {
                        eprintln!("Draft already exists with content: {}", target.display());
                        eprintln!("Use: roko prd draft edit {slug}");
                        return Ok(1);
                    }
                    eprintln!("Found empty scaffold from previous run — regenerating.");
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
                         If you have file tools, read the codebase to understand what exists \
                         and write the PRD directly to {path}. \
                         If you do NOT have file tools, output the complete PRD markdown \
                         (with YAML frontmatter) as your response — do not wrap in code fences. \
                         Follow the PRD quality standards in your system prompt exactly.",
                        path = target.display()
                    ),
                );
                let task_prompt = format!(
                    "Generate a complete PRD for: {title}. \
                     If you have file tools available, search the codebase to understand \
                     what exists and write the completed PRD to {path}. \
                     Otherwise, output the complete PRD markdown with YAML frontmatter. \
                     Include specific requirements, machine-verifiable acceptance criteria, \
                     and a design section.",
                    path = target.display()
                );
                // Snapshot file mtime before agent runs so we can detect
                // whether a CLI agent wrote the file directly.
                let mtime_before = std::fs::metadata(&target).and_then(|m| m.modified()).ok();

                let started = Instant::now();
                let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                    role: Some("scribe"),
                })
                .await?;

                // Check if the agent already wrote the file (CLI agents with tools).
                let mtime_after = std::fs::metadata(&target).and_then(|m| m.modified()).ok();
                let file_was_modified = match (mtime_before, mtime_after) {
                    (Some(before), Some(after)) => after > before,
                    _ => false,
                };

                if file_was_modified {
                    // Agent wrote the file directly — verify it has content.
                    let content = std::fs::read_to_string(&target).unwrap_or_default();
                    let has_content = roko_cli::prd::has_substantive_markdown_content(&content);
                    if has_content {
                        println!("📄 Draft written to {}", target.display());
                    } else {
                        eprintln!(
                            "Agent modified file but left it empty at {}",
                            target.display()
                        );
                    }
                } else if exit_code == 0 && !output.trim().is_empty() {
                    // Agent returned content as text — write it to the file.
                    let content =
                        roko_cli::prd::materialize_agent_markdown_output(&output, Some(&scaffold))
                            .unwrap_or_else(|| scaffold.clone());
                    std::fs::write(&target, content)?;
                    println!("📄 Draft written to {}", target.display());
                } else if exit_code != 0 {
                    eprintln!(
                        "Agent failed (exit {exit_code}). Scaffold preserved at {}",
                        target.display()
                    );
                } else {
                    eprintln!(
                        "Agent returned empty output. Scaffold preserved at {}",
                        target.display()
                    );
                }
                let _ = persist_capture_episode(
                    &workdir,
                    &agent_command,
                    model_ref,
                    "prd-draft-new",
                    &format!("prd:draft:new:{slug}"),
                    &task_prompt,
                    &output,
                    exit_code == 0,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                Ok(exit_code)
            }
            PrdDraftCmd::Edit { slug } => {
                let draft = roko_cli::workspace_paths::draft_prd_path(&workdir, &slug);
                if !draft.exists() {
                    eprintln!("Draft not found: {}", draft.display());
                    return Ok(1);
                }
                println!("📝 Refining draft: {slug}");
                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Read and improve the draft PRD at {path}. \
                         If you have file tools, update that file directly. \
                         If you do NOT have file tools, output the complete improved PRD markdown \
                         with YAML frontmatter and no code fences. \
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
                     Search the codebase to verify claims. \
                     If you have file tools, update the file in place. \
                     Otherwise, output the complete improved PRD markdown with YAML frontmatter.",
                    path = draft.display()
                );
                let mtime_before = std::fs::metadata(&draft).and_then(|m| m.modified()).ok();
                let started = Instant::now();
                let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                    role: Some("scribe"),
                })
                .await?;
                let mtime_after = std::fs::metadata(&draft).and_then(|m| m.modified()).ok();
                let file_was_modified = match (mtime_before, mtime_after) {
                    (Some(before), Some(after)) => after > before,
                    _ => false,
                };
                if file_was_modified {
                    let content = std::fs::read_to_string(&draft).unwrap_or_default();
                    if roko_cli::prd::has_substantive_markdown_content(&content) {
                        println!("📄 Draft updated at {}", draft.display());
                    } else {
                        eprintln!(
                            "Agent modified file but left it empty at {}",
                            draft.display()
                        );
                    }
                } else if exit_code == 0 {
                    if let Some(content) =
                        roko_cli::prd::materialize_agent_markdown_output(&output, None)
                    {
                        std::fs::write(&draft, content)?;
                        println!("📄 Draft updated at {}", draft.display());
                    } else {
                        eprintln!(
                            "Agent returned empty output. Existing draft preserved at {}",
                            draft.display()
                        );
                    }
                } else if !output.is_empty() {
                    print!("{output}");
                }
                let _ = persist_capture_episode(
                    &workdir,
                    &agent_command,
                    model_ref,
                    "prd-draft-edit",
                    &format!("prd:draft:edit:{slug}"),
                    &task_prompt,
                    &output,
                    exit_code == 0,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                Ok(exit_code)
            }
            PrdDraftCmd::Promote { slug, auto_execute } => {
                roko_cli::prd::cmd_promote(&workdir, &slug, auto_execute).await?;
                Ok(0)
            }
            PrdDraftCmd::List => {
                let drafts = roko_cli::workspace_paths::drafts_dir(&workdir);
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
                let dir = roko_cli::workspace_paths::prd_dir(&workdir).join(dir_name);
                for path in roko_cli::prd::list_md_files(&dir) {
                    if let Ok(c) = std::fs::read_to_string(&path) {
                        let truncated: String = c.lines().take(50).collect::<Vec<_>>().join("\n");
                        let _ = write!(all_context, "### {}\n{truncated}\n---\n\n", path.display());
                    }
                }
            }
            let ideas = std::fs::read_to_string(roko_cli::workspace_paths::ideas_path(&workdir))
                .unwrap_or_default();
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
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("strategist"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "prd-consolidate",
                "prd:draft:consolidate",
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
    }
}

// Make list_md_files public so main.rs can use it for draft list
// (it's already pub in prd.rs)

/// Auto-detect the project domain from file patterns in the target directory.
fn detect_project_domain(target: &Path) -> &'static str {
    if target.join("Cargo.toml").exists() {
        "rust"
    } else if target.join("package.json").exists() {
        "typescript"
    } else if target.join("go.mod").exists() {
        "go"
    } else if target.join("requirements.txt").exists()
        || target.join("pyproject.toml").exists()
        || target.join("setup.py").exists()
    {
        "python"
    } else if target.join("Gemfile").exists() {
        "ruby"
    } else if target.join("pom.xml").exists() || target.join("build.gradle").exists() {
        "java"
    } else {
        "general"
    }
}

/// Gate configuration hint based on domain profile.
fn domain_gate_hint(domain: &str) -> &'static str {
    match domain {
        "rust" => "compile (cargo check), test (cargo test), clippy (cargo clippy)",
        "typescript" => "compile (tsc --noEmit), test (npm test), lint (eslint)",
        "go" => "compile (go build), test (go test), lint (golangci-lint)",
        "python" => "test (pytest), lint (ruff), typecheck (mypy)",
        "ruby" => "test (rspec), lint (rubocop)",
        "java" => "compile (mvn compile), test (mvn test)",
        _ => "compile, test, lint (configure in roko.toml)",
    }
}

async fn cmd_init(path: Option<PathBuf>, cloud: bool, profile: Option<String>) -> Result<()> {
    let target = path.unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let roko_dir = target.join(".roko");
    tokio::fs::create_dir_all(&roko_dir)
        .await
        .with_context(|| format!("create {}", roko_dir.display()))?;

    // Create all top-level layout directories and VERSION file via RokoLayout.
    // This ensures doctor checks pass and all subsystems have their dirs.
    let layout = RokoLayout::for_project(&target);
    layout
        .ensure_dirs()
        .await
        .with_context(|| "create .roko layout directories")?;

    // Create additional directories used by CLI subsystems but not in
    // RokoLayout::top_level_dirs() (jobs, prd, task-outputs, etc.).
    for extra in &[
        roko_dir.join("jobs"),
        roko_dir.join("prd"),
        roko_dir.join("prd").join("published"),
        roko_dir.join("prd").join("drafts"),
        roko_dir.join("task-outputs"),
        roko_dir.join("research"),
        roko_dir.join("subscriptions"),
        roko_dir.join("templates"),
    ] {
        tokio::fs::create_dir_all(extra)
            .await
            .with_context(|| format!("create {}", extra.display()))?;
    }

    let engrams_path = roko_dir.join("engrams.jsonl");
    if !engrams_path.exists() {
        // Migrate from legacy name if present.
        let legacy = roko_dir.join("signals.jsonl");
        if legacy.exists() {
            tokio::fs::rename(&legacy, &engrams_path)
                .await
                .with_context(|| {
                    format!("migrate {} -> {}", legacy.display(), engrams_path.display())
                })?;
        } else {
            tokio::fs::write(&engrams_path, b"")
                .await
                .with_context(|| format!("create {}", engrams_path.display()))?;
        }
    }

    // Domain detection: use --profile if given, otherwise auto-detect.
    let domain = if let Some(ref p) = profile {
        p.as_str()
    } else {
        detect_project_domain(&target)
    };

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
    println!("detected project domain: {domain}");
    println!("suggested gates: {}", domain_gate_hint(domain));
    println!(
        "agent command set to \"claude\". \
         Edit roko.toml [agent] command to use a different agent CLI."
    );

    // Check for interrupted session from a previous run.
    let snapshot = roko_dir.join("state").join("executor.json");
    if snapshot.is_file() {
        println!();
        println!("interrupted session found: {}", snapshot.display());
        println!(
            "resume with: roko plan run plans/ --resume {}",
            snapshot.display()
        );
    }

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
    if !cli.quiet {
        tracing::info!(
            workdir = %workdir.display(),
            json = cli.json,
            cfactor,
            "collecting status snapshot"
        );
    }
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
    let learn_dir = workdir.join(".roko").join("learn");
    let costs_log = CostsLog::at(learn_dir.join("costs.jsonl"));
    let total_cost_usd = costs_log.total_cost().await.ok();
    let today_cost_usd = costs_log
        .daily_cost(1)
        .await
        .ok()
        .and_then(|days| days.last().map(|(_, cost)| *cost));
    let cost_by_model = costs_log.cost_by_model().await.unwrap_or_default();
    let cost_by_plan = costs_log.cost_by_plan().await.unwrap_or_default();

    if cli.json {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for sig in &all {
            *counts.entry(sig.kind.to_string()).or_default() += 1;
        }
        let episode_count = counts.get("episode").copied().unwrap_or(0);

        // Gate verdicts from substrate.
        let verdicts_json = substrate
            .query(&Query::of_kind(Kind::GateVerdict), &ctx)
            .await
            .map_err(|e| anyhow!("query verdicts: {e}"))?;
        let gate_pass = verdicts_json
            .iter()
            .filter(|v| v.tag("passed") == Some("true"))
            .count();
        let gate_fail = verdicts_json
            .iter()
            .filter(|v| v.tag("passed") == Some("false"))
            .count();

        // Running agents from runtime directory.
        let runtime_dir_json = workdir.join(".roko").join("runtime");
        let mut running_agents_json: usize = 0;
        if let Ok(mut entries) = tokio::fs::read_dir(&runtime_dir_json).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".pid") {
                    running_agents_json += 1;
                }
            }
        }

        // Active plans from executor snapshot.
        let executor_path_json = workdir.join(".roko").join("state").join("executor.json");
        let active_plans_json: usize = if executor_path_json.is_file() {
            tokio::fs::read_to_string(&executor_path_json)
                .await
                .ok()
                .and_then(|contents| {
                    serde_json::from_str::<serde_json::Value>(&contents)
                        .ok()?
                        .get("plans")?
                        .as_array()
                        .map(|arr| arr.len())
                })
                .unwrap_or(0)
        } else {
            0
        };

        // Most recent episode.
        let mut episodes_json = substrate
            .query(&Query::of_kind(Kind::Episode), &ctx)
            .await
            .map_err(|e| anyhow!("query episodes: {e}"))?;
        episodes_json.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
        let last_passed = episodes_json
            .first()
            .and_then(|ep| ep.tag("passed").map(|v| v == "true"));

        let status = SessionStatus {
            session_id: cli.resume.clone(),
            workdir: workdir.clone(),
            daemon_running: false,
            signal_count: Some(all.len()),
            episode_count: Some(episode_count),
            last_episode_passed: last_passed,
            cfactor: cfactor_snapshot,
            total_cost_usd,
            today_cost_usd,
        };

        // Build enriched JSON with gate verdicts, workspace info, and signal counts.
        let counts_json = serde_json::to_string(&counts).unwrap_or_else(|_| "{}".to_string());
        let cost_by_model_json =
            serde_json::to_string(&cost_by_model).unwrap_or_else(|_| "{}".to_string());
        let cost_by_plan_json =
            serde_json::to_string(&cost_by_plan).unwrap_or_else(|_| "{}".to_string());
        let base = status.display_json();
        // Splice additional fields before the closing brace.
        let enriched = format!(
            "{},\"gates\":{{\"pass\":{gate_pass},\"fail\":{gate_fail}}},\"workspace\":{{\"agents\":{running_agents_json},\"plans\":{active_plans_json}}},\"signal_counts\":{counts_json},\"cost_by_model\":{cost_by_model_json},\"cost_by_plan\":{cost_by_plan_json},\"health\":\"ready\"}}",
            &base[..base.len() - 1],
        );
        println!("{enriched}");
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

    // Running agents from runtime directory.
    let runtime_dir = workdir.join(".roko").join("runtime");
    let mut running_agents: usize = 0;
    if let Ok(mut entries) = tokio::fs::read_dir(&runtime_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".pid") {
                running_agents += 1;
            }
        }
    }

    // Active plans from executor snapshot.
    let executor_path = workdir.join(".roko").join("state").join("executor.json");
    let active_plans: usize = if executor_path.is_file() {
        // Parse minimally: count plans with active=true.
        match tokio::fs::read_to_string(&executor_path).await {
            Ok(contents) => {
                // Quick JSON parse: count occurrences of "active":true or
                // plan entries. For a lightweight check, use serde_json::Value.
                serde_json::from_str::<serde_json::Value>(&contents)
                    .ok()
                    .and_then(|val| val.get("plans")?.as_array().map(|arr| arr.len()))
                    .unwrap_or(0)
            }
            Err(_) => 0,
        }
    } else {
        0
    };

    println!();
    println!(
        "workspace: {} agent pid(s), {} plan(s) in executor snapshot",
        running_agents, active_plans
    );

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

    if total_cost_usd.is_some() || !cost_by_model.is_empty() || !cost_by_plan.is_empty() {
        println!();
        println!("Cost Summary:");
        if let Some(total_cost_usd) = total_cost_usd {
            println!("  Total:    ${total_cost_usd:.4}");
        }
        if let Some(today_cost_usd) = today_cost_usd {
            println!("  Today:    ${today_cost_usd:.4}");
        }
        if !cost_by_model.is_empty() {
            println!("  By model: {}", format_cost_breakdown(&cost_by_model, 5));
        }
        if !cost_by_plan.is_empty() {
            println!("  By plan:  {}", format_cost_breakdown(&cost_by_plan, 5));
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
            cfactor.components.social_perceptiveness
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

fn format_cost_breakdown(costs: &HashMap<String, f64>, limit: usize) -> String {
    let mut entries = costs
        .iter()
        .map(|(name, cost)| (name.as_str(), *cost))
        .collect::<Vec<_>>();
    entries.sort_by(|(left_name, left_cost), (right_name, right_cost)| {
        right_cost
            .total_cmp(left_cost)
            .then_with(|| left_name.cmp(right_name))
    });
    entries.truncate(limit);
    if entries.is_empty() {
        return "none".to_string();
    }

    entries
        .into_iter()
        .map(|(name, cost)| format!("{name}=${cost:.4}"))
        .collect::<Vec<_>>()
        .join(", ")
}

async fn cmd_replay(workdir: Option<PathBuf>, hash: String, forensic: bool) -> Result<i32> {
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
            if forensic {
                println!("{indent}{} {}", sig.kind, sig.id,);
                println!("{indent}  hash:      {}", sig.id,);
                println!("{indent}  author:    {}", sig.provenance.author,);
                println!("{indent}  created:   {}", sig.created_at_ms,);
                println!(
                    "{indent}  lineage:   [{}]",
                    sig.lineage
                        .iter()
                        .map(|h| h.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if !sig.tags.is_empty() {
                    println!("{indent}  tags:      {:?}", sig.tags,);
                }
                if let Ok(text) = sig.body.as_text() {
                    let body_preview: String = text.chars().take(120).collect();
                    println!("{indent}  body:      {body_preview}");
                }
                println!();
            } else {
                println!(
                    "{indent}{} {}  (author={})",
                    sig.kind, sig.id, sig.provenance.author
                );
            }
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

async fn cmd_dream(cli: &Cli, cmd: DreamCmdLegacy) -> Result<i32> {
    match cmd {
        DreamCmdLegacy::Run { workdir } => {
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
        DreamCmdLegacy::Report { workdir } => {
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
        DreamCmdLegacy::Schedule { workdir } => {
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

fn cmd_dreams(cli: &Cli, cmd: DreamsCmd) -> Result<i32> {
    match cmd {
        DreamsCmd::Journal { limit, workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let journal = roko_dreams::phase2::DreamJournal::standard(&workdir);
            match journal.read_recent(limit) {
                Ok(entries) if entries.is_empty() => {
                    println!("no dream journal entries found");
                }
                Ok(entries) => {
                    for entry in &entries {
                        println!(
                            "[{}] cycle={} agent={} hypotheses={}/{}/{} tokens={} {}",
                            entry.cycle_start.format("%Y-%m-%d %H:%M"),
                            entry.cycle_id,
                            entry.agent_id,
                            entry.hypotheses_generated,
                            entry.hypotheses_staged,
                            entry.hypotheses_promoted,
                            entry.total_tokens,
                            if entry.early_termination {
                                "(early termination)"
                            } else {
                                ""
                            },
                        );
                    }
                    println!("\n{} entries shown (of last {})", entries.len(), limit);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!(
                        "no dream journal found at {}",
                        journal.journal_path.display()
                    );
                }
                Err(e) => return Err(e.into()),
            }
            Ok(EXIT_SUCCESS)
        }
        DreamsCmd::Archive { limit, workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let archive = roko_dreams::phase2::DreamArchive::standard(&workdir);
            match archive.read_recent(limit) {
                Ok(entries) if entries.is_empty() => {
                    println!("no dream archive entries found");
                }
                Ok(entries) => {
                    for entry in &entries {
                        println!(
                            "[{}] {} ({:?}) quality={:.2} -- {}",
                            entry.archived_at.format("%Y-%m-%d %H:%M"),
                            entry.entry_id,
                            entry.kind,
                            entry.quality_score,
                            entry.summary,
                        );
                    }
                    println!("\n{} entries shown (of last {})", entries.len(), limit);
                }
                Err(e) => return Err(e.into()),
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

async fn cmd_deploy(cli: &Cli, cmd: DeployCmd) -> Result<i32> {
    match cmd {
        DeployCmd::Railway { workdir } => cmd_deploy_railway(cli, workdir).await,
        DeployCmd::Fly { workdir } => cmd_deploy_fly(cli, workdir).await,
        DeployCmd::Docker { workdir, registry } => cmd_deploy_docker(cli, workdir, registry).await,
    }
}

fn cmd_update(verify: bool) -> Result<i32> {
    if verify {
        println!(
            "release verification uses Sigstore bundles with cosign; pass a downloaded artifact and bundle to the SigstoreVerifier API or verify with cosign verify-blob"
        );
    }

    println!(
        "self-update installer receipts are not wired in this source build; reinstall with cargo install roko-cli --locked or use the cargo-dist installer release"
    );
    Ok(EXIT_SUCCESS)
}

// -----------------------------------------------------------------------
// Index subcommands
// -----------------------------------------------------------------------

fn cmd_index(cli: &Cli, cmd: IndexCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    match cmd {
        IndexCmd::Build { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let start = Instant::now();
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;
            let elapsed = start.elapsed();
            let stats = idx.stats();
            println!("Index built in {:.2}s", elapsed.as_secs_f64());
            println!("  Files:   {}", stats.indexed_files);
            println!("  Symbols: {}", stats.total_symbols);
            println!("  Edges:   {}", stats.total_edges);
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Rebuild { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            // Remove the existing index database if present.
            let db_path = target.join(".roko").join("index.db");
            if db_path.exists() {
                std::fs::remove_file(&db_path)
                    .with_context(|| format!("remove old index at {}", db_path.display()))?;
                println!("Removed old index: {}", db_path.display());
            }
            // Rebuild from scratch.
            let start = Instant::now();
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("rebuild index for {}", target.display()))?;
            let elapsed = start.elapsed();
            let stats = idx.stats();
            println!("Index rebuilt in {:.2}s", elapsed.as_secs_f64());
            println!("  Files:   {}", stats.indexed_files);
            println!("  Symbols: {}", stats.total_symbols);
            println!("  Edges:   {}", stats.total_edges);
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Search {
            query,
            kind,
            strategy,
            limit,
            path,
        } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;

            let sym_kind = if let Some(ref k) = kind {
                Some(parse_symbol_kind(k)?)
            } else {
                None
            };

            let search_strategy = match strategy.as_str() {
                "keyword" => roko_index::SearchStrategy::Keyword(roko_index::KeywordQuery {
                    text: query.clone(),
                    scope: roko_index::SearchScope::Both,
                    case_sensitive: false,
                    whole_word: false,
                }),
                "structural" => {
                    roko_index::SearchStrategy::Structural(roko_index::StructuralQuery {
                        kind: sym_kind,
                        visibility: None,
                        file_pattern: Some(query.clone()),
                        has_callers: None,
                        min_pagerank: None,
                    })
                }
                "hybrid" => roko_index::SearchStrategy::Hybrid {
                    keyword: Some(roko_index::KeywordQuery {
                        text: query.clone(),
                        scope: roko_index::SearchScope::Both,
                        case_sensitive: false,
                        whole_word: false,
                    }),
                    structural: sym_kind.map(|k| roko_index::StructuralQuery {
                        kind: Some(k),
                        ..Default::default()
                    }),
                    hdc: None,
                },
                other => bail!(
                    "unknown search strategy: {other} (expected keyword, structural, or hybrid)"
                ),
            };

            let results = idx.search(search_strategy, limit);
            if results.is_empty() {
                println!("No results found for \"{query}\"");
            } else {
                println!("{:<50} {:<10} {:<6} {:<8}", "NAME", "KIND", "LINE", "SCORE");
                println!("{}", "-".repeat(76));
                for r in &results {
                    println!(
                        "{:<50} {:<10} {:<6} {:.4}",
                        r.symbol.id.symbol_name,
                        format!("{:?}", r.symbol.id.kind),
                        r.symbol.line,
                        r.score,
                    );
                }
                println!("\n{} result(s)", results.len());
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Stats { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;
            let stats = idx.stats();

            println!("=== Index Statistics ===\n");
            println!("Files indexed:  {}", stats.indexed_files);
            println!("Total symbols:  {}", stats.total_symbols);
            println!("Total edges:    {}", stats.total_edges);

            println!("\nEdge breakdown:");
            for (kind, count) in &stats.edge_breakdown {
                println!("  {kind}: {count}");
            }

            println!("\nLanguages:");
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }

            if !stats.top_symbols_by_pagerank.is_empty() {
                println!("\nTop-10 symbols by PageRank:");
                println!("{:<50} {:<10} {:<8}", "NAME", "KIND", "SCORE");
                println!("{}", "-".repeat(70));
                for r in &stats.top_symbols_by_pagerank {
                    println!(
                        "{:<50} {:<10} {:.6}",
                        r.symbol.id.symbol_name,
                        format!("{:?}", r.symbol.id.kind),
                        r.score,
                    );
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

fn parse_symbol_kind(s: &str) -> Result<roko_core::language::SymbolKind> {
    use roko_core::language::SymbolKind;
    match s.to_lowercase().as_str() {
        "function" | "fn" => Ok(SymbolKind::Function),
        "struct" => Ok(SymbolKind::Struct),
        "enum" => Ok(SymbolKind::Enum),
        "trait" => Ok(SymbolKind::Trait),
        "const" => Ok(SymbolKind::Const),
        "type" => Ok(SymbolKind::Type),
        "module" | "mod" => Ok(SymbolKind::Module),
        "impl" => Ok(SymbolKind::Impl),
        other => bail!(
            "unknown symbol kind: {other} (expected function, struct, enum, trait, const, type, module, impl)"
        ),
    }
}

// ── Tune command ──────────────────────────────────────────────────────

/// `roko tune [subsystem]` — display and optionally adjust adaptive thresholds.
async fn cmd_tune(workdir: &std::path::Path, subsystem: &str, dry_run: bool) -> Result<i32> {
    match subsystem {
        "gates" => {
            let path = workdir.join(".roko/learn/gate-thresholds.json");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let thresholds: serde_json::Value = serde_json::from_str(&content)?;
                println!("Gate adaptive thresholds ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&thresholds)?);
            } else {
                println!("No gate thresholds found at {}.", path.display());
                println!("Run some plans first to generate adaptive thresholds.");
            }
        }
        "routing" => {
            let path = workdir.join(".roko/learn/cascade-router.json");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let router: serde_json::Value = serde_json::from_str(&content)?;
                println!("Cascade router state ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&router)?);
            } else {
                println!("No cascade router state found at {}.", path.display());
            }
        }
        "budget" => {
            let path = workdir.join(".roko/learn/efficiency.jsonl");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let count = content.lines().filter(|l| !l.trim().is_empty()).count();
                println!("Efficiency log: {} entries at {}", count, path.display());
            } else {
                println!("No efficiency log found at {}.", path.display());
            }
        }
        other => {
            eprintln!("Unknown subsystem '{other}'. Available: gates, routing, budget");
            return Ok(1);
        }
    }
    if dry_run {
        println!("(dry-run: no changes applied)");
    }
    Ok(EXIT_SUCCESS)
}

// ── Learn command ─────────────────────────────────────────────────────

/// `roko learn [what]` — display learning subsystem state.
async fn cmd_learn(workdir: &std::path::Path, what: &str) -> Result<i32> {
    let show_all = what == "all";

    if show_all || what == "router" {
        print_learn_router(workdir);
    }

    if show_all || what == "experiments" {
        print_learn_experiments(workdir);
    }

    if show_all || what == "efficiency" {
        print_learn_efficiency(workdir).await;
    }

    if show_all || what == "episodes" {
        print_learn_episodes(workdir);
    }

    if !show_all && !["router", "experiments", "efficiency", "episodes"].contains(&what) {
        eprintln!(
            "Unknown learning area '{what}'. Available: router, experiments, efficiency, episodes, all"
        );
        return Ok(1);
    }

    Ok(EXIT_SUCCESS)
}

fn print_learn_router(workdir: &std::path::Path) {
    let path = workdir.join(".roko/learn/cascade-router.json");
    if !path.exists() {
        println!("Cascade router: not initialized");
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Cascade router: unreadable");
        return;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        println!("Cascade router: parse error");
        return;
    };

    // Model slugs
    let slugs: Vec<&str> = val
        .get("model_slugs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let total_obs = val
        .get("arms")
        .and_then(|v| v.as_array())
        .map(|arms| {
            arms.iter()
                .filter_map(|arm| arm.get("observations").and_then(|v| v.as_u64()))
                .sum::<u64>()
        })
        .unwrap_or(0);

    println!(
        "Cascade router: {} models, {} total observations",
        slugs.len(),
        total_obs
    );

    // Per-arm summary
    if let Some(arms) = val.get("arms").and_then(|v| v.as_array()) {
        for arm in arms {
            let slug = arm
                .get("model_slug")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let obs = arm.get("observations").and_then(|v| v.as_u64()).unwrap_or(0);
            let reward = arm.get("mean_reward").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if obs > 0 {
                println!("  {slug}: {obs} obs, mean_reward={reward:.3}");
            }
        }
    }
}

fn print_learn_experiments(workdir: &std::path::Path) {
    // Prompt experiments
    let prompt_path = workdir.join(".roko/learn/experiments.json");
    let prompt_store = ExperimentStore::load_or_new(&prompt_path);
    let running = prompt_store.running_count();
    let concluded = prompt_store.concluded_count();
    if running > 0 || concluded > 0 {
        println!(
            "Prompt experiments: {} running, {} concluded",
            running, concluded
        );
    } else {
        println!("Prompt experiments: none");
    }

    // Model experiments
    let model_path = workdir
        .join(".roko")
        .join("learn")
        .join("model-experiments.json");
    let model_store =
        roko_learn::model_experiment::ModelExperimentStore::load_or_new(&model_path);
    let model_running = model_store.running_count();
    let model_concluded = model_store.concluded_experiments().len();
    if model_running > 0 || model_concluded > 0 {
        println!(
            "Model experiments: {} running, {} concluded",
            model_running, model_concluded
        );
        for exp in model_store.iter() {
            println!(
                "  {} [{:?}] role={} variants={} winner={}",
                exp.experiment_id,
                exp.status,
                exp.role.as_deref().unwrap_or("any"),
                exp.variants.len(),
                exp.winner_id.as_deref().unwrap_or("-"),
            );
        }
    } else {
        println!("Model experiments: none");
    }
}

#[allow(clippy::cast_precision_loss)]
async fn print_learn_efficiency(workdir: &std::path::Path) {
    let path = workdir.join(".roko/learn/efficiency.jsonl");
    let events = match read_efficiency_events(&path).await {
        Ok(events) => events,
        Err(_) => {
            println!("Efficiency log: empty");
            return;
        }
    };
    if events.is_empty() {
        println!("Efficiency log: empty");
        return;
    }

    let total_cost: f64 = events.iter().map(|e| e.cost_usd).sum();
    let total_input: u64 = events.iter().map(|e| e.input_tokens).sum();
    let total_output: u64 = events.iter().map(|e| e.output_tokens).sum();
    let success_count = events.iter().filter(|e| e.gate_passed).count();
    let pass_rate = if events.is_empty() {
        0.0
    } else {
        success_count as f64 / events.len() as f64 * 100.0
    };

    println!(
        "Efficiency: {} events, ${:.2} total, {:.0}% pass rate",
        events.len(),
        total_cost,
        pass_rate
    );
    println!(
        "  Tokens: {}K input, {}K output",
        total_input / 1000,
        total_output / 1000
    );

    // Per-model breakdown
    let mut by_model: std::collections::HashMap<&str, (usize, f64, usize)> =
        std::collections::HashMap::new();
    for ev in &events {
        let entry = by_model.entry(ev.model.as_str()).or_default();
        entry.0 += 1;
        entry.1 += ev.cost_usd;
        if ev.gate_passed {
            entry.2 += 1;
        }
    }
    let mut model_summary: Vec<_> = by_model.into_iter().collect();
    model_summary.sort_by(|a, b| b.1 .1.partial_cmp(&a.1 .1).unwrap_or(std::cmp::Ordering::Equal));
    for (model, (count, cost, passed)) in &model_summary {
        let model_pass = if *count == 0 {
            0.0
        } else {
            *passed as f64 / *count as f64 * 100.0
        };
        println!(
            "  {model}: {count} runs, ${cost:.2}, {model_pass:.0}% pass",
        );
    }
}

fn print_learn_episodes(workdir: &std::path::Path) {
    let path = workdir.join(".roko/episodes.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Episodes: none");
        return;
    };

    let episodes: Vec<Episode> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if episodes.is_empty() {
        println!("Episodes: none");
        return;
    }

    let success_count = episodes.iter().filter(|e| e.success).count();
    let total_cost: f64 = episodes.iter().map(|e| f64::from(e.usage.cost_usd)).sum();
    let pass_rate = if episodes.is_empty() {
        0.0
    } else {
        success_count as f64 / episodes.len() as f64 * 100.0
    };

    println!(
        "Episodes: {} recorded, {:.0}% success, ${:.2} total cost",
        episodes.len(),
        pass_rate,
        total_cost,
    );

    // Per-model summary
    let mut by_model: std::collections::HashMap<&str, (usize, usize)> =
        std::collections::HashMap::new();
    for ep in &episodes {
        let entry = by_model.entry(ep.model.as_str()).or_default();
        entry.0 += 1;
        if ep.success {
            entry.1 += 1;
        }
    }
    let mut model_summary: Vec<_> = by_model.into_iter().collect();
    model_summary.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (model, (count, passed)) in &model_summary {
        let model_pass = if *count == 0 {
            0.0
        } else {
            *passed as f64 / *count as f64 * 100.0
        };
        println!("  {model}: {count} episodes, {model_pass:.0}% success");
    }

    // Last 5 episodes
    let last_n = episodes.len().min(5);
    if last_n > 0 {
        println!("  Recent:");
        for ep in episodes.iter().rev().take(last_n) {
            let status = if ep.success { "pass" } else { "fail" };
            println!(
                "    {} {} {} [{}] ${:.4}",
                ep.timestamp.format("%Y-%m-%d %H:%M"),
                ep.model,
                ep.task_id,
                status,
                ep.usage.cost_usd,
            );
        }
    }
}

fn print_completions(shell: CompletionShell) {
    let words = completion_words();
    let subcommand_map = nested_subcommand_words();
    let dynamic = dynamic_completion_words();
    match shell {
        CompletionShell::Bash => print_bash_completions(&words, &subcommand_map, &dynamic),
        CompletionShell::Zsh => print_zsh_completions(&words, &subcommand_map, &dynamic),
        CompletionShell::Fish => print_fish_completions(&words, &subcommand_map, &dynamic),
    }
}

fn completion_words() -> Vec<String> {
    let mut command = Cli::command();
    command.build();
    let mut words = command
        .get_subcommands()
        .map(|cmd| cmd.get_name().to_string())
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words
}

/// Collect nested subcommand names for each top-level command.
fn nested_subcommand_words() -> Vec<(String, Vec<String>)> {
    let mut command = Cli::command();
    command.build();
    let mut result = Vec::new();
    for sub in command.get_subcommands() {
        let name = sub.get_name().to_string();
        let nested: Vec<String> = sub
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        if !nested.is_empty() {
            result.push((name, nested));
        }
    }
    result
}

/// Scan the filesystem for dynamic completion words (plan names, PRD slugs).
fn dynamic_completion_words() -> Vec<(String, Vec<String>)> {
    let mut result = Vec::new();

    // Scan plans/ directory for plan names.
    if let Ok(entries) = std::fs::read_dir("plans") {
        let plans: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir() || e.path().extension().is_some_and(|x| x == "toml"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        if !plans.is_empty() {
            result.push(("plan".to_string(), plans));
        }
    }

    // Scan .roko/prd/ directory for PRD slugs.
    if let Ok(entries) = std::fs::read_dir(".roko/prd") {
        let prds: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                e.path()
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        if !prds.is_empty() {
            result.push(("prd".to_string(), prds));
        }
    }

    result
}

/// Global flag names for flag completion (UX-1c).
fn completion_flag_words() -> Vec<String> {
    let mut command = Cli::command();
    command.build();
    let mut flags: Vec<String> = command
        .get_arguments()
        .filter_map(|arg| arg.get_long().map(|l| format!("--{l}")))
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

fn print_bash_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let top_words = words.join(" ");
    let flag_words = completion_flag_words().join(" ");
    println!(r#"# roko bash completions (DEPLOY-06: dynamic + nested + flags)"#);
    println!(r#"_roko()"#);
    println!(r#"{{"#);
    println!(r#"    local cur="${{COMP_WORDS[COMP_CWORD]}}""#);
    println!(r#"    local prev="${{COMP_WORDS[COMP_CWORD-1]}}""#);
    println!();
    // Flag completions when current word starts with -.
    println!(r#"    if [[ "$cur" == -* ]]; then"#);
    println!(r#"        COMPREPLY=( $(compgen -W "{flag_words}" -- "$cur") )"#);
    println!(r#"        return 0"#);
    println!(r#"    fi"#);
    println!();
    // Nested subcommand completions.
    println!(r#"    case "$prev" in"#);
    for (parent, children) in subcommands {
        let child_words = children.join(" ");
        println!(r#"        {parent})"#);
        println!(r#"            COMPREPLY=( $(compgen -W "{child_words}" -- "$cur") )"#);
        println!(r#"            return 0"#);
        println!(r#"            ;;"#);
    }
    // Dynamic completions for plan/prd subcommands.
    for (parent, items) in dynamic {
        let item_words = items.join(" ");
        // Add dynamic words to existing subcommand completions.
        println!(r#"        {parent})"#);
        println!(r#"            COMPREPLY=( $(compgen -W "{item_words}" -- "$cur") )"#);
        println!(r#"            return 0"#);
        println!(r#"            ;;"#);
    }
    println!(r#"    esac"#);
    println!();
    // Top-level completions.
    println!(r#"    COMPREPLY=( $(compgen -W "{top_words}" -- "$cur") )"#);
    println!(r#"}}"#);
    println!(r#"complete -F _roko roko"#);
}

fn print_zsh_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let flags = completion_flag_words();
    println!(r#"#compdef roko"#);
    println!(r#"# roko zsh completions (DEPLOY-06: dynamic + nested + flags)"#);
    println!(r#"_roko() {{"#);
    println!(r#"  local -a commands flags"#);
    let top_words = words.join(" ");
    let flag_words = flags.join(" ");
    println!(r#"  commands=({top_words})"#);
    println!(r#"  flags=({flag_words})"#);
    println!();
    // Flag completion at any position when current word starts with -.
    println!(r#"  if [[ "$words[CURRENT]" == -* ]]; then"#);
    println!(r#"    _describe 'roko flag' flags"#);
    println!(r#"    return"#);
    println!(r#"  fi"#);
    println!();
    println!(r#"  if (( CURRENT == 2 )); then"#);
    println!(r#"    _describe 'roko command' commands"#);
    println!(r#"  elif (( CURRENT == 3 )); then"#);
    println!(r#"    case $words[2] in"#);
    for (parent, children) in subcommands {
        let child_words = children.join(" ");
        println!(r#"      {parent})"#);
        println!(r#"        local -a subcmds"#);
        println!(r#"        subcmds=({child_words})"#);
        println!(r#"        _describe '{parent} subcommand' subcmds"#);
        println!(r#"        ;;"#);
    }
    for (parent, items) in dynamic {
        let item_words = items.join(" ");
        println!(r#"      {parent})"#);
        println!(r#"        local -a slugs"#);
        println!(r#"        slugs=({item_words})"#);
        println!(r#"        _describe '{parent} item' slugs"#);
        println!(r#"        ;;"#);
    }
    println!(r#"    esac"#);
    println!(r#"  fi"#);
    println!(r#"}}"#);
    println!(r#"_roko "$@""#);
}

fn print_fish_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let flags = completion_flag_words();
    println!("# roko fish completions (DEPLOY-06: dynamic + nested + flags)");
    for word in words {
        println!("complete -c roko -f -n '__fish_use_subcommand' -a '{word}'");
    }
    // Global flag completions.
    for flag in &flags {
        let short = flag.trim_start_matches('-');
        println!("complete -c roko -l '{short}'");
    }
    // Nested subcommand completions.
    for (parent, children) in subcommands {
        for child in children {
            println!("complete -c roko -f -n '__fish_seen_subcommand_from {parent}' -a '{child}'");
        }
    }
    // Dynamic completions.
    for (parent, items) in dynamic {
        for item in items {
            println!("complete -c roko -f -n '__fish_seen_subcommand_from {parent}' -a '{item}'");
        }
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
internal_port = 6677
force_https = true
auto_stop_machines = true
auto_start_machines = true
min_machines_running = 0

[[http_service.checks]]
interval = "30s"
timeout = "5s"
grace_period = "10s"
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
///
/// Detects when the user is running from inside a `.roko/` directory, which
/// would cause a nested `.roko/.roko/` and silent data loss.
fn resolve_workdir(cli: &Cli) -> PathBuf {
    let dir = cli.repo.clone().unwrap_or_else(|| PathBuf::from("."));

    // Detect if we're running from inside a .roko/ directory.
    if let Ok(abs) = dir.canonicalize() {
        for ancestor in abs.ancestors() {
            if ancestor.file_name().and_then(|n| n.to_str()) == Some(".roko") {
                eprintln!(
                    "\x1b[33m\u{26a0} Warning: running from inside a .roko/ directory ({}).\x1b[0m",
                    abs.display()
                );
                eprintln!("  This will create a nested .roko/.roko/ which causes data loss.");
                eprintln!(
                    "  Run from the project root instead: cd {}",
                    ancestor
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "..".to_string())
                );
                eprintln!();
                break;
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
    fn cli_parses_top_level_secret_subcommand() {
        let cli = Cli::try_parse_from(["roko", "secret", "get", "anthropic.api_key"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Secret {
                cmd: roko_cli::SecretsCmd::Get { namespace, key }
            }) if namespace == "anthropic.api_key" && key.is_none()
        ));
    }

    #[test]
    fn cli_parses_replay_subcommand() {
        let cli = Cli::try_parse_from(["roko", "replay", "abcd1234"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Replay { .. })));
    }

    #[test]
    fn cli_parses_update_subcommand() {
        let cli = Cli::try_parse_from(["roko", "update", "--verify"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Update { verify: true })
        ));
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
    fn cli_parses_neuro_backup_subcommand() {
        let cli = Cli::try_parse_from(["roko", "neuro", "backup", "/tmp/neuro-backup"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Neuro {
                cmd: NeuroCmd::Backup { destination, force, .. }
            }) if destination == PathBuf::from("/tmp/neuro-backup") && !force
        ));
    }

    #[test]
    fn cli_parses_neuro_restore_subcommand() {
        let cli = Cli::try_parse_from(["roko", "neuro", "restore", "/tmp/neuro-backup", "--force"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Neuro {
                cmd: NeuroCmd::Restore { source, force, .. }
            }) if source == PathBuf::from("/tmp/neuro-backup") && force
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
    fn cli_chat_defaults_to_canonical_serve_url() {
        let cli = Cli::try_parse_from(["roko", "chat"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Chat { serve_url, .. }) if serve_url == roko_cli::DEFAULT_SERVE_URL
        ));
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
        bootstrap_observability_dirs(tmp.path()).unwrap();
        let roko = tmp.path().join(".roko");
        assert!(roko.join("traces").is_dir());
        assert!(roko.join("metrics").is_dir());
        assert!(roko.join("runtime").is_dir());
        assert!(roko.join("runs").is_dir());
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

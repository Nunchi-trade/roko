//! Command and TUI surface inventory.
//!
//! This module documents every operator-visible entry point across CLI
//! commands, TUI tabs/subviews, and their backend dependencies. It serves
//! as the single source of truth for what surfaces exist and their status.
//!
//! Use `roko status --surfaces` to print the full inventory.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Wiring status of a surface entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceStatus {
    /// Fully wired to real backend state.
    Wired,
    /// Renders but some data is stubbed or missing.
    Partial,
    /// Scaffold only, not functional.
    Stub,
    /// Planned but not yet started.
    Missing,
}

/// What kind of surface this entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// Top-level CLI subcommand (e.g. `roko plan run`).
    CliCommand,
    /// Top-level TUI tab (F1-F9).
    TuiTab,
    /// Sub-view within a TUI tab (number-key selection).
    TuiSubView,
    /// Modal / overlay (e.g. approval dialog, create-job form).
    Modal,
}

/// A single surface in the inventory.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceEntry {
    /// Human-readable name (e.g. "plan run", "F4 Git", "CreateJob").
    pub name: String,
    /// What kind of surface this is.
    pub kind: SurfaceKind,
    /// Current wiring status.
    pub status: SurfaceStatus,
    /// Backend crate or subsystem this depends on.
    pub backend_dependency: String,
    /// Free-form notes about status or gaps.
    pub notes: String,
}

impl SurfaceEntry {
    fn cli(name: &str, status: SurfaceStatus, backend: &str, notes: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SurfaceKind::CliCommand,
            status,
            backend_dependency: backend.to_string(),
            notes: notes.to_string(),
        }
    }

    fn tab(name: &str, status: SurfaceStatus, backend: &str, notes: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SurfaceKind::TuiTab,
            status,
            backend_dependency: backend.to_string(),
            notes: notes.to_string(),
        }
    }

    fn subview(name: &str, status: SurfaceStatus, backend: &str, notes: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SurfaceKind::TuiSubView,
            status,
            backend_dependency: backend.to_string(),
            notes: notes.to_string(),
        }
    }

    fn modal(name: &str, status: SurfaceStatus, backend: &str, notes: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: SurfaceKind::Modal,
            status,
            backend_dependency: backend.to_string(),
            notes: notes.to_string(),
        }
    }
}

impl std::fmt::Display for SurfaceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wired => write!(f, "wired"),
            Self::Partial => write!(f, "partial"),
            Self::Stub => write!(f, "stub"),
            Self::Missing => write!(f, "missing"),
        }
    }
}

impl std::fmt::Display for SurfaceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CliCommand => write!(f, "cli"),
            Self::TuiTab => write!(f, "tab"),
            Self::TuiSubView => write!(f, "subview"),
            Self::Modal => write!(f, "modal"),
        }
    }
}

// ---------------------------------------------------------------------------
// Full inventory
// ---------------------------------------------------------------------------

/// Return the complete surface inventory.
///
/// Each entry is based on a source-level audit of the actual handler or
/// view implementation as of the initial inventory pass.
#[must_use]
pub fn full_inventory() -> Vec<SurfaceEntry> {
    let mut v = Vec::with_capacity(96);

    // ── CLI commands ────────────────────────────────────────────────────
    v.push(SurfaceEntry::cli(
        "init",
        SurfaceStatus::Wired,
        "roko-fs",
        "Creates .roko/ dir and roko.toml; supports --cloud and --profile",
    ));
    v.push(SurfaceEntry::cli(
        "run / do",
        SurfaceStatus::Wired,
        "roko-agent, roko-compose, roko-gate",
        "Single-prompt universal loop: compose -> agent -> gate -> persist",
    ));
    v.push(SurfaceEntry::cli(
        "status",
        SurfaceStatus::Wired,
        "roko-fs, roko-learn",
        "Signal counts, episodes, gate verdicts, efficiency, cost, health, c-factor",
    ));
    v.push(SurfaceEntry::cli(
        "doctor",
        SurfaceStatus::Wired,
        "roko-core, roko-serve",
        "Workspace bootstrap diagnostics; probes roko-serve health endpoint",
    ));
    v.push(SurfaceEntry::cli(
        "replay / inspect",
        SurfaceStatus::Wired,
        "roko-fs",
        "Walk signal lineage DAG by hash; --forensic for full detail",
    ));
    v.push(SurfaceEntry::cli(
        "dream run",
        SurfaceStatus::Wired,
        "roko-dreams",
        "Runs a dream consolidation cycle immediately",
    ));
    v.push(SurfaceEntry::cli(
        "dream report",
        SurfaceStatus::Wired,
        "roko-dreams",
        "Shows latest dream report",
    ));
    v.push(SurfaceEntry::cli(
        "dream schedule",
        SurfaceStatus::Wired,
        "roko-dreams",
        "Shows next scheduled dream time",
    ));
    v.push(SurfaceEntry::cli(
        "dreams journal",
        SurfaceStatus::Wired,
        "roko-dreams",
        "Displays recent dream journal entries",
    ));
    v.push(SurfaceEntry::cli(
        "dreams archive",
        SurfaceStatus::Wired,
        "roko-dreams",
        "Displays recent dream archive entries",
    ));
    v.push(SurfaceEntry::cli(
        "config init/wizard",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Interactive wizard; detects LLM CLIs, writes global config",
    ));
    v.push(SurfaceEntry::cli(
        "config show",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Prints effective merged config with per-field source tags",
    ));
    v.push(SurfaceEntry::cli(
        "config path",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Prints resolved global + project + env config paths",
    ));
    v.push(SurfaceEntry::cli(
        "config edit",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Opens $EDITOR on chosen config file",
    ));
    v.push(SurfaceEntry::cli(
        "config set",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Sets a dotted key in global or project layer",
    ));
    v.push(SurfaceEntry::cli(
        "config set-secret",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Stores secret in ~/.roko/.env",
    ));
    v.push(SurfaceEntry::cli(
        "config check-secrets",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Validates ${VAR} references in config against available secrets",
    ));
    v.push(SurfaceEntry::cli(
        "config validate",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Validates roko.toml syntax, schema, and semantic references",
    ));
    v.push(SurfaceEntry::cli(
        "config migrate",
        SurfaceStatus::Wired,
        "roko-cli config",
        "Migrates legacy project roko.toml to explicit provider/model tables",
    ));
    v.push(SurfaceEntry::cli(
        "secret / secrets",
        SurfaceStatus::Wired,
        "roko-cli secrets",
        "Profile-aware secrets management",
    ));
    v.push(SurfaceEntry::cli(
        "custody list",
        SurfaceStatus::Wired,
        "roko-cli custody",
        "Lists recent custody audit records",
    ));
    v.push(SurfaceEntry::cli(
        "custody show",
        SurfaceStatus::Wired,
        "roko-cli custody",
        "Shows full custody record by index",
    ));
    v.push(SurfaceEntry::cli(
        "custody verify",
        SurfaceStatus::Wired,
        "roko-cli custody",
        "Verifies integrity of the custody chain",
    ));
    v.push(SurfaceEntry::cli(
        "agent create",
        SurfaceStatus::Wired,
        "roko-agent lifecycle",
        "Creates agent manifest at .roko/agents/<name>/manifest.toml; validates and resolves",
    ));
    v.push(SurfaceEntry::cli(
        "agent delete",
        SurfaceStatus::Wired,
        "roko-agent lifecycle",
        "8-step ordered shutdown: stop, flush, backup, deregister, release, archive, clean, confirm",
    ));
    v.push(SurfaceEntry::cli(
        "agent serve",
        SurfaceStatus::Wired,
        "roko-agent-server",
        "Starts per-agent HTTP sidecar with real LLM dispatch",
    ));
    v.push(SurfaceEntry::cli(
        "agent list",
        SurfaceStatus::Missing,
        "roko-agent lifecycle",
        "Not implemented; no way to list provisioned agents from CLI",
    ));
    v.push(SurfaceEntry::cli(
        "agent start/stop/status",
        SurfaceStatus::Missing,
        "roko-agent lifecycle",
        "Not implemented; create/delete/serve exist but no start/stop/status lifecycle",
    ));
    v.push(SurfaceEntry::cli(
        "inject",
        SurfaceStatus::Wired,
        "roko-cli inject",
        "Injects directive/abort/context signal into a running session",
    ));
    v.push(SurfaceEntry::cli(
        "plan list",
        SurfaceStatus::Wired,
        "roko-cli plan",
        "Lists all plans in workspace",
    ));
    v.push(SurfaceEntry::cli(
        "plan show",
        SurfaceStatus::Wired,
        "roko-cli plan",
        "Shows plan details by ID",
    ));
    v.push(SurfaceEntry::cli(
        "plan create",
        SurfaceStatus::Wired,
        "roko-cli plan",
        "Creates a new plan with ID, title, description",
    ));
    v.push(SurfaceEntry::cli(
        "plan validate",
        SurfaceStatus::Wired,
        "roko-cli plan_validate",
        "Lints tasks.toml files without executing; supports --strict and --json",
    ));
    v.push(SurfaceEntry::cli(
        "plan run",
        SurfaceStatus::Wired,
        "roko-cli orchestrate",
        "Full orchestration loop: DAG executor, agent dispatch, gates, persistence",
    ));
    v.push(SurfaceEntry::cli(
        "plan generate",
        SurfaceStatus::Wired,
        "roko-cli plan_generate",
        "Generates implementation plans from a prompt, file, or PRD",
    ));
    v.push(SurfaceEntry::cli(
        "plan regenerate",
        SurfaceStatus::Wired,
        "roko-cli plan_generate",
        "Regenerates existing plan from its source PRD",
    ));
    v.push(SurfaceEntry::cli(
        "prd idea",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Captures a quick idea as a work item",
    ));
    v.push(SurfaceEntry::cli(
        "prd list",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Lists all PRDs (published, drafts, ideas)",
    ));
    v.push(SurfaceEntry::cli(
        "prd status",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Coverage report across PRDs and plans",
    ));
    v.push(SurfaceEntry::cli(
        "prd draft new",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Creates new draft PRD (agent-assisted)",
    ));
    v.push(SurfaceEntry::cli(
        "prd draft edit",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Refines existing draft",
    ));
    v.push(SurfaceEntry::cli(
        "prd draft promote",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Promotes draft to published; --auto_execute triggers plan run",
    ));
    v.push(SurfaceEntry::cli(
        "prd draft list",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Lists all drafts",
    ));
    v.push(SurfaceEntry::cli(
        "prd plan",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Generates tasks.toml from PRD; triggers auto-plan subscriber",
    ));
    v.push(SurfaceEntry::cli(
        "prd consolidate",
        SurfaceStatus::Wired,
        "roko-cli prd",
        "Scans PRDs for duplicates, gaps, inconsistencies",
    ));
    v.push(SurfaceEntry::cli(
        "research topic",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Deep research with citations; --deep for Perplexity async",
    ));
    v.push(SurfaceEntry::cli(
        "research enhance-prd",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Enhances PRD with citations and research",
    ));
    v.push(SurfaceEntry::cli(
        "research enhance-plan",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Optimizes plan with research-backed decomposition",
    ));
    v.push(SurfaceEntry::cli(
        "research enhance-tasks",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Splits/optimizes tasks for efficiency and model selection",
    ));
    v.push(SurfaceEntry::cli(
        "research analyze",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Analyzes execution episodes for self-learning insights",
    ));
    v.push(SurfaceEntry::cli(
        "research list",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Lists all research artifacts",
    ));
    v.push(SurfaceEntry::cli(
        "research search",
        SurfaceStatus::Wired,
        "roko-cli research",
        "Direct web search via Perplexity; supports --domains and --recency",
    ));
    v.push(SurfaceEntry::cli(
        "chat",
        SurfaceStatus::Wired,
        "roko-cli chat, roko-serve",
        "Interactive REPL backed by roko-serve agent messaging",
    ));
    v.push(SurfaceEntry::cli(
        "neuro query",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Queries durable knowledge store by topic",
    ));
    v.push(SurfaceEntry::cli(
        "neuro stats",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Shows aggregate knowledge store statistics",
    ));
    v.push(SurfaceEntry::cli(
        "neuro gc",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Runs garbage collection on knowledge store",
    ));
    v.push(SurfaceEntry::cli(
        "neuro backup",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Backs up knowledge store; supports --top-n genomic bottleneck",
    ));
    v.push(SurfaceEntry::cli(
        "neuro restore",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Restores from backup with confidence decay",
    ));
    v.push(SurfaceEntry::cli(
        "neuro sync",
        SurfaceStatus::Wired,
        "roko-neuro",
        "Version-vector-based delta sync with peer agent",
    ));
    v.push(SurfaceEntry::cli(
        "subscription list/add/remove/enable/disable",
        SurfaceStatus::Wired,
        "roko-cli subscriptions",
        "Manages event subscriptions for agent template triggers",
    ));
    v.push(SurfaceEntry::cli(
        "event-sources list",
        SurfaceStatus::Wired,
        "roko-cli event_sources",
        "Lists configured cron schedules and file watchers",
    ));
    v.push(SurfaceEntry::cli(
        "provider list",
        SurfaceStatus::Wired,
        "roko-core config",
        "Lists configured providers and connection status",
    ));
    v.push(SurfaceEntry::cli(
        "provider health",
        SurfaceStatus::Wired,
        "roko-learn provider_health",
        "Shows persisted circuit-breaker health and latency",
    ));
    v.push(SurfaceEntry::cli(
        "provider test",
        SurfaceStatus::Wired,
        "roko-agent",
        "Sends minimal request to verify provider connectivity",
    ));
    v.push(SurfaceEntry::cli(
        "model list",
        SurfaceStatus::Wired,
        "roko-core config",
        "Lists configured models and capabilities",
    ));
    v.push(SurfaceEntry::cli(
        "model route",
        SurfaceStatus::Wired,
        "roko-learn cascade_router",
        "Shows routing decision with optional --explain trace",
    ));
    v.push(SurfaceEntry::cli(
        "experiment",
        SurfaceStatus::Wired,
        "roko-learn prompt_experiment",
        "Manages model experiments (list, create, conclude, etc.)",
    ));
    v.push(SurfaceEntry::cli(
        "deploy railway",
        SurfaceStatus::Partial,
        "roko-cli deployment",
        "Deploys workspace to Railway via GraphQL API; requires RAILWAY_TOKEN",
    ));
    v.push(SurfaceEntry::cli(
        "deploy fly",
        SurfaceStatus::Partial,
        "roko-cli deployment",
        "Generates fly.toml and deploys via Fly.io CLI; requires flyctl",
    ));
    v.push(SurfaceEntry::cli(
        "deploy docker",
        SurfaceStatus::Partial,
        "roko-cli deployment",
        "Builds Docker image and tags for registry",
    ));
    v.push(SurfaceEntry::cli(
        "update",
        SurfaceStatus::Partial,
        "self-update",
        "Self-update binary; --verify for Sigstore cosign. Download logic may be stubbed.",
    ));
    v.push(SurfaceEntry::cli(
        "completions",
        SurfaceStatus::Wired,
        "clap_complete",
        "Generates shell completions for bash/zsh/fish",
    ));
    v.push(SurfaceEntry::cli(
        "daemon start/stop/status/logs/reload/restart",
        SurfaceStatus::Wired,
        "roko-cli daemon",
        "Full daemon lifecycle management with launchd support",
    ));
    v.push(SurfaceEntry::cli(
        "daemon install/uninstall",
        SurfaceStatus::Wired,
        "roko-cli daemon",
        "macOS launchd plist generation and removal",
    ));
    v.push(SurfaceEntry::cli(
        "dashboard / watch",
        SurfaceStatus::Wired,
        "roko-cli tui",
        "Interactive ratatui TUI (F1-F9); --text for non-interactive fallback",
    ));
    v.push(SurfaceEntry::cli(
        "serve",
        SurfaceStatus::Wired,
        "roko-serve",
        "HTTP control plane with ~85 REST routes + SSE + WebSocket on configurable port",
    ));
    v.push(SurfaceEntry::cli(
        "worker",
        SurfaceStatus::Partial,
        "roko-cli worker",
        "Deployed worker mode; reads template from env, serves tasks",
    ));
    v.push(SurfaceEntry::cli(
        "index build",
        SurfaceStatus::Wired,
        "roko-index",
        "Builds code index for workspace",
    ));
    v.push(SurfaceEntry::cli(
        "index rebuild",
        SurfaceStatus::Wired,
        "roko-index",
        "Drops and rebuilds index from source files",
    ));
    v.push(SurfaceEntry::cli(
        "index search",
        SurfaceStatus::Wired,
        "roko-index",
        "Searches code index; supports --kind, --strategy, --limit",
    ));
    v.push(SurfaceEntry::cli(
        "index stats",
        SurfaceStatus::Wired,
        "roko-index",
        "Shows index statistics",
    ));
    v.push(SurfaceEntry::cli(
        "tune",
        SurfaceStatus::Wired,
        "roko-learn, roko-gate",
        "Tunes adaptive gate thresholds, routing, budget parameters",
    ));
    v.push(SurfaceEntry::cli(
        "learn / ask",
        SurfaceStatus::Wired,
        "roko-learn",
        "Shows learning state: cascade router, experiments, efficiency, episodes",
    ));
    v.push(SurfaceEntry::cli(
        "explain",
        SurfaceStatus::Wired,
        "roko-cli explain",
        "Progressive disclosure (3 depth levels) for roko concepts",
    ));
    v.push(SurfaceEntry::cli(
        "new",
        SurfaceStatus::Wired,
        "roko-cli scaffold",
        "Generates boilerplate for gates, scorers, routers, policies, etc.",
    ));
    v.push(SurfaceEntry::cli(
        "plugin list",
        SurfaceStatus::Partial,
        "roko-cli plugin",
        "Lists plugins; plugin system framework exists but registry is not live",
    ));
    v.push(SurfaceEntry::cli(
        "plugin install/remove/audit",
        SurfaceStatus::Partial,
        "roko-cli plugin",
        "Plugin lifecycle commands exist; no live registry backend",
    ));
    v.push(SurfaceEntry::cli(
        "archive",
        SurfaceStatus::Wired,
        "roko-fs",
        "Moves old engrams to cold storage; supports --older-than, --batch-size, --dry-run",
    ));

    // ── TUI tabs ────────────────────────────────────────────────────────
    v.push(SurfaceEntry::tab(
        "F1 Dashboard",
        SurfaceStatus::Wired,
        "StateHub push-based",
        "Master-detail layout: plan tree, agents, output, diff, gate, git, MCP, learning, processes",
    ));
    v.push(SurfaceEntry::tab(
        "F2 Plans",
        SurfaceStatus::Wired,
        "StateHub plan_summaries",
        "Wave browser + plan detail with tasks, gate results, timing",
    ));
    v.push(SurfaceEntry::tab(
        "F3 Agents",
        SurfaceStatus::Wired,
        "StateHub agent_summaries",
        "Agent roster, output stream, token burn, gate results, role tabs",
    ));
    v.push(SurfaceEntry::tab(
        "F4 Git",
        SurfaceStatus::Wired,
        "git subprocess calls",
        "Real git data: branches, worktrees, commits, status. Populated by background refresh via collect_git_data()",
    ));
    v.push(SurfaceEntry::tab(
        "F5 Logs",
        SurfaceStatus::Wired,
        "StateHub + JSONL files",
        "Multi-source unified log: signals, episodes, efficiency events, gate results. Level filtering.",
    ));
    v.push(SurfaceEntry::tab(
        "F6 Config",
        SurfaceStatus::Wired,
        "roko.toml + StateHub",
        "Interactive config editor with inline value editing, save button. Runtime sections (efficiency, cascade router, gates, experiments) read-only.",
    ));
    v.push(SurfaceEntry::tab(
        "F7 Inspect",
        SurfaceStatus::Wired,
        "StateHub + efficiency events",
        "System health, token burn by role, cost by model, cascade router, conductor alerts, gate thresholds",
    ));
    v.push(SurfaceEntry::tab(
        "F8 Marketplace",
        SurfaceStatus::Partial,
        ".roko/jobs/*.json",
        "Job list and detail panels fully wired from local job files. CreateJob sub-view is a stub with placeholder form.",
    ));
    v.push(SurfaceEntry::tab(
        "F9 Atelier",
        SurfaceStatus::Wired,
        "StateHub atelier_prds",
        "PRD list, plan detail with task list. Stats bar. Read-only -- inline PRD editing not wired.",
    ));

    // ── TUI sub-views ───────────────────────────────────────────────────
    // F1 Dashboard sub-views
    v.push(SurfaceEntry::subview(
        "DashboardHealth",
        SurfaceStatus::Wired,
        "StateHub",
        "Health gauges, plan progress, budget, headline metrics",
    ));
    v.push(SurfaceEntry::subview(
        "MeshStatus",
        SurfaceStatus::Wired,
        "StateHub agent_summaries",
        "Agent mesh / collective status overview",
    ));
    v.push(SurfaceEntry::subview(
        "CostOverview",
        SurfaceStatus::Wired,
        "StateHub efficiency_summary",
        "Cost and budget overview from efficiency events",
    ));

    // F2 Plans sub-views
    v.push(SurfaceEntry::subview(
        "PlanDagView",
        SurfaceStatus::Wired,
        "StateHub plan_summaries",
        "Plan DAG visualization with wave groups and progress bars",
    ));
    v.push(SurfaceEntry::subview(
        "TaskDetail",
        SurfaceStatus::Wired,
        "StateHub plan_summaries",
        "Selected task detail with gate results and timing",
    ));
    v.push(SurfaceEntry::subview(
        "WaveProgress",
        SurfaceStatus::Wired,
        "StateHub plan_summaries",
        "Wave progress overview",
    ));

    // F3 Agents sub-views
    v.push(SurfaceEntry::subview(
        "AgentOutputStream",
        SurfaceStatus::Wired,
        "StateHub agent_summaries",
        "Live output stream from selected agent with ANSI color parsing",
    ));
    v.push(SurfaceEntry::subview(
        "AgentGateResults",
        SurfaceStatus::Wired,
        "StateHub gate_results_page",
        "Gate results for the selected agent",
    ));
    v.push(SurfaceEntry::subview(
        "AgentTokenBurn",
        SurfaceStatus::Wired,
        "StateHub efficiency_events",
        "Token burn / cost metrics per agent with sparklines",
    ));

    // F4 Git sub-views
    v.push(SurfaceEntry::subview(
        "BranchTree",
        SurfaceStatus::Wired,
        "git subprocess (for-each-ref)",
        "Hierarchical branch listing with ahead/behind counts; populated by background refresh",
    ));
    v.push(SurfaceEntry::subview(
        "CommitGraph",
        SurfaceStatus::Wired,
        "git subprocess (log --graph)",
        "Rendered git log with graph characters; scrollable",
    ));
    v.push(SurfaceEntry::subview(
        "WorktreeList",
        SurfaceStatus::Wired,
        "git subprocess (worktree list)",
        "Table with path, branch, status; parsed from porcelain output",
    ));

    // F5 Logs sub-views
    v.push(SurfaceEntry::subview(
        "FilteredLog",
        SurfaceStatus::Wired,
        "StateHub + JSONL",
        "Default log view with level-based coloring and filtering",
    ));
    v.push(SurfaceEntry::subview(
        "SignalStream",
        SurfaceStatus::Wired,
        "StateHub recent_signals",
        "Signal stream viewer from signal data",
    ));

    // F6 Config sub-views
    v.push(SurfaceEntry::subview(
        "ConfigEditor",
        SurfaceStatus::Wired,
        "roko.toml + config_meta",
        "Scrollable editable field list grouped by section with inline editing and save button",
    ));
    v.push(SurfaceEntry::subview(
        "ProviderHealth",
        SurfaceStatus::Wired,
        "StateHub cascade_router",
        "Provider table with status, model, trials, rate. Derived from cascade router confidence stats.",
    ));
    v.push(SurfaceEntry::subview(
        "ModelComparison",
        SurfaceStatus::Wired,
        "StateHub efficiency_events + cascade_router",
        "Per-model cost, tier, gate%, uses, avg time, token I/O. Best-column highlighting.",
    ));

    // F7 Inspect sub-views
    v.push(SurfaceEntry::subview(
        "EngramDag",
        SurfaceStatus::Wired,
        "StateHub recent_signals",
        "Indented ASCII tree of signal lineage. Scrollable, color-coded by kind.",
    ));
    v.push(SurfaceEntry::subview(
        "EpisodeReplay",
        SurfaceStatus::Wired,
        "StateHub episodes_cache",
        "Episode table with gate icon, task, role, model, turns, cost, time. Summary bar.",
    ));
    v.push(SurfaceEntry::subview(
        "KnowledgeBrowse",
        SurfaceStatus::Wired,
        "StateHub knowledge_entries",
        "Knowledge table with topic, confidence bar, source, preview. Reads from neuro store.",
    ));

    // F8 Marketplace sub-views
    v.push(SurfaceEntry::subview(
        "JobList",
        SurfaceStatus::Wired,
        ".roko/jobs/*.json",
        "Job listing with status icons, type tags, priority. Keyboard navigation.",
    ));
    v.push(SurfaceEntry::subview(
        "JobDetail",
        SurfaceStatus::Wired,
        ".roko/jobs/*.json",
        "Full job detail: metadata table + word-wrapped description",
    ));
    v.push(SurfaceEntry::subview(
        "CreateJob",
        SurfaceStatus::Stub,
        "not wired",
        "Placeholder form only. Text says 'backend submission is not wired yet.'",
    ));

    // F9 Atelier sub-views
    v.push(SurfaceEntry::subview(
        "PrdWorkshop",
        SurfaceStatus::Wired,
        "StateHub atelier_prds",
        "PRD list + plan detail side-by-side. Read-only browsing.",
    ));
    v.push(SurfaceEntry::subview(
        "PlanExplorer",
        SurfaceStatus::Wired,
        "StateHub atelier_prds + atelier_tasks_by_slug",
        "Full-width plan detail with task table for selected PRD",
    ));

    // ── Modals ──────────────────────────────────────────────────────────
    v.push(SurfaceEntry::modal(
        "ApprovalDialog",
        SurfaceStatus::Wired,
        "ApprovalChannel",
        "Approval overlay during plan run --approval. Connected to orchestrator via mpsc channel.",
    ));
    v.push(SurfaceEntry::modal(
        "PRD inline editor",
        SurfaceStatus::Missing,
        "n/a",
        "No inline PRD editing in TUI. Must use CLI: roko prd draft edit <slug>",
    ));
    v.push(SurfaceEntry::modal(
        "Job submission",
        SurfaceStatus::Missing,
        "roko-serve jobs API",
        "CreateJob form exists as stub but cannot submit. Needs roko-serve wiring.",
    ));

    v
}

// ---------------------------------------------------------------------------
// TUI parity inventory (TUI-PARITY-01)
// ---------------------------------------------------------------------------

/// How a TUI view obtains its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceKind {
    /// Reads from StateHub push updates (via `DashboardSnapshot`).
    PushBased,
    /// Reads from filesystem directly (JSONL, JSON, git subprocess).
    FileBased,
    /// Reads from HTTP API (e.g. agent topology fetch).
    ApiBased,
    /// Mix of sources (e.g. push + file fallback).
    Mixed,
    /// Placeholder data or not yet wired.
    Stub,
}

impl std::fmt::Display for DataSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PushBased => write!(f, "push"),
            Self::FileBased => write!(f, "file"),
            Self::ApiBased => write!(f, "api"),
            Self::Mixed => write!(f, "mixed"),
            Self::Stub => write!(f, "stub"),
        }
    }
}

/// Parity status between TUI tab and its dashboard/CLI equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiParityStatus {
    /// Dashboard and TUI show equivalent data.
    Equivalent,
    /// TUI has less detail than the dashboard or CLI equivalent.
    TuiLimited,
    /// TUI has more detail (e.g. keyboard navigation, live output).
    TuiEnhanced,
    /// No dashboard equivalent exists; TUI-only surface.
    TuiOnly,
    /// Dashboard equivalent exists but TUI view does not.
    DashboardOnly,
}

impl std::fmt::Display for TuiParityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Equivalent => write!(f, "equivalent"),
            Self::TuiLimited => write!(f, "tui-limited"),
            Self::TuiEnhanced => write!(f, "tui-enhanced"),
            Self::TuiOnly => write!(f, "tui-only"),
            Self::DashboardOnly => write!(f, "dashboard-only"),
        }
    }
}

/// TUI parity detail for a single tab or subview.
#[derive(Debug, Clone, Serialize)]
pub struct TuiParityDetail {
    /// Tab name (e.g. "F1 Dashboard").
    pub tab: String,
    /// Sub-view name if applicable (e.g. "BranchTree").
    pub subview: Option<String>,
    /// Equivalent route or page in the HTTP dashboard, if any.
    pub dashboard_equivalent: Option<String>,
    /// Equivalent CLI command, if any.
    pub cli_equivalent: Option<String>,
    /// How the view obtains its data at render time.
    pub data_source: DataSourceKind,
    /// Parity status relative to the dashboard/CLI equivalent.
    pub parity_status: TuiParityStatus,
    /// Free-form notes about refresh semantics and data gaps.
    pub notes: String,
}

/// Produce the complete TUI parity inventory.
///
/// Each entry maps a TUI tab or sub-view to its dashboard/CLI equivalent,
/// documents the data source kind, and records parity status. This is
/// derived from a source-level audit of every view renderer, the
/// `DashboardData` file-polling path, the `TuiState` push-based path,
/// and the `apply_dashboard_snapshot` bridge in `app.rs`.
#[must_use]
pub fn tui_parity_inventory() -> Vec<TuiParityDetail> {
    let mut v = Vec::with_capacity(40);

    // ── F1 Dashboard ────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F1 Dashboard".into(),
        subview: None,
        dashboard_equivalent: Some("/api/dashboard (roko-serve)".into()),
        cli_equivalent: Some("roko status".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Left panel (plan tree, phase, task progress) reads TuiState (push). \
                Right panel sub-tabs are all TuiState-driven except the token sparkline \
                in the bottom ribbon which still reads DashboardData.efficiency_events \
                (file-polled). All other sub-tabs prefix DashboardData param with _ (unused)."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F1 Dashboard".into(),
        subview: Some("DashboardHealth".into()),
        dashboard_equivalent: Some("/api/dashboard (health section)".into()),
        cli_equivalent: Some("roko status".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Health gauges, plan progress, budget from TuiState fields. \
                Interactive plan tree with expand/collapse is TUI-only."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F1 Dashboard".into(),
        subview: Some("MeshStatus".into()),
        dashboard_equivalent: Some("/api/agents/topology".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "Agent roster from TuiState (push). Topology overlay fetched via \
                HTTP GET /api/agents/topology (API-based, one-shot fetch in app.rs). \
                No CLI equivalent."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F1 Dashboard".into(),
        subview: Some("CostOverview".into()),
        dashboard_equivalent: Some("/api/efficiency".into()),
        cli_equivalent: Some("roko status (cost section)".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Token sparkline reads DashboardData.efficiency_events (file-polled \
                via build_efficiency_snapshot). Token counters in TuiState are push-based. \
                TUI-PARITY-02: sparkline should migrate to TuiState.efficiency_events."
            .into(),
    });

    // ── F2 Plans ────────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F2 Plans".into(),
        subview: None,
        dashboard_equivalent: Some("/api/plans".into()),
        cli_equivalent: Some("roko plan list / roko plan show".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "All plan data comes from TuiState (push via DashboardSnapshot). \
                DashboardData param is passed but prefixed _ (unused in render). \
                Wave browser + task detail + timing are TUI-only features."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F2 Plans".into(),
        subview: Some("PlanDagView".into()),
        dashboard_equivalent: None,
        cli_equivalent: Some("roko plan show <id>".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "Wave-grouped DAG with gradient progress bars. TUI-only visualization.".into(),
    });
    v.push(TuiParityDetail {
        tab: "F2 Plans".into(),
        subview: Some("TaskDetail".into()),
        dashboard_equivalent: Some("/api/plans/:id/tasks".into()),
        cli_equivalent: Some("roko plan show <id>".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Task detail with gate results and timing from TuiState.".into(),
    });
    v.push(TuiParityDetail {
        tab: "F2 Plans".into(),
        subview: Some("WaveProgress".into()),
        dashboard_equivalent: None,
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "Wave progress overview. TUI-only visualization.".into(),
    });

    // ── F3 Agents ───────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F3 Agents".into(),
        subview: None,
        dashboard_equivalent: Some("/api/agents".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Agent roster and output from TuiState (push). DashboardData param \
                passed through but unused in roster/output renderers. Token sparkline \
                in left panel reads DashboardData.efficiency_events (file-polled). \
                `agent list` CLI command is missing."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F3 Agents".into(),
        subview: Some("AgentOutputStream".into()),
        dashboard_equivalent: Some("/api/agents/:id/stream (WebSocket)".into()),
        cli_equivalent: Some("roko chat --agent <id>".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Live output via WebSocket agent stream client in app.rs. \
                Falls back to TuiState output cache. ANSI color parsing is TUI-only."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F3 Agents".into(),
        subview: Some("AgentGateResults".into()),
        dashboard_equivalent: Some("/api/gates".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Gate results from TuiState.gate_result_summaries (push).".into(),
    });
    v.push(TuiParityDetail {
        tab: "F3 Agents".into(),
        subview: Some("AgentTokenBurn".into()),
        dashboard_equivalent: Some("/api/efficiency".into()),
        cli_equivalent: Some("roko status (cost section)".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Token burn sparklines per agent. Uses DashboardData for \
                build_efficiency_snapshot (file-polled). TuiState holds \
                cumulative counters (push). TUI-PARITY-02: sparkline source \
                should migrate to TuiState.efficiency_events."
            .into(),
    });

    // ── F4 Git ──────────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F4 Git".into(),
        subview: None,
        dashboard_equivalent: None,
        cli_equivalent: None,
        data_source: DataSourceKind::FileBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "Git data collected by background thread (collect_git_data) running \
                git subprocesses. Stored in TuiState.git_view_data. Refreshed on \
                debounced git watcher events. No file-polling on render path -- \
                data is pre-populated. No dashboard or CLI equivalent."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F4 Git".into(),
        subview: Some("BranchTree".into()),
        dashboard_equivalent: None,
        cli_equivalent: None,
        data_source: DataSourceKind::FileBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "git for-each-ref parsed into hierarchical branch tree. \
                Background refresh, zero I/O on render path."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F4 Git".into(),
        subview: Some("CommitGraph".into()),
        dashboard_equivalent: None,
        cli_equivalent: None,
        data_source: DataSourceKind::FileBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "git log --graph parsed into scrollable commit list. \
                Background refresh."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F4 Git".into(),
        subview: Some("WorktreeList".into()),
        dashboard_equivalent: None,
        cli_equivalent: None,
        data_source: DataSourceKind::FileBased,
        parity_status: TuiParityStatus::TuiOnly,
        notes: "git worktree list porcelain output parsed. Background refresh.".into(),
    });

    // ── F5 Logs ─────────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F5 Logs".into(),
        subview: None,
        dashboard_equivalent: Some("/api/events (SSE)".into()),
        cli_equivalent: Some("roko status / roko replay".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Unified log built from TuiState fields: recent_signals, \
                episodes_cache, efficiency_events, gate_result_summaries, \
                event_log. All push-based. Level filtering and tail mode \
                are TUI-only."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F5 Logs".into(),
        subview: Some("FilteredLog".into()),
        dashboard_equivalent: Some("/api/events (SSE)".into()),
        cli_equivalent: Some("roko status".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Level-based colored log with filter toggles. Reads TuiState only.".into(),
    });
    v.push(TuiParityDetail {
        tab: "F5 Logs".into(),
        subview: Some("SignalStream".into()),
        dashboard_equivalent: Some("/api/signals".into()),
        cli_equivalent: Some("roko replay".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Signal stream from TuiState.recent_signals (push).".into(),
    });

    // ── F6 Config ───────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F6 Config".into(),
        subview: None,
        dashboard_equivalent: Some("/api/config".into()),
        cli_equivalent: Some("roko config show".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Config editor reads roko.toml from filesystem via build_flat_items \
                on every render frame (file-based). Runtime sections (efficiency, \
                cascade router, gates, experiments) read from TuiState (push). \
                TUI-PARITY-02: config_view.build_flat_items does fs::read_to_string \
                on roko.toml on every render call -- should cache in TuiState."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F6 Config".into(),
        subview: Some("ConfigEditor".into()),
        dashboard_equivalent: Some("/api/config".into()),
        cli_equivalent: Some("roko config show / config set".into()),
        data_source: DataSourceKind::Mixed,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Inline editing with save button. File-based read of roko.toml \
                per render + TuiState push for runtime sections. Interactive \
                editing is TUI-only."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F6 Config".into(),
        subview: Some("ProviderHealth".into()),
        dashboard_equivalent: Some("/api/providers/health".into()),
        cli_equivalent: Some("roko provider health".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Provider health table derived from TuiState.cascade_router \
                confidence stats (push via DashboardSnapshot)."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F6 Config".into(),
        subview: Some("ModelComparison".into()),
        dashboard_equivalent: Some("/api/models".into()),
        cli_equivalent: Some("roko model list".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Per-model cost, tier, gate%, uses, avg time, token I/O. \
                Derived from TuiState.efficiency_events + cascade_router (push). \
                Best-column highlighting is TUI-only."
            .into(),
    });

    // ── F7 Inspect ──────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F7 Inspect".into(),
        subview: None,
        dashboard_equivalent: Some("/api/efficiency".into()),
        cli_equivalent: Some("roko learn / roko status".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "System health overview, token burn by role, cost by model, \
                cascade router, conductor alerts, gate thresholds. All from TuiState."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F7 Inspect".into(),
        subview: Some("EngramDag".into()),
        dashboard_equivalent: None,
        cli_equivalent: Some("roko replay".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Indented ASCII tree of signal lineage from TuiState.recent_signals. \
                Scrollable, color-coded by kind. Interactive navigation is TUI-only."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F7 Inspect".into(),
        subview: Some("EpisodeReplay".into()),
        dashboard_equivalent: Some("/api/episodes".into()),
        cli_equivalent: Some("roko learn (episodes section)".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Episode table from TuiState.episodes_cache (push).".into(),
    });
    v.push(TuiParityDetail {
        tab: "F7 Inspect".into(),
        subview: Some("KnowledgeBrowse".into()),
        dashboard_equivalent: Some("/api/neuro/query".into()),
        cli_equivalent: Some("roko neuro query".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiEnhanced,
        notes: "Knowledge table from TuiState.knowledge_entries (push). \
                Interactive browse with confidence bars is TUI-only."
            .into(),
    });

    // ── F8 Marketplace ──────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F8 Marketplace".into(),
        subview: None,
        dashboard_equivalent: Some("/api/jobs".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiLimited,
        notes: "Job list from TuiState.marketplace_jobs (push via DashboardSnapshot). \
                CreateJob sub-view is a stub. No CLI equivalent for marketplace."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F8 Marketplace".into(),
        subview: Some("JobList".into()),
        dashboard_equivalent: Some("/api/jobs".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Job listing with status icons, type tags, priority. \
                From TuiState.marketplace_jobs."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F8 Marketplace".into(),
        subview: Some("JobDetail".into()),
        dashboard_equivalent: Some("/api/jobs/:id".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Full job detail from TuiState.marketplace_jobs[selected].".into(),
    });
    v.push(TuiParityDetail {
        tab: "F8 Marketplace".into(),
        subview: Some("CreateJob".into()),
        dashboard_equivalent: Some("POST /api/jobs".into()),
        cli_equivalent: None,
        data_source: DataSourceKind::Stub,
        parity_status: TuiParityStatus::TuiLimited,
        notes: "Placeholder form. Backend submission is not wired.".into(),
    });

    // ── F9 Atelier ──────────────────────────────────────────────────────
    v.push(TuiParityDetail {
        tab: "F9 Atelier".into(),
        subview: None,
        dashboard_equivalent: Some("/api/prds".into()),
        cli_equivalent: Some("roko prd list / roko prd status".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiLimited,
        notes: "PRD list and plan detail from TuiState.atelier_prds (push). \
                Read-only -- inline PRD editing not wired. CLI has full \
                draft/edit/promote lifecycle."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F9 Atelier".into(),
        subview: Some("PrdWorkshop".into()),
        dashboard_equivalent: Some("/api/prds".into()),
        cli_equivalent: Some("roko prd list".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::TuiLimited,
        notes: "PRD list with status badges. Read-only browse. CLI supports \
                full PRD lifecycle (idea/draft/publish/plan)."
            .into(),
    });
    v.push(TuiParityDetail {
        tab: "F9 Atelier".into(),
        subview: Some("PlanExplorer".into()),
        dashboard_equivalent: Some("/api/plans".into()),
        cli_equivalent: Some("roko plan show".into()),
        data_source: DataSourceKind::PushBased,
        parity_status: TuiParityStatus::Equivalent,
        notes: "Plan detail with task table from TuiState.atelier_tasks_by_slug (push).".into(),
    });

    v
}

/// Print the TUI parity inventory as a formatted table.
pub fn print_parity_table(entries: &[TuiParityDetail], json_mode: bool) {
    if json_mode {
        if let Ok(json) = serde_json::to_string_pretty(entries) {
            println!("{json}");
        }
        return;
    }

    println!(
        "{:<16} {:<18} {:<10} {:<14} {:<30} {}",
        "TAB", "SUBVIEW", "SOURCE", "PARITY", "DASHBOARD", "NOTES"
    );
    println!("{}", "-".repeat(130));

    for entry in entries {
        let subview = entry.subview.as_deref().unwrap_or("-");
        let dashboard = entry.dashboard_equivalent.as_deref().unwrap_or("(none)");
        let notes = if entry.notes.len() > 50 {
            format!("{}...", &entry.notes[..47])
        } else {
            entry.notes.clone()
        };
        println!(
            "{:<16} {:<18} {:<10} {:<14} {:<30} {}",
            entry.tab, subview, entry.data_source, entry.parity_status, dashboard, notes,
        );
    }

    // Summary
    let push_count = entries
        .iter()
        .filter(|e| e.data_source == DataSourceKind::PushBased)
        .count();
    let mixed_count = entries
        .iter()
        .filter(|e| e.data_source == DataSourceKind::Mixed)
        .count();
    let file_count = entries
        .iter()
        .filter(|e| e.data_source == DataSourceKind::FileBased)
        .count();
    let stub_count = entries
        .iter()
        .filter(|e| e.data_source == DataSourceKind::Stub)
        .count();
    println!();
    println!(
        "Total: {}  Push: {}  Mixed: {}  File: {}  Stub: {}",
        entries.len(),
        push_count,
        mixed_count,
        file_count,
        stub_count,
    );
}

// ---------------------------------------------------------------------------
// TUI refresh correction audit (TUI-PARITY-02)
// ---------------------------------------------------------------------------

/// Identified file-read on the render path that should migrate to push-based.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshCorrection {
    /// View or module where the file read occurs.
    pub location: String,
    /// What is being read.
    pub reads: String,
    /// Current data source.
    pub current_source: DataSourceKind,
    /// Recommended data source.
    pub recommended_source: DataSourceKind,
    /// Migration status.
    pub status: RefreshCorrectionStatus,
    /// Detailed notes on the correction.
    pub notes: String,
}

/// Status of a refresh correction migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshCorrectionStatus {
    /// Already migrated or not needed.
    Done,
    /// Identified but migration deferred (requires upstream change).
    Deferred,
    /// Low priority -- current approach is acceptable.
    Acceptable,
}

impl std::fmt::Display for RefreshCorrectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => write!(f, "done"),
            Self::Deferred => write!(f, "deferred"),
            Self::Acceptable => write!(f, "acceptable"),
        }
    }
}

/// Produce the refresh correction audit.
///
/// Documents every identified case where a TUI view reads from the
/// filesystem or `DashboardData` when it could instead read from
/// `TuiState` (push-based via `DashboardSnapshot`).
#[must_use]
pub fn refresh_correction_audit() -> Vec<RefreshCorrection> {
    vec![
        // ── Token sparkline (DashboardData -> should use TuiState) ──
        RefreshCorrection {
            location: "widgets/token_sparkline.rs".into(),
            reads: "DashboardData.efficiency_events, DashboardData.efficiency".into(),
            current_source: DataSourceKind::FileBased,
            recommended_source: DataSourceKind::PushBased,
            status: RefreshCorrectionStatus::Deferred,
            notes: "build_efficiency_snapshot(data) reads file-polled \
                    DashboardData.efficiency_events to build the sparkline series. \
                    TuiState already has efficiency_events and efficiency_summary \
                    fields populated by update_from_dashboard_snapshot. Migration \
                    requires changing build_efficiency_snapshot to accept TuiState \
                    instead of DashboardData, which touches the legacy pages/ module."
                .into(),
        },
        // ── Config editor (fs::read_to_string on render path) ──
        RefreshCorrection {
            location: "tui/config_meta.rs::build_flat_items".into(),
            reads: "roko.toml via fs::read_to_string".into(),
            current_source: DataSourceKind::FileBased,
            recommended_source: DataSourceKind::PushBased,
            status: RefreshCorrectionStatus::Acceptable,
            notes: "build_flat_items reads roko.toml from disk on every render \
                    frame to build the config editor field list. This is the only \
                    direct filesystem read in a view renderer. Acceptable because: \
                    (a) roko.toml rarely changes, (b) the read is fast (small file), \
                    (c) the config editor needs the actual on-disk state to show \
                    pending vs saved differences. Could be optimized with a file \
                    watcher stamp cache."
                .into(),
        },
        // ── Git data (subprocess -> background thread, already correct) ──
        RefreshCorrection {
            location: "views/git_view.rs + app.rs::collect_git_bg_data".into(),
            reads: "git subprocess calls (for-each-ref, log, worktree list, status)".into(),
            current_source: DataSourceKind::FileBased,
            recommended_source: DataSourceKind::FileBased,
            status: RefreshCorrectionStatus::Done,
            notes: "Git data is already correctly handled: collected by a background \
                    thread on debounced watcher events, stored in TuiState.git_view_data. \
                    The render path does zero I/O -- it reads the pre-populated field. \
                    No migration needed."
                .into(),
        },
        // ── DashboardData.tick() standalone fallback ──
        RefreshCorrection {
            location: "tui/dashboard.rs::DashboardData::tick".into(),
            reads: "executor.json, efficiency.jsonl, experiments.json, \
                    gate-thresholds.json, cascade-router.json, engrams.jsonl, \
                    episodes.jsonl, events.json, task-outputs/"
                .into(),
            current_source: DataSourceKind::FileBased,
            recommended_source: DataSourceKind::PushBased,
            status: RefreshCorrectionStatus::Acceptable,
            notes: "tick() is already deprecated and only called when \
                    snapshot_rx is None (standalone `roko dashboard` mode with \
                    no orchestrator attached). In connected mode, the TUI is \
                    fully push-based via DashboardSnapshot. The file-polling \
                    path is a correct fallback for standalone operation."
                .into(),
        },
        // ── DashboardData fields duplicating TuiState ──
        RefreshCorrection {
            location: "tui/dashboard.rs::DashboardData struct".into(),
            reads: "plans, active_tasks, agents, gate_results, efficiency, \
                    efficiency_events, cascade_router, experiments, recent_signals, \
                    conductor_alerts, cfactor, event_log"
                .into(),
            current_source: DataSourceKind::FileBased,
            recommended_source: DataSourceKind::PushBased,
            status: RefreshCorrectionStatus::Done,
            notes: "DashboardData and TuiState have overlapping fields. The \
                    bridge is update_from_snapshot (DashboardData -> TuiState) \
                    for standalone mode and update_from_dashboard_snapshot \
                    (DashboardSnapshot -> TuiState) for connected mode. Views \
                    already read from TuiState, not DashboardData -- confirmed \
                    by source audit: plans_view, agents_view, logs_view, \
                    context_view, marketplace_view, atelier_view all prefix the \
                    DashboardData param with _ (unused). The duplication is only \
                    for the standalone fallback path."
                .into(),
        },
        // ── Views that pass DashboardData but don't read it ──
        RefreshCorrection {
            location: "views/plans_view.rs, agents_view.rs, logs_view.rs, \
                      context_view.rs, marketplace_view.rs, atelier_view.rs"
                .into(),
            reads: "DashboardData (parameter passed but unused)".into(),
            current_source: DataSourceKind::PushBased,
            recommended_source: DataSourceKind::PushBased,
            status: RefreshCorrectionStatus::Done,
            notes: "All six views accept &DashboardData but prefix it with _ \
                    (unused). They read exclusively from &TuiState. The parameter \
                    is kept for signature compatibility with render_tab_content \
                    dispatch but could be removed in a future cleanup pass."
                .into(),
        },
    ]
}

/// Print the refresh correction audit as a formatted table.
pub fn print_correction_table(entries: &[RefreshCorrection], json_mode: bool) {
    if json_mode {
        if let Ok(json) = serde_json::to_string_pretty(entries) {
            println!("{json}");
        }
        return;
    }

    println!(
        "{:<10} {:<50} {:<10} {:<10} {}",
        "STATUS", "LOCATION", "CURRENT", "TARGET", "READS"
    );
    println!("{}", "-".repeat(120));

    for entry in entries {
        let reads = if entry.reads.len() > 40 {
            format!("{}...", &entry.reads[..37])
        } else {
            entry.reads.clone()
        };
        println!(
            "{:<10} {:<50} {:<10} {:<10} {}",
            entry.status, entry.location, entry.current_source, entry.recommended_source, reads,
        );
    }
}

// ---------------------------------------------------------------------------
// Summary helpers
// ---------------------------------------------------------------------------

/// Summary counts for a filtered set of surface entries.
pub struct InventorySummary {
    pub total: usize,
    pub wired: usize,
    pub partial: usize,
    pub stub: usize,
    pub missing: usize,
}

/// Compute summary counts for the given entries.
#[must_use]
pub fn summarize(entries: &[SurfaceEntry]) -> InventorySummary {
    let mut s = InventorySummary {
        total: entries.len(),
        wired: 0,
        partial: 0,
        stub: 0,
        missing: 0,
    };
    for e in entries {
        match e.status {
            SurfaceStatus::Wired => s.wired += 1,
            SurfaceStatus::Partial => s.partial += 1,
            SurfaceStatus::Stub => s.stub += 1,
            SurfaceStatus::Missing => s.missing += 1,
        }
    }
    s
}

/// Print the inventory as a formatted table to stdout.
pub fn print_table(entries: &[SurfaceEntry], json_mode: bool) {
    if json_mode {
        if let Ok(json) = serde_json::to_string_pretty(entries) {
            println!("{json}");
        }
        return;
    }

    // Header
    println!(
        "{:<8} {:<8} {:<40} {:<28} {}",
        "KIND", "STATUS", "NAME", "BACKEND", "NOTES"
    );
    println!("{}", "-".repeat(120));

    for entry in entries {
        let status_str = match entry.status {
            SurfaceStatus::Wired => "wired",
            SurfaceStatus::Partial => "PARTIAL",
            SurfaceStatus::Stub => "STUB",
            SurfaceStatus::Missing => "MISSING",
        };
        let notes = if entry.notes.len() > 60 {
            format!("{}...", &entry.notes[..57])
        } else {
            entry.notes.clone()
        };
        println!(
            "{:<8} {:<8} {:<40} {:<28} {}",
            entry.kind, status_str, entry.name, entry.backend_dependency, notes,
        );
    }

    // Summary
    let summary = summarize(entries);
    println!();
    println!(
        "Total: {}  Wired: {}  Partial: {}  Stub: {}  Missing: {}",
        summary.total, summary.wired, summary.partial, summary.stub, summary.missing,
    );
}

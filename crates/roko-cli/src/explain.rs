//! Progressive help system for `roko explain <topic>`.
//!
//! Each topic has three disclosure levels:
//! - Level 1: one-paragraph summary (always shown)
//! - Level 2: how it works in roko (shown with `--depth 2`)
//! - Level 3: internals and advanced details (shown with `--depth 3`)

/// A single topic entry with three-level progressive disclosure.
#[derive(Debug, Clone)]
pub struct TopicEntry {
    /// Machine name used on the command line.
    pub name: &'static str,
    /// Human-readable title.
    pub title: &'static str,
    /// Level 1: brief summary (2-3 sentences).
    pub summary: &'static str,
    /// Level 2: how it works in roko.
    pub detail: &'static str,
    /// Level 3: internals and configuration.
    pub internals: &'static str,
}

/// All registered topics.
pub static TOPICS: &[TopicEntry] = &[
    TopicEntry {
        name: "gates",
        title: "Gate Pipeline",
        summary: "Gates validate agent output before it is accepted. Each gate is a \
                  pass/fail check (compile, test, clippy, diff-review) that ensures \
                  code quality. Tasks must pass all configured gates to advance.",
        detail: "Roko runs a 7-rung gate pipeline after every agent turn. The rungs \
                 are: syntax, compile, test, clippy, diff, review, and integration. \
                 Each rung has an adaptive threshold that tunes itself based on \
                 historical pass rates (EMA). Gate results are recorded in \
                 `.roko/episodes.jsonl` and feed back into the learning subsystem. \
                 Configure gates in `roko.toml` under `[[gates]]`.",
        internals: "Gate implementations live in `crates/roko-gate/src/`. The \
                    `GatePipeline` struct runs gates sequentially or in parallel. \
                    Adaptive thresholds persist at `.roko/learn/gate-thresholds.json` \
                    and use exponential moving averages (alpha=0.1 by default). The \
                    `HotellingGate` uses Hotelling's T-squared statistic for \
                    multivariate anomaly detection. Gate verdicts emit \
                    `DashboardEvent::GateVerdict` for real-time TUI updates.",
    },
    TopicEntry {
        name: "routing",
        title: "Cascade Router (Model Routing)",
        summary: "The cascade router selects which LLM model handles each task. It \
                  starts with the cheapest model and escalates to more capable (and \
                  expensive) models only when gates fail.",
        detail: "The `CascadeRouter` maintains a bandit-style policy that maps task \
                 complexity tiers to model choices. When a task fails a gate at a \
                 given model tier, the router escalates to the next tier. Over time, \
                 it learns which models are cost-effective for which task types. State \
                 persists at `.roko/learn/cascade-router.json`. Configure models in \
                 `roko.toml` under `[routing]`.",
        internals: "Implementation is in `crates/roko-learn/src/cascade_router.rs`. \
                    The router uses Thompson sampling over per-tier success rates. \
                    Tier assignment is computed by `roko-primitives` HDC vectors. \
                    The router emits efficiency events to \
                    `.roko/learn/efficiency.jsonl` for offline analysis.",
    },
    TopicEntry {
        name: "cognitive",
        title: "Cognitive Architecture",
        summary: "Roko's cognitive architecture is built around one noun (Signal) and \
                  six verb traits: Substrate, Scorer, Gate, Router, Composer, and \
                  Policy. Every operation follows the universal loop: query, score, \
                  route, compose, act, verify, write, react.",
        detail: "The six traits define the contract for each phase of processing. \
                 `Substrate` handles storage, `Scorer` evaluates quality, `Gate` \
                 validates correctness, `Router` selects models/agents, `Composer` \
                 assembles prompts, and `Policy` governs agent behavior. The \
                 `SystemPromptBuilder` in `roko-compose` assembles 9-layer prompts \
                 from role templates, domain context, and runtime state.",
        internals: "Trait definitions live in `crates/roko-core/src/lib.rs`. The \
                    universal loop is wired in `crates/roko-cli/src/run.rs` via \
                    `run_once()`. Prompt assembly uses `RoleSystemPromptSpec` in \
                    `orchestrate.rs`. Templates are in \
                    `crates/roko-compose/src/templates/` (9 role templates).",
    },
    TopicEntry {
        name: "neuro",
        title: "Durable Knowledge Store (Neuro)",
        summary: "The neuro subsystem is roko's long-term memory. It stores distilled \
                  knowledge, learned patterns, and factual summaries that persist \
                  across sessions and inform future agent decisions.",
        detail: "Knowledge entries are stored as engrams in `roko-neuro`. The \
                 distillation pipeline extracts key insights from completed episodes \
                 and stores them with embeddings for retrieval. Tier progression \
                 (novice -> competent -> proficient -> expert) tracks mastery of \
                 topics. Use `roko neuro search <query>` to query the knowledge base.",
        internals: "Implementation lives in `crates/roko-neuro/`. Engrams persist as \
                    JSONL in `.roko/neuro/`. The knowledge graph uses HDC vectors \
                    from `roko-primitives` for similarity search. Distillation runs \
                    during dream cycles (see `dreams` topic). Tier progression is \
                    tracked per-domain in `.roko/neuro/tiers.json`.",
    },
    TopicEntry {
        name: "daimon",
        title: "Daimon (Behavioral Primitives)",
        summary: "Daimons are behavioral building blocks that give agents personality \
                  and adaptive behavior. They encode preferences, habits, and \
                  response patterns that emerge from experience.",
        detail: "Each daimon tracks an affect dimension (curiosity, caution, \
                 confidence, etc.) as a value that shifts based on agent outcomes. \
                 High curiosity leads to more exploration; high caution triggers \
                 extra validation. Daimons influence prompt composition, model \
                 selection, and gate thresholds through the Policy trait.",
        internals: "Implementation is in `crates/roko-daimon/`. Affect values are \
                    f64 in [-1, 1] with drift and decay. The `AffectMap` struct \
                    holds all dimensions. Daimon state persists at `.roko/daimon/`. \
                    The conductor watches affect changes to trigger behavioral \
                    adjustments.",
    },
    TopicEntry {
        name: "dreams",
        title: "Dream Cycles (Offline Consolidation)",
        summary: "Dreams are offline consolidation cycles that process completed \
                  episodes to extract patterns, distill knowledge, and tune \
                  parameters. They run when the system is idle, like sleep for AI.",
        detail: "A dream cycle has three phases: hypnagogia (light review of recent \
                 episodes), imagination (creative recombination of patterns), and \
                 deep sleep (parameter consolidation). Use `roko dream run` to \
                 trigger a cycle manually, or configure automatic scheduling in \
                 `roko.toml` under `[dreams]`.",
        internals: "Implementation lives in `crates/roko-dreams/`. The \
                    `DreamRunner` orchestrates the three phases. Hypnagogia reads \
                    `.roko/episodes.jsonl` and produces oneirography (dream logs). \
                    Imagination uses the knowledge store for creative connections. \
                    Deep sleep updates cascade router weights and gate thresholds. \
                    Dream artifacts persist at `.roko/dreams/`.",
    },
    TopicEntry {
        name: "engram",
        title: "Engrams (Signal Storage)",
        summary: "Engrams are the fundamental unit of data in roko. Every piece of \
                  information (prompts, outputs, gate results, episodes) is stored \
                  as a content-addressed engram with a blake3 hash and DAG lineage.",
        detail: "Engrams form a directed acyclic graph (DAG) where each engram \
                 references its parent(s). This creates an immutable audit trail of \
                 every decision and action. Use `roko replay <hash>` to walk the \
                 lineage DAG from any engram. Engrams persist in `.roko/engrams.jsonl` \
                 via the `FileSubstrate` in `roko-fs`.",
        internals: "The `Signal` type in `crates/roko-core/src/lib.rs` is the base \
                    engram structure. `FileSubstrate` in `crates/roko-fs/` handles \
                    JSONL persistence with append-only semantics. GC runs periodically \
                    to compact old entries. The DAG walker in `crates/roko-cli/` \
                    reconstructs lineage chains for `roko replay`.",
    },
    TopicEntry {
        name: "cfactor",
        title: "C-Factor (Collective Intelligence)",
        summary: "The C-Factor measures how well agents collaborate. It aggregates \
                  coordination metrics (shared context usage, handoff quality, \
                  conflict rate) into a single 0-1 score that reflects collective \
                  intelligence.",
        detail: "C-Factor is computed from: (1) gate pass rate correlation between \
                 agents, (2) knowledge reuse rate from the neuro store, (3) conflict \
                 resolution speed in multi-agent plans, and (4) communication \
                 efficiency. Use `roko status --cfactor` to compute and display the \
                 latest score. History is tracked for trend analysis in the dashboard.",
        internals: "C-Factor computation lives in `crates/roko-core/`. The metric \
                    aggregates sub-scores using weighted geometric mean. Weights are \
                    adaptive based on plan complexity. History persists at \
                    `.roko/cfactor.jsonl`. The TUI dashboard renders C-Factor as a \
                    trend chart on the Health page.",
    },
    TopicEntry {
        name: "plans",
        title: "Plan Execution",
        summary: "Plans are directed acyclic graphs of tasks that roko executes to \
                  accomplish complex goals. Each task has dependencies, an agent role, \
                  and must pass gates to complete.",
        detail: "Plans are generated from PRDs via `roko prd plan <slug>`. The DAG \
                 executor in `roko-orchestrator` runs tasks in parallel where \
                 dependencies allow. Each task is dispatched to an agent with a role \
                 (implementer, reviewer, architect), runs through gates, and persists \
                 results. Use `roko plan run <dir>` to execute, `--resume` to \
                 continue from a snapshot.",
        internals: "Plan execution lives in `crates/roko-cli/src/orchestrate.rs` via \
                    `PlanRunner`. DAG scheduling is in `crates/roko-orchestrator/`. \
                    Snapshots persist at `.roko/state/executor.json` for resumability. \
                    The merge queue handles concurrent task outputs. Process \
                    supervision via `roko-runtime` tracks agent lifecycles.",
    },
    TopicEntry {
        name: "agents",
        title: "Agent System",
        summary: "Roko supports multiple LLM backends (Claude CLI, Claude API, Codex, \
                  Cursor, OpenAI-compatible, Ollama, Gemini, Perplexity) and manages \
                  agent lifecycles including spawning, health monitoring, and shutdown.",
        detail: "Agents are dispatched via the `AgentDispatcher` in `roko-agent`, which \
                 selects the appropriate backend based on configuration. Each agent runs \
                 in a supervised process with MCP tool access. The tool loop validates \
                 every tool call through the safety layer (role auth, pre/post checks). \
                 Agent output streams to the TUI in real time via WebSocket.",
        internals: "Agent dispatch is in `crates/roko-agent/src/dispatcher/mod.rs`. The \
                    tool loop is in `crates/roko-agent/src/tool_loop/`. Safety checks \
                    are integrated into `ToolDispatcher`. MCP config is passed through \
                    from `roko.toml` `[agent]` section. The per-agent HTTP sidecar in \
                    `crates/roko-agent-server/` provides `/message`, `/stream` (WS), \
                    and `/predictions` endpoints.",
    },
];

/// Look up a topic by name (case-insensitive).
#[must_use]
pub fn find_topic(name: &str) -> Option<&'static TopicEntry> {
    let lower = name.to_ascii_lowercase();
    TOPICS.iter().find(|t| t.name == lower)
}

/// List all available topic names.
#[must_use]
pub fn topic_names() -> Vec<&'static str> {
    TOPICS.iter().map(|t| t.name).collect()
}

/// Render a topic at the given depth level (1, 2, or 3).
pub fn render_topic(topic: &TopicEntry, depth: u8) -> String {
    let mut out = String::new();
    out.push_str(&format!("== {} ==\n\n", topic.title));
    out.push_str(topic.summary);
    out.push('\n');

    if depth >= 2 {
        out.push_str("\n--- How it works ---\n\n");
        out.push_str(topic.detail);
        out.push('\n');
    }

    if depth >= 3 {
        out.push_str("\n--- Internals ---\n\n");
        out.push_str(topic.internals);
        out.push('\n');
    }

    if depth < 3 {
        out.push_str(&format!(
            "\n(use --depth {} for more detail)\n",
            depth + 1
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_known_topic() {
        let t = find_topic("gates").unwrap();
        assert_eq!(t.name, "gates");
        assert!(!t.summary.is_empty());
    }

    #[test]
    fn find_topic_case_insensitive() {
        assert!(find_topic("Gates").is_some());
        assert!(find_topic("ROUTING").is_some());
    }

    #[test]
    fn find_unknown_topic_returns_none() {
        assert!(find_topic("nonexistent").is_none());
    }

    #[test]
    fn all_topics_have_content() {
        for t in TOPICS {
            assert!(!t.name.is_empty(), "topic name empty");
            assert!(!t.title.is_empty(), "topic {} title empty", t.name);
            assert!(!t.summary.is_empty(), "topic {} summary empty", t.name);
            assert!(!t.detail.is_empty(), "topic {} detail empty", t.name);
            assert!(!t.internals.is_empty(), "topic {} internals empty", t.name);
        }
    }

    #[test]
    fn at_least_8_topics() {
        assert!(
            TOPICS.len() >= 8,
            "expected at least 8 topics, got {}",
            TOPICS.len()
        );
    }

    #[test]
    fn required_topics_present() {
        let required = [
            "gates", "routing", "cognitive", "neuro", "daimon", "dreams", "engram", "cfactor",
        ];
        for name in &required {
            assert!(find_topic(name).is_some(), "missing required topic: {name}");
        }
    }

    #[test]
    fn render_depth_1_shows_summary_only() {
        let t = find_topic("gates").unwrap();
        let out = render_topic(t, 1);
        assert!(out.contains("Gate Pipeline"));
        assert!(out.contains(t.summary));
        assert!(!out.contains("How it works"));
        assert!(!out.contains("Internals"));
        assert!(out.contains("--depth 2"));
    }

    #[test]
    fn render_depth_2_shows_detail() {
        let t = find_topic("gates").unwrap();
        let out = render_topic(t, 2);
        assert!(out.contains("How it works"));
        assert!(!out.contains("Internals"));
        assert!(out.contains("--depth 3"));
    }

    #[test]
    fn render_depth_3_shows_all() {
        let t = find_topic("gates").unwrap();
        let out = render_topic(t, 3);
        assert!(out.contains("How it works"));
        assert!(out.contains("Internals"));
        assert!(!out.contains("--depth 4"));
    }

    #[test]
    fn topic_names_returns_all() {
        let names = topic_names();
        assert!(names.len() >= 8);
        assert!(names.contains(&"gates"));
        assert!(names.contains(&"cfactor"));
    }
}

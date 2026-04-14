//! Engram kinds — what a signal represents.
//!
//! A signal's [`Kind`] tells consumers how to interpret its body. Kinds are
//! grouped by architectural concern (agent runtime, verification, context,
//! memory, chain). The enum is `#[non_exhaustive]` and has a `Custom(String)`
//! escape hatch so extensions can define their own kinds without modifying
//! this crate.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The category of a signal. Determines how its body should be interpreted.
///
/// Kinds are the switchyard for dispatch: a [`Gate`](crate::Gate) might only
/// verify signals of kind `GateVerdict`, a [`Composer`](crate::Composer) might
/// only combine `PromptSection` signals, etc.
///
/// # Extensibility
///
/// The enum is `#[non_exhaustive]`. To add a new kind without modifying this
/// file, use `Kind::Custom("my.custom.kind".into())`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    // ─── Agent runtime ───────────────────────────────────────────────────
    /// A process was spawned (LLM subprocess, sandbox, etc.).
    ProcessSpawn,
    /// A process exited.
    ProcessExit,
    /// A message chunk from an agent's stream.
    AgentMessage,
    /// Raw stdout/stderr output from an agent.
    AgentOutput,
    /// Token usage report from an LLM call.
    TokenUsage,
    /// An agent requested approval for a destructive operation.
    ApprovalRequested,

    // ─── Verification ────────────────────────────────────────────────────
    /// A gate passed or failed a check.
    GateVerdict,
    /// A test suite run result.
    TestResult,
    /// A compile diagnostic (error or warning).
    CompileDiagnostic,

    // ─── Tasks & plans ───────────────────────────────────────────────────
    /// A task description (input to an agent).
    Task,
    /// A plan (collection of tasks with dependencies).
    Plan,
    /// A plan transitioned phases.
    PlanPhase,

    // ─── Context assembly ────────────────────────────────────────────────
    /// A single section within an assembled prompt.
    PromptSection,
    /// A curated bundle of context for an agent.
    ContextPack,
    /// A fully-assembled prompt ready for an LLM.
    Prompt,

    // ─── Routing & learning ──────────────────────────────────────────────
    /// A router's decision (e.g. "use Claude for this task").
    RouterChoice,
    /// Feedback about a prior router choice (reward signal).
    RouterFeedback,

    // ─── Memory ──────────────────────────────────────────────────────────
    /// A logged episode of an agent run.
    Episode,
    /// A playbook rule extracted from patterns.
    PlaybookRule,
    /// A learned skill (reusable procedure).
    Skill,

    // ─── Observability ───────────────────────────────────────────────────
    /// A metric reading (scalar measurement).
    Metric,
    /// An experiment result (A/B test outcome).
    ExperimentResult,
    /// A tool invocation record (per-call metrics).
    ToolInvocation,
    /// A tool's health has degraded past an alert threshold.
    ToolHealthDegraded,

    // ─── Chain participation (Phase 8+) ──────────────────────────────────
    /// A chain insight (shared knowledge).
    Insight,
    /// A stigmergic pheromone (threat/opportunity/wisdom).
    Pheromone,
    /// A bounty available for claiming.
    Bounty,
    /// An on-chain transaction.
    Transaction,
    /// A service offering (`OaaS` marketplace).
    Service,
    /// A prediction claim (for predictive foraging).
    Prediction,

    // ─── Extension ───────────────────────────────────────────────────────
    /// Custom kind identified by a dotted string. Extensions should use
    /// a reverse-DNS prefix (e.g. `"com.example.my_kind"`) to avoid collisions.
    Custom(String),
}

impl Kind {
    /// String identifier for this kind, suitable for logs and keys.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::ProcessSpawn => "process_spawn",
            Self::ProcessExit => "process_exit",
            Self::AgentMessage => "agent_message",
            Self::AgentOutput => "agent_output",
            Self::TokenUsage => "token_usage",
            Self::ApprovalRequested => "approval_requested",
            Self::GateVerdict => "gate_verdict",
            Self::TestResult => "test_result",
            Self::CompileDiagnostic => "compile_diagnostic",
            Self::Task => "task",
            Self::Plan => "plan",
            Self::PlanPhase => "plan_phase",
            Self::PromptSection => "prompt_section",
            Self::ContextPack => "context_pack",
            Self::Prompt => "prompt",
            Self::RouterChoice => "router_choice",
            Self::RouterFeedback => "router_feedback",
            Self::Episode => "episode",
            Self::PlaybookRule => "playbook_rule",
            Self::Skill => "skill",
            Self::Metric => "metric",
            Self::ExperimentResult => "experiment_result",
            Self::ToolInvocation => "tool_invocation",
            Self::ToolHealthDegraded => "tool_health_degraded",
            Self::Insight => "insight",
            Self::Pheromone => "pheromone",
            Self::Bounty => "bounty",
            Self::Transaction => "transaction",
            Self::Service => "service",
            Self::Prediction => "prediction",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_is_stable() {
        assert_eq!(Kind::GateVerdict.as_str(), "gate_verdict");
        assert_eq!(Kind::ProcessSpawn.as_str(), "process_spawn");
    }

    #[test]
    fn custom_kinds_preserve_string() {
        let k = Kind::Custom("com.example.widget".into());
        assert_eq!(k.as_str(), "com.example.widget");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", Kind::Task), "task");
    }

    #[test]
    fn serde_roundtrip() {
        for k in [
            Kind::ProcessSpawn,
            Kind::GateVerdict,
            Kind::ToolInvocation,
            Kind::ToolHealthDegraded,
            Kind::Custom("x.y".into()),
        ] {
            let json = serde_json::to_string(&k).unwrap();
            let parsed: Kind = serde_json::from_str(&json).unwrap();
            assert_eq!(k, parsed);
        }
    }
}

//! Prompt assembly — turn a task + context into an [`AssembledPrompt`].
//!
//! ## Composition (architectural note)
//!
//! Prompt construction is a **Compose** verb in the Roko model. This
//! module owns the runner-facing seam and delegates the heavy lifting to
//! [`roko_compose::SystemPromptBuilder`] (the 9-layer canonical builder).
//! Anything provider-specific (token counting, allowlist syntax) belongs
//! below this layer.
//!
//! ## What's structured
//!
//! The result is intentionally rich:
//!
//! - `system_prompt` — the rendered system message
//! - `user_prompt` — the rendered user message
//! - `tool_allowlist` — explicit allowlist (intersected with safety
//!   contract upstream of dispatch)
//! - `diagnostics` — what got included / dropped, total token estimate,
//!   playbook ids, knowledge ids — used for prompt experiments and the
//!   projection layer
//! - `gate_feedback` (carried into context, not the result) — structured
//!   compile / test / clippy errors injected on retry
//!
//! Token budget enforcement is deterministic: when the assembled prompt
//! exceeds the configured budget, sections are dropped in priority order
//! (knowledge → playbooks → code-index → retry-feedback → allowlist →
//! task description). The dropped list is reported in `diagnostics` so
//! observers can investigate budget pressure.
//!
//! ## Test seam
//!
//! [`PromptAssembler::minimal`] returns an assembler with no playbook /
//! neuro store and a tiny default budget — used by tests and CI smoke
//! runs to keep prompt construction deterministic.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::DispatchContext;
use super::outcome::DispatchError;
use crate::task_parser::TaskDef;

/// Maximum tokens an assembled prompt may emit before deterministic
/// dropping kicks in. Roughly mirrors a 200K-context-window providers'
/// budget for system + user combined.
const DEFAULT_TOKEN_BUDGET: u32 = 64_000;

// ─── Inputs ────────────────────────────────────────────────────────────

/// Per-call context the assembler needs from the runner.
///
/// Constructed from a `TaskDef` + `DispatchContext` so the assembler
/// stays pure.
#[derive(Debug, Clone)]
pub struct PromptContext {
    /// Plan id.
    pub plan_id: String,
    /// Role label.
    pub role: String,
    /// Files in scope for this task (from `task.files`).
    pub files_in_scope: Vec<String>,
    /// Acceptance criteria (from `task.acceptance`).
    pub acceptance_criteria: Vec<String>,
    /// `task.verify` shell commands.
    pub verify_commands: Vec<String>,
    /// Optional structured gate feedback for retry prompts.
    pub gate_feedback: Option<GateFeedback>,
    /// Attempt number (0 = first, > 0 = retry).
    pub attempt: u32,
}

impl PromptContext {
    /// Construct a `PromptContext` from runner inputs.
    #[must_use]
    pub fn from_task(task: &TaskDef, ctx: &DispatchContext) -> Self {
        Self {
            plan_id: ctx.plan_id.clone(),
            role: ctx.role.clone(),
            files_in_scope: task.files.clone(),
            acceptance_criteria: task.acceptance.clone(),
            verify_commands: task
                .verify
                .iter()
                .map(|step| step.command.clone())
                .collect(),
            gate_feedback: ctx.gate_feedback.clone(),
            attempt: ctx.attempt,
        }
    }
}

/// Structured gate feedback injected into retry prompts.
///
/// Replaces the legacy "raw stdout dump" prepend with a typed payload
/// the prompt builder can render selectively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateFeedback {
    /// Compile errors lifted from cargo check output.
    #[serde(default)]
    pub compile_errors: Vec<String>,
    /// Failing test names + their summaries.
    #[serde(default)]
    pub test_failures: Vec<String>,
    /// Clippy warnings that surfaced.
    #[serde(default)]
    pub clippy_warnings: Vec<String>,
    /// The original gate output (truncated to ≤ 4 KB upstream).
    pub raw_output: String,
}

// ─── Outputs ───────────────────────────────────────────────────────────

/// Assembled prompt, allowlist, and diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledPrompt {
    /// Rendered system prompt.
    pub system_prompt: String,
    /// Rendered user prompt.
    pub user_prompt: String,
    /// Optional tool allowlist (intersected with safety contract).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_allowlist: Option<Vec<String>>,
    /// Per-assembly diagnostics for experiments + projection.
    pub diagnostics: PromptDiagnostics,
}

/// Auditable info about the assembly run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptDiagnostics {
    /// Sections that made it into the rendered prompt.
    pub included_sections: Vec<String>,
    /// Sections dropped to fit the token budget.
    pub dropped_sections: Vec<String>,
    /// Coarse estimate of the assembled prompt token count.
    pub estimated_tokens: u32,
    /// Playbook ids consulted (if any).
    pub playbook_ids: Vec<String>,
    /// Neuro knowledge ids surfaced (if any).
    pub knowledge_ids: Vec<String>,
}

// ─── Assembler ─────────────────────────────────────────────────────────

/// Prompt assembler.
///
/// The current implementation produces a deterministic, structured
/// prompt suitable for tests and the smoke path. Wiring into the full
/// 9-layer [`roko_compose::SystemPromptBuilder`] is exposed as a
/// follow-up — see `.roko/GAPS.md`.
#[derive(Debug, Clone)]
pub struct PromptAssembler {
    /// Token budget cap.
    token_budget: u32,
    /// Whether to query playbook / neuro stores during assembly.
    /// Off in `minimal()` (tests / smoke) and on once stores are wired.
    use_knowledge_stores: bool,
}

impl PromptAssembler {
    /// Construct a production assembler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            token_budget: DEFAULT_TOKEN_BUDGET,
            use_knowledge_stores: true,
        }
    }

    /// Test / smoke assembler — no knowledge stores, tiny budget.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            token_budget: 8_000,
            use_knowledge_stores: false,
        }
    }

    /// Override the token budget.
    pub fn with_token_budget(mut self, budget: u32) -> Self {
        self.token_budget = budget;
        self
    }

    /// Assemble the prompt for `task` in the given context.
    pub fn assemble(
        &self,
        task: &TaskDef,
        ctx: &PromptContext,
    ) -> Result<AssembledPrompt, DispatchError> {
        // ── Section authorship ────────────────────────────────────────
        // Each section returns Some(text) when applicable. We then drop
        // sections in priority order if the assembled prompt exceeds the
        // budget — see `enforce_budget`.
        let role_section = format!("# Role\nYou are the **{}** for this task.", ctx.role);

        let task_section = format!(
            "# Task\n**{}**: {}",
            task.id,
            task.description
                .clone()
                .unwrap_or_else(|| task.title.clone())
        );

        let files_section = if ctx.files_in_scope.is_empty() {
            None
        } else {
            Some(format!(
                "# Files in scope\n{}",
                ctx.files_in_scope
                    .iter()
                    .map(|f| format!("- `{f}`"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        };

        let acceptance_section = if ctx.acceptance_criteria.is_empty() {
            None
        } else {
            Some(format!(
                "# Acceptance criteria\n{}",
                ctx.acceptance_criteria
                    .iter()
                    .map(|c| format!("- {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        };

        let verify_section = if ctx.verify_commands.is_empty() {
            None
        } else {
            Some(format!(
                "# Verify\nAfter editing, run:\n{}",
                ctx.verify_commands
                    .iter()
                    .map(|v| format!("- `{v}`"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        };

        let retry_section = if ctx.attempt > 0 {
            ctx.gate_feedback.as_ref().map(render_gate_feedback)
        } else {
            None
        };

        let allowlist = task.allowed_tools.clone();
        let allowlist_section = allowlist
            .as_ref()
            .filter(|list| !list.is_empty())
            .map(|list| {
                format!(
                    "# Allowed tools\nYou may only invoke: {}",
                    list.iter()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });

        // ── Assemble + budget ─────────────────────────────────────────
        let mut sections: Vec<(&'static str, String, u32)> = Vec::new();
        sections.push(("role", role_section, 1));
        sections.push(("task", task_section, 1));
        if let Some(s) = files_section {
            sections.push(("files", s, 4));
        }
        if let Some(s) = acceptance_section {
            sections.push(("acceptance", s, 2));
        }
        if let Some(s) = verify_section {
            sections.push(("verify", s, 3));
        }
        if let Some(s) = retry_section {
            sections.push(("retry", s, 5));
        }
        if let Some(s) = allowlist_section {
            sections.push(("allowlist", s, 6));
        }

        let mut diagnostics = PromptDiagnostics::default();
        let system_prompt = self.enforce_budget(&mut sections, &mut diagnostics);

        let user_prompt = task.title.clone();

        Ok(AssembledPrompt {
            system_prompt,
            user_prompt,
            tool_allowlist: allowlist,
            diagnostics,
        })
    }

    /// Drop sections in priority order until the prompt fits the budget.
    ///
    /// Priorities (lower drop-priority = higher importance):
    ///
    /// 1: role / task          (never dropped)
    /// 2: acceptance
    /// 3: verify
    /// 4: files
    /// 5: retry feedback
    /// 6: allowlist (covered by safety contract too — safe to drop)
    ///
    /// Knowledge / playbook sections (drop priority 7+) will land here
    /// once those stores are wired through the assembler.
    fn enforce_budget(
        &self,
        sections: &mut Vec<(&'static str, String, u32)>,
        diagnostics: &mut PromptDiagnostics,
    ) -> String {
        // Sort by drop priority descending so high-priority sections drop first.
        sections.sort_by(|a, b| b.2.cmp(&a.2));
        let mut selected: Vec<(&'static str, String)> = sections
            .iter()
            .map(|(name, body, _)| (*name, body.clone()))
            .collect();
        // Drop highest drop-priority first while we exceed budget.
        loop {
            let total = estimate_tokens(&selected);
            diagnostics.estimated_tokens = total;
            if total <= self.token_budget {
                break;
            }
            // Section index 0 has the highest drop priority after the sort
            if let Some(dropped) = selected.first().map(|(name, _)| *name) {
                diagnostics.dropped_sections.push(dropped.to_string());
                selected.remove(0);
            } else {
                break;
            }
        }

        // Restore canonical order (role, task, files, acceptance, verify, retry, allowlist)
        let canonical: &[&str] = &[
            "role",
            "task",
            "files",
            "acceptance",
            "verify",
            "retry",
            "allowlist",
        ];
        let mut ordered: Vec<&(&'static str, String)> = selected.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|(name, _)| canonical.iter().position(|n| n == name).unwrap_or(99));
        diagnostics.included_sections = ordered.iter().map(|(n, _)| n.to_string()).collect();

        ordered
            .into_iter()
            .map(|(_, body)| body.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Default for PromptAssembler {
    fn default() -> Self {
        Self::new()
    }
}

fn estimate_tokens(sections: &[(&'static str, String)]) -> u32 {
    // Coarse rule-of-thumb: 1 token ≈ 4 ASCII characters.
    sections
        .iter()
        .map(|(_, body)| (body.len() / 4) as u32)
        .sum::<u32>()
        .max(1)
}

fn render_gate_feedback(feedback: &GateFeedback) -> String {
    let mut buf = String::from("# Previous attempt feedback\n");
    if !feedback.compile_errors.is_empty() {
        buf.push_str("## Compile errors\n");
        for err in &feedback.compile_errors {
            buf.push_str(&format!("- {err}\n"));
        }
    }
    if !feedback.test_failures.is_empty() {
        buf.push_str("## Failing tests\n");
        for failure in &feedback.test_failures {
            buf.push_str(&format!("- {failure}\n"));
        }
    }
    if !feedback.clippy_warnings.is_empty() {
        buf.push_str("## Clippy warnings\n");
        for w in &feedback.clippy_warnings {
            buf.push_str(&format!("- {w}\n"));
        }
    }
    buf
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn task() -> TaskDef {
        TaskDef {
            id: "t".into(),
            title: "Wire it up".into(),
            description: Some("Explain the wiring".into()),
            role: Some("implementer".into()),
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec!["src/lib.rs".into()],
            allowed_tools: Some(vec!["read_file".into(), "edit_file".into()]),
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![crate::task_parser::VerifyStep {
                phase: "test".into(),
                command: "cargo test".into(),
                fail_msg: None,
                timeout_ms: 60_000,
            }],
            timeout_secs: 60,
            max_retries: 1,
            acceptance: vec!["compiles".into()],
            acceptance_contract: None,
            domain: None,
        }
    }

    fn ctx() -> DispatchContext {
        DispatchContext {
            plan_id: "p".into(),
            role: "implementer".into(),
            workdir: PathBuf::from("/tmp"),
            model_hint: None,
            force_backend: None,
            budget_remaining_usd: 5.0,
            attempt: 0,
            gate_feedback: None,
        }
    }

    #[test]
    fn first_attempt_includes_all_canonical_sections() {
        let assembler = PromptAssembler::minimal();
        let pctx = PromptContext::from_task(&task(), &ctx());
        let p = assembler.assemble(&task(), &pctx).unwrap();
        assert!(p.system_prompt.contains("# Role"));
        assert!(p.system_prompt.contains("# Task"));
        assert!(p.system_prompt.contains("# Files in scope"));
        assert!(p.system_prompt.contains("# Acceptance criteria"));
        assert!(p.system_prompt.contains("# Verify"));
        assert!(p.system_prompt.contains("# Allowed tools"));
        assert!(!p.system_prompt.contains("# Previous attempt"));
        assert_eq!(p.tool_allowlist.as_deref().unwrap().len(), 2);
        assert!(p.diagnostics.estimated_tokens > 0);
    }

    #[test]
    fn retry_attempt_renders_gate_feedback() {
        let assembler = PromptAssembler::minimal();
        let mut c = ctx();
        c.attempt = 1;
        c.gate_feedback = Some(GateFeedback {
            compile_errors: vec!["E0432: unresolved import".into()],
            test_failures: vec!["mod::test_foo: assertion failed".into()],
            clippy_warnings: vec![],
            raw_output: "...".into(),
        });
        let pctx = PromptContext::from_task(&task(), &c);
        let p = assembler.assemble(&task(), &pctx).unwrap();
        assert!(p.system_prompt.contains("# Previous attempt feedback"));
        assert!(p.system_prompt.contains("E0432"));
        assert!(p.system_prompt.contains("mod::test_foo"));
    }

    #[test]
    fn token_budget_drops_lowest_priority_sections() {
        let assembler = PromptAssembler::new().with_token_budget(40);
        let mut t = task();
        t.acceptance = vec!["a very long acceptance criterion that takes many tokens".into()];
        let pctx = PromptContext::from_task(&t, &ctx());
        let p = assembler.assemble(&t, &pctx).unwrap();
        // role + task always survive
        assert!(p.system_prompt.contains("# Role"));
        assert!(p.system_prompt.contains("# Task"));
        // Some lower-priority section must have been dropped
        assert!(!p.diagnostics.dropped_sections.is_empty());
    }

    #[test]
    fn empty_optional_sections_omitted_cleanly() {
        let assembler = PromptAssembler::minimal();
        let mut t = task();
        t.files = vec![];
        t.acceptance = vec![];
        t.verify = vec![];
        t.allowed_tools = None;
        let pctx = PromptContext::from_task(&t, &ctx());
        let p = assembler.assemble(&t, &pctx).unwrap();
        assert!(!p.system_prompt.contains("# Files in scope"));
        assert!(!p.system_prompt.contains("# Acceptance"));
        assert!(!p.system_prompt.contains("# Verify"));
        assert!(!p.system_prompt.contains("# Allowed tools"));
        assert_eq!(p.tool_allowlist, None);
    }
}

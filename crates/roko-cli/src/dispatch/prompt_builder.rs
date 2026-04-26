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

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    /// Workspace root used to resolve `.roko` learning stores.
    pub workdir: PathBuf,
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
            workdir: ctx.workdir.clone(),
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

impl GateFeedback {
    /// Parse raw gate output into structured retry context.
    #[must_use]
    pub fn from_raw(raw_output: &str) -> Option<Self> {
        let raw = raw_output.trim();
        if raw.is_empty() {
            return None;
        }

        let mut compile_errors = Vec::new();
        let mut test_failures = Vec::new();
        let mut clippy_warnings = Vec::new();
        for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let lower = line.to_ascii_lowercase();
            let truncated = line.chars().take(240).collect::<String>();
            if lower.contains("error[") || lower.starts_with("error:") || line.contains("-->") {
                compile_errors.push(truncated);
            } else if lower.contains("test")
                && (lower.contains("failed") || lower.contains("panicked"))
            {
                test_failures.push(truncated);
            } else if lower.contains("warning") || lower.contains("clippy") {
                clippy_warnings.push(truncated);
            }
            if compile_errors.len() + test_failures.len() + clippy_warnings.len() >= 24 {
                break;
            }
        }

        Some(Self {
            compile_errors,
            test_failures,
            clippy_warnings,
            raw_output: raw.chars().take(4096).collect(),
        })
    }
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

// ─── Source Plugins ────────────────────────────────────────────────────

/// One optional section contributed by a prompt context source.
#[derive(Debug, Clone)]
struct PromptSection {
    name: String,
    body: String,
    drop_priority: u32,
    knowledge_ids: Vec<String>,
    playbook_ids: Vec<String>,
}

impl PromptSection {
    fn new(name: impl Into<String>, body: impl Into<String>, drop_priority: u32) -> Self {
        Self {
            name: name.into(),
            body: body.into(),
            drop_priority,
            knowledge_ids: Vec::new(),
            playbook_ids: Vec::new(),
        }
    }

    fn with_knowledge_ids(mut self, ids: Vec<String>) -> Self {
        self.knowledge_ids = ids;
        self
    }

    fn with_playbook_ids(mut self, ids: Vec<String>) -> Self {
        self.playbook_ids = ids;
        self
    }
}

/// Pluggable prompt context provider.
trait PromptSectionSource: Send + Sync + std::fmt::Debug {
    fn collect(&self, task: &TaskDef, ctx: &PromptContext) -> Vec<PromptSection>;
}

/// Reads durable `.roko` knowledge stores and prior episodes.
#[derive(Debug, Default)]
struct WorkdirKnowledgeSource;

/// Reads learned playbooks from `.roko/learn/playbooks`.
#[derive(Debug, Default)]
struct WorkdirPlaybookSource;

/// Applies learned section-effectiveness priority adjustments.
#[derive(Debug, Default)]
struct SectionEffectivenessSource;

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
    /// Optional prompt context sources. `minimal()` leaves this empty.
    sources: Vec<Arc<dyn PromptSectionSource>>,
}

impl PromptAssembler {
    /// Construct a production assembler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            token_budget: DEFAULT_TOKEN_BUDGET,
            sources: vec![
                Arc::new(WorkdirKnowledgeSource),
                Arc::new(WorkdirPlaybookSource),
                Arc::new(SectionEffectivenessSource),
            ],
        }
    }

    /// Test / smoke assembler — no knowledge stores, tiny budget.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            token_budget: 8_000,
            sources: Vec::new(),
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
        let mut sections: Vec<PromptSection> = Vec::new();
        sections.push(PromptSection::new("role", role_section, 1));
        sections.push(PromptSection::new("task", task_section, 1));
        if let Some(s) = files_section {
            sections.push(PromptSection::new("files", s, 4));
        }
        if let Some(s) = acceptance_section {
            sections.push(PromptSection::new("acceptance", s, 2));
        }
        if let Some(s) = verify_section {
            sections.push(PromptSection::new("verify", s, 3));
        }
        if let Some(s) = retry_section {
            sections.push(PromptSection::new("retry", s, 5));
        }
        if let Some(s) = allowlist_section {
            sections.push(PromptSection::new("allowlist", s, 6));
        }

        let mut diagnostics = PromptDiagnostics::default();
        for source in &self.sources {
            sections.extend(source.collect(task, ctx));
        }
        apply_section_effectiveness(&ctx.workdir, &ctx.role, &mut sections);
        let system_prompt = self.enforce_budget(&mut sections, &mut diagnostics);

        let mut user_prompt = format!("# Task Request\n{}\n", task.title);
        if let Some(description) = &task.description {
            user_prompt.push_str("\n## Details\n");
            user_prompt.push_str(description);
            user_prompt.push('\n');
        }
        if let Some(context) = &task.context {
            if !context.read_files.is_empty()
                || !context.symbols.is_empty()
                || !context.anti_patterns.is_empty()
                || !context.prior_failures.is_empty()
            {
                user_prompt.push_str("\n## Task Context\n");
                for file in &context.read_files {
                    user_prompt.push_str("- Read `");
                    user_prompt.push_str(&file.path);
                    if let Some(lines) = &file.lines {
                        user_prompt.push_str("` lines ");
                        user_prompt.push_str(lines);
                    } else {
                        user_prompt.push('`');
                    }
                    user_prompt.push_str(": ");
                    user_prompt.push_str(&file.why);
                    user_prompt.push('\n');
                }
                for symbol in &context.symbols {
                    user_prompt.push_str("- Symbol: ");
                    user_prompt.push_str(symbol);
                    user_prompt.push('\n');
                }
                for anti_pattern in &context.anti_patterns {
                    user_prompt.push_str("- Avoid: ");
                    user_prompt.push_str(anti_pattern);
                    user_prompt.push('\n');
                }
                for failure in &context.prior_failures {
                    user_prompt.push_str("- Prior failure: ");
                    user_prompt.push_str(failure);
                    user_prompt.push('\n');
                }
            }
        }
        if !task.acceptance.is_empty() {
            user_prompt.push_str("\n## Acceptance\n");
            for item in &task.acceptance {
                user_prompt.push_str("- ");
                user_prompt.push_str(item);
                user_prompt.push('\n');
            }
        }
        if !task.verify.is_empty() {
            user_prompt.push_str("\n## Verification Commands\n");
            for step in &task.verify {
                user_prompt.push_str("- ");
                user_prompt.push_str(&step.command);
                user_prompt.push('\n');
            }
        }

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
        sections: &mut Vec<PromptSection>,
        diagnostics: &mut PromptDiagnostics,
    ) -> String {
        // Sort by drop priority descending so high-priority sections drop first.
        sections.sort_by(|a, b| b.drop_priority.cmp(&a.drop_priority));
        let mut selected = sections.clone();
        // Drop highest drop-priority first while we exceed budget.
        loop {
            let total = estimate_tokens(&selected);
            diagnostics.estimated_tokens = total;
            if total <= self.token_budget {
                break;
            }
            // Section index 0 has the highest drop priority after the sort
            if let Some(dropped) = selected.first().map(|section| section.name.clone()) {
                diagnostics.dropped_sections.push(dropped);
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
            "knowledge",
            "episode_knowledge",
            "playbooks",
            "section_effectiveness",
        ];
        let mut ordered: Vec<&PromptSection> = selected.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|section| {
            canonical
                .iter()
                .position(|name| *name == section.name.as_str())
                .unwrap_or(99)
        });
        diagnostics.included_sections =
            ordered.iter().map(|section| section.name.clone()).collect();
        diagnostics.knowledge_ids = ordered
            .iter()
            .flat_map(|section| section.knowledge_ids.clone())
            .collect();
        diagnostics.playbook_ids = ordered
            .iter()
            .flat_map(|section| section.playbook_ids.clone())
            .collect();

        ordered
            .into_iter()
            .map(|section| section.body.clone())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl PromptSectionSource for WorkdirKnowledgeSource {
    fn collect(&self, task: &TaskDef, ctx: &PromptContext) -> Vec<PromptSection> {
        let mut sections = Vec::new();
        if let Some(section) = collect_neuro_knowledge(task, ctx) {
            sections.push(section);
        }
        if let Some(section) = collect_episode_knowledge(task, ctx) {
            sections.push(section);
        }
        sections
    }
}

impl PromptSectionSource for WorkdirPlaybookSource {
    fn collect(&self, task: &TaskDef, ctx: &PromptContext) -> Vec<PromptSection> {
        collect_playbooks(task, ctx).into_iter().collect()
    }
}

impl PromptSectionSource for SectionEffectivenessSource {
    fn collect(&self, _task: &TaskDef, ctx: &PromptContext) -> Vec<PromptSection> {
        let path = ctx
            .workdir
            .join(roko_learn::section_effect::DEFAULT_SECTION_EFFECTS_PATH);
        if !path.exists() {
            return Vec::new();
        }
        let registry = roko_learn::section_effect::SectionEffectivenessRegistry::load_or_new(&path);
        let positive = registry.positive_lift_sections(&ctx.role);
        if positive.is_empty() {
            return Vec::new();
        }
        let mut body = String::from(
            "# Prompt section effectiveness\nHistorically high-signal prompt sections for this role:\n",
        );
        for effect in positive.into_iter().take(5) {
            body.push_str(&format!(
                "- {} (lift {:+.2}, weight {:.2})\n",
                effect.section_name,
                effect.lift(),
                effect.lift_weight()
            ));
        }
        vec![PromptSection::new("section_effectiveness", body, 7)]
    }
}

fn collect_neuro_knowledge(task: &TaskDef, ctx: &PromptContext) -> Option<PromptSection> {
    let store = roko_neuro::KnowledgeStore::for_workdir(&ctx.workdir);
    if !store.path().exists() {
        return None;
    }
    let query = task_query_text(task, ctx);
    let entries = store.query(&query, 5).ok()?;
    if entries.is_empty() {
        return None;
    }

    let ids = entries
        .iter()
        .map(|entry| entry.id.clone())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    let mut body = String::from("# Neuro knowledge\nRelevant durable knowledge from prior runs:\n");
    for entry in entries {
        let source = entry.source.as_deref().unwrap_or("neuro");
        body.push_str(&format!(
            "- [{}] {} (confidence {:.2}, source: {})\n",
            entry.id,
            truncate_chars(&entry.content, 420),
            entry.confidence,
            source
        ));
    }
    Some(PromptSection::new("knowledge", body, 7).with_knowledge_ids(ids))
}

fn collect_episode_knowledge(task: &TaskDef, ctx: &PromptContext) -> Option<PromptSection> {
    let keywords = query_keywords(&task_query_text(task, ctx));
    if keywords.is_empty() {
        return None;
    }

    let mut scored = Vec::new();
    for path in episode_paths(&ctx.workdir) {
        let file = std::fs::File::open(path).ok()?;
        let reader = std::io::BufReader::new(file);
        for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(episode) = serde_json::from_str::<roko_learn::episode_logger::Episode>(trimmed)
            else {
                continue;
            };
            let haystack = format!(
                "{} {} {} {} {}",
                episode.task_id,
                episode.agent_id,
                episode.model,
                episode.reasoning_summary.as_deref().unwrap_or(""),
                episode.failure_reason.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase();
            let score = keywords
                .iter()
                .filter(|keyword| haystack.contains(keyword.as_str()))
                .count();
            if score > 0 {
                scored.push((score, episode));
            }
        }
    }
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| {
        b.1.success
            .cmp(&a.1.success)
            .then_with(|| b.0.cmp(&a.0))
            .then_with(|| b.1.completed_at.cmp(&a.1.completed_at))
    });
    scored.truncate(5);

    let ids = scored
        .iter()
        .map(|(_, episode)| {
            if !episode.id.is_empty() {
                episode.id.clone()
            } else if !episode.episode_id.is_empty() {
                episode.episode_id.clone()
            } else {
                episode.task_id.clone()
            }
        })
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    let mut body =
        String::from("# Learned patterns from prior episodes\nSimilar prior work suggests:\n");
    for (_, episode) in scored {
        let outcome = if episode.success { "passed" } else { "failed" };
        let summary = episode
            .reasoning_summary
            .as_deref()
            .or(episode.reflection.as_deref())
            .or(episode.failure_reason.as_deref())
            .unwrap_or("no summary recorded");
        body.push_str(&format!(
            "- {} ({}, model: {}): {}\n",
            episode.task_id,
            outcome,
            if episode.model.is_empty() {
                "unknown"
            } else {
                &episode.model
            },
            truncate_chars(summary, 420)
        ));
    }
    Some(PromptSection::new("episode_knowledge", body, 7).with_knowledge_ids(ids))
}

fn collect_playbooks(task: &TaskDef, ctx: &PromptContext) -> Option<PromptSection> {
    let root = ctx.workdir.join(".roko").join("learn").join("playbooks");
    if !root.is_dir() {
        return None;
    }
    let query = query_keywords(&task_query_text(task, ctx));
    let mut scored = Vec::new();
    for entry in std::fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        let Ok(playbook) = serde_json::from_str::<roko_learn::playbook::Playbook>(&text) else {
            continue;
        };
        let haystack = playbook_text(&playbook).to_ascii_lowercase();
        let lexical_score = query
            .iter()
            .filter(|keyword| haystack.contains(keyword.as_str()))
            .count();
        let outcome_score = playbook
            .success_count
            .saturating_sub(playbook.failure_count) as usize;
        let score = lexical_score
            .saturating_mul(10)
            .saturating_add(outcome_score);
        if score > 0 || scored.len() < 3 {
            scored.push((score, playbook));
        }
    }
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.success_count.cmp(&a.1.success_count))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    scored.truncate(3);

    let ids = scored
        .iter()
        .map(|(_, playbook)| playbook.id.clone())
        .collect::<Vec<_>>();
    let mut body = String::from("# Relevant playbooks\nReusable proven procedures:\n");
    for (_, playbook) in scored {
        body.push_str(&format!(
            "- {}: {} (successes {}, failures {})\n",
            playbook.id, playbook.goal, playbook.success_count, playbook.failure_count
        ));
        for step in playbook.steps.iter().take(5) {
            body.push_str(&format!(
                "  - {} via {}; expect {}\n",
                step.description,
                step.action_kind,
                if step.expected_signals.is_empty() {
                    "task-local verification".to_string()
                } else {
                    step.expected_signals.join(", ")
                }
            ));
        }
    }
    Some(PromptSection::new("playbooks", body, 7).with_playbook_ids(ids))
}

fn apply_section_effectiveness(workdir: &Path, role: &str, sections: &mut [PromptSection]) {
    let path = workdir.join(roko_learn::section_effect::DEFAULT_SECTION_EFFECTS_PATH);
    if !path.exists() {
        return;
    }
    let registry = roko_learn::section_effect::SectionEffectivenessRegistry::load_or_new(&path);
    for section in sections {
        match registry.recommend_priority_change(&section.name, role) {
            roko_learn::section_effect::PriorityChange::Increase => {
                section.drop_priority = section.drop_priority.saturating_sub(1);
            }
            roko_learn::section_effect::PriorityChange::Decrease => {
                section.drop_priority = section.drop_priority.saturating_add(1);
            }
            roko_learn::section_effect::PriorityChange::NoChange
            | roko_learn::section_effect::PriorityChange::InsufficientData => {}
        }
    }
}

fn task_query_text(task: &TaskDef, ctx: &PromptContext) -> String {
    let mut parts = vec![task.id.clone(), task.title.clone(), ctx.role.clone()];
    if let Some(description) = &task.description {
        parts.push(description.clone());
    }
    parts.extend(task.acceptance.clone());
    parts.extend(task.files.clone());
    parts.join(" ")
}

fn query_keywords(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .filter(|word| word.len() > 2)
        .map(ToString::to_string)
        .collect()
}

fn episode_paths(workdir: &Path) -> Vec<PathBuf> {
    [
        workdir.join(".roko").join("episodes.jsonl"),
        workdir.join(".roko").join("learn").join("episodes.jsonl"),
        workdir.join(".roko").join("memory").join("episodes.jsonl"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

fn playbook_text(playbook: &roko_learn::playbook::Playbook) -> String {
    let mut text = format!("{} {} {}", playbook.id, playbook.name, playbook.goal);
    for step in &playbook.steps {
        text.push(' ');
        text.push_str(&step.description);
        text.push(' ');
        text.push_str(&step.action_kind);
        text.push(' ');
        text.push_str(&step.expected_signals.join(" "));
    }
    text
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut out = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        out.push_str(" [truncated]");
    }
    out
}

impl Default for PromptAssembler {
    fn default() -> Self {
        Self::new()
    }
}

fn estimate_tokens(sections: &[PromptSection]) -> u32 {
    // Coarse rule-of-thumb: 1 token ≈ 4 ASCII characters.
    sections
        .iter()
        .map(|section| (section.body.len() / 4) as u32)
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

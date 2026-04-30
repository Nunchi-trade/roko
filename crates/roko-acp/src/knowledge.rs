//! Dispatch-time knowledge retrieval and formatting for ACP.
//!
//! This module queries the durable knowledge store and playbook store once
//! per dispatch, then renders the result both as a visible card and as
//! bounded prompt context.

use std::path::{Path, PathBuf};

use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore};
use roko_neuro::{KnowledgeKind, KnowledgeQueryHit, KnowledgeStore, KnowledgeTier};
use tokio::task;
use tracing::warn;

/// Knowledge surfaced for a single dispatch.
#[derive(Debug, Clone, Default)]
pub(crate) struct DispatchKnowledge {
    /// Ranked knowledge hits from `roko-neuro`.
    pub hits: Vec<KnowledgeQueryHit>,
    /// Ranked playbooks from `roko-learn`.
    pub playbooks: Vec<Playbook>,
}

/// A visible knowledge card for the editor UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnowledgeCard {
    /// Card title shown in the tool-call header.
    pub title: String,
    /// Markdown body shown in the tool-call completion text.
    pub body: String,
}

impl DispatchKnowledge {
    /// Returns `true` when there is nothing to show or inject.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.hits.is_empty() && self.playbooks.is_empty()
    }

    /// Render a visible card for the editor, or `None` when there are no results.
    #[must_use]
    pub(crate) fn card(&self) -> Option<KnowledgeCard> {
        if self.is_empty() {
            return None;
        }

        let body = render_card_body(&self.hits, &self.playbooks)?;
        let total = self.hits.len() + self.playbooks.len();
        let title = format!("Prior knowledge - {total} result{}", plural(total));

        Some(KnowledgeCard { title, body })
    }

    /// Render prompt context suitable for system-prompt injection.
    #[must_use]
    pub(crate) fn context_text(&self) -> String {
        render_context_body(&self.hits, &self.playbooks).unwrap_or_default()
    }
}

/// Query the durable knowledge store and playbook store for a prompt.
///
/// Errors are logged and converted into empty result sets so dispatch can
/// continue normally.
#[must_use]
pub(crate) async fn query_dispatch_knowledge(workdir: &Path, prompt: &str) -> DispatchKnowledge {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return DispatchKnowledge::default();
    }

    let workdir = workdir.to_path_buf();
    let prompt_for_knowledge = prompt.to_owned();
    let prompt_for_playbooks = prompt.to_owned();

    let knowledge_future = query_knowledge_hits(workdir.clone(), prompt_for_knowledge);
    let playbooks_future = query_playbooks(workdir, prompt_for_playbooks);
    let (hits, playbooks) = tokio::join!(knowledge_future, playbooks_future);

    DispatchKnowledge { hits, playbooks }
}

async fn query_knowledge_hits(workdir: PathBuf, prompt: String) -> Vec<KnowledgeQueryHit> {
    let store = KnowledgeStore::for_workdir(&workdir);
    match task::spawn_blocking(move || store.query_hits(&prompt, 5)).await {
        Ok(Ok(hits)) => hits,
        Ok(Err(err)) => {
            warn!(
                workdir = %workdir.display(),
                error = %err,
                "knowledge query failed"
            );
            Vec::new()
        }
        Err(err) => {
            warn!(
                workdir = %workdir.display(),
                error = %err,
                "knowledge query task failed"
            );
            Vec::new()
        }
    }
}

async fn query_playbooks(workdir: PathBuf, prompt: String) -> Vec<Playbook> {
    let store = PlaybookStore::new(workdir.join(".roko").join("learn").join("playbooks"));
    match store.relevant(&prompt, 3).await {
        Ok(playbooks) => playbooks,
        Err(err) => {
            warn!(
                workdir = %workdir.display(),
                error = %err,
                "playbook query failed"
            );
            Vec::new()
        }
    }
}

/// Append `extra` to `base` with a blank line separator.
#[must_use]
pub(crate) fn append_context(base: &str, extra: &str) -> String {
    join_context(base, extra, false)
}

/// Prepend `extra` to `base` with a blank line separator.
#[must_use]
pub(crate) fn prepend_context(base: &str, extra: &str) -> String {
    join_context(base, extra, true)
}

fn join_context(base: &str, extra: &str, prepend: bool) -> String {
    let base = base.trim();
    let extra = extra.trim();

    if base.is_empty() {
        return extra.to_owned();
    }
    if extra.is_empty() {
        return base.to_owned();
    }

    if prepend {
        format!("{extra}\n\n{base}")
    } else {
        format!("{base}\n\n{extra}")
    }
}

fn render_card_body(hits: &[KnowledgeQueryHit], playbooks: &[Playbook]) -> Option<String> {
    let mut lines = Vec::new();

    if !playbooks.is_empty() {
        lines.push("**Playbooks:**".to_string());
        for playbook in playbooks {
            lines.push(format!(
                "  - {}{}",
                truncate_summary(playbook.goal.trim(), 120),
                success_rate_suffix(playbook)
            ));
        }
    }

    if !hits.is_empty() {
        lines.push("**Knowledge:**".to_string());
        for hit in hits.iter().take(5) {
            let tier_label = tier_label(hit.entry.tier);
            let summary = truncate_summary(hit.entry.content.trim(), 80);
            lines.push(format!(
                "  - [{tier_label}] {:.2} - {summary}",
                hit.total_score
            ));
        }
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn render_context_body(hits: &[KnowledgeQueryHit], playbooks: &[Playbook]) -> Option<String> {
    let mut sections = Vec::new();

    if !playbooks.is_empty() {
        let mut section = String::from("## Relevant playbooks from past tasks:");
        for playbook in playbooks {
            section.push_str("\n\n");
            section.push_str(&render_playbook_context(playbook));
        }
        sections.push(section);
    }

    if !hits.is_empty() {
        let mut section = String::from("## Relevant knowledge:");
        for hit in hits.iter().take(5) {
            section.push_str("\n\n");
            section.push_str(&render_hit_context(hit));
        }
        sections.push(section);
    }

    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

fn render_playbook_context(playbook: &Playbook) -> String {
    let mut out = format!(
        "### {}\nGoal: {}\nSuccess rate: {}",
        non_empty_or(playbook.name.trim(), "Untitled playbook"),
        non_empty_or(playbook.goal.trim(), "No goal recorded"),
        success_rate_text(playbook)
    );

    if !playbook.steps.is_empty() {
        out.push_str("\nSteps:");
        for step in playbook.steps.iter().take(6) {
            out.push_str("\n  ");
            out.push_str(&render_playbook_step(step));
        }

        if playbook.steps.len() > 6 {
            out.push_str(&format!(
                "\n  ... and {} more step{}",
                playbook.steps.len() - 6,
                plural(playbook.steps.len() - 6)
            ));
        }
    }

    out
}

fn render_playbook_step(step: &PlaybookStep) -> String {
    let mut out = format!(
        "{}. {} [{}]",
        step.index + 1,
        truncate_summary(step.description.trim(), 120),
        non_empty_or(step.action_kind.trim(), "unknown")
    );

    if !step.expected_signals.is_empty() {
        out.push_str(" -> ");
        out.push_str(&step.expected_signals.join(", "));
    }

    out
}

fn render_hit_context(hit: &KnowledgeQueryHit) -> String {
    let kind = kind_label(hit.entry.kind);
    let tier = tier_label(hit.entry.tier);
    let summary = truncate_summary(hit.entry.content.trim(), 240);
    format!("- [{kind} / {tier}] {:.2}\n  {summary}", hit.total_score)
}

fn success_rate_suffix(playbook: &Playbook) -> String {
    playbook
        .success_rate()
        .map(|rate| format!(" ({:.0}% success)", rate * 100.0))
        .unwrap_or_default()
}

fn success_rate_text(playbook: &Playbook) -> String {
    playbook
        .success_rate()
        .map(|rate| format!("{:.0}% success", rate * 100.0))
        .unwrap_or_else(|| "unknown".to_string())
}

fn tier_label(tier: KnowledgeTier) -> &'static str {
    match tier {
        KnowledgeTier::Persistent => "P",
        KnowledgeTier::Consolidated => "C",
        KnowledgeTier::Working => "W",
        KnowledgeTier::Transient => "T",
    }
}

fn kind_label(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Insight => "insight",
        KnowledgeKind::Heuristic => "heuristic",
        KnowledgeKind::AntiKnowledge => "anti-pattern",
        KnowledgeKind::Warning => "warning",
        KnowledgeKind::CausalLink => "causal",
        KnowledgeKind::StrategyFragment => "strategy",
    }
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn truncate_summary(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if max_chars == 0 {
        return String::new();
    }

    let mut chars = text.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        match chars.next() {
            Some(ch) => out.push(ch),
            None => return out,
        }
    }

    if chars.next().is_some() {
        out.push_str("...");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use tempfile::TempDir;

    fn sample_hit(content: &str) -> KnowledgeQueryHit {
        KnowledgeQueryHit {
            entry: roko_neuro::KnowledgeEntry {
                id: "kn-1".to_string(),
                kind: KnowledgeKind::Heuristic,
                content: content.to_string(),
                confidence: 0.92,
                tier: KnowledgeTier::Persistent,
                created_at: Utc::now(),
                ..Default::default()
            },
            total_score: 0.91,
            breakdown: roko_neuro::KnowledgeQueryBreakdown {
                keyword_score: 0.8,
                effective_confidence: 0.9,
                recency_factor: 1.0,
                emotional_boost: 1.0,
                hdc_similarity: None,
            },
        }
    }

    fn sample_playbook() -> Playbook {
        let mut playbook = Playbook::new("fix-concurrency", "Resolve Send + Sync errors");
        playbook.success_count = 3;
        playbook.failure_count = 1;
        playbook.steps.push(PlaybookStep::new(
            0,
            "Replace shared HashMap with DashMap",
            "edit_file",
            vec!["compile_ok".into()],
        ));
        playbook.steps.push(PlaybookStep::new(
            1,
            "Run cargo test to confirm the fix",
            "run_command",
            vec!["tests_pass".into()],
        ));
        playbook
    }

    #[test]
    fn card_and_context_include_results() {
        let knowledge = DispatchKnowledge {
            hits: vec![sample_hit(
                "Prefer smaller retries after gate failures because they keep the feedback loop tight.",
            )],
            playbooks: vec![sample_playbook()],
        };

        let card = knowledge.card().expect("card");
        assert!(card.title.contains("2 results"));
        assert!(card.body.contains("Playbooks"));
        assert!(card.body.contains("Knowledge"));
        assert!(card.body.contains("75% success"));
        assert!(card.body.contains("[P] 0.91"));
        assert!(card.body.contains("..."));

        let context = knowledge.context_text();
        assert!(context.contains("Relevant playbooks"));
        assert!(context.contains("Steps:"));
        assert!(context.contains("1. Replace shared HashMap with DashMap [edit_file]"));
        assert!(context.contains("Relevant knowledge"));
        assert!(context.contains("[heuristic / P] 0.91"));
    }

    #[tokio::test]
    async fn missing_stores_return_empty_results() {
        let tmp = TempDir::new().expect("tempdir");
        let knowledge = query_dispatch_knowledge(tmp.path(), "fix the bug").await;

        assert!(knowledge.is_empty());
        assert!(knowledge.card().is_none());
        assert!(knowledge.context_text().is_empty());
    }

    #[test]
    fn context_merges_in_the_expected_direction() {
        assert_eq!(append_context("base", "extra"), "base\n\nextra");
        assert_eq!(prepend_context("base", "extra"), "extra\n\nbase");
        assert_eq!(append_context("", "extra"), "extra");
        assert_eq!(prepend_context("base", ""), "base");
    }
}

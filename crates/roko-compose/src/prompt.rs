//! Prompt assembly: compose typed sections into a final prompt under a token budget.

use std::fmt::Write as _;

use roko_core::{
    Body, Budget, Composer, Context, Engram, Kind, Provenance, Scorer,
    error::{Result, RokoError},
};
use serde::{Deserialize, Serialize};

/// Estimate token count for a text blob.
///
/// Uses the GPT/Claude rule-of-thumb of ≈4 bytes per token. This is coarse
/// but fast — adequate for budget accounting. Real tokenization would
/// require calling the provider tokenizer (unavailable offline).
#[must_use]
pub const fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

// ─── Section payload ───────────────────────────────────────────────────────

/// Priority of a prompt section. Higher priorities survive budget pressure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionPriority {
    /// Drop first under pressure (fluff, historical context).
    Low = 0,
    /// Keep if possible (conventions, hints).
    #[default]
    Normal = 1,
    /// Essential to the task (the actual task, acceptance criteria).
    High = 2,
    /// Never drop (role instructions, safety hooks).
    Critical = 3,
}

/// Which cache layer this section belongs to, in an LLM prefix-cache model.
///
/// Stable layers must come before volatile layers so the model can reuse
/// the longest possible KV-cache prefix across related turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheLayer {
    /// System prompt, role instructions, tool definitions.
    Role = 0,
    /// Workspace map, cross-plan context, durable project context.
    Workspace = 1,
    /// Plan/task brief content that is stable within a plan.
    #[default]
    Plan = 2,
    /// Turn-local content such as review feedback or error output.
    Volatile = 3,
}

/// Where in the final prompt the section should be placed.
///
/// U-shaped attention (Start/End) defeats "Lost in the Middle" effects for
/// large context windows. Use `Start` for role/instructions and `End` for
/// the current task.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Place near the top (role prompt, critical instructions).
    Start,
    /// Middle — most vulnerable to attention loss.
    #[default]
    Middle,
    /// Place near the bottom (current task, recent errors).
    End,
}

/// A single labeled fragment of a prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSection {
    /// Human-readable label (e.g. "role", "task", "`workspace_map`").
    pub name: String,
    /// The section's text content.
    pub content: String,
    /// Priority for budget-pressure dropping.
    pub priority: SectionPriority,
    /// Cache layer for LLM prefix-cache optimization.
    pub cache_layer: CacheLayer,
    /// Where in the final prompt to place this section.
    pub placement: Placement,
    /// Optional per-section token ceiling. When set, the composer truncates
    /// content that exceeds `hard_cap` tokens before inclusion (preserving
    /// the head of the content). `None` means "unlimited, subject only to
    /// the overall budget".
    pub hard_cap: Option<usize>,
}

impl PromptSection {
    /// Create a new section with defaults (Normal priority, Plan cache layer, Middle placement).
    #[must_use]
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            priority: SectionPriority::Normal,
            cache_layer: CacheLayer::Plan,
            placement: Placement::Middle,
            hard_cap: None,
        }
    }

    /// Set the priority.
    #[must_use]
    pub const fn with_priority(mut self, p: SectionPriority) -> Self {
        self.priority = p;
        self
    }

    /// Set the cache layer.
    #[must_use]
    pub const fn with_cache_layer(mut self, l: CacheLayer) -> Self {
        self.cache_layer = l;
        self
    }

    /// Set the placement.
    #[must_use]
    pub const fn with_placement(mut self, p: Placement) -> Self {
        self.placement = p;
        self
    }

    /// Attach a per-section hard token cap. The composer will truncate
    /// content to fit before inclusion.
    #[must_use]
    pub const fn with_hard_cap(mut self, tokens: usize) -> Self {
        self.hard_cap = Some(tokens);
        self
    }

    /// Approximate token count (≈4 bytes per token).
    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        estimate_tokens(&self.content)
    }

    /// Return this section with content truncated to `hard_cap` tokens (if set).
    ///
    /// Truncation keeps the head and appends `…[truncated N tokens]`. If
    /// `hard_cap` is unset or already satisfied, returns self unchanged.
    #[must_use]
    pub fn enforce_hard_cap(mut self) -> Self {
        let Some(cap) = self.hard_cap else {
            return self;
        };
        let current = self.estimated_tokens();
        if current <= cap {
            return self;
        }
        // Truncate content to roughly `cap` tokens (4 bytes/token).
        let keep_bytes = cap.saturating_mul(4);
        if keep_bytes < self.content.len() {
            // Find a char boundary at or below keep_bytes.
            let mut boundary = keep_bytes;
            while boundary > 0 && !self.content.is_char_boundary(boundary) {
                boundary -= 1;
            }
            let dropped = current - cap;
            let mut truncated = self.content[..boundary].to_string();
            let _ = write!(truncated, "…[truncated {dropped} tokens]");
            self.content = truncated;
        }
        self
    }

    /// Wrap this section in a `Engram<Kind::PromptSection>`.
    ///
    /// # Errors
    ///
    /// Returns an error if the section cannot be serialized to JSON.
    pub fn into_signal(self) -> Result<Engram> {
        let body = Body::from_json(&self)?;
        Ok(Engram::builder(Kind::PromptSection)
            .body(body)
            .provenance(Provenance::trusted("prompt_section"))
            .tag("name", &self.name)
            .tag("priority", priority_tag(self.priority))
            .tag("cache_layer", cache_tag(self.cache_layer))
            .build())
    }

    /// Extract a `PromptSection` from a signal's body.
    ///
    /// # Errors
    ///
    /// Returns an error if the signal body isn't a `PromptSection` JSON value.
    pub fn from_signal(signal: &Engram) -> Result<Self> {
        signal.body.as_json()
    }
}

const fn priority_tag(p: SectionPriority) -> &'static str {
    match p {
        SectionPriority::Low => "low",
        SectionPriority::Normal => "normal",
        SectionPriority::High => "high",
        SectionPriority::Critical => "critical",
    }
}

const fn cache_tag(l: CacheLayer) -> &'static str {
    match l {
        CacheLayer::Role => "role",
        CacheLayer::Workspace => "workspace",
        CacheLayer::Plan => "plan",
        CacheLayer::Volatile => "volatile",
    }
}

// ─── Composer ──────────────────────────────────────────────────────────────

/// Assembles `Engram<PromptSection>` inputs into a final `Engram<Prompt>`
/// under a token budget.
///
/// # Algorithm
///
/// 1. Decode all input sections from signal bodies.
/// 2. Drop any that don't decode (provenance-tainted or wrong kind).
/// 3. Sort by `cache_layer` ASC (cache-wins first) then priority DESC.
/// 4. Greedily include sections until budget is exhausted — but NEVER drop
///    `Critical` priority sections (that's a contract violation).
/// 5. Order the kept sections by placement (Start → Middle → End), ties
///    broken by `cache_layer` order.
/// 6. Concatenate with section headers, wrap in a `Engram<Kind::Prompt>`.
///
/// # Budget
///
/// Respects `Budget::max_tokens`. If unset, only `max_signals` limits
/// inclusion. If a critical section alone exceeds `max_tokens`, the composer
/// returns an error rather than silently dropping it.
pub struct PromptComposer {
    name: String,
    /// Include section headers (e.g. `--- role ---`) in the output.
    include_headers: bool,
}

impl Default for PromptComposer {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptComposer {
    /// Create a default prompt composer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: "prompt_composer".into(),
            include_headers: true,
        }
    }

    /// Don't emit section headers in the output (pure concatenation).
    #[must_use]
    pub const fn without_headers(mut self) -> Self {
        self.include_headers = false;
        self
    }

    /// Override the composer's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Composer for PromptComposer {
    fn compose(
        &self,
        signals: &[Engram],
        budget: &Budget,
        _scorer: &dyn Scorer,
        _ctx: &Context,
    ) -> Result<Engram> {
        // Decode sections; skip anything that doesn't parse. Enforce any
        // per-section hard cap at decode time so downstream accounting
        // reflects the actual bytes that will land in the prompt.
        // Split critical sections out — they MUST be included.
        let (critical, optional): (Vec<_>, Vec<_>) = signals
            .iter()
            .filter_map(|s| PromptSection::from_signal(s).ok().map(|p| (p, s)))
            .map(|(p, s)| (p.enforce_hard_cap(), s))
            .partition(|(p, _)| p.priority == SectionPriority::Critical);

        let critical_tokens: usize = critical.iter().map(|(s, _)| s.estimated_tokens()).sum();

        if let Some(max) = budget.max_tokens {
            if critical_tokens > max {
                return Err(RokoError::BudgetExceeded {
                    dimension: "tokens",
                    used: critical_tokens,
                    limit: max,
                });
            }
        }

        // Sort optional sections: cache_layer ASC (stable layers first), then priority DESC.
        let mut optional = optional;
        optional.sort_by(|a, b| {
            a.0.cache_layer
                .cmp(&b.0.cache_layer)
                .then_with(|| (b.0.priority as u8).cmp(&(a.0.priority as u8)))
        });

        // Greedy inclusion: take optional sections until we'd exceed budget.
        let remaining_tokens = budget
            .max_tokens
            .map_or(usize::MAX, |m| m.saturating_sub(critical_tokens));
        let remaining_signals = budget
            .max_signals
            .map_or(usize::MAX, |m| m.saturating_sub(critical.len()));

        let mut kept: Vec<(PromptSection, &Engram)> = critical;
        let mut token_total = critical_tokens;

        for (section, source_signal) in optional {
            let toks = section.estimated_tokens();
            if token_total.saturating_add(toks) > budget.max_tokens.unwrap_or(usize::MAX) {
                continue; // too big — skip
            }
            if kept.len() >= remaining_signals.saturating_add(critical_count(&kept)) {
                break;
            }
            kept.push((section, source_signal));
            token_total += toks;
            if token_total >= remaining_tokens.saturating_add(critical_tokens) {
                break;
            }
        }

        // Order by placement for final output (U-shaped).
        kept.sort_by(|a, b| {
            placement_order(a.0.placement)
                .cmp(&placement_order(b.0.placement))
                .then_with(|| a.0.cache_layer.cmp(&b.0.cache_layer))
        });

        // Concatenate.
        let prompt_text = render_sections(&kept, self.include_headers);

        // Build the output signal. Lineage = all source signal ids.
        let lineage: Vec<_> = kept.iter().map(|(_, s)| s.id).collect();
        let sig = Engram::builder(Kind::Prompt)
            .body(Body::text(prompt_text))
            .provenance(Provenance::trusted(&self.name))
            .lineage(lineage)
            .tag("sections", kept.len().to_string())
            .tag("tokens", token_total.to_string())
            .build();
        Ok(sig)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Context-gathering strategy used to produce a prompt.
///
/// Mirrors Mori's `context_strategy` field on `PromptBuild` — a recorded
/// note of how the prompt was assembled so downstream observability can
/// attribute success/failure to strategy choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategy {
    /// Full budget allotted, all sections considered.
    #[default]
    Full,
    /// Complexity-trimmed (PRD/research/decomposition dropped for simple tasks).
    Trimmed,
    /// Retry iteration — prior error digest prioritized.
    Retry,
    /// Minimal (quick reviewer, auto-fixer): smallest viable context.
    Minimal,
}

/// The output of a prompt assembly: the text plus metadata about how
/// it was assembled.
///
/// Matches Mori's `PromptBuild` in `apps/mori/src/orchestrator/prompts/assembly.rs`.
/// Used by agent spawners to attach the prompt to a turn while recording
/// metadata for observability (cache hit rate, playbook retrieval count, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBuild {
    /// The assembled prompt text.
    pub prompt: String,
    /// Which strategy produced this prompt.
    pub context_strategy: ContextStrategy,
    /// Whether the prefix cache hit (estimated from stable prefix size).
    pub cache_hit: bool,
    /// Number of playbook episodes retrieved and injected.
    pub playbook_hits: usize,
    /// Estimated token count of the final prompt.
    pub tokens: usize,
    /// Number of sections that survived budget pressure.
    pub sections_kept: usize,
    /// Number of sections dropped by budget pressure.
    pub sections_dropped: usize,
}

impl PromptBuild {
    /// Construct a prompt build from text, filling metadata defaults.
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        let prompt = prompt.into();
        let tokens = estimate_tokens(&prompt);
        Self {
            prompt,
            context_strategy: ContextStrategy::default(),
            cache_hit: false,
            playbook_hits: 0,
            tokens,
            sections_kept: 0,
            sections_dropped: 0,
        }
    }

    /// Set the context strategy used.
    #[must_use]
    pub const fn with_strategy(mut self, s: ContextStrategy) -> Self {
        self.context_strategy = s;
        self
    }

    /// Mark whether the prefix cache hit.
    #[must_use]
    pub const fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }

    /// Record how many playbook episodes were injected.
    #[must_use]
    pub const fn with_playbook_hits(mut self, n: usize) -> Self {
        self.playbook_hits = n;
        self
    }

    /// Record kept/dropped section counts.
    #[must_use]
    pub const fn with_section_counts(mut self, kept: usize, dropped: usize) -> Self {
        self.sections_kept = kept;
        self.sections_dropped = dropped;
        self
    }
}

fn critical_count(kept: &[(PromptSection, &Engram)]) -> usize {
    kept.iter()
        .filter(|(p, _)| p.priority == SectionPriority::Critical)
        .count()
}

const fn placement_order(p: Placement) -> u8 {
    match p {
        Placement::Start => 0,
        Placement::Middle => 1,
        Placement::End => 2,
    }
}

fn render_sections(kept: &[(PromptSection, &Engram)], headers: bool) -> String {
    let mut out = String::new();
    for (section, _) in kept {
        if headers {
            out.push_str("--- ");
            out.push_str(&section.name);
            out.push_str(" ---\n");
        }
        out.push_str(&section.content);
        if !section.content.ends_with('\n') {
            out.push('\n');
        }
        if headers {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_std::NoOpScorer;

    fn section(name: &str, content: &str, pri: SectionPriority) -> Engram {
        PromptSection::new(name, content)
            .with_priority(pri)
            .into_signal()
            .unwrap()
    }

    #[test]
    fn estimated_tokens_is_roughly_length_over_4() {
        let s = PromptSection::new("x", "hello world 12345");
        // 17 chars → 5 tokens (4.25, rounded up)
        assert_eq!(s.estimated_tokens(), 5);
    }

    #[test]
    fn section_roundtrips_signal() {
        let s = PromptSection::new("role", "you are an agent")
            .with_priority(SectionPriority::Critical)
            .with_cache_layer(CacheLayer::Role)
            .with_placement(Placement::Start);
        let sig = s.clone().into_signal().unwrap();
        let decoded = PromptSection::from_signal(&sig).unwrap();
        assert_eq!(decoded, s);
    }

    #[test]
    fn cache_layer_ordering() {
        assert!(CacheLayer::Role < CacheLayer::Workspace);
        assert!(CacheLayer::Workspace < CacheLayer::Plan);
        assert!(CacheLayer::Plan < CacheLayer::Volatile);

        let mut sections = vec![
            PromptSection::new("volatile", "v").with_cache_layer(CacheLayer::Volatile),
            PromptSection::new("plan", "p").with_cache_layer(CacheLayer::Plan),
            PromptSection::new("role", "r").with_cache_layer(CacheLayer::Role),
            PromptSection::new("workspace", "w").with_cache_layer(CacheLayer::Workspace),
        ];
        sections.sort_by_key(|section| section.cache_layer);

        let ordered: Vec<_> = sections.into_iter().map(|section| section.name).collect();
        assert_eq!(ordered, vec!["role", "workspace", "plan", "volatile"]);
    }

    #[test]
    fn composer_includes_all_sections_when_under_budget() {
        let composer = PromptComposer::new();
        let sections = [
            section("role", "you are an agent", SectionPriority::Critical),
            section("task", "implement feature X", SectionPriority::High),
            section("hint", "prefer small diffs", SectionPriority::Low),
        ];
        let out = composer
            .compose(
                &sections,
                &Budget::unlimited(),
                &NoOpScorer,
                &Context::at(0),
            )
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("you are an agent"));
        assert!(text.contains("implement feature X"));
        assert!(text.contains("prefer small diffs"));
        assert_eq!(out.tag("sections"), Some("3"));
    }

    #[test]
    fn composer_drops_low_priority_under_pressure() {
        let composer = PromptComposer::new().without_headers();
        // High-pri section ~10 tokens; low-pri ~100 tokens; budget 20 tokens.
        let sections = [
            section("keep", "small important section", SectionPriority::High),
            section("drop", &"word ".repeat(100), SectionPriority::Low),
        ];
        let out = composer
            .compose(&sections, &Budget::tokens(20), &NoOpScorer, &Context::at(0))
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("small important"));
        assert!(!text.contains("word word"));
    }

    #[test]
    fn composer_never_drops_critical_sections() {
        let composer = PromptComposer::new();
        let sections = [
            section("role", "you are an agent", SectionPriority::Critical),
            section("fluff", &"x".repeat(1000), SectionPriority::Low),
        ];
        // Budget that can only fit the role, not the fluff.
        let out = composer
            .compose(&sections, &Budget::tokens(20), &NoOpScorer, &Context::at(0))
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("you are an agent"));
        assert!(!text.contains("xxxxxx"));
    }

    #[test]
    fn composer_errors_when_critical_exceeds_budget() {
        let composer = PromptComposer::new();
        let sections = [section(
            "gigantic",
            &"x".repeat(10_000),
            SectionPriority::Critical,
        )];
        let result = composer.compose(
            &sections,
            &Budget::tokens(100),
            &NoOpScorer,
            &Context::at(0),
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            roko_core::RokoError::BudgetExceeded { .. }
        ));
    }

    #[test]
    fn composer_places_sections_in_u_shape() {
        let composer = PromptComposer::new().without_headers();
        let start_sig = PromptSection::new("role", "I am start")
            .with_placement(Placement::Start)
            .into_signal()
            .unwrap();
        let middle_sig = PromptSection::new("ctx", "I am middle")
            .with_placement(Placement::Middle)
            .into_signal()
            .unwrap();
        let end_sig = PromptSection::new("task", "I am end")
            .with_placement(Placement::End)
            .into_signal()
            .unwrap();

        // Pass in scrambled order.
        let out = composer
            .compose(
                &[end_sig, start_sig, middle_sig],
                &Budget::unlimited(),
                &NoOpScorer,
                &Context::at(0),
            )
            .unwrap();
        let text = out.body.as_text().unwrap();
        // Start section should appear before middle, middle before end.
        let start_pos = text.find("I am start").unwrap();
        let middle_pos = text.find("I am middle").unwrap();
        let end_pos = text.find("I am end").unwrap();
        assert!(start_pos < middle_pos);
        assert!(middle_pos < end_pos);
    }

    #[test]
    fn composer_output_has_lineage_of_inputs() {
        let composer = PromptComposer::new();
        let sections = [
            section("a", "a content", SectionPriority::High),
            section("b", "b content", SectionPriority::High),
        ];
        let input_ids: Vec<_> = sections.iter().map(|s| s.id).collect();
        let out = composer
            .compose(
                &sections,
                &Budget::unlimited(),
                &NoOpScorer,
                &Context::at(0),
            )
            .unwrap();
        assert_eq!(out.lineage.len(), 2);
        assert!(out.lineage.contains(&input_ids[0]));
        assert!(out.lineage.contains(&input_ids[1]));
    }

    #[test]
    fn composer_ignores_non_section_signals() {
        let composer = PromptComposer::new();
        let real_section = section("task", "implement X", SectionPriority::High);
        let fake = Engram::builder(Kind::Task)
            .body(Body::text("this is not a section"))
            .build();
        let out = composer
            .compose(
                &[real_section, fake],
                &Budget::unlimited(),
                &NoOpScorer,
                &Context::at(0),
            )
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("implement X"));
        assert!(!text.contains("not a section"));
    }

    #[test]
    fn composer_respects_max_signals() {
        let composer = PromptComposer::new();
        let sections: Vec<_> = (0..10)
            .map(|i| {
                section(
                    &format!("s{i}"),
                    &format!("content{i}"),
                    SectionPriority::Normal,
                )
            })
            .collect();
        let out = composer
            .compose(
                &sections,
                &Budget {
                    max_tokens: None,
                    max_signals: Some(3),
                    max_bytes: None,
                    max_wall_ms: None,
                },
                &NoOpScorer,
                &Context::at(0),
            )
            .unwrap();
        assert_eq!(out.tag("sections"), Some("3"));
    }

    #[test]
    fn hard_cap_truncates_oversized_content() {
        let s = PromptSection::new("big", "x".repeat(400))
            .with_hard_cap(10)
            .enforce_hard_cap();
        // 400 bytes → 100 tokens; capped to 10 → keeps ≤40 bytes + truncation note
        assert!(s.estimated_tokens() <= 30); // allow room for truncation marker
        assert!(s.content.contains("[truncated"));
    }

    #[test]
    fn hard_cap_noop_when_below_cap() {
        let s = PromptSection::new("small", "hello")
            .with_hard_cap(100)
            .enforce_hard_cap();
        assert_eq!(s.content, "hello");
    }

    #[test]
    fn hard_cap_noop_when_unset() {
        let s = PromptSection::new("any", "x".repeat(1000)).enforce_hard_cap();
        assert_eq!(s.content.len(), 1000);
    }

    #[test]
    fn composer_enforces_per_section_hard_cap() {
        let composer = PromptComposer::new().without_headers();
        let sig = PromptSection::new("bounded", "a".repeat(400))
            .with_priority(SectionPriority::High)
            .with_hard_cap(5)
            .into_signal()
            .unwrap();
        let out = composer
            .compose(&[sig], &Budget::unlimited(), &NoOpScorer, &Context::at(0))
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("[truncated"));
        // Well below the original 400 bytes.
        assert!(text.len() < 60);
    }

    #[test]
    fn prompt_build_records_metadata() {
        let pb = PromptBuild::new("hello world")
            .with_strategy(ContextStrategy::Trimmed)
            .with_cache_hit(true)
            .with_playbook_hits(3)
            .with_section_counts(7, 2);
        assert_eq!(pb.context_strategy, ContextStrategy::Trimmed);
        assert!(pb.cache_hit);
        assert_eq!(pb.playbook_hits, 3);
        assert_eq!(pb.sections_kept, 7);
        assert_eq!(pb.sections_dropped, 2);
        assert_eq!(pb.tokens, estimate_tokens("hello world"));
    }

    #[test]
    fn estimate_tokens_rounds_up_four_byte_chunks() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn headers_can_be_disabled() {
        let composer = PromptComposer::new().without_headers();
        let s = section("role", "agent here", SectionPriority::Critical);
        let out = composer
            .compose(&[s], &Budget::unlimited(), &NoOpScorer, &Context::at(0))
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(!text.contains("---"));
        assert!(text.contains("agent here"));
    }
}

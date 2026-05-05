//! Prompt assembly: compose typed sections into a final prompt under a token budget.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use roko_core::{
    Body, Budget, Compose, Context, Kind, PromptSectionAudit, Provenance, Signal,
    error::{Result, RokoError},
};
use serde::{Deserialize, Serialize};

use crate::auction::{
    AffectModulation, AuctionDiagnostics, LearningBidder, VcgAllocation, VcgBid, vcg_allocate,
};
use crate::foraging::{MultiPatchForager, should_stop_searching};
use crate::strategy::{CompositionStrategy, DEFAULT_VCG_WARMUP_OBSERVATIONS};

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

/// Which cognitive subsystem is bidding for prompt budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionBidder {
    /// Durable knowledge retrieved from Neuro.
    Neuro,
    /// Affect or somatic guidance from Daimon.
    Daimon,
    /// Recent turns, retries, and prior task outputs.
    IterationMemory,
    /// Symbols, files, and structural workspace context.
    CodeIntelligence,
    /// Skills, playbooks, and distilled reusable rules.
    PlaybookRules,
    /// Research memos and external domain context.
    Research,
    /// Task brief, plan brief, verification, PRD slices, and related directives.
    #[default]
    TaskContext,
    /// Predictions, warnings, or forecast-like oracle outputs.
    Oracles,
}

/// A single labeled fragment of a prompt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSection {
    /// Stable section identifier. Defaults to `prompt:<normalized name>`.
    #[serde(default)]
    pub section_id: String,
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
    /// Which subsystem is bidding for this section's inclusion.
    #[serde(default)]
    pub bidder: AttentionBidder,
    /// Stable source type key, if the section was derived from a structured source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    /// Stable source identifier, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Compact provenance label. Raw section content must not be copied here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    /// Experiment id that caused this section to be considered, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
}

impl PromptSection {
    /// Create a new section with defaults (Normal priority, Plan cache layer, Middle placement).
    #[must_use]
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            section_id: stable_section_id("prompt", &name, None, None),
            name,
            content: content.into(),
            priority: SectionPriority::Normal,
            cache_layer: CacheLayer::Plan,
            placement: Placement::Middle,
            hard_cap: None,
            bidder: AttentionBidder::default(),
            source_type: None,
            source_id: None,
            provenance: None,
            experiment_id: None,
        }
    }

    /// Override the stable section id.
    #[must_use]
    pub fn with_section_id(mut self, section_id: impl Into<String>) -> Self {
        self.section_id = section_id.into();
        self
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

    /// Set the subsystem bidder for this section.
    #[must_use]
    pub const fn with_bidder(mut self, bidder: AttentionBidder) -> Self {
        self.bidder = bidder;
        self
    }

    /// Attach compact source metadata.
    #[must_use]
    pub fn with_source(
        mut self,
        source_type: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Self {
        self.source_type = Some(source_type.into());
        self.source_id = Some(source_id.into());
        if self.section_id.trim().is_empty()
            || self.section_id == stable_section_id("prompt", &self.name, None, None)
        {
            self.section_id = stable_section_id(
                "prompt",
                &self.name,
                self.source_type.as_deref(),
                self.source_id.as_deref(),
            );
        }
        self
    }

    /// Attach compact provenance metadata.
    #[must_use]
    pub fn with_provenance(mut self, provenance: impl Into<String>) -> Self {
        self.provenance = Some(provenance.into());
        self
    }

    /// Attach an experiment id.
    #[must_use]
    pub fn with_experiment_id(mut self, experiment_id: impl Into<String>) -> Self {
        self.experiment_id = Some(experiment_id.into());
        self
    }

    /// Stable action id for future prompt/context bandits.
    #[must_use]
    pub fn action_id(&self) -> String {
        section_action_id(
            "prompt_section",
            &self.name,
            self.source_type.as_deref(),
            self.source_id.as_deref(),
            self.experiment_id.as_deref(),
        )
    }

    /// Stable section id, deriving one for deserialized legacy sections.
    #[must_use]
    pub fn stable_section_id(&self) -> String {
        if self.section_id.trim().is_empty() {
            stable_section_id(
                "prompt",
                &self.name,
                self.source_type.as_deref(),
                self.source_id.as_deref(),
            )
        } else {
            self.section_id.clone()
        }
    }

    /// Build a raw-content-free audit row for this section.
    #[must_use]
    pub fn audit_row(
        &self,
        included: bool,
        tokens_used: usize,
        reason: impl Into<String>,
    ) -> PromptSectionAudit {
        PromptSectionAudit {
            section_id: self.stable_section_id(),
            section_name: self.name.clone(),
            action_id: self.action_id(),
            included,
            estimated_tokens: self.estimated_tokens(),
            tokens_used,
            token_budget: self.hard_cap,
            priority: priority_tag(self.priority).to_string(),
            cache_layer: cache_tag(self.cache_layer).to_string(),
            placement: placement_tag(self.placement).to_string(),
            bidder: bidder_tag(self.bidder).to_string(),
            source_type: self.source_type.clone(),
            source_id: self.source_id.clone(),
            provenance: self.provenance.clone(),
            experiment_id: self.experiment_id.clone(),
            reason: reason.into(),
        }
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

    /// Wrap this section in a `Signal<Kind::PromptSection>`.
    ///
    /// # Errors
    ///
    /// Returns an error if the section cannot be serialized to JSON.
    pub fn into_signal(self) -> Result<Signal> {
        let body = Body::from_json(&self)?;
        Ok(Signal::builder(Kind::PromptSection)
            .body(body)
            .provenance(Provenance::trusted("prompt_section"))
            .tag("section_id", self.stable_section_id())
            .tag("action_id", self.action_id())
            .tag("name", &self.name)
            .tag("priority", priority_tag(self.priority))
            .tag("cache_layer", cache_tag(self.cache_layer))
            .tag("bidder", bidder_tag(self.bidder))
            .build())
    }

    /// Extract a `PromptSection` from a signal's body.
    ///
    /// # Errors
    ///
    /// Returns an error if the signal body isn't a `PromptSection` JSON value.
    pub fn from_signal(signal: &Signal) -> Result<Self> {
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

const fn bidder_tag(bidder: AttentionBidder) -> &'static str {
    match bidder {
        AttentionBidder::Neuro => "neuro",
        AttentionBidder::Daimon => "daimon",
        AttentionBidder::IterationMemory => "iteration_memory",
        AttentionBidder::CodeIntelligence => "code_intelligence",
        AttentionBidder::PlaybookRules => "playbook_rules",
        AttentionBidder::Research => "research",
        AttentionBidder::TaskContext => "task_context",
        AttentionBidder::Oracles => "oracles",
    }
}

const fn placement_tag(placement: Placement) -> &'static str {
    match placement {
        Placement::Start => "start",
        Placement::Middle => "middle",
        Placement::End => "end",
    }
}

fn stable_section_id(
    kind: &str,
    name: &str,
    source_type: Option<&str>,
    source_id: Option<&str>,
) -> String {
    let mut id = format!("{kind}:{}", normalize_action_part(name));
    if let Some(source_type) = source_type.and_then(non_empty) {
        id.push(':');
        id.push_str(&normalize_action_part(source_type));
    }
    if let Some(source_id) = source_id.and_then(non_empty) {
        id.push(':');
        id.push_str(&normalize_action_part(source_id));
    }
    id
}

fn section_action_id(
    kind: &str,
    name: &str,
    source_type: Option<&str>,
    source_id: Option<&str>,
    experiment_id: Option<&str>,
) -> String {
    let mut id = stable_section_id(kind, name, source_type, source_id);
    if let Some(experiment_id) = experiment_id.and_then(non_empty) {
        id.push_str("|experiment:");
        id.push_str(&normalize_action_part(experiment_id));
    }
    id
}

fn normalize_action_part(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_sep = false;
    for ch in raw.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_sep = false;
        } else if !last_sep {
            out.push('-');
            last_sep = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Signal tag containing a JSON-encoded [`CompositionManifest`].
pub const COMPOSITION_MANIFEST_TAG: &str = "composition_manifest";

/// Metadata from one prompt-composition pass.
///
/// This is intentionally raw-content-free: it records selection, auction, and
/// attribution identifiers without copying prompt text into telemetry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompositionManifest {
    /// Strategy requested by the caller.
    pub requested_strategy: CompositionStrategy,
    /// Strategy actually used after `Auto` resolution.
    pub selected_strategy: CompositionStrategy,
    /// Sections included in the rendered prompt.
    pub included: Vec<IncludedSectionMeta>,
    /// Sections excluded by budget or auction pressure.
    pub excluded: Vec<ExcludedSectionMeta>,
    /// VCG diagnostics when the selected strategy is VCG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcg_diagnostics: Option<AuctionDiagnostics>,
    /// Sum of estimated tokens included in the rendered prompt.
    pub total_tokens: usize,
    /// Budget token limit used for selection.
    pub token_budget_limit: Option<usize>,
}

impl CompositionManifest {
    /// Serialize for storage in a Signal tag.
    #[must_use]
    pub fn to_tag_value(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse a manifest from a Signal tag value.
    #[must_use]
    pub fn from_tag_value(value: &str) -> Option<Self> {
        serde_json::from_str(value).ok()
    }

    /// VCG payments keyed by section id.
    #[must_use]
    pub fn vcg_payments(&self) -> Vec<(String, f64)> {
        self.included
            .iter()
            .filter_map(|section| {
                section
                    .vcg_payment
                    .map(|payment| (section.section_id.clone(), payment))
            })
            .collect()
    }

    /// Included sections in the shape expected by [`crate::CostAttribution`].
    #[must_use]
    pub fn included_for_cost_attribution(&self) -> Vec<(String, String, AttentionBidder, usize)> {
        self.included
            .iter()
            .map(|section| {
                (
                    section.section_id.clone(),
                    section.name.clone(),
                    section.bidder,
                    section.estimated_tokens,
                )
            })
            .collect()
    }
}

/// Included section metadata for composition learning and attribution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IncludedSectionMeta {
    /// Stable section id.
    pub section_id: String,
    /// Stable action id for bandits/learning.
    pub action_id: String,
    /// Human-readable section name.
    pub name: String,
    /// Owning subsystem bidder.
    pub bidder: AttentionBidder,
    /// Estimated section tokens after hard caps.
    pub estimated_tokens: usize,
    /// Base section score before token-density conversion.
    pub score: f32,
    /// Selection bid value used by the chosen strategy.
    pub bid_value: f32,
    /// VCG payment when selected by VCG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcg_payment: Option<f64>,
    /// Inclusion reason.
    pub reason: String,
}

/// Excluded section metadata for composition diagnostics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExcludedSectionMeta {
    /// Stable section id.
    pub section_id: String,
    /// Stable action id for bandits/learning.
    pub action_id: String,
    /// Human-readable section name.
    pub name: String,
    /// Owning subsystem bidder.
    pub bidder: AttentionBidder,
    /// Estimated section tokens after hard caps.
    pub estimated_tokens: usize,
    /// Base section score before token-density conversion.
    pub score: f32,
    /// Selection bid value used by the chosen strategy.
    pub bid_value: f32,
    /// Exclusion reason.
    pub reason: String,
}

// ─── Compose ──────────────────────────────────────────────────────────────

/// Assembles `Signal<PromptSection>` inputs into a final `Signal<Prompt>`
/// under a token budget.
///
/// # Algorithm
///
/// 1. Decode all input sections from signal bodies.
/// 2. Drop any that don't decode (provenance-tainted or wrong kind).
/// 3. Sort by `cache_layer` ASC (cache-wins first) then priority DESC.
/// 4. Allocate optional sections with the configured composition strategy,
///    but NEVER drop `Critical` priority sections (that's a contract violation).
/// 5. Order the kept sections by placement (Start → Middle → End), ties
///    broken by `cache_layer` order.
/// 6. Concatenate with section headers, wrap in a `Signal<Kind::Prompt>`.
///
/// # Budget
///
/// Respects `Budget::max_tokens`. If unset, only `max_pulses` limits
/// inclusion. If a critical section alone exceeds `max_tokens`, the composer
/// returns an error rather than silently dropping it.
pub struct PromptComposer {
    name: String,
    /// Include section headers (e.g. `--- role ---`) in the output.
    include_headers: bool,
    /// Budget allocation strategy.
    composition_strategy: CompositionStrategy,
    /// Minimum observations per active bidder before `Auto` activates VCG.
    vcg_warmup_observations: u32,
    /// COMP-02: Per-subsystem learning bidders that adjust bids based on
    /// prior task outcomes. When populated, the composer multiplies each
    /// candidate's base bid by the bidder's learned section value.
    learning_bidders: HashMap<AttentionBidder, LearningBidder>,
    /// COMP-03: MVT foraging pre-pass. When set, the composer uses the
    /// foraging stopping rule to decide how many optional candidates to
    /// evaluate before committing to the auction.
    foraging: Option<MultiPatchForager>,
    /// COMP-04: HDC dedup similarity threshold. When > 0.0 and the `hdc`
    /// feature is enabled, candidates with content similarity above this
    /// threshold are deduplicated before the auction.
    hdc_dedup_threshold: f64,
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
            composition_strategy: CompositionStrategy::Auto,
            vcg_warmup_observations: DEFAULT_VCG_WARMUP_OBSERVATIONS,
            learning_bidders: HashMap::new(),
            foraging: None,
            hdc_dedup_threshold: 0.0,
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

    /// Select a budget-allocation strategy.
    #[must_use]
    pub const fn with_strategy(mut self, strategy: CompositionStrategy) -> Self {
        self.composition_strategy = strategy;
        self
    }

    /// Configure the `Auto` warmup threshold for VCG activation.
    #[must_use]
    pub const fn with_vcg_warmup_observations(mut self, observations: u32) -> Self {
        self.vcg_warmup_observations = observations;
        self
    }

    // ── COMP-02: VCG Learning Bidder integration ─────────────────────

    /// Register a [`LearningBidder`] for a subsystem. During composition,
    /// the bidder's learned section values are multiplied into the base bid
    /// density, replacing the prior-only fallback.
    pub fn register_bidder(&mut self, bidder: AttentionBidder, learning_bidder: LearningBidder) {
        self.learning_bidders.insert(bidder, learning_bidder);
    }

    /// Set learning bidders for multiple subsystems at once.
    #[must_use]
    pub fn with_learning_bidders(
        mut self,
        bidders: HashMap<AttentionBidder, LearningBidder>,
    ) -> Self {
        self.learning_bidders = bidders;
        self
    }

    /// Update all registered learning bidders after observing a task outcome.
    ///
    /// `included_sections`: names of sections that were included in the prompt.
    /// `gate_passed`: whether the downstream gate passed.
    pub fn update_bidders(&mut self, included_sections: &[String], gate_passed: bool) {
        for bidder in self.learning_bidders.values_mut() {
            for section_name in included_sections {
                bidder.update(section_name, true, gate_passed);
            }
        }
    }

    /// Update registered bidders from section-level cost attribution.
    pub fn update_bidders_with_cost(
        &mut self,
        section_costs: &[(AttentionBidder, String, bool, bool, f64, usize)],
    ) {
        for (
            bidder_id,
            section_name,
            was_included,
            gate_passed,
            attributed_cost_usd,
            estimated_tokens,
        ) in section_costs
        {
            if let Some(bidder) = self.learning_bidders.get_mut(bidder_id) {
                bidder.update_with_cost(
                    section_name,
                    *was_included,
                    *gate_passed,
                    *attributed_cost_usd,
                    *estimated_tokens,
                );
            }
        }
    }

    /// Borrow the current learning bidders (for persistence).
    #[must_use]
    pub fn learning_bidders(&self) -> &HashMap<AttentionBidder, LearningBidder> {
        &self.learning_bidders
    }

    fn bidder_observation_counts(
        &self,
        candidates: &[AuctionCandidate<'_>],
    ) -> HashMap<AttentionBidder, u32> {
        candidates
            .iter()
            .map(|candidate| candidate.section.bidder)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(|bidder| {
                let observations = self
                    .learning_bidders
                    .get(&bidder)
                    .map(LearningBidder::observation_count)
                    .unwrap_or(0);
                (bidder, observations)
            })
            .collect()
    }

    // ── COMP-03: MVT foraging pre-pass ──────────────────────────────

    /// Set a [`MultiPatchForager`] for context retrieval stopping decisions.
    ///
    /// When set, the composer uses the foraging stopping rule to limit how
    /// many optional candidates are evaluated before committing to the
    /// auction. This prevents wasting budget on diminishing-returns sources.
    #[must_use]
    pub fn with_foraging(mut self, forager: MultiPatchForager) -> Self {
        self.foraging = Some(forager);
        self
    }

    // ── COMP-04: HDC dedup ──────────────────────────────────────────

    /// Enable HDC-based deduplication of candidates before the auction.
    ///
    /// Candidates with content similarity above `threshold` (range 0.0..1.0)
    /// are deduplicated, keeping the higher-scoring one. The spec recommends
    /// 0.85. Requires the `hdc` feature to be enabled at compile time;
    /// otherwise this is a no-op.
    #[must_use]
    pub fn with_hdc_dedup(mut self, threshold: f64) -> Self {
        self.hdc_dedup_threshold = threshold.clamp(0.0, 1.0);
        self
    }
}

impl Compose for PromptComposer {
    fn compose(
        &self,
        signals: &[Signal],
        budget: &Budget,
        scorer: &dyn roko_core::traits::Score,
        ctx: &Context,
    ) -> Result<Signal> {
        // Decode sections; skip anything that doesn't parse. Enforce any
        // per-section hard cap at decode time so downstream accounting
        // reflects the actual bytes that will land in the prompt.
        // Split critical sections out — they MUST be included.
        let decoded_sections = signals
            .iter()
            .filter_map(|s| PromptSection::from_signal(s).ok().map(|p| (p, s)))
            .map(|(p, s)| (p.enforce_hard_cap(), s))
            .collect::<Vec<_>>();
        let decoded_section_names = decoded_sections
            .iter()
            .map(|(section, _)| section.name.clone())
            .collect::<Vec<_>>();
        let (critical, optional): (Vec<_>, Vec<_>) = decoded_sections
            .into_iter()
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

        let remaining_tokens = budget
            .max_tokens
            .map_or(usize::MAX, |m| m.saturating_sub(critical_tokens));
        let remaining_signals = budget
            .max_pulses
            .map_or(usize::MAX, |m| m.saturating_sub(critical.len()));

        let mut kept: Vec<(PromptSection, &Signal)> = critical;
        let mut token_total = critical_tokens;
        let affect = AuctionAffectState::from_context(ctx);

        // COMP-02: Compute bid density, incorporating learning bidders when available.
        let mut optional = optional
            .into_iter()
            .map(|(section, source_signal)| {
                let score = candidate_score(&section, source_signal, scorer, ctx);
                let token_cost = section.estimated_tokens().max(1) as f32;
                // Multiply by the learning bidder's posterior for this section.
                let learned_multiplier = self
                    .learning_bidders
                    .get(&section.bidder)
                    .map(|bidder| bidder.bid_with_cost(&section.name, 1.0) as f32)
                    .unwrap_or(1.0);
                let bid_value = score * learned_multiplier;
                AuctionCandidate {
                    score,
                    bid_value,
                    bid_density: bid_value / token_cost,
                    section,
                    source_signal,
                }
            })
            .collect::<Vec<_>>();

        // COMP-04: HDC-based deduplication before the auction.
        #[cfg(feature = "hdc")]
        if self.hdc_dedup_threshold > 0.0 {
            optional = hdc_dedup_candidates(optional, self.hdc_dedup_threshold);
        }

        // COMP-03: MVT foraging pre-pass — limit candidates when foraging
        // says sufficient context has been gathered.
        if let Some(forager) = &self.foraging {
            optional = foraging_prepass(optional, forager);
        }

        let bidder_observations = self.bidder_observation_counts(&optional);
        let selected_strategy = self
            .composition_strategy
            .resolve(&bidder_observations, self.vcg_warmup_observations);

        let (allocation, payment_summary, vcg_allocation) =
            if selected_strategy == CompositionStrategy::Vcg {
                let (allocation, payment_summary, vcg_allocation) =
                    select_vcg_candidates(&optional, remaining_tokens, remaining_signals, affect);
                (allocation, payment_summary, Some(vcg_allocation))
            } else {
                optional.sort_by(|a, b| {
                    b.bid_density
                        .partial_cmp(&a.bid_density)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.section.cache_layer.cmp(&b.section.cache_layer))
                        .then_with(|| (b.section.priority as u8).cmp(&(a.section.priority as u8)))
                });

                let allocation = select_optional_candidates(
                    &optional,
                    remaining_tokens,
                    remaining_signals,
                    affect.as_ref(),
                    None,
                );
                let payment_summary = vcg_payment_summary(
                    &optional,
                    &allocation.selected,
                    remaining_tokens,
                    remaining_signals,
                    affect.as_ref(),
                );
                (allocation, payment_summary, None)
            };

        for winner in &allocation.selected {
            let candidate = &optional[winner.candidate_index];
            token_total += candidate.section.estimated_tokens();
            kept.push((candidate.section.clone(), candidate.source_signal));
        }

        // Order by placement for final output (U-shaped).
        kept.sort_by(|a, b| {
            placement_order(a.0.placement)
                .cmp(&placement_order(b.0.placement))
                .then_with(|| a.0.cache_layer.cmp(&b.0.cache_layer))
        });

        let kept_section_names = kept
            .iter()
            .map(|(section, _)| section.name.clone())
            .collect::<Vec<_>>();
        let kept_section_action_ids = kept
            .iter()
            .map(|(section, _)| section.action_id())
            .collect::<Vec<_>>();
        let kept_name_set = kept_section_names
            .iter()
            .cloned()
            .collect::<HashSet<String>>();
        let dropped_section_names = decoded_section_names
            .iter()
            .filter(|name| !kept_name_set.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        let kept_action_id_set = kept_section_action_ids
            .iter()
            .cloned()
            .collect::<HashSet<String>>();
        let dropped_section_action_ids = optional
            .iter()
            .filter(|candidate| !kept_action_id_set.contains(&candidate.section.action_id()))
            .map(|candidate| candidate.section.action_id())
            .collect::<Vec<_>>();
        let manifest = build_composition_manifest(
            self.composition_strategy,
            selected_strategy,
            &kept,
            &optional,
            &allocation.selected,
            vcg_allocation.as_ref(),
            token_total,
            budget.max_tokens,
            scorer,
            ctx,
        );

        // Concatenate.
        let prompt_text = render_sections(&kept, self.include_headers);

        // Build the output signal. Lineage = all source signal ids.
        let lineage: Vec<_> = kept.iter().map(|(_, s)| s.id).collect();
        let sig = Signal::builder(Kind::Prompt)
            .body(Body::text(prompt_text))
            .provenance(Provenance::trusted(&self.name))
            .lineage(lineage)
            .tag("sections", kept.len().to_string())
            .tag("sections_decoded", decoded_section_names.len().to_string())
            .tag("sections_dropped", dropped_section_names.len().to_string())
            .tag("tokens", token_total.to_string())
            .tag("composition_strategy", strategy_tag(selected_strategy))
            .tag(
                "composition_strategy_requested",
                strategy_tag(self.composition_strategy),
            )
            .tag(COMPOSITION_MANIFEST_TAG, manifest.to_tag_value())
            .tag(
                "budget_tokens_limit",
                budget
                    .max_tokens
                    .map_or_else(|| "unlimited".to_string(), |limit| limit.to_string()),
            )
            .tag("kept_section_names", kept_section_names.join(","))
            .tag("dropped_section_names", dropped_section_names.join(","))
            .tag("kept_section_action_ids", kept_section_action_ids.join(","))
            .tag(
                "dropped_section_action_ids",
                dropped_section_action_ids.join(","),
            )
            .tag("distinct_bidders", bidder_count(&kept).to_string())
            .tag("auction_total_bid", format!("{:.4}", allocation.total_bid))
            .tag(
                "auction_total_payments",
                format!("{:.4}", payment_summary.total_payments),
            )
            .tag(
                "auction_urgency",
                format!(
                    "{:.4}",
                    affect
                        .as_ref()
                        .map_or(1.0, AuctionAffectState::urgency_multiplier)
                ),
            )
            .tag(
                "auction_affect_weight",
                format!(
                    "{:.4}",
                    affect
                        .as_ref()
                        .map_or(1.0, AuctionAffectState::affect_weight_multiplier)
                ),
            )
            .tag(
                "highest_payment_section",
                payment_summary
                    .highest_payment_section
                    .unwrap_or_else(|| "none".to_string()),
            )
            .tag(
                "highest_payment_value",
                format!("{:.4}", payment_summary.highest_payment_value),
            )
            .build();
        Ok(sig)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone)]
struct AuctionCandidate<'a> {
    score: f32,
    bid_value: f32,
    bid_density: f32,
    section: PromptSection,
    source_signal: &'a Signal,
}

#[derive(Clone, Copy, Debug)]
struct AuctionAffectState {
    pleasure: f32,
    arousal: f32,
    dominance: f32,
}

impl AuctionAffectState {
    fn from_context(ctx: &Context) -> Option<Self> {
        let pleasure = ctx.attr("roko.daimon.pleasure")?.parse::<f32>().ok()?;
        let arousal = ctx.attr("roko.daimon.arousal")?.parse::<f32>().ok()?;
        let dominance = ctx.attr("roko.daimon.dominance")?.parse::<f32>().ok()?;
        Some(Self {
            pleasure: pleasure.clamp(-1.0, 1.0),
            arousal: arousal.clamp(-1.0, 1.0),
            dominance: dominance.clamp(-1.0, 1.0),
        })
    }

    fn urgency_multiplier(&self) -> f32 {
        1.0 + self.arousal.clamp(0.0, 1.0) * 0.5
    }

    fn affect_weight_multiplier(&self) -> f32 {
        1.0 + 0.3 * (self.pleasure - 0.5).abs()
    }

    fn low_dominance_pressure(&self) -> f32 {
        (-self.dominance).clamp(0.0, 1.0)
    }

    fn low_pleasure_pressure(&self) -> f32 {
        (0.5 - self.pleasure).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug)]
struct SelectedCandidate {
    candidate_index: usize,
    adjusted_bid: f32,
}

#[derive(Clone, Debug, Default)]
struct AuctionAllocation {
    selected: Vec<SelectedCandidate>,
    total_bid: f32,
}

#[derive(Clone, Debug, Default)]
struct PaymentSummary {
    total_payments: f32,
    highest_payment_value: f32,
    highest_payment_section: Option<String>,
}

fn bidder_count(kept: &[(PromptSection, &Signal)]) -> usize {
    kept.iter()
        .map(|(section, _)| section.bidder)
        .collect::<HashSet<_>>()
        .len()
}

fn select_optional_candidates(
    candidates: &[AuctionCandidate<'_>],
    remaining_tokens: usize,
    remaining_signals: usize,
    affect: Option<&AuctionAffectState>,
    excluded_candidate: Option<usize>,
) -> AuctionAllocation {
    let mut allocation = AuctionAllocation::default();
    let mut bidder_wins: HashMap<AttentionBidder, usize> = HashMap::new();
    let mut used_tokens = 0usize;
    let mut remaining: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, _)| (Some(idx) != excluded_candidate).then_some(idx))
        .collect();

    while !remaining.is_empty()
        && used_tokens < remaining_tokens
        && allocation.selected.len() < remaining_signals
    {
        let mut winner_idx = None;
        let mut winner_bid = f32::MIN;

        for &idx in &remaining {
            let candidate = &candidates[idx];
            let toks = candidate.section.estimated_tokens();
            if used_tokens.saturating_add(toks) > remaining_tokens {
                continue;
            }
            let adjusted_bid = effective_candidate_bid(candidate, &bidder_wins, affect);
            if adjusted_bid > winner_bid {
                winner_bid = adjusted_bid;
                winner_idx = Some(idx);
            }
        }

        let Some(winner_idx) = winner_idx else {
            break;
        };
        if winner_bid <= 0.0 {
            break;
        }

        let winner = &candidates[winner_idx];
        let bidder = winner.section.bidder;
        *bidder_wins.entry(bidder).or_insert(0) += 1;
        used_tokens += winner.section.estimated_tokens();
        allocation.total_bid += winner_bid;
        allocation.selected.push(SelectedCandidate {
            candidate_index: winner_idx,
            adjusted_bid: winner_bid,
        });
        remaining.retain(|idx| *idx != winner_idx);
    }

    allocation
}

fn vcg_payment_summary(
    candidates: &[AuctionCandidate<'_>],
    winners: &[SelectedCandidate],
    remaining_tokens: usize,
    remaining_signals: usize,
    affect: Option<&AuctionAffectState>,
) -> PaymentSummary {
    let actual_total = winners
        .iter()
        .map(|winner| winner.adjusted_bid)
        .sum::<f32>();
    let mut summary = PaymentSummary::default();

    for winner in winners {
        let without_winner = select_optional_candidates(
            candidates,
            remaining_tokens,
            remaining_signals,
            affect,
            Some(winner.candidate_index),
        );
        let others_with_winner = actual_total - winner.adjusted_bid;
        let payment = (without_winner.total_bid - others_with_winner).max(0.0);
        summary.total_payments += payment;
        if payment > summary.highest_payment_value {
            summary.highest_payment_value = payment;
            summary.highest_payment_section =
                Some(candidates[winner.candidate_index].section.name.clone());
        }
    }

    summary
}

fn select_vcg_candidates(
    candidates: &[AuctionCandidate<'_>],
    remaining_tokens: usize,
    remaining_signals: usize,
    affect: Option<AuctionAffectState>,
) -> (AuctionAllocation, PaymentSummary, VcgAllocation) {
    let modulation = affect
        .map(AuctionAffectState::to_vcg_modulation)
        .unwrap_or_default();
    let bids = candidates
        .iter()
        .map(|candidate| {
            let raw_bid = candidate.bid_value.max(0.0) as f64;
            let valence = section_valence(&candidate.section);
            VcgBid {
                bidder: candidate.section.bidder,
                section_name: candidate.section.stable_section_id(),
                tokens: candidate.section.estimated_tokens(),
                raw_bid,
                adjusted_bid: modulation.adjust_bid(raw_bid, valence),
                valence,
            }
        })
        .collect::<Vec<_>>();
    let vcg_allocation = vcg_allocate(bids, remaining_tokens, &modulation);

    let candidate_by_id = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.section.stable_section_id(), index))
        .collect::<HashMap<_, _>>();
    let payment_by_id = vcg_allocation
        .payments
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();

    let mut allocation = AuctionAllocation::default();
    let mut summary = PaymentSummary::default();
    for winner in vcg_allocation.winners.iter().take(remaining_signals) {
        let Some(candidate_index) = candidate_by_id.get(&winner.section_name).copied() else {
            continue;
        };
        let adjusted_bid = winner.adjusted_bid as f32;
        allocation.total_bid += adjusted_bid;
        allocation.selected.push(SelectedCandidate {
            candidate_index,
            adjusted_bid,
        });

        let payment = payment_by_id
            .get(&winner.section_name)
            .copied()
            .unwrap_or(0.0) as f32;
        summary.total_payments += payment;
        if payment > summary.highest_payment_value {
            summary.highest_payment_value = payment;
            summary.highest_payment_section =
                Some(candidates[candidate_index].section.name.clone());
        }
    }

    (allocation, summary, vcg_allocation)
}

fn effective_candidate_bid(
    candidate: &AuctionCandidate<'_>,
    bidder_wins: &HashMap<AttentionBidder, usize>,
    affect: Option<&AuctionAffectState>,
) -> f32 {
    let bidder = candidate.section.bidder;
    let wins = bidder_wins.get(&bidder).copied().unwrap_or(0);
    let diversity_boost = if wins == 0 { 1.18 } else { 1.0 };
    let diminishing_returns = 0.82_f32.powi(wins as i32);
    let affect_multiplier = bidder_affect_multiplier(&candidate.section, affect);
    candidate.bid_density * diversity_boost * diminishing_returns * affect_multiplier
}

fn bidder_affect_multiplier(section: &PromptSection, affect: Option<&AuctionAffectState>) -> f32 {
    let Some(affect) = affect else {
        return 1.0;
    };
    let urgency = affect.urgency_multiplier();
    let affect_weight = affect.affect_weight_multiplier();
    let low_dominance = affect.low_dominance_pressure();
    let low_pleasure = affect.low_pleasure_pressure();

    let text = format!(
        "{} {}",
        section.name.to_ascii_lowercase(),
        section.content.to_ascii_lowercase()
    );
    let warningish = keyword_weight(
        &text,
        &[
            "warning",
            "caution",
            "avoid",
            "risk",
            "failure",
            "regression",
            "blocker",
        ],
    );
    let exploratory = keyword_weight(
        &text,
        &[
            "explore",
            "novel",
            "research",
            "investigate",
            "hypothesis",
            "experiment",
        ],
    );
    let proven = keyword_weight(
        &text,
        &[
            "playbook",
            "proven",
            "stable",
            "known good",
            "best practice",
            "repeatable",
        ],
    );
    let conservative = keyword_weight(
        &text,
        &[
            "conservative",
            "safe",
            "warning",
            "avoid",
            "guardrail",
            "rollback",
        ],
    );
    let deadline = keyword_weight(
        &text,
        &[
            "deadline",
            "blocking",
            "critical",
            "must",
            "acceptance",
            "verify",
            "required",
        ],
    );
    let failure = keyword_weight(
        &text,
        &[
            "failure",
            "failed",
            "error",
            "retry",
            "regression",
            "incident",
            "blocker",
        ],
    );
    let prediction = keyword_weight(
        &text,
        &[
            "prediction",
            "forecast",
            "uncertain",
            "error",
            "warning",
            "confidence",
            "probe",
        ],
    );

    let subsystem_bias = match section.bidder {
        AttentionBidder::Neuro => {
            1.0 + urgency * 0.12 * warningish
                + low_dominance * 0.18 * exploratory
                + low_pleasure * 0.28 * warningish
        }
        AttentionBidder::Daimon => {
            1.0 + urgency * 0.20 + low_dominance * 0.18 + low_pleasure * 0.24
        }
        AttentionBidder::IterationMemory => {
            1.0 + urgency * 0.15 * failure + low_pleasure * 0.32 * failure
        }
        AttentionBidder::CodeIntelligence => 1.0 + urgency * 0.18 * warningish.max(deadline),
        AttentionBidder::PlaybookRules => {
            1.0 + urgency * 0.14 * proven
                + low_dominance * 0.20 * exploratory
                + low_pleasure * 0.22 * conservative.max(proven)
        }
        AttentionBidder::Research => 1.0 + low_dominance * 0.30 * exploratory.max(1.0),
        AttentionBidder::TaskContext => 1.0 + urgency * 0.18 * deadline.max(1.0),
        AttentionBidder::Oracles => 1.0 + urgency * 0.22 * prediction.max(1.0),
    };

    urgency * affect_weight * subsystem_bias
}

impl AuctionAffectState {
    fn to_vcg_modulation(self) -> AffectModulation {
        AffectModulation::from_pad(self.pleasure as f64, self.arousal as f64)
    }
}

fn keyword_weight(text: &str, keywords: &[&str]) -> f32 {
    if keywords.iter().any(|keyword| text.contains(keyword)) {
        1.0
    } else {
        0.0
    }
}

fn section_valence(section: &PromptSection) -> f64 {
    let text = format!(
        "{} {}",
        section.name.to_ascii_lowercase(),
        section.content.to_ascii_lowercase()
    );
    let positive = keyword_weight(
        &text,
        &[
            "success",
            "passed",
            "proven",
            "stable",
            "known good",
            "opportunity",
        ],
    );
    let negative = keyword_weight(
        &text,
        &[
            "failure",
            "failed",
            "error",
            "warning",
            "risk",
            "regression",
            "threat",
        ],
    );
    (positive as f64 - negative as f64).clamp(-1.0, 1.0)
}

fn strategy_tag(strategy: CompositionStrategy) -> &'static str {
    match strategy {
        CompositionStrategy::Auto => "auto",
        CompositionStrategy::DensityGreedy => "density_greedy",
        CompositionStrategy::WeightedSum => "weighted_sum",
        CompositionStrategy::Vcg => "vcg",
    }
}

#[allow(clippy::too_many_arguments)]
fn build_composition_manifest(
    requested_strategy: CompositionStrategy,
    selected_strategy: CompositionStrategy,
    kept: &[(PromptSection, &Signal)],
    optional: &[AuctionCandidate<'_>],
    selected: &[SelectedCandidate],
    vcg_allocation: Option<&VcgAllocation>,
    total_tokens: usize,
    token_budget_limit: Option<usize>,
    scorer: &dyn roko_core::traits::Score,
    ctx: &Context,
) -> CompositionManifest {
    let selected_indices = selected
        .iter()
        .map(|winner| winner.candidate_index)
        .collect::<HashSet<_>>();
    let optional_by_id = optional
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.section.stable_section_id(), (index, candidate)))
        .collect::<HashMap<_, _>>();
    let adjusted_bid_by_index = selected
        .iter()
        .map(|winner| (winner.candidate_index, winner.adjusted_bid))
        .collect::<HashMap<_, _>>();
    let payment_by_id = vcg_allocation
        .map(|allocation| {
            allocation
                .payments
                .iter()
                .cloned()
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let included = kept
        .iter()
        .map(|(section, source_signal)| {
            let section_id = section.stable_section_id();
            if let Some((index, candidate)) = optional_by_id.get(&section_id).copied() {
                let bid_value = adjusted_bid_by_index
                    .get(&index)
                    .copied()
                    .unwrap_or(candidate.bid_value);
                IncludedSectionMeta {
                    section_id: section_id.clone(),
                    action_id: section.action_id(),
                    name: section.name.clone(),
                    bidder: section.bidder,
                    estimated_tokens: section.estimated_tokens(),
                    score: candidate.score,
                    bid_value,
                    vcg_payment: payment_by_id.get(&section_id).copied(),
                    reason: if selected_strategy == CompositionStrategy::Vcg {
                        "selected_by_vcg".to_string()
                    } else {
                        "selected_by_density".to_string()
                    },
                }
            } else {
                let score = candidate_score(section, source_signal, scorer, ctx);
                IncludedSectionMeta {
                    section_id,
                    action_id: section.action_id(),
                    name: section.name.clone(),
                    bidder: section.bidder,
                    estimated_tokens: section.estimated_tokens(),
                    score,
                    bid_value: score,
                    vcg_payment: None,
                    reason: "critical".to_string(),
                }
            }
        })
        .collect::<Vec<_>>();

    let excluded = optional
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected_indices.contains(index))
        .map(|(_, candidate)| {
            let section_id = candidate.section.stable_section_id();
            ExcludedSectionMeta {
                section_id,
                action_id: candidate.section.action_id(),
                name: candidate.section.name.clone(),
                bidder: candidate.section.bidder,
                estimated_tokens: candidate.section.estimated_tokens(),
                score: candidate.score,
                bid_value: candidate.bid_value,
                reason: if selected_strategy == CompositionStrategy::Vcg {
                    "excluded_by_vcg".to_string()
                } else {
                    "dropped_by_density_budget".to_string()
                },
            }
        })
        .collect::<Vec<_>>();

    CompositionManifest {
        requested_strategy,
        selected_strategy,
        included,
        excluded,
        vcg_diagnostics: vcg_allocation.map(|allocation| allocation.diagnostics.clone()),
        total_tokens,
        token_budget_limit,
    }
}

fn candidate_score(
    section: &PromptSection,
    signal: &Signal,
    scorer: &dyn roko_core::traits::Score,
    ctx: &Context,
) -> f32 {
    let score = scorer.score(signal, ctx);
    let learned = score.effective();
    let fallback = fallback_section_score(section, signal, ctx);
    learned.max(fallback)
}

fn fallback_section_score(section: &PromptSection, signal: &Signal, ctx: &Context) -> f32 {
    let priority = match section.priority {
        SectionPriority::Critical => 1.0,
        SectionPriority::High => 0.82,
        SectionPriority::Normal => 0.55,
        SectionPriority::Low => 0.25,
    };
    let cache = match section.cache_layer {
        CacheLayer::Role => 0.95,
        CacheLayer::Workspace => 0.8,
        CacheLayer::Plan => 0.65,
        CacheLayer::Volatile => 0.6,
    };
    let age_ms = (ctx.now_ms - signal.created_at_ms).max(0) as f32;
    let recency = if age_ms <= 60.0 * 60.0 * 1000.0 {
        1.0
    } else if age_ms >= 24.0 * 60.0 * 60.0 * 1000.0 {
        0.35
    } else {
        1.0 - (age_ms - 60.0 * 60.0 * 1000.0) / (23.0 * 60.0 * 60.0 * 1000.0) * 0.65
    };
    priority * (0.55 + 0.45 * cache) * recency
}

// ─── COMP-03: MVT foraging pre-pass ─────────────────────────────────────

/// Apply the MVT foraging stopping rule to limit candidate evaluation.
///
/// Sorts candidates by bid density, then walks them in order. After each
/// candidate, computes the marginal gain ratio (current candidate's density
/// vs running average). When `should_stop_searching` returns true, the
/// remaining lower-value candidates are dropped. This prevents wasting
/// token budget on diminishing-returns context sections.
fn foraging_prepass<'a>(
    mut candidates: Vec<AuctionCandidate<'a>>,
    forager: &MultiPatchForager,
) -> Vec<AuctionCandidate<'a>> {
    if candidates.is_empty() || forager.environment_rate <= 0.0 {
        return candidates;
    }

    // Sort by bid density descending for MVT evaluation order.
    candidates.sort_by(|a, b| {
        b.bid_density
            .partial_cmp(&a.bid_density)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut kept = Vec::with_capacity(candidates.len());
    let mut running_sum = 0.0_f64;
    let mut count = 0usize;
    let mut total_content_tokens = 0usize;

    for candidate in candidates {
        count += 1;
        running_sum += candidate.bid_density as f64;
        let avg = running_sum / count as f64;

        // MVT ratio: marginal gain of this candidate vs environment average.
        let mvt_ratio = if avg > 0.0 {
            candidate.bid_density as f64 / avg
        } else {
            1.0
        };

        total_content_tokens += candidate.section.estimated_tokens();

        // Coverage sufficiency: approximate as the fraction of token budget
        // that has been consumed. Once we have gathered many candidates,
        // additional low-value ones are unlikely to help.
        let sufficiency = if forager.environment_rate > 0.0 {
            // Normalize to a [0, 1] range using the environment rate as proxy
            // for the expected number of useful candidates.
            let expected_useful = (1.0 / forager.environment_rate).max(3.0);
            (count as f64 / expected_useful).min(1.0)
        } else {
            0.0
        };

        kept.push(candidate);

        // Require at least 3 candidates before stopping to avoid premature
        // cutoff on small input sets.
        if should_stop_searching(mvt_ratio, sufficiency, 0.8) && count >= 3 {
            break;
        }
    }

    let _ = total_content_tokens; // Available for future cost tracking.
    kept
}

// ─── COMP-04: HDC-based deduplication ───────────────────────────────────

/// Remove near-duplicate candidates using HDC fingerprint similarity.
///
/// For each candidate in bid-density order, computes an HDC fingerprint from
/// its content, then compares against all accepted candidates. If similarity
/// exceeds `threshold` (typically 0.85), the candidate is rejected as a
/// near-duplicate.
#[cfg(feature = "hdc")]
fn hdc_dedup_candidates(
    candidates: Vec<AuctionCandidate<'_>>,
    threshold: f64,
) -> Vec<AuctionCandidate<'_>> {
    use roko_primitives::HdcVector;

    if candidates.is_empty() || threshold <= 0.0 {
        return candidates;
    }

    // Compute HDC fingerprints for all candidates.
    let fingerprints: Vec<HdcVector> = candidates
        .iter()
        .map(|c| HdcVector::from_seed(c.section.content.as_bytes()))
        .collect();

    let mut kept = Vec::with_capacity(candidates.len());
    let mut kept_fingerprints = Vec::with_capacity(candidates.len());

    for (candidate, fingerprint) in candidates.into_iter().zip(fingerprints) {
        let is_duplicate = kept_fingerprints
            .iter()
            .any(|accepted: &HdcVector| f64::from(fingerprint.similarity(accepted)) > threshold);

        if !is_duplicate {
            kept_fingerprints.push(fingerprint);
            kept.push(candidate);
        }
    }

    kept
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Raw-content-free section audit rows for kept and dropped sections.
    #[serde(default)]
    pub section_metadata: Vec<PromptSectionAudit>,
    /// Raw-content-free auction/selection metadata for this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_manifest: Option<CompositionManifest>,
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
            section_metadata: Vec::new(),
            composition_manifest: None,
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

    /// Record raw-content-free section metadata.
    #[must_use]
    pub fn with_section_metadata(mut self, metadata: Vec<PromptSectionAudit>) -> Self {
        self.section_metadata = metadata;
        self
    }

    /// Record raw-content-free composition selection metadata.
    #[must_use]
    pub fn with_composition_manifest(mut self, manifest: CompositionManifest) -> Self {
        self.composition_manifest = Some(manifest);
        self
    }
}

const fn placement_order(p: Placement) -> u8 {
    match p {
        Placement::Start => 0,
        Placement::Middle => 1,
        Placement::End => 2,
    }
}

fn render_sections(kept: &[(PromptSection, &Signal)], headers: bool) -> String {
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

    fn section(name: &str, content: &str, pri: SectionPriority) -> Signal {
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
    fn composer_spreads_budget_across_bidders_when_scores_are_close() {
        let composer = PromptComposer::new().without_headers();
        let sections = [
            PromptSection::new("code-a", "relevant symbol context")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::CodeIntelligence)
                .into_signal()
                .unwrap(),
            PromptSection::new("code-b", "more symbol context")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::CodeIntelligence)
                .into_signal()
                .unwrap(),
            PromptSection::new("research", "important research memo")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::Research)
                .into_signal()
                .unwrap(),
        ];
        let out = composer
            .compose(&sections, &Budget::tokens(12), &NoOpScorer, &Context::at(0))
            .unwrap();
        let text = out.body.as_text().unwrap();
        assert!(text.contains("relevant symbol context") || text.contains("more symbol context"));
        assert!(text.contains("important research memo"));
        assert_eq!(out.tag("distinct_bidders"), Some("2"));
    }

    #[test]
    fn auction_affect_multiplier_boosts_urgent_task_context() {
        let section = PromptSection::new(
            "deadline-prd",
            "must ship before deadline and verify acceptance criteria",
        )
        .with_priority(SectionPriority::High)
        .with_bidder(AttentionBidder::TaskContext);
        let neutral = AuctionAffectState {
            pleasure: 0.5,
            arousal: 0.0,
            dominance: 0.2,
        };
        let urgent = AuctionAffectState {
            pleasure: -0.3,
            arousal: 0.8,
            dominance: -0.4,
        };

        assert!(
            bidder_affect_multiplier(&section, Some(&urgent))
                > bidder_affect_multiplier(&section, Some(&neutral))
        );
    }

    #[test]
    fn composer_emits_auction_payment_diagnostics() {
        let composer = PromptComposer::new().without_headers();
        let sections = [
            PromptSection::new("code-a", "relevant symbol context")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::CodeIntelligence)
                .into_signal()
                .unwrap(),
            PromptSection::new("research", "important research memo")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::Research)
                .into_signal()
                .unwrap(),
            PromptSection::new(
                "deadline-prd",
                "must ship before deadline and verify acceptance criteria",
            )
            .with_priority(SectionPriority::High)
            .with_bidder(AttentionBidder::TaskContext)
            .into_signal()
            .unwrap(),
        ];
        let ctx = Context::at(0)
            .with_attr("roko.daimon.pleasure", "-0.3")
            .with_attr("roko.daimon.arousal", "0.8")
            .with_attr("roko.daimon.dominance", "-0.4");
        let out = composer
            .compose(&sections, &Budget::tokens(16), &NoOpScorer, &ctx)
            .unwrap();

        assert_ne!(out.tag("highest_payment_section"), Some("none"));
        let total_payments = out
            .tag("auction_total_payments")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap();
        let urgency = out
            .tag("auction_urgency")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap();
        let affect_weight = out
            .tag("auction_affect_weight")
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap();
        assert!(total_payments >= 0.0);
        assert!(urgency > 1.0);
        assert!(affect_weight > 1.0);
    }

    #[test]
    fn composer_emits_composition_manifest_tag() {
        let composer = PromptComposer::new().without_headers();
        let sections = [
            PromptSection::new("role", "you are an agent")
                .with_priority(SectionPriority::Critical)
                .into_signal()
                .unwrap(),
            PromptSection::new("task", "implement feature")
                .with_priority(SectionPriority::High)
                .into_signal()
                .unwrap(),
            PromptSection::new("extra", &"optional ".repeat(80))
                .with_priority(SectionPriority::Low)
                .into_signal()
                .unwrap(),
        ];

        let out = composer
            .compose(&sections, &Budget::tokens(24), &NoOpScorer, &Context::at(0))
            .unwrap();
        let manifest = CompositionManifest::from_tag_value(
            out.tag(COMPOSITION_MANIFEST_TAG)
                .expect("composition manifest tag"),
        )
        .expect("manifest parses");

        assert_eq!(
            manifest.selected_strategy,
            CompositionStrategy::DensityGreedy
        );
        assert!(
            manifest
                .included
                .iter()
                .any(|section| section.name == "role")
        );
        assert!(
            manifest
                .excluded
                .iter()
                .any(|section| section.name == "extra")
        );
        assert!(
            manifest
                .included
                .iter()
                .all(|section| !section.action_id.is_empty())
        );
    }

    #[test]
    fn composer_vcg_path_emits_diagnostics_and_payments() {
        let composer = PromptComposer::new()
            .without_headers()
            .with_strategy(CompositionStrategy::Vcg);
        let sections = [
            PromptSection::new("code", "relevant symbol context")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::CodeIntelligence)
                .into_signal()
                .unwrap(),
            PromptSection::new("research", "important research memo")
                .with_priority(SectionPriority::High)
                .with_bidder(AttentionBidder::Research)
                .into_signal()
                .unwrap(),
            PromptSection::new("large", &"low value ".repeat(80))
                .with_priority(SectionPriority::Low)
                .with_bidder(AttentionBidder::Research)
                .into_signal()
                .unwrap(),
        ];

        let out = composer
            .compose(&sections, &Budget::tokens(16), &NoOpScorer, &Context::at(0))
            .unwrap();
        let manifest =
            CompositionManifest::from_tag_value(out.tag(COMPOSITION_MANIFEST_TAG).unwrap())
                .expect("manifest parses");

        assert_eq!(manifest.selected_strategy, CompositionStrategy::Vcg);
        assert!(manifest.vcg_diagnostics.is_some());
        assert!(manifest.total_tokens <= 16);
        assert!(
            manifest
                .included
                .iter()
                .any(|section| section.vcg_payment.is_some())
        );
    }

    #[test]
    fn composer_auto_selects_vcg_when_bidders_are_warm() {
        let mut bidder = LearningBidder::new(AttentionBidder::TaskContext, 1.0);
        for _ in 0..10 {
            bidder.update_with_cost("task", true, true, 0.001, 10);
        }
        let composer = PromptComposer::new()
            .without_headers()
            .with_learning_bidders(HashMap::from([(AttentionBidder::TaskContext, bidder)]));
        let sections = [PromptSection::new("task", "implement feature")
            .with_priority(SectionPriority::High)
            .with_bidder(AttentionBidder::TaskContext)
            .into_signal()
            .unwrap()];

        let out = composer
            .compose(&sections, &Budget::tokens(16), &NoOpScorer, &Context::at(0))
            .unwrap();
        let manifest =
            CompositionManifest::from_tag_value(out.tag(COMPOSITION_MANIFEST_TAG).unwrap())
                .expect("manifest parses");

        assert_eq!(manifest.selected_strategy, CompositionStrategy::Vcg);
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
    fn prompt_section_records_stable_action_ids_without_content() {
        let section = PromptSection::new("Workspace Map", "secret prompt text")
            .with_source("file", "src/lib.rs:1-20")
            .with_experiment_id("exp-a");

        assert_eq!(
            section.stable_section_id(),
            "prompt:workspace-map:file:src-lib-rs-1-20"
        );
        assert_eq!(
            section.action_id(),
            "prompt_section:workspace-map:file:src-lib-rs-1-20|experiment:exp-a"
        );
        let audit = section.audit_row(true, section.estimated_tokens(), "included");
        let encoded = serde_json::to_string(&audit).expect("audit serializes");
        assert!(encoded.contains("prompt_section:workspace-map"));
        assert!(!encoded.contains("secret prompt text"));
    }

    #[test]
    fn composer_ignores_non_section_signals() {
        let composer = PromptComposer::new();
        let real_section = section("task", "implement X", SectionPriority::High);
        let fake = Signal::builder(Kind::Task)
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
    fn composer_respects_max_pulses() {
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
                    max_pulses: Some(3),
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

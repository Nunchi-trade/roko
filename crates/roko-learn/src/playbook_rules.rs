//! Playbook rule extraction (§16.2.2–§16.2.4).
//!
//! A TOML-backed store of if-then [`Rule`]s mined from the [`Episode`] stream.
//! Each rule fires when incoming context matches its [`Triggers`] (file globs,
//! tags, categories, error signatures, or role) and injects its `body` text
//! into the Implementer's prompt.
//!
//! Confidence dynamics (event-driven, not time-based):
//! - validation: `confidence = min(0.95, confidence + 0.05)`
//! - contradiction: `confidence = max(0.0, confidence - 0.10)`
//! - prune threshold: `confidence < min_confidence` (strict)
//!
//! See `tmp/roko-progress/COMPONENTS/learn/playbook.md` for the full spec.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use globset::Glob;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::episode_logger::Episode;

// ─── Types ───────────────────────────────────────────────────────────────────

/// Conditions that must be present in a [`MatchContext`] for a [`Rule`] to fire.
///
/// Matching uses OR semantics across the five trigger kinds: a rule fires if
/// ANY of its trigger lists intersects the context. An all-empty [`Triggers`]
/// matches **nothing** — it never fires (guards against accidental universal
/// rules).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Triggers {
    /// Shell glob patterns matched against the files in [`MatchContext::files`].
    pub file_globs: Vec<String>,
    /// Tag strings; case-insensitive overlap with [`MatchContext::tags`].
    pub tags: Vec<String>,
    /// Task categories matched against [`MatchContext::category`].
    pub categories: Vec<String>,
    /// Error signature strings matched against [`MatchContext::error_signature`].
    pub error_signatures: Vec<String>,
    /// Agent roles matched against [`MatchContext::role`].
    pub roles: Vec<String>,
}

impl Triggers {
    /// Returns `true` when all five trigger lists are empty (i.e. this rule
    /// would never fire under any context).
    fn is_empty(&self) -> bool {
        self.file_globs.is_empty()
            && self.tags.is_empty()
            && self.categories.is_empty()
            && self.error_signatures.is_empty()
            && self.roles.is_empty()
    }
}

/// One playbook rule.
///
/// Rules are stored in TOML, retrieved at prompt-compose time, and carry a
/// bounded confidence score that climbs on validated predictions and decays
/// on contradiction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    /// Stable identifier synthesized from the `(category, error_signature)`
    /// clustering key.
    pub rule_id: String,
    /// Short human-readable label (≤80 chars).
    pub title: String,
    /// Text injected into the Implementer prompt (≤`max_body_tokens * 4` bytes).
    pub body: String,
    /// Conditions that cause this rule to fire.
    pub triggers: Triggers,
    /// Confidence score; bounded to `[0.0, 0.95]`.
    pub confidence: f64,
    /// Number of times a prediction by this rule was validated.
    pub validations: u32,
    /// Number of times evidence contradicted this rule.
    pub contradictions: u32,
    /// Timestamp of most recent [`PlaybookRules::select`] call that returned
    /// this rule.
    pub last_applied: Option<DateTime<Utc>>,
    /// Timestamp when the rule was first created.
    pub created_at: DateTime<Utc>,
    /// Identifiers of the episodes whose cluster generated this rule.
    pub source_episodes: Vec<String>,
    /// Attention budget decayed via Gesellian demurrage tax.
    /// Rules must be actively validated to replenish balance.
    /// Rules with depleted balance are deprioritized in retrieval.
    #[serde(default = "default_balance")]
    pub balance: f64,
    /// Hourly decay rate for the attention budget (default 0.01 per hour).
    #[serde(default = "default_demurrage_rate")]
    pub demurrage_rate: f64,
    /// Millisecond timestamp of the last demurrage decay application.
    #[serde(default)]
    pub last_decay_at_ms: i64,
}

fn default_balance() -> f64 {
    1.0
}

fn default_demurrage_rate() -> f64 {
    0.01
}

impl Rule {
    /// Returns a new [`Rule`] with sane defaults for optional fields.
    fn new(
        rule_id: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        triggers: Triggers,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            title: title.into(),
            body: body.into(),
            triggers,
            confidence: 0.5,
            validations: 0,
            contradictions: 0,
            last_applied: None,
            created_at: Utc::now(),
            source_episodes: Vec::new(),
            balance: 1.0,
            demurrage_rate: 0.01,
            last_decay_at_ms: Utc::now().timestamp_millis(),
        }
    }

    /// Apply continuous demurrage decay to the rule's attention budget.
    ///
    /// `balance *= (1 - demurrage_rate) ^ elapsed_hours`
    pub fn tick_demurrage(&mut self, now_ms: i64) {
        let elapsed_hours = (now_ms - self.last_decay_at_ms) as f64 / 3_600_000.0;
        if elapsed_hours > 0.0 {
            self.balance *= (1.0 - self.demurrage_rate).powf(elapsed_hours);
            self.last_decay_at_ms = now_ms;
        }
    }

    /// Replenish the attention budget (capped at 1.0).
    pub fn replenish(&mut self, amount: f64) {
        self.balance = (self.balance + amount).min(1.0);
    }
}

impl roko_core::Demurrage for Rule {
    fn balance(&self) -> f64 {
        self.balance
    }

    fn demurrage_rate(&self) -> f64 {
        self.demurrage_rate
    }

    fn tick(&mut self, elapsed_hours: f64) {
        let now_ms = self.last_decay_at_ms + (elapsed_hours * 3_600_000.0) as i64;
        self.tick_demurrage(now_ms);
    }

    fn replenish(&mut self, amount: f64) {
        Rule::replenish(self, amount);
    }
}

/// Context supplied at prompt-compose time for trigger matching.
#[derive(Debug, Clone)]
pub struct MatchContext {
    /// Files touched by the task.
    pub files: Vec<String>,
    /// Tags associated with the task.
    pub tags: Vec<String>,
    /// Optional task category.
    pub category: Option<String>,
    /// Optional error signature from a prior gate failure.
    pub error_signature: Option<String>,
    /// Agent role (e.g. `"implementer"`, `"auto_fixer"`).
    pub role: String,
}

/// Configuration parameters for rule extraction.
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Minimum number of episodes in a cluster before a rule is synthesized.
    /// Default: 5.
    pub min_pattern_size: usize,
    /// Minimum fraction of the cluster that must be failures. Default: 0.7.
    pub min_failure_rate: f64,
    /// Upper bound on rule body length in tokens (approximate: 1 token ≈ 4
    /// bytes). Default: 400.
    pub max_body_tokens: usize,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            min_pattern_size: 5,
            min_failure_rate: 0.7,
            max_body_tokens: 400,
        }
    }
}

// ─── TOML envelope ───────────────────────────────────────────────────────────

/// Serde wrapper for the TOML file format:
/// ```toml
/// [[rule]]
/// rule_id = "..."
/// ...
/// ```
#[derive(Serialize, Deserialize, Default)]
struct PlaybookRulesFile {
    #[serde(default, rename = "rule")]
    rules: Vec<Rule>,
}

// ─── PlaybookRules store ──────────────────────────────────────────────────────

/// TOML-backed store for [`Rule`]s.
///
/// All mutations go through a [`parking_lot::RwLock`] so the store is
/// thread-safe. Persistence is atomic: [`PlaybookRules::save`] writes to a
/// `.tmp` file and renames into place.
pub struct PlaybookRules {
    path: PathBuf,
    rules: RwLock<Vec<Rule>>,
}

impl PlaybookRules {
    /// Open (or create) the rule store at `path`.
    ///
    /// If the file does not exist, an empty store is returned — no error.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the file exists but cannot be read or
    /// parsed.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let rules = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: PlaybookRulesFile = toml::from_str(&text)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                file.rules
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            path,
            rules: RwLock::new(rules),
        })
    }

    /// Atomically persist all rules to disk.
    ///
    /// Writes to `{path}.tmp` then renames into place.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] on any filesystem or serialization failure.
    pub fn save(&self) -> io::Result<()> {
        let snapshot = self.rules.read().clone();
        let file = PlaybookRulesFile { rules: snapshot };
        let text = toml::to_string_pretty(&file)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Create parent directory if needed.
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let tmp = self.path.with_extension("toml.tmp");
        std::fs::write(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Return rules whose triggers match `ctx`, sorted by `(confidence desc,
    /// last_applied desc)`, truncated to `limit`.
    ///
    /// Updates `last_applied` on all returned rules (non-blocking write lock).
    /// An empty [`Triggers`] rule is never returned.
    pub fn select(&self, ctx: &MatchContext, limit: usize) -> Vec<Rule> {
        let now = Utc::now();

        // Collect matching rule ids under a brief read lock, then release it
        // before acquiring the write lock (parking_lot RwLock is not
        // upgradeable — read + write would deadlock on the same thread).
        let matching_ids: Vec<String> = self
            .rules
            .read()
            .iter()
            .filter(|r| !r.triggers.is_empty() && triggers_match(&r.triggers, ctx))
            .map(|r| r.rule_id.clone())
            .collect();

        if matching_ids.is_empty() {
            return Vec::new();
        }

        // Build a sorted list of (rule_id, confidence, last_applied, balance)
        // for matching rules, sort it, then acquire the write lock to stamp
        // last_applied.
        let mut order: Vec<(String, f64, Option<DateTime<Utc>>, f64)> = self
            .rules
            .read()
            .iter()
            .filter(|r| matching_ids.contains(&r.rule_id))
            .map(|r| (r.rule_id.clone(), r.confidence, r.last_applied, r.balance))
            .collect();

        // Sort: rules with balance < 0.1 are deprioritized (sorted after
        // higher-balance rules). Within each group, sort by confidence
        // descending, then last_applied descending.
        order.sort_by(|a, b| {
            let a_healthy = a.3 >= 0.1;
            let b_healthy = b.3 >= 0.1;
            b_healthy
                .cmp(&a_healthy)
                .then_with(|| {
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| cmp_opt_dt_desc(a.2, b.2))
        });

        let selected_ids: Vec<String> =
            order.into_iter().map(|(id, _, _, _)| id).take(limit).collect();

        // Acquire write lock to stamp last_applied and collect clones.
        let mut guard = self.rules.write();

        for rule in guard.iter_mut() {
            if selected_ids.contains(&rule.rule_id) {
                rule.last_applied = Some(now);
            }
        }

        // Collect in sorted order while the write lock is still held, then
        // drop the guard so the lock is released before returning.
        let result: Vec<Rule> = selected_ids
            .iter()
            .filter_map(|id| guard.iter().find(|r| &r.rule_id == id).cloned())
            .collect();
        drop(guard);
        result
    }

    /// Add a new rule or replace an existing rule with the same `rule_id`.
    ///
    /// Rejects rules whose `body` exceeds `max_body_tokens * 4` bytes (the
    /// default limit from [`ExtractionConfig`]: 400 × 4 = 1 600 bytes).
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] if the body is oversized or
    /// contains null bytes.
    pub fn upsert(&self, rule: Rule) -> io::Result<()> {
        // Body size guard (approximate token budget).
        const DEFAULT_MAX_BODY_BYTES: usize = 400 * 4;
        if rule.body.len() > DEFAULT_MAX_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "rule body too large: {} bytes (max {})",
                    rule.body.len(),
                    DEFAULT_MAX_BODY_BYTES
                ),
            ));
        }
        if rule.body.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rule body must not contain null bytes",
            ));
        }

        {
            let mut guard = self.rules.write();
            if let Some(pos) = guard.iter().position(|r| r.rule_id == rule.rule_id) {
                guard[pos] = rule;
            } else {
                guard.push(rule);
            }
        }
        Ok(())
    }

    /// Adjust the confidence of the rule identified by `rule_id`.
    ///
    /// On validation (`validated = true`): `confidence += 0.05`, capped at
    /// `0.95`, `validations += 1`.
    ///
    /// On contradiction (`validated = false`): `confidence -= 0.10`, floored
    /// at `0.0`, `contradictions += 1`.
    ///
    /// No-ops if the `rule_id` is not found.
    pub fn record_outcome(&self, rule_id: &str, validated: bool) {
        let mut guard = self.rules.write();
        if let Some(rule) = guard.iter_mut().find(|r| r.rule_id == rule_id) {
            if validated {
                rule.confidence = (rule.confidence + 0.05).min(0.95);
                rule.validations = rule.validations.saturating_add(1);
                rule.replenish(0.05);
            } else {
                rule.confidence = (rule.confidence - 0.10).max(0.0);
                rule.contradictions = rule.contradictions.saturating_add(1);
            }
        }
    }

    /// Record a validated prediction for `rule_id`.
    pub fn validate(&self, rule_id: &str) {
        self.record_outcome(rule_id, true);
    }

    /// Record a contradicted prediction for `rule_id`.
    pub fn contradict(&self, rule_id: &str) {
        self.record_outcome(rule_id, false);
    }

    /// Apply demurrage decay to all rules' attention budgets.
    ///
    /// Call this at the start of each feedback cycle (e.g. `record_completed_run`).
    pub fn tick_demurrage_all(&self, now_ms: i64) {
        let mut guard = self.rules.write();
        for rule in guard.iter_mut() {
            rule.tick_demurrage(now_ms);
        }
    }

    /// Remove all rules with `confidence < min_confidence` (strict).
    ///
    /// Returns the count of rules removed.
    pub fn prune(&self, min_confidence: f64) -> usize {
        let mut guard = self.rules.write();
        let before = guard.len();
        guard.retain(|r| r.confidence >= min_confidence);
        before - guard.len()
    }

    /// Mine `episodes` for recurring failure clusters and synthesize new rules.
    ///
    /// Only episodes with at least one failed [`GateVerdict`] contribute. A
    /// cluster is keyed by `(first_failed_signature, category)`. Clusters
    /// below `config.min_pattern_size` or `config.min_failure_rate` are
    /// skipped.
    ///
    /// Existing rules with the same `rule_id` are merged (max confidence,
    /// union of `source_episodes`).
    ///
    /// Returns the list of newly synthesized (or updated) rules — does NOT
    /// auto-save.
    pub fn extract_rules<I: IntoIterator<Item = Episode>>(
        &self,
        episodes: I,
        config: &ExtractionConfig,
    ) -> Vec<Rule> {
        // Collect episodes then build cluster maps (two helpers keep this fn short).
        let episodes: Vec<Episode> = episodes.into_iter().collect();
        let (failed_map, total_map) = build_clusters(&episodes);

        // Synthesize one Rule per qualifying cluster.
        let synthesized = synthesize_from_clusters(&failed_map, &total_map, config);

        // Merge synthesized rules with the store (dedup by rule_id).
        let mut guard = self.rules.write();
        for new_rule in &synthesized {
            merge_rule_into(&mut guard, new_rule);
        }

        synthesized
    }

    /// Total number of rules in the store.
    pub fn count(&self) -> usize {
        self.rules.read().len()
    }

    /// Return a snapshot of all rules.
    pub fn snapshot(&self) -> Vec<Rule> {
        self.rules.read().clone()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Cluster key type: `(first_failed_signature, task_category)`.
type ClusterKey = (String, String);

/// Build `(failed_map, total_map)` from a slice of episodes.
///
/// `failed_map[key]` = episodes in that cluster whose gate failed.
/// `total_map[key]`  = total episodes with that cluster key (pass or fail).
///
/// Episodes without any failed gate verdict with a non-empty signature are
/// skipped because they cannot form a meaningful failure cluster.
fn build_clusters(
    episodes: &[Episode],
) -> (
    HashMap<ClusterKey, Vec<&Episode>>,
    HashMap<ClusterKey, usize>,
) {
    let mut failed_map: HashMap<ClusterKey, Vec<&Episode>> = HashMap::new();
    let mut total_map: HashMap<ClusterKey, usize> = HashMap::new();

    for ep in episodes {
        let category = ep
            .extra
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let first_failed_sig = ep
            .gate_verdicts
            .iter()
            .find(|gv| !gv.passed)
            .and_then(|gv| gv.signature.clone())
            .unwrap_or_default();

        if first_failed_sig.is_empty() {
            continue;
        }

        let key = (first_failed_sig, category);
        *total_map.entry(key.clone()).or_insert(0) += 1;

        if ep.gate_verdicts.iter().any(|gv| !gv.passed) {
            failed_map.entry(key).or_default().push(ep);
        }
    }

    (failed_map, total_map)
}

/// Synthesize one [`Rule`] per cluster in `failed_map` that passes both the
/// `min_pattern_size` and `min_failure_rate` gates.
fn synthesize_from_clusters(
    failed_map: &HashMap<ClusterKey, Vec<&Episode>>,
    total_map: &HashMap<ClusterKey, usize>,
    config: &ExtractionConfig,
) -> Vec<Rule> {
    let mut out = Vec::new();

    for (key, failed_episodes) in failed_map {
        let (signature, category) = key;

        if failed_episodes.len() < config.min_pattern_size {
            continue;
        }

        let total = *total_map.get(key).unwrap_or(&0);
        #[allow(clippy::cast_precision_loss)]
        let failure_rate = if total == 0 {
            0.0_f64
        } else {
            (failed_episodes.len() as f64) / (total as f64)
        };
        if failure_rate < config.min_failure_rate {
            continue;
        }

        let rule_id = synthesize_rule_id(category, signature);
        let cat_label = if category.is_empty() {
            "unknown"
        } else {
            category.as_str()
        };
        let title = format!("{cat_label}: {signature}");
        let body = build_body(failed_episodes, config.max_body_tokens * 4);
        let triggers = build_triggers(failed_episodes);
        let source_episodes: Vec<String> = failed_episodes.iter().map(|ep| ep.id.clone()).collect();

        let mut rule = Rule::new(&rule_id, &title, body, triggers);
        rule.source_episodes = source_episodes;
        out.push(rule);
    }

    out
}

/// Collect up to 3 distinct `failure_reason` strings, joined by `"\n\n"`,
/// clamped to `max_bytes`.
fn build_body(episodes: &[&Episode], max_bytes: usize) -> String {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut parts: Vec<String> = Vec::new();
    for ep in episodes {
        if let Some(reason) = &ep.failure_reason {
            if !reason.is_empty() && seen.insert(reason.clone()) {
                parts.push(reason.clone());
                if parts.len() >= 3 {
                    break;
                }
            }
        }
    }
    let mut body = parts.join("\n\n");
    if body.len() > max_bytes {
        body.truncate(max_bytes);
    }
    body
}

/// Union the trigger fields from all episodes in a cluster.
fn build_triggers(episodes: &[&Episode]) -> Triggers {
    let mut file_globs: BTreeSet<String> = BTreeSet::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    let mut categories: BTreeSet<String> = BTreeSet::new();
    let mut error_signatures: BTreeSet<String> = BTreeSet::new();
    let mut roles: BTreeSet<String> = BTreeSet::new();

    for ep in episodes {
        if let Some(arr) = ep.extra.get("files_changed").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    file_globs.insert(s.to_string());
                }
            }
        }
        if let Some(arr) = ep.extra.get("tags").and_then(|v| v.as_array()) {
            for t in arr {
                if let Some(s) = t.as_str() {
                    tags.insert(s.to_lowercase());
                }
            }
        }
        if let Some(s) = ep.extra.get("category").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                categories.insert(s.to_string());
            }
        }
        for gv in ep.gate_verdicts.iter().filter(|gv| !gv.passed) {
            if let Some(sig) = &gv.signature {
                error_signatures.insert(sig.clone());
            }
        }
        if let Some(s) = ep.extra.get("role").and_then(|v| v.as_str()) {
            if !s.is_empty() {
                roles.insert(s.to_string());
            }
        }
    }

    Triggers {
        file_globs: file_globs.into_iter().collect(),
        tags: tags.into_iter().collect(),
        categories: categories.into_iter().collect(),
        error_signatures: error_signatures.into_iter().collect(),
        roles: roles.into_iter().collect(),
    }
}

/// Merge `new_rule` into `store`: if a rule with the same `rule_id` exists,
/// take max confidence and union `source_episodes`; otherwise append.
fn merge_rule_into(store: &mut Vec<Rule>, new_rule: &Rule) {
    if let Some(existing) = store.iter_mut().find(|r| r.rule_id == new_rule.rule_id) {
        existing.confidence = existing.confidence.max(new_rule.confidence);
        let mut ep_set: BTreeSet<String> = existing.source_episodes.iter().cloned().collect();
        ep_set.extend(new_rule.source_episodes.iter().cloned());
        existing.source_episodes = ep_set.into_iter().collect();
        existing.body.clone_from(&new_rule.body);
        existing.triggers.clone_from(&new_rule.triggers);
    } else {
        store.push(new_rule.clone());
    }
}

/// Check whether any trigger in `triggers` intersects the given `ctx`.
///
/// OR semantics: returns `true` as soon as any one trigger kind matches.
/// An all-empty `Triggers` always returns `false`.
fn triggers_match(triggers: &Triggers, ctx: &MatchContext) -> bool {
    // File glob match.
    for glob_pat in &triggers.file_globs {
        if let Ok(glob) = Glob::new(glob_pat) {
            let matcher = glob.compile_matcher();
            for file in &ctx.files {
                if matcher.is_match(file) {
                    return true;
                }
            }
        }
    }

    // Tag match (case-insensitive).
    for trigger_tag in &triggers.tags {
        let trigger_lower = trigger_tag.to_lowercase();
        for ctx_tag in &ctx.tags {
            if ctx_tag.to_lowercase() == trigger_lower {
                return true;
            }
        }
    }

    // Category match.
    if let Some(ctx_cat) = &ctx.category {
        for cat in &triggers.categories {
            if cat == ctx_cat {
                return true;
            }
        }
    }

    // Error signature match.
    if let Some(ctx_sig) = &ctx.error_signature {
        for sig in &triggers.error_signatures {
            if sig == ctx_sig {
                return true;
            }
        }
    }

    // Role match.
    for role in &triggers.roles {
        if role == &ctx.role {
            return true;
        }
    }

    false
}

/// Synthesize a deterministic `rule_id` from a `(category, signature)` pair.
///
/// Non-alphanumeric characters are stripped so the id is safe as a TOML key.
fn synthesize_rule_id(category: &str, signature: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
            .to_lowercase()
    };
    let cat = clean(category);
    let sig = clean(signature);
    let combined = if cat.is_empty() {
        format!("rule-{sig}")
    } else if sig.is_empty() {
        format!("rule-{cat}")
    } else {
        format!("rule-{cat}-{sig}")
    };
    // Truncate to a sane max length.
    if combined.len() > 80 {
        combined[..80].to_string()
    } else {
        combined
    }
}

/// Compare two `Option<DateTime<Utc>>` in descending order (greater = earlier
/// in the sorted list). `None` sorts last.
fn cmp_opt_dt_desc(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Fixtures ────────────────────────────────────────────────────────────

    fn tmp_path(dir: &TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    fn make_rule(id: &str, confidence: f64, triggers: Triggers) -> Rule {
        let now = Utc::now();
        Rule {
            rule_id: id.to_string(),
            title: format!("Rule {id}"),
            body: "example body".to_string(),
            triggers,
            confidence,
            validations: 0,
            contradictions: 0,
            last_applied: None,
            created_at: now,
            source_episodes: Vec::new(),
            balance: 1.0,
            demurrage_rate: 0.01,
            last_decay_at_ms: now.timestamp_millis(),
        }
    }

    fn role_trigger(role: &str) -> Triggers {
        Triggers {
            roles: vec![role.to_string()],
            ..Default::default()
        }
    }

    fn tag_trigger(tag: &str) -> Triggers {
        Triggers {
            tags: vec![tag.to_string()],
            ..Default::default()
        }
    }

    fn glob_trigger(glob: &str) -> Triggers {
        Triggers {
            file_globs: vec![glob.to_string()],
            ..Default::default()
        }
    }

    fn cat_trigger(cat: &str) -> Triggers {
        Triggers {
            categories: vec![cat.to_string()],
            ..Default::default()
        }
    }

    fn sig_trigger(sig: &str) -> Triggers {
        Triggers {
            error_signatures: vec![sig.to_string()],
            ..Default::default()
        }
    }

    fn default_ctx() -> MatchContext {
        MatchContext {
            files: vec!["src/main.rs".to_string()],
            tags: vec!["async".to_string()],
            category: Some("ConcurrencyRefactor".to_string()),
            error_signature: Some("E0277:Send+Sync".to_string()),
            role: "implementer".to_string(),
        }
    }

    /// Build a minimal failed episode for extraction tests.
    fn failed_ep(id: &str, sig: &str, category: &str, failure_reason: &str) -> Episode {
        let mut ep = Episode::new("agent", id);
        ep.id = id.to_string();
        ep.gate_verdicts
            .push(crate::episode_logger::GateVerdict::new("compile", false).with_signature(sig));
        ep.failure_reason = Some(failure_reason.to_string());
        ep.extra.insert(
            "category".to_string(),
            serde_json::Value::String(category.to_string()),
        );
        ep
    }

    /// Build a passing episode for the same (sig, category) — used for
    /// failure-rate tests.
    fn passing_ep(id: &str, sig: &str, category: &str) -> Episode {
        let mut ep = Episode::new("agent", id);
        ep.id = id.to_string();
        ep.gate_verdicts
            .push(crate::episode_logger::GateVerdict::new("compile", true).with_signature(sig));
        ep.extra.insert(
            "category".to_string(),
            serde_json::Value::String(category.to_string()),
        );
        ep
    }

    // ── Test 1 ──────────────────────────────────────────────────────────────

    #[test]
    fn open_missing_file_yields_empty_store() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "missing.toml")).expect("open missing file");
        assert_eq!(store.count(), 0);
    }

    // ── Test 2 ──────────────────────────────────────────────────────────────

    #[test]
    fn upsert_then_save_then_open_roundtrips() {
        let dir = TempDir::new().expect("create tempdir");
        let path = tmp_path(&dir, "rules.toml");

        let now = Utc::now();
        let mut rule = make_rule("r1", 0.7, role_trigger("implementer"));
        rule.validations = 3;
        rule.contradictions = 1;
        rule.last_applied = Some(now);
        rule.source_episodes = vec!["ep1".to_string(), "ep2".to_string()];

        let store = PlaybookRules::open(&path).expect("open");
        store.upsert(rule.clone()).expect("upsert");
        store.save().expect("save");

        let store2 = PlaybookRules::open(&path).expect("reopen");
        let snap = store2.snapshot();
        assert_eq!(snap.len(), 1);
        let loaded = &snap[0];
        assert_eq!(loaded.rule_id, rule.rule_id);
        assert_eq!(loaded.title, rule.title);
        assert_eq!(loaded.body, rule.body);
        assert_eq!(loaded.confidence, rule.confidence);
        assert_eq!(loaded.validations, rule.validations);
        assert_eq!(loaded.contradictions, rule.contradictions);
        assert_eq!(loaded.source_episodes, rule.source_episodes);
        // DateTime roundtrip (second-level precision is fine for TOML).
        assert_eq!(
            loaded.last_applied.expect("last_applied").timestamp(),
            now.timestamp()
        );
    }

    // ── Test 3 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_by_file_glob_matches_matching_rules() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("glob-rs", 0.8, glob_trigger("**/*.rs")))
            .expect("upsert");
        store
            .upsert(make_rule("glob-ts", 0.8, glob_trigger("**/*.ts")))
            .expect("upsert");

        let ctx = MatchContext {
            files: vec!["crates/foo/src/lib.rs".to_string()],
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: "other".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "glob-rs");
    }

    // ── Test 4 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_by_tag_matches_matching_rules() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("async-rule", 0.6, tag_trigger("async")))
            .expect("upsert");
        store
            .upsert(make_rule("sync-rule", 0.6, tag_trigger("sync")))
            .expect("upsert");

        let ctx = MatchContext {
            files: Vec::new(),
            tags: vec!["ASYNC".to_string()], // case-insensitive
            category: None,
            error_signature: None,
            role: "other".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "async-rule");
    }

    // ── Test 5 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_by_category_matches() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("concur", 0.5, cat_trigger("ConcurrencyRefactor")))
            .expect("upsert");
        store
            .upsert(make_rule("hook", 0.5, cat_trigger("HookDevelopment")))
            .expect("upsert");

        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: Some("ConcurrencyRefactor".to_string()),
            error_signature: None,
            role: "other".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "concur");
    }

    // ── Test 6 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_by_error_signature_matches() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("e277", 0.5, sig_trigger("E0277:Send+Sync")))
            .expect("upsert");
        store
            .upsert(make_rule("e308", 0.5, sig_trigger("E0308:type-mismatch")))
            .expect("upsert");

        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: Some("E0277:Send+Sync".to_string()),
            role: "other".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "e277");
    }

    // ── Test 7 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_by_role_matches() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("impl-rule", 0.5, role_trigger("implementer")))
            .expect("upsert");
        store
            .upsert(make_rule("fixer-rule", 0.5, role_trigger("auto_fixer")))
            .expect("upsert");

        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: "implementer".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_id, "impl-rule");
    }

    // ── Test 8 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_returns_none_for_empty_triggers() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        // A rule with all-empty triggers must never fire.
        store
            .upsert(make_rule("universal", 0.9, Triggers::default()))
            .expect("upsert");

        let ctx = default_ctx();
        let results = store.select(&ctx, 10);
        assert!(
            results.is_empty(),
            "empty triggers must not match any context"
        );
    }

    // ── Test 9 ──────────────────────────────────────────────────────────────

    #[test]
    fn select_sorts_by_confidence_desc_then_last_applied_desc() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");

        // Same role trigger so all match.
        let t = role_trigger("implementer");

        let now = Utc::now();
        let earlier = now - chrono::Duration::seconds(100);

        let mut low_conf_recent = make_rule("low-recent", 0.4, t.clone());
        low_conf_recent.last_applied = Some(now);

        let mut low_conf_old = make_rule("low-old", 0.4, t.clone());
        low_conf_old.last_applied = Some(earlier);

        let high_conf = make_rule("high", 0.9, t.clone());

        store.upsert(low_conf_recent).expect("upsert");
        store.upsert(low_conf_old).expect("upsert");
        store.upsert(high_conf).expect("upsert");

        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: "implementer".to_string(),
        };
        let results = store.select(&ctx, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].rule_id, "high");
        // Among the two low-confidence rules, the one with more recent
        // last_applied comes first.
        assert_eq!(results[1].rule_id, "low-recent");
        assert_eq!(results[2].rule_id, "low-old");
    }

    // ── Test 10 ─────────────────────────────────────────────────────────────

    #[test]
    fn select_respects_limit() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let t = role_trigger("implementer");
        for i in 0..8u32 {
            store
                .upsert(make_rule(&format!("r{i}"), 0.5, t.clone()))
                .expect("upsert");
        }
        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: "implementer".to_string(),
        };
        let results = store.select(&ctx, 3);
        assert_eq!(results.len(), 3);
    }

    // ── Test 11 ─────────────────────────────────────────────────────────────

    #[test]
    fn select_updates_last_applied_on_returned_rules() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("r1", 0.5, role_trigger("implementer")))
            .expect("upsert");

        let before = Utc::now();
        let ctx = MatchContext {
            files: Vec::new(),
            tags: Vec::new(),
            category: None,
            error_signature: None,
            role: "implementer".to_string(),
        };
        let results = store.select(&ctx, 10);
        let after = Utc::now();

        assert_eq!(results.len(), 1);
        // The returned clone reflects last_applied at time-of-select.
        // Check the store's internal state.
        let snap = store.snapshot();
        let ts = snap[0].last_applied.expect("last_applied set after select");
        assert!(ts >= before, "last_applied {ts} < before {before}");
        assert!(ts <= after, "last_applied {ts} > after {after}");
    }

    // ── Test 12 ─────────────────────────────────────────────────────────────

    #[test]
    fn record_outcome_true_caps_at_095() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        // Start at 0.92 — two validations would exceed 0.95.
        let mut rule = make_rule("r1", 0.92, role_trigger("implementer"));
        rule.confidence = 0.92;
        store.upsert(rule).expect("upsert");

        store.record_outcome("r1", true);
        store.record_outcome("r1", true);

        let snap = store.snapshot();
        assert!(
            (snap[0].confidence - 0.95).abs() < f64::EPSILON,
            "expected 0.95, got {}",
            snap[0].confidence
        );
    }

    // ── Test 13 ─────────────────────────────────────────────────────────────

    #[test]
    fn record_outcome_false_floors_at_0() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let mut rule = make_rule("r1", 0.05, role_trigger("implementer"));
        rule.confidence = 0.05;
        store.upsert(rule).expect("upsert");

        // 0.05 - 0.10 = -0.05 → should floor to 0.0.
        store.record_outcome("r1", false);

        let snap = store.snapshot();
        assert!(
            snap[0].confidence.abs() < f64::EPSILON,
            "expected 0.0, got {}",
            snap[0].confidence
        );
    }

    // ── Test 14 ─────────────────────────────────────────────────────────────

    #[test]
    fn record_outcome_increments_counters() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        store
            .upsert(make_rule("r1", 0.5, role_trigger("implementer")))
            .expect("upsert");

        store.record_outcome("r1", true);
        store.record_outcome("r1", true);
        store.record_outcome("r1", false);

        let snap = store.snapshot();
        assert_eq!(snap[0].validations, 2);
        assert_eq!(snap[0].contradictions, 1);
    }

    // ── Test 15 ─────────────────────────────────────────────────────────────

    #[test]
    fn prune_removes_below_min_confidence_strictly() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");

        let t = role_trigger("implementer");
        let mut below = make_rule("below", 0.1, t.clone());
        below.confidence = 0.1;
        let mut exact = make_rule("exact", 0.2, t.clone());
        exact.confidence = 0.2;
        let mut above = make_rule("above", 0.5, t.clone());
        above.confidence = 0.5;

        store.upsert(below).expect("upsert");
        store.upsert(exact).expect("upsert");
        store.upsert(above).expect("upsert");

        let removed = store.prune(0.2);
        assert_eq!(removed, 1, "only the strictly-below rule should be pruned");

        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(snap.iter().any(|r| r.rule_id == "exact"));
        assert!(snap.iter().any(|r| r.rule_id == "above"));
    }

    // ── Test 16 ─────────────────────────────────────────────────────────────

    #[test]
    fn extract_returns_empty_below_min_pattern_size() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let config = ExtractionConfig::default(); // min_pattern_size = 5

        // Only 3 failed episodes with the same signature.
        let episodes: Vec<Episode> = (0..3)
            .map(|i| failed_ep(&format!("ep{i}"), "E0277", "Refactor", "type error"))
            .collect();

        let new_rules = store.extract_rules(episodes, &config);
        assert!(
            new_rules.is_empty(),
            "fewer than 5 episodes must produce no rules"
        );
        assert_eq!(store.count(), 0);
    }

    // ── Test 17 ─────────────────────────────────────────────────────────────

    #[test]
    fn extract_returns_rule_for_five_failures_same_signature() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let config = ExtractionConfig::default();

        let episodes: Vec<Episode> = (0..5)
            .map(|i| {
                failed_ep(
                    &format!("ep{i}"),
                    "E0277:Send+Sync",
                    "ConcurrencyRefactor",
                    &format!("reason {i}"),
                )
            })
            .collect();

        let new_rules = store.extract_rules(episodes, &config);
        assert_eq!(new_rules.len(), 1, "5 failures must produce exactly 1 rule");
        assert_eq!(
            new_rules[0].confidence, 0.5,
            "initial confidence must be 0.5"
        );
        assert_eq!(store.count(), 1);
    }

    // ── Test 18 ─────────────────────────────────────────────────────────────

    #[test]
    fn extract_skips_cluster_below_min_failure_rate() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let config = ExtractionConfig::default(); // min_failure_rate = 0.7

        // 5 episodes total, only 2 fail → 40% failure rate.
        let mut episodes: Vec<Episode> = (0..2)
            .map(|i| failed_ep(&format!("fail{i}"), "E0308", "TypeCheck", "type mismatch"))
            .collect();
        let passing: Vec<Episode> = (0..3)
            .map(|i| passing_ep(&format!("pass{i}"), "E0308", "TypeCheck"))
            .collect();
        episodes.extend(passing);

        let new_rules = store.extract_rules(episodes, &config);
        assert!(
            new_rules.is_empty(),
            "40% failure rate must not produce a rule (threshold is 70%)"
        );
    }

    // ── Test 19 ─────────────────────────────────────────────────────────────

    #[test]
    fn extract_populates_source_episodes_with_all_cluster_members() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let config = ExtractionConfig::default();

        let episodes: Vec<Episode> = (0..5)
            .map(|i| {
                failed_ep(
                    &format!("ep{i}"),
                    "E0382:use-of-moved",
                    "Ownership",
                    "moved value",
                )
            })
            .collect();

        let new_rules = store.extract_rules(episodes, &config);
        assert_eq!(new_rules.len(), 1);
        assert_eq!(
            new_rules[0].source_episodes.len(),
            5,
            "all 5 cluster members must be in source_episodes"
        );
        for i in 0..5 {
            assert!(
                new_rules[0].source_episodes.contains(&format!("ep{i}")),
                "ep{i} missing from source_episodes"
            );
        }
    }

    // ── Test 20 ─────────────────────────────────────────────────────────────

    #[test]
    fn extract_deduplicates_on_rerun_same_episodes() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        let config = ExtractionConfig::default();

        let episodes: Vec<Episode> = (0..5)
            .map(|i| {
                failed_ep(
                    &format!("ep{i}"),
                    "E0499:cannot-borrow",
                    "BorrowCheck",
                    "borrow error",
                )
            })
            .collect();

        // First extraction.
        store.extract_rules(episodes.clone(), &config);
        assert_eq!(store.count(), 1);

        // Second extraction with identical episodes must not add a duplicate rule.
        store.extract_rules(episodes, &config);
        assert_eq!(
            store.count(),
            1,
            "re-extracting same episodes must not duplicate the rule"
        );
    }

    // ── Test 21 ─────────────────────────────────────────────────────────────

    #[test]
    fn upsert_rejects_oversized_body() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");

        // Body exceeding 400 * 4 = 1600 bytes.
        let big_body = "x".repeat(1601);
        let mut rule = make_rule("big", 0.5, role_trigger("implementer"));
        rule.body = big_body;

        let result = store.upsert(rule);
        assert!(result.is_err(), "oversized body must be rejected");
        assert_eq!(
            result.expect_err("must fail").kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(store.count(), 0);
    }

    // ── Test 22 ─────────────────────────────────────────────────────────────

    #[test]
    fn count_and_snapshot_reflect_state() {
        let dir = TempDir::new().expect("create tempdir");
        let store = PlaybookRules::open(tmp_path(&dir, "r.toml")).expect("open");
        assert_eq!(store.count(), 0);
        assert!(store.snapshot().is_empty());

        store
            .upsert(make_rule("r1", 0.5, role_trigger("implementer")))
            .expect("upsert");
        store
            .upsert(make_rule("r2", 0.6, role_trigger("auto_fixer")))
            .expect("upsert");

        assert_eq!(store.count(), 2);
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);

        // Verify upsert-replace doesn't grow the store.
        store
            .upsert(make_rule("r1", 0.8, role_trigger("implementer")))
            .expect("upsert r1 again");
        assert_eq!(store.count(), 2);
        let snap2 = store.snapshot();
        let r1 = snap2
            .iter()
            .find(|r| r.rule_id == "r1")
            .expect("r1 present");
        assert!(
            (r1.confidence - 0.8).abs() < f64::EPSILON,
            "confidence should be updated"
        );
    }
}

//! In-memory costs database for recording and querying per-request LLM costs.
//!
//! This module provides a lightweight alternative to a full SQL database. The
//! [`CostsDb`] struct stores cost records in memory with `Vec`-backed storage
//! and supports queries by multiple dimensions: model, role, plan, complexity
//! band, and time range.
//!
//! # Design
//!
//! No `SQLite` dependency. Records are appended to a `Vec<CostRecord>` behind a
//! `parking_lot::RwLock`. Query methods scan the vec with linear complexity —
//! fine for up to ~100k records per run. For longer histories, records should
//! be serialized to JSONL and loaded on demand.
//!
//! Thread-safe: all read/write access is lock-protected.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── CostRecord ─────────────────────────────────────────────────────────────

/// One cost entry per LLM API request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostRecord {
    /// ISO-8601 UTC timestamp.
    pub timestamp: String,
    /// Model slug.
    pub model: String,
    /// Provider / backend (e.g. `"anthropic"`, `"openai"`).
    pub provider: String,
    /// Agent role.
    pub role: String,
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// Complexity band.
    pub complexity_band: String,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cached input tokens.
    pub cached_tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Wall-clock milliseconds for the request.
    pub duration_ms: u64,
    /// Whether the request succeeded.
    pub success: bool,
    /// Optional session / run identifier for grouping.
    pub session_id: String,
}

// ─── CostSummary ────────────────────────────────────────────────────────────

/// Aggregate summary over a set of cost records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSummary {
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total cached tokens.
    pub total_cached_tokens: u64,
    /// Number of records.
    pub record_count: usize,
    /// Average cost per record.
    pub avg_cost_usd: f64,
    /// Average duration per record.
    pub avg_duration_ms: f64,
    /// Success rate.
    pub success_rate: f64,
}

impl CostSummary {
    /// Compute a summary from a slice of records.
    #[allow(clippy::cast_precision_loss)]
    pub fn from_records(records: &[CostRecord]) -> Self {
        if records.is_empty() {
            return Self {
                total_cost_usd: 0.0,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cached_tokens: 0,
                record_count: 0,
                avg_cost_usd: 0.0,
                avg_duration_ms: 0.0,
                success_rate: 0.0,
            };
        }

        let n = records.len() as f64;
        let total_cost: f64 = records.iter().map(|r| r.cost_usd).sum();
        let total_input: u64 = records.iter().map(|r| r.input_tokens).sum();
        let total_output: u64 = records.iter().map(|r| r.output_tokens).sum();
        let total_cached: u64 = records.iter().map(|r| r.cached_tokens).sum();
        let total_duration: f64 = records.iter().map(|r| r.duration_ms as f64).sum();
        let successes = records.iter().filter(|r| r.success).count();

        Self {
            total_cost_usd: total_cost,
            total_input_tokens: total_input,
            total_output_tokens: total_output,
            total_cached_tokens: total_cached,
            record_count: records.len(),
            avg_cost_usd: total_cost / n,
            avg_duration_ms: total_duration / n,
            success_rate: successes as f64 / n,
        }
    }
}

// ─── CostTable ──────────────────────────────────────────────────────────────

/// Per-model pricing data stored in the default cost table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Input token cost per 1M tokens, in USD.
    pub input_per_m: f64,
    /// Output token cost per 1M tokens, in USD.
    pub output_per_m: f64,
    /// Cached input token cost per 1M tokens, in USD.
    pub cache_read_per_m: Option<f64>,
    /// Cache write cost per 1M tokens, in USD.
    pub cache_write_per_m: Option<f64>,
    /// Flat per-request fee, in USD (e.g. Perplexity search fee).
    pub per_request: Option<f64>,
}

impl ModelPricing {
    /// Estimate total cost for a request given token counts.
    ///
    /// Adds per-request fee (if any) on top of token-based costs.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn estimate_total(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let token_cost = (input_tokens as f64 / 1_000_000.0) * self.input_per_m
            + (output_tokens as f64 / 1_000_000.0) * self.output_per_m;
        token_cost + self.per_request.unwrap_or(0.0)
    }
}

/// Default pricing table keyed by model slug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostTable {
    /// Pricing records keyed by model slug.
    pub models: HashMap<String, ModelPricing>,
}

impl CostTable {
    /// Look up the pricing entry for a model slug.
    #[must_use]
    pub fn lookup(&self, model: &str) -> Option<&ModelPricing> {
        self.models.get(model)
    }

    /// Build the default cost table with the GLM, OpenRouter, and Perplexity pricing rows.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut models = HashMap::new();
        models.insert(
            "kimi-k2.5".to_string(),
            ModelPricing {
                input_per_m: 0.60,
                output_per_m: 3.00,
                cache_read_per_m: Some(0.10),
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "moonshotai/kimi-k2.5".to_string(),
            ModelPricing {
                input_per_m: 0.38,
                output_per_m: 1.72,
                cache_read_per_m: Some(0.10),
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "kimi-k2-thinking".to_string(),
            ModelPricing {
                input_per_m: 0.60,
                output_per_m: 2.50,
                cache_read_per_m: Some(0.15),
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "glm-5.1".to_string(),
            ModelPricing {
                input_per_m: 1.40,
                output_per_m: 4.40,
                cache_read_per_m: Some(0.26),
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "z-ai/glm-5.1".to_string(),
            ModelPricing {
                input_per_m: 1.26,
                output_per_m: 3.96,
                cache_read_per_m: Some(0.26),
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "glm-5".to_string(),
            ModelPricing {
                input_per_m: 1.00,
                output_per_m: 3.20,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "glm-4.7".to_string(),
            ModelPricing {
                input_per_m: 0.60,
                output_per_m: 2.20,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: None,
            },
        );
        models.insert(
            "anthropic/claude-opus-4-6".to_string(),
            ModelPricing {
                input_per_m: 15.00,
                output_per_m: 75.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: None,
            },
        );

        // Perplexity Sonar models — include per-request search fee.
        models.insert(
            "sonar".to_string(),
            ModelPricing {
                input_per_m: 1.00,
                output_per_m: 1.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: Some(0.005),
            },
        );
        models.insert(
            "sonar-pro".to_string(),
            ModelPricing {
                input_per_m: 3.00,
                output_per_m: 15.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: Some(0.014),
            },
        );
        models.insert(
            "sonar-reasoning".to_string(),
            ModelPricing {
                input_per_m: 1.00,
                output_per_m: 5.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: Some(0.005),
            },
        );
        models.insert(
            "sonar-reasoning-pro".to_string(),
            ModelPricing {
                input_per_m: 2.00,
                output_per_m: 8.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: Some(0.008),
            },
        );
        models.insert(
            "sonar-deep-research".to_string(),
            ModelPricing {
                input_per_m: 2.00,
                output_per_m: 8.00,
                cache_read_per_m: None,
                cache_write_per_m: None,
                per_request: Some(0.005),
            },
        );

        Self { models }
    }
}

impl Default for CostTable {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ─── CostsDb ────────────────────────────────────────────────────────────────

/// In-memory costs database with query methods.
///
/// Thread-safe via `parking_lot::RwLock`.
pub struct CostsDb {
    records: RwLock<Vec<CostRecord>>,
}

impl CostsDb {
    /// Create an empty database.
    pub const fn new() -> Self {
        Self {
            records: RwLock::new(Vec::new()),
        }
    }

    /// Insert a single cost record.
    pub fn insert(&self, record: CostRecord) {
        self.records.write().push(record);
    }

    /// Insert multiple cost records.
    pub fn insert_batch(&self, records: Vec<CostRecord>) {
        self.records.write().extend(records);
    }

    /// Total number of records in the database.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }

    /// Retrieve all records (clone).
    pub fn all(&self) -> Vec<CostRecord> {
        self.records.read().clone()
    }

    /// Query records by model.
    pub fn by_model(&self, model: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.model == model)
            .cloned()
            .collect()
    }

    /// Query records by provider.
    pub fn by_provider(&self, provider: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.provider == provider)
            .cloned()
            .collect()
    }

    /// Query records by role.
    pub fn by_role(&self, role: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.role == role)
            .cloned()
            .collect()
    }

    /// Query records by plan.
    pub fn by_plan(&self, plan_id: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.plan_id == plan_id)
            .cloned()
            .collect()
    }

    /// Query records by complexity band.
    pub fn by_complexity(&self, band: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.complexity_band == band)
            .cloned()
            .collect()
    }

    /// Query records by session.
    pub fn by_session(&self, session_id: &str) -> Vec<CostRecord> {
        self.records
            .read()
            .iter()
            .filter(|r| r.session_id == session_id)
            .cloned()
            .collect()
    }

    /// Compute a summary over all records.
    pub fn summary(&self) -> CostSummary {
        let records = self.records.read();
        CostSummary::from_records(&records)
    }

    /// Compute a summary grouped by model.
    pub fn summary_by_model(&self) -> HashMap<String, CostSummary> {
        let snapshot = self.all();
        let mut groups: HashMap<String, Vec<CostRecord>> = HashMap::new();
        for r in snapshot {
            groups.entry(r.model.clone()).or_default().push(r);
        }
        groups
            .into_iter()
            .map(|(k, v)| (k, CostSummary::from_records(&v)))
            .collect()
    }

    /// Compute a summary grouped by role.
    pub fn summary_by_role(&self) -> HashMap<String, CostSummary> {
        let snapshot = self.all();
        let mut groups: HashMap<String, Vec<CostRecord>> = HashMap::new();
        for r in snapshot {
            groups.entry(r.role.clone()).or_default().push(r);
        }
        groups
            .into_iter()
            .map(|(k, v)| (k, CostSummary::from_records(&v)))
            .collect()
    }

    /// Compute a summary grouped by plan.
    pub fn summary_by_plan(&self) -> HashMap<String, CostSummary> {
        let snapshot = self.all();
        let mut groups: HashMap<String, Vec<CostRecord>> = HashMap::new();
        for r in snapshot {
            groups.entry(r.plan_id.clone()).or_default().push(r);
        }
        groups
            .into_iter()
            .map(|(k, v)| (k, CostSummary::from_records(&v)))
            .collect()
    }

    /// Total cost across all records.
    pub fn total_cost(&self) -> f64 {
        self.records.read().iter().map(|r| r.cost_usd).sum()
    }

    /// Clear all records.
    pub fn clear(&self) {
        self.records.write().clear();
    }

    /// Export all records as JSONL text.
    ///
    /// # Errors
    ///
    /// Returns an error if any record fails to serialize (should never
    /// happen in practice).
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        let snapshot = self.all();
        let mut out = String::new();
        for r in &snapshot {
            let line = serde_json::to_string(r)?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// Import records from JSONL text, appending to existing records.
    ///
    /// Tolerant of corrupted lines — skips unparseable lines and returns the
    /// count of successfully imported records.
    pub fn from_jsonl(&self, text: &str) -> usize {
        let mut parsed = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(r) = serde_json::from_str::<CostRecord>(trimmed) {
                parsed.push(r);
            }
        }
        let count = parsed.len();
        self.records.write().extend(parsed);
        count
    }
}

impl Default for CostsDb {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Create a test fixture [`CostRecord`].
#[cfg(test)]
fn make_test_record(
    model: &str,
    provider: &str,
    role: &str,
    plan_id: &str,
    cost: f64,
    success: bool,
) -> CostRecord {
    CostRecord {
        timestamp: "2026-04-06T12:00:00Z".into(),
        model: model.into(),
        provider: provider.into(),
        role: role.into(),
        plan_id: plan_id.into(),
        task_id: "t1".into(),
        complexity_band: "standard".into(),
        input_tokens: 1000,
        output_tokens: 200,
        cached_tokens: 100,
        cost_usd: cost,
        duration_ms: 5000,
        success,
        session_id: "session-1".into(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glm_cost_table() {
        let table = CostTable::default();

        let glm_5_1 = table.lookup("glm-5.1").expect("glm-5.1 pricing");
        assert!((glm_5_1.input_per_m - 1.40).abs() < 1e-9);
        assert!((glm_5_1.output_per_m - 4.40).abs() < 1e-9);
        assert_eq!(glm_5_1.cache_read_per_m, Some(0.26));

        let glm_5 = table.lookup("glm-5").expect("glm-5 pricing");
        assert!((glm_5.input_per_m - 1.00).abs() < 1e-9);
        assert!((glm_5.output_per_m - 3.20).abs() < 1e-9);
        assert_eq!(glm_5.cache_read_per_m, None);

        let glm_4_7 = table.lookup("glm-4.7").expect("glm-4.7 pricing");
        assert!((glm_4_7.input_per_m - 0.60).abs() < 1e-9);
        assert!((glm_4_7.output_per_m - 2.20).abs() < 1e-9);
        assert_eq!(glm_4_7.cache_read_per_m, None);
    }

    #[test]
    fn kimi_cost_table() {
        let table = CostTable::default();

        let kimi_k2_5 = table.lookup("kimi-k2.5").expect("kimi-k2.5 pricing");
        assert!((kimi_k2_5.input_per_m - 0.60).abs() < 1e-9);
        assert!((kimi_k2_5.output_per_m - 3.00).abs() < 1e-9);
        assert_eq!(kimi_k2_5.cache_read_per_m, Some(0.10));

        let kimi_k2_thinking = table
            .lookup("kimi-k2-thinking")
            .expect("kimi-k2-thinking pricing");
        assert!((kimi_k2_thinking.input_per_m - 0.60).abs() < 1e-9);
        assert!((kimi_k2_thinking.output_per_m - 2.50).abs() < 1e-9);
        assert_eq!(kimi_k2_thinking.cache_read_per_m, Some(0.15));
    }

    #[test]
    fn openrouter_cost_table() {
        let table = CostTable::default();

        let glm_5_1 = table.lookup("z-ai/glm-5.1").expect("z-ai/glm-5.1 pricing");
        assert!((glm_5_1.input_per_m - 1.26).abs() < 1e-9);
        assert!((glm_5_1.output_per_m - 3.96).abs() < 1e-9);

        let kimi_k2_5 = table
            .lookup("moonshotai/kimi-k2.5")
            .expect("moonshotai/kimi-k2.5 pricing");
        assert!((kimi_k2_5.input_per_m - 0.38).abs() < 1e-9);
        assert!((kimi_k2_5.output_per_m - 1.72).abs() < 1e-9);

        let claude_opus = table
            .lookup("anthropic/claude-opus-4-6")
            .expect("anthropic/claude-opus-4-6 pricing");
        assert!((claude_opus.input_per_m - 15.00).abs() < 1e-9);
        assert!((claude_opus.output_per_m - 75.00).abs() < 1e-9);
    }

    #[test]
    fn costs_db_insert_and_len() {
        let db = CostsDb::new();
        assert!(db.is_empty());

        db.insert(make_test_record(
            "sonnet",
            "anthropic",
            "Impl",
            "p1",
            0.50,
            true,
        ));
        assert_eq!(db.len(), 1);
        assert!(!db.is_empty());
    }

    #[test]
    fn costs_db_insert_batch() {
        let db = CostsDb::new();
        db.insert_batch(vec![
            make_test_record("sonnet", "anthropic", "Impl", "p1", 0.50, true),
            make_test_record("haiku", "anthropic", "Review", "p2", 0.10, true),
        ]);
        assert_eq!(db.len(), 2);
    }

    #[test]
    fn costs_db_query_by_model() {
        let db = CostsDb::new();
        db.insert(make_test_record(
            "sonnet",
            "anthropic",
            "Impl",
            "p1",
            0.50,
            true,
        ));
        db.insert(make_test_record(
            "haiku",
            "anthropic",
            "Impl",
            "p2",
            0.10,
            true,
        ));
        db.insert(make_test_record(
            "sonnet",
            "anthropic",
            "Review",
            "p3",
            0.30,
            true,
        ));

        let sonnet = db.by_model("sonnet");
        assert_eq!(sonnet.len(), 2);
        let haiku = db.by_model("haiku");
        assert_eq!(haiku.len(), 1);
        let opus = db.by_model("opus");
        assert!(opus.is_empty());
    }

    #[test]
    fn costs_db_query_by_role() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Implementer", "p1", 0.50, true));
        db.insert(make_test_record("s", "a", "Reviewer", "p2", 0.10, true));
        db.insert(make_test_record("s", "a", "Implementer", "p3", 0.30, true));

        let impls = db.by_role("Implementer");
        assert_eq!(impls.len(), 2);
        let reviews = db.by_role("Reviewer");
        assert_eq!(reviews.len(), 1);
    }

    #[test]
    fn costs_db_query_by_plan() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "plan-1", 0.50, true));
        db.insert(make_test_record("s", "a", "Impl", "plan-2", 0.10, true));

        let p1 = db.by_plan("plan-1");
        assert_eq!(p1.len(), 1);
    }

    #[test]
    fn costs_db_query_by_provider() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "anthropic", "Impl", "p1", 0.50, true));
        db.insert(make_test_record(
            "gpt-4o", "openai", "Impl", "p2", 0.30, true,
        ));

        assert_eq!(db.by_provider("anthropic").len(), 1);
        assert_eq!(db.by_provider("openai").len(), 1);
    }

    #[test]
    fn costs_db_query_by_complexity() {
        let db = CostsDb::new();
        let mut r = make_test_record("s", "a", "Impl", "p1", 0.50, true);
        r.complexity_band = "complex".into();
        db.insert(r);
        db.insert(make_test_record("s", "a", "Impl", "p2", 0.10, true));

        assert_eq!(db.by_complexity("complex").len(), 1);
        assert_eq!(db.by_complexity("standard").len(), 1);
    }

    #[test]
    fn costs_db_query_by_session() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "p1", 0.50, true));
        let mut r = make_test_record("s", "a", "Impl", "p2", 0.30, true);
        r.session_id = "session-2".into();
        db.insert(r);

        assert_eq!(db.by_session("session-1").len(), 1);
        assert_eq!(db.by_session("session-2").len(), 1);
    }

    #[test]
    fn costs_db_summary() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "p1", 0.50, true));
        db.insert(make_test_record("s", "a", "Impl", "p2", 0.30, false));

        let s = db.summary();
        assert_eq!(s.record_count, 2);
        assert!((s.total_cost_usd - 0.80).abs() < 1e-9);
        assert!((s.avg_cost_usd - 0.40).abs() < 1e-9);
        assert!((s.success_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn costs_db_summary_empty() {
        let db = CostsDb::new();
        let s = db.summary();
        assert_eq!(s.record_count, 0);
        assert!((s.total_cost_usd).abs() < 1e-9);
    }

    #[test]
    fn costs_db_summary_by_model() {
        let db = CostsDb::new();
        db.insert(make_test_record("sonnet", "a", "Impl", "p1", 0.50, true));
        db.insert(make_test_record("sonnet", "a", "Impl", "p2", 0.30, true));
        db.insert(make_test_record("haiku", "a", "Impl", "p3", 0.10, true));

        let by_model = db.summary_by_model();
        assert_eq!(by_model.len(), 2);
        assert_eq!(by_model.get("sonnet").map(|s| s.record_count), Some(2));
        assert_eq!(by_model.get("haiku").map(|s| s.record_count), Some(1));
    }

    #[test]
    fn costs_db_summary_by_role() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Implementer", "p1", 0.50, true));
        db.insert(make_test_record("s", "a", "Reviewer", "p2", 0.20, true));

        let by_role = db.summary_by_role();
        assert_eq!(by_role.len(), 2);
        assert!(by_role.contains_key("Implementer"));
        assert!(by_role.contains_key("Reviewer"));
    }

    #[test]
    fn costs_db_summary_by_plan() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "plan-A", 0.50, true));
        db.insert(make_test_record("s", "a", "Impl", "plan-A", 0.30, true));
        db.insert(make_test_record("s", "a", "Impl", "plan-B", 0.10, true));

        let by_plan = db.summary_by_plan();
        assert_eq!(by_plan.len(), 2);
        let plan_a = by_plan.get("plan-A").expect("plan-A should exist");
        assert!((plan_a.total_cost_usd - 0.80).abs() < 1e-9);
    }

    #[test]
    fn costs_db_total_cost() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "p1", 0.50, true));
        db.insert(make_test_record("s", "a", "Impl", "p2", 0.30, true));
        assert!((db.total_cost() - 0.80).abs() < 1e-9);
    }

    #[test]
    fn costs_db_clear() {
        let db = CostsDb::new();
        db.insert(make_test_record("s", "a", "Impl", "p1", 0.50, true));
        assert_eq!(db.len(), 1);
        db.clear();
        assert!(db.is_empty());
    }

    #[test]
    fn costs_db_jsonl_roundtrip() {
        let db = CostsDb::new();
        db.insert(make_test_record(
            "sonnet",
            "anthropic",
            "Impl",
            "p1",
            0.50,
            true,
        ));
        db.insert(make_test_record(
            "haiku",
            "anthropic",
            "Review",
            "p2",
            0.10,
            false,
        ));

        let jsonl = db.to_jsonl().expect("serialize");

        let db2 = CostsDb::new();
        let imported = db2.from_jsonl(&jsonl);
        assert_eq!(imported, 2);
        assert_eq!(db2.len(), 2);

        let all1 = db.all();
        let all2 = db2.all();
        assert_eq!(all1, all2);
    }

    #[test]
    fn costs_db_jsonl_tolerates_bad_lines() {
        let db = CostsDb::new();
        let good = make_test_record("s", "a", "Impl", "p1", 0.50, true);
        let line = serde_json::to_string(&good).expect("serialize");
        let text = format!("{line}\nnot-valid-json\n{line}\n");

        let imported = db.from_jsonl(&text);
        assert_eq!(imported, 2);
    }

    #[test]
    fn costs_db_default() {
        let db = CostsDb::default();
        assert!(db.is_empty());
    }

    #[test]
    fn costs_db_cost_record_serialization() {
        let r = make_test_record("sonnet", "anthropic", "Impl", "p1", 0.42, true);
        let json = serde_json::to_string(&r).expect("serialize");
        let r2: CostRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, r2);
    }

    #[test]
    fn perplexity_costs() {
        let table = CostTable::default();

        // sonar: $1.00/M in, $1.00/M out, $0.005 per-request
        let sonar = table.lookup("sonar").expect("sonar pricing");
        assert!((sonar.input_per_m - 1.00).abs() < 1e-9);
        assert!((sonar.output_per_m - 1.00).abs() < 1e-9);
        assert_eq!(sonar.per_request, Some(0.005));

        // sonar-pro: $3.00/M in, $15.00/M out, $0.014 per-request
        let sonar_pro = table.lookup("sonar-pro").expect("sonar-pro pricing");
        assert!((sonar_pro.input_per_m - 3.00).abs() < 1e-9);
        assert!((sonar_pro.output_per_m - 15.00).abs() < 1e-9);
        assert_eq!(sonar_pro.per_request, Some(0.014));

        // sonar-reasoning: $1.00/M in, $5.00/M out, $0.005 per-request
        let sonar_r = table
            .lookup("sonar-reasoning")
            .expect("sonar-reasoning pricing");
        assert!((sonar_r.input_per_m - 1.00).abs() < 1e-9);
        assert!((sonar_r.output_per_m - 5.00).abs() < 1e-9);
        assert_eq!(sonar_r.per_request, Some(0.005));

        // sonar-reasoning-pro: $2.00/M in, $8.00/M out, $0.008 per-request
        let sonar_rp = table
            .lookup("sonar-reasoning-pro")
            .expect("sonar-reasoning-pro pricing");
        assert!((sonar_rp.input_per_m - 2.00).abs() < 1e-9);
        assert!((sonar_rp.output_per_m - 8.00).abs() < 1e-9);
        assert_eq!(sonar_rp.per_request, Some(0.008));

        // sonar-deep-research: $2.00/M in, $8.00/M out, $0.005 per-request
        let sonar_dr = table
            .lookup("sonar-deep-research")
            .expect("sonar-deep-research pricing");
        assert!((sonar_dr.input_per_m - 2.00).abs() < 1e-9);
        assert!((sonar_dr.output_per_m - 8.00).abs() < 1e-9);
        assert_eq!(sonar_dr.per_request, Some(0.005));

        // estimate_total includes the per-request fee.
        // 1M input + 1M output on sonar = $1.00 + $1.00 + $0.005 = $2.005
        let total = sonar.estimate_total(1_000_000, 1_000_000);
        assert!((total - 2.005).abs() < 1e-9);

        // 500k input + 200k output on sonar-pro:
        // token = 0.5 * $3.00 + 0.2 * $15.00 = $1.50 + $3.00 = $4.50
        // + $0.014 per-request = $4.514
        let total_pro = sonar_pro.estimate_total(500_000, 200_000);
        assert!((total_pro - 4.514).abs() < 1e-9);

        // Non-Perplexity model has no per-request fee.
        let glm_5 = table.lookup("glm-5").expect("glm-5 pricing");
        assert_eq!(glm_5.per_request, None);
        let glm_total = glm_5.estimate_total(1_000_000, 1_000_000);
        assert!((glm_total - 4.20).abs() < 1e-9);
    }

    #[test]
    fn costs_db_cost_summary_from_records() {
        let records = vec![
            make_test_record("s", "a", "Impl", "p1", 0.50, true),
            make_test_record("s", "a", "Impl", "p2", 0.30, true),
            make_test_record("s", "a", "Impl", "p3", 0.20, false),
        ];

        let summary = CostSummary::from_records(&records);
        assert_eq!(summary.record_count, 3);
        assert!((summary.total_cost_usd - 1.00).abs() < 1e-9);
        assert!((summary.success_rate - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(summary.total_input_tokens, 3000);
        assert_eq!(summary.total_output_tokens, 600);
        assert_eq!(summary.total_cached_tokens, 300);
    }
}

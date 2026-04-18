//! Phase 2 sleep-time compute stubs.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sleep-time pre-computation settings for predictable query patterns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleepTimePrecompute {
    /// Whether to generate pre-computed summaries during NREM.
    pub enable_precompute: bool,
    /// Maximum summary token count per cached chunk.
    pub max_summary_tokens: usize,
    /// Minimum predictability required before pre-computing.
    pub predictability_threshold: f64,
    /// Maximum cached summaries to retain.
    pub max_cached_summaries: usize,
    /// Cache time-to-live in hours.
    pub cache_ttl_hours: u64,
    /// Whether savings should be measured and logged.
    pub measure_savings: bool,
}

impl Default for SleepTimePrecompute {
    fn default() -> Self {
        Self {
            enable_precompute: true,
            max_summary_tokens: 512,
            predictability_threshold: 0.60,
            max_cached_summaries: 100,
            cache_ttl_hours: 24,
            measure_savings: true,
        }
    }
}

impl SleepTimePrecompute {
    /// Construct the documented default pre-compute settings.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            enable_precompute: true,
            max_summary_tokens: 512,
            predictability_threshold: 0.60,
            max_cached_summaries: 100,
            cache_ttl_hours: 24,
            measure_savings: true,
        }
    }
}

/// Pre-computed summary for a recurring query pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrecomputedSummary {
    /// Stable summary identifier.
    pub id: String,
    /// Recurring query pattern that triggered the summary.
    pub query_pattern: String,
    /// Cached summary content.
    pub summary_content: String,
    /// Token count of the cached summary.
    pub token_count: usize,
    /// Predictability score for the query pattern.
    pub predictability_score: f64,
    /// Time at which the summary was created.
    pub created_at: DateTime<Utc>,
    /// Expiration time for the summary.
    pub expires_at: DateTime<Utc>,
    /// Number of times the summary has been used.
    pub times_used: usize,
    /// Estimated tokens saved by caching the summary.
    pub estimated_tokens_saved: usize,
}

impl PrecomputedSummary {
    /// Construct a pre-computed summary record with neutral stub metadata.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        query_pattern: impl Into<String>,
        summary_content: impl Into<String>,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        let summary_content = summary_content.into();
        Self {
            id: id.into(),
            query_pattern: query_pattern.into(),
            token_count: summary_content.split_whitespace().count(),
            summary_content,
            predictability_score: 0.0,
            created_at,
            expires_at,
            times_used: 0,
            estimated_tokens_saved: 0,
        }
    }

    /// Check whether the summary has expired at the supplied time.
    #[must_use]
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

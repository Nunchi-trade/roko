//! Rate limiting middleware for the `chain_*` JSON-RPC surface (§38.e).
//!
//! This module exposes a pure, lock-free-ish [`RateLimiter`] that callers can
//! consult **before** dispatching a method. It does not wire itself into the
//! RPC server — that is a later synthesis step. The API is intentionally a
//! simple `check_method` function so that HTTP and `WebSocket` transports can
//! both consult the same limiter.
//!
//! # Design
//!
//! * Per-method token buckets (keyed by method name) enforce the per-method
//!   RPS budgets required by §38.17 (reads: 100/s, writes: 10/s).
//! * Per-author token buckets enforce the per-author write quota required by
//!   §38.18 (default: 3600 posts/hour == 1/s sliding average).
//! * Classification ([`classify_method`]) maps each of the 12 `chain_*`
//!   methods onto [`MethodClass::Read`], [`MethodClass::Write`], or
//!   [`MethodClass::Subscription`]. Subscription methods are exempt from RPS
//!   checks — their rate is bounded by the connection-count limits enforced
//!   elsewhere (out of scope for §38.e).
//! * Each bucket uses `parking_lot::Mutex`; the critical section is O(1) and
//!   the Mutex itself is uncontended under typical load.
//! * Token buckets are pure: a denied request does **not** deduct tokens or
//!   mutate refill state. Only `Ok(())` returns consume a token.
//!
//! # Error surface
//!
//! Violations produce a [`RateLimitError`] which callers convert to a JSON-RPC
//! error via [`to_rpc_error`]. The error code is [`RATE_LIMIT_ERROR_CODE`]
//! (`-32105`), which sits in the `chain_rpc::err_code` namespace but is
//! intentionally defined here to avoid a cyclic dependency between modules.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::significant_drop_tightening,
    clippy::needless_pass_by_value,
    clippy::too_long_first_doc_paragraph
)]

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use parking_lot::Mutex;
use serde_json::json;

/// Default per-method RPS budget for read methods (§38.17).
pub const DEFAULT_READ_RPS: u32 = 100;

/// Default per-method RPS budget for write methods (§38.17).
pub const DEFAULT_WRITE_RPS: u32 = 10;

/// Default per-author write quota per hour (§38.18).
pub const DEFAULT_AUTHOR_WRITES_PER_HOUR: u32 = 3600;

/// JSON-RPC error code reserved for rate-limit violations (§38.19).
///
/// Follows the `chain_rpc::err_code` range (`-32100..=-32199`).
pub const RATE_LIMIT_ERROR_CODE: i32 = -32105;

/// Classification of a `chain_*` method for rate-limit purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodClass {
    /// Read method. Subject to the read-RPS budget.
    Read,
    /// Write method. Subject to the write-RPS budget and per-author quota.
    Write,
    /// Subscription lifecycle method (subscribe / unsubscribe). Exempt from
    /// RPS checks; bounded by connection-count limits elsewhere.
    Subscription,
}

/// Classifies a `chain_*` method name for rate-limit purposes.
///
/// Unknown methods default to [`MethodClass::Read`] — they do not escape
/// rate limiting entirely, merely take the more generous budget.
#[must_use]
pub fn classify_method(method: &str) -> MethodClass {
    match method {
        "chain_searchInsights"
        | "chain_queryPheromones"
        | "chain_getInsight"
        | "chain_stats"
        | "chain_version"
        | "chain_listKinds"
        | "chain_methodSchema" => MethodClass::Read,
        "chain_postInsight"
        | "chain_confirmInsight"
        | "chain_challengeInsight"
        | "chain_depositPheromone"
        | "chain_applyDecay" => MethodClass::Write,
        "chain_subscribePheromones" | "chain_subscribeInsights" | "chain_unsubscribe" => {
            MethodClass::Subscription
        }
        _ => MethodClass::Read,
    }
}

/// Configuration for a [`RateLimiter`].
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Per-method RPS budget for [`MethodClass::Read`] methods.
    pub read_rps: u32,
    /// Per-method RPS budget for [`MethodClass::Write`] methods.
    pub write_rps: u32,
    /// Per-author writes-per-hour quota.
    pub author_writes_per_hour: u32,
    /// Per-method overrides keyed by the full method name. Values are RPS.
    pub per_method_overrides: HashMap<String, u32>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            read_rps: DEFAULT_READ_RPS,
            write_rps: DEFAULT_WRITE_RPS,
            author_writes_per_hour: DEFAULT_AUTHOR_WRITES_PER_HOUR,
            per_method_overrides: HashMap::new(),
        }
    }
}

/// Rate-limit violation returned by [`RateLimiter::check_method`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitError {
    /// The per-method RPS budget is exhausted.
    MethodBudgetExceeded {
        /// The method that exceeded its budget.
        method: String,
        /// The configured RPS limit for this method.
        limit_rps: u32,
        /// Milliseconds the caller should wait before retrying.
        retry_after_ms: u64,
    },
    /// The per-author quota is exhausted.
    AuthorQuotaExceeded {
        /// The author identity (arbitrary caller-supplied string).
        author: String,
        /// The configured per-hour quota.
        limit_per_hour: u32,
        /// Milliseconds until the quota resets.
        reset_at_ms: u64,
    },
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MethodBudgetExceeded {
                method,
                limit_rps,
                retry_after_ms,
            } => write!(
                f,
                "rate limit exceeded for method '{method}' (limit {limit_rps}/s, retry after {retry_after_ms}ms)"
            ),
            Self::AuthorQuotaExceeded {
                author,
                limit_per_hour,
                reset_at_ms,
            } => write!(
                f,
                "author quota exceeded for '{author}' (limit {limit_per_hour}/hour, resets in {reset_at_ms}ms)"
            ),
        }
    }
}

impl std::error::Error for RateLimitError {}

/// Snapshot of the limiter's in-flight accounting.
#[derive(Debug, Clone, Default)]
pub struct RateLimitStats {
    /// Estimated current RPS per method. Computed from bucket occupancy so
    /// it reflects recent demand rather than raw call counters.
    pub per_method_rps_current: HashMap<String, f64>,
    /// Remaining token budget per method (in tokens; fractional).
    pub per_method_budget_remaining: HashMap<String, f64>,
    /// Remaining author quota (in whole writes) per known author.
    pub per_author_remaining: HashMap<String, u32>,
    /// Total requests that passed `check_method` across the limiter's life.
    pub total_allowed: u64,
    /// Total requests that were denied.
    pub total_denied: u64,
}

/// A refillable token bucket.
///
/// The bucket holds fractional tokens (`f64`) and refills at a fixed rate of
/// `capacity / refill_period`. Refill is computed at check time from the
/// monotonic clock; there is no background task.
#[derive(Debug)]
struct TokenBucket {
    capacity: f64,
    refill_period: Duration,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: u32, refill_period: Duration) -> Self {
        let cap = f64::from(capacity);
        Self {
            capacity: cap,
            refill_period,
            tokens: cap,
            last_refill: Instant::now(),
        }
    }

    /// Refill based on elapsed time, capping at capacity.
    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        if elapsed.is_zero() {
            return;
        }
        let period_secs = self.refill_period.as_secs_f64();
        if period_secs <= 0.0 {
            self.tokens = self.capacity;
            self.last_refill = now;
            return;
        }
        let refill_rate = self.capacity / period_secs;
        let refill = elapsed.as_secs_f64() * refill_rate;
        self.tokens = (self.tokens + refill).min(self.capacity);
        self.last_refill = now;
    }

    /// Try to consume one token. Returns `Ok(())` on success, or `Err` with
    /// the millisecond delay until the next token would be available.
    ///
    /// Pure on denial: `last_refill` and `tokens` reflect the refill computed
    /// at `now`, but no token is deducted.
    fn try_consume(&mut self, now: Instant) -> Result<(), u64> {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let period_secs = self.refill_period.as_secs_f64();
            if period_secs <= 0.0 || self.capacity <= 0.0 {
                return Err(0);
            }
            let refill_rate = self.capacity / period_secs;
            let deficit = 1.0 - self.tokens;
            let wait_secs = deficit / refill_rate;
            let wait_ms = (wait_secs * 1000.0).ceil() as u64;
            Err(wait_ms.max(1))
        }
    }
}

/// Token-bucket rate limiter with per-method and per-author enforcement.
///
/// Internally stores one [`TokenBucket`] per method (created lazily on first
/// observation) plus one bucket per author for write-method quota
/// enforcement. Buckets are each guarded by a short-lived `Mutex`; the hot
/// path is a single `Mutex` acquire per `check_method` call.
#[derive(Debug)]
pub struct RateLimiter {
    config: RateLimitConfig,
    method_buckets: Mutex<HashMap<String, Mutex<TokenBucket>>>,
    author_buckets: Mutex<HashMap<String, Mutex<TokenBucket>>>,
    total_allowed: AtomicU64,
    total_denied: AtomicU64,
}

impl RateLimiter {
    /// Constructs a new limiter with the provided configuration.
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            method_buckets: Mutex::new(HashMap::new()),
            author_buckets: Mutex::new(HashMap::new()),
            total_allowed: AtomicU64::new(0),
            total_denied: AtomicU64::new(0),
        }
    }

    /// Returns the effective RPS limit for a given method.
    fn method_limit_rps(&self, method: &str, class: MethodClass) -> u32 {
        if let Some(override_rps) = self.config.per_method_overrides.get(method) {
            return *override_rps;
        }
        match class {
            MethodClass::Read => self.config.read_rps,
            MethodClass::Write => self.config.write_rps,
            MethodClass::Subscription => 0, // unused
        }
    }

    /// Gets or creates the per-method bucket with the correct capacity.
    fn with_method_bucket<R>(
        &self,
        method: &str,
        capacity: u32,
        f: impl FnOnce(&mut TokenBucket) -> R,
    ) -> R {
        // Fast path: bucket already exists.
        let map = self.method_buckets.lock();
        if let Some(cell) = map.get(method) {
            let mut b = cell.lock();
            return f(&mut b);
        }
        drop(map);
        // Slow path: insert a fresh bucket.
        let mut map = self.method_buckets.lock();
        let cell = map
            .entry(method.to_owned())
            .or_insert_with(|| Mutex::new(TokenBucket::new(capacity, Duration::from_secs(1))));
        let mut b = cell.lock();
        f(&mut b)
    }

    /// Gets or creates the per-author bucket.
    fn with_author_bucket<R>(&self, author: &str, f: impl FnOnce(&mut TokenBucket) -> R) -> R {
        let capacity = self.config.author_writes_per_hour;
        let map = self.author_buckets.lock();
        if let Some(cell) = map.get(author) {
            let mut b = cell.lock();
            return f(&mut b);
        }
        drop(map);
        let mut map = self.author_buckets.lock();
        let cell = map
            .entry(author.to_owned())
            .or_insert_with(|| Mutex::new(TokenBucket::new(capacity, Duration::from_secs(3600))));
        let mut b = cell.lock();
        f(&mut b)
    }

    /// Consults both the per-method and (if writable) per-author budgets.
    ///
    /// * `method` — the full JSON-RPC method name (e.g. `"chain_postInsight"`).
    /// * `author` — the author identity (required for write methods if you
    ///   want per-author enforcement; optional otherwise).
    ///
    /// On success, both buckets have been debited. On failure, **no tokens
    /// are consumed** — callers may retry after the returned delay.
    ///
    /// # Errors
    ///
    /// * [`RateLimitError::MethodBudgetExceeded`] — the per-method RPS budget
    ///   is exhausted.
    /// * [`RateLimitError::AuthorQuotaExceeded`] — the per-author writes/hour
    ///   quota is exhausted.
    pub fn check_method(&self, method: &str, author: Option<&str>) -> Result<(), RateLimitError> {
        let class = classify_method(method);
        if matches!(class, MethodClass::Subscription) {
            self.total_allowed.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }

        let limit_rps = self.method_limit_rps(method, class);
        let now = Instant::now();

        // Per-method RPS check. On denial, bail — nothing else was mutated.
        let method_res =
            self.with_method_bucket(method, limit_rps, |bucket| bucket.try_consume(now));
        if let Err(retry_after_ms) = method_res {
            self.total_denied.fetch_add(1, Ordering::Relaxed);
            return Err(RateLimitError::MethodBudgetExceeded {
                method: method.to_owned(),
                limit_rps,
                retry_after_ms,
            });
        }

        // Per-author quota check for write methods.
        if matches!(class, MethodClass::Write) {
            if let Some(author_id) = author {
                let res = self.with_author_bucket(author_id, |bucket| bucket.try_consume(now));
                if let Err(reset_at_ms) = res {
                    // Refund the method-bucket token we just consumed so the
                    // failed attempt does not also exhaust method budget.
                    // (Refund is best-effort: recompute and add one back.)
                    self.with_method_bucket(method, limit_rps, |bucket| {
                        bucket.tokens = (bucket.tokens + 1.0).min(bucket.capacity);
                    });
                    self.total_denied.fetch_add(1, Ordering::Relaxed);
                    return Err(RateLimitError::AuthorQuotaExceeded {
                        author: author_id.to_owned(),
                        limit_per_hour: self.config.author_writes_per_hour,
                        reset_at_ms,
                    });
                }
            }
        }

        self.total_allowed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Returns a snapshot of current rate-limit state.
    #[must_use]
    pub fn stats(&self) -> RateLimitStats {
        let now = Instant::now();
        let mut per_method_rps_current = HashMap::new();
        let mut per_method_budget_remaining = HashMap::new();
        let mut per_author_remaining = HashMap::new();

        let method_map = self.method_buckets.lock();
        for (method, cell) in method_map.iter() {
            let mut b = cell.lock();
            b.refill(now);
            per_method_budget_remaining.insert(method.clone(), b.tokens);
            // Current demand estimate: capacity - remaining = tokens consumed
            // in the most recent window (approx "RPS" for a 1s bucket).
            let used = (b.capacity - b.tokens).max(0.0);
            per_method_rps_current.insert(method.clone(), used);
        }
        drop(method_map);

        let author_map = self.author_buckets.lock();
        for (author, cell) in author_map.iter() {
            let mut b = cell.lock();
            b.refill(now);
            let remaining = b.tokens.floor().max(0.0) as u32;
            per_author_remaining.insert(author.clone(), remaining);
        }
        drop(author_map);

        RateLimitStats {
            per_method_rps_current,
            per_method_budget_remaining,
            per_author_remaining,
            total_allowed: self.total_allowed.load(Ordering::Relaxed),
            total_denied: self.total_denied.load(Ordering::Relaxed),
        }
    }
}

/// Converts a [`RateLimitError`] into a JSON-RPC error using
/// [`RATE_LIMIT_ERROR_CODE`]. The `data` field carries a structured payload
/// (`retry_after_ms` / `reset_at_ms` / `limit`) so clients can implement
/// precise backoff.
#[must_use]
pub fn to_rpc_error(err: RateLimitError) -> ErrorObjectOwned {
    let message = err.to_string();
    let data = match &err {
        RateLimitError::MethodBudgetExceeded {
            method,
            limit_rps,
            retry_after_ms,
        } => json!({
            "kind": "method_budget_exceeded",
            "method": method,
            "limit_rps": limit_rps,
            "retry_after_ms": retry_after_ms,
        }),
        RateLimitError::AuthorQuotaExceeded {
            author,
            limit_per_hour,
            reset_at_ms,
        } => json!({
            "kind": "author_quota_exceeded",
            "author": author,
            "limit_per_hour": limit_per_hour,
            "reset_at_ms": reset_at_ms,
        }),
    };
    ErrorObject::owned(RATE_LIMIT_ERROR_CODE, message, Some(data))
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        thread,
    };

    use super::*;

    fn tight_config() -> RateLimitConfig {
        RateLimitConfig {
            read_rps: 100,
            write_rps: 10,
            author_writes_per_hour: 3600,
            per_method_overrides: HashMap::new(),
        }
    }

    #[test]
    fn token_bucket_fills_and_drains() {
        let mut bucket = TokenBucket::new(5, Duration::from_secs(1));
        let t0 = Instant::now();
        for _ in 0..5 {
            bucket.try_consume(t0).expect("five tokens available");
        }
        bucket
            .try_consume(t0)
            .expect_err("sixth consumption should fail");
        // After ~1s all 5 tokens should be back.
        let t1 = t0 + Duration::from_millis(1_100);
        for _ in 0..5 {
            bucket.try_consume(t1).expect("refilled five tokens");
        }
    }

    #[test]
    fn token_bucket_pure_on_denial() {
        let mut bucket = TokenBucket::new(2, Duration::from_secs(1));
        let t0 = Instant::now();
        bucket.try_consume(t0).unwrap();
        bucket.try_consume(t0).unwrap();
        // Denied — should NOT drop tokens below zero or mutate state
        // destructively.
        let before = bucket.tokens;
        let _ = bucket.try_consume(t0);
        let _ = bucket.try_consume(t0);
        let _ = bucket.try_consume(t0);
        assert!((bucket.tokens - before).abs() < 1e-9);
        assert!(bucket.tokens >= 0.0);
    }

    #[test]
    fn read_budget_rejects_after_exhaustion() {
        let limiter = RateLimiter::new(tight_config());
        // 100 reads succeed within a single second window.
        for _ in 0..100 {
            limiter
                .check_method("chain_searchInsights", None)
                .expect("within read budget");
        }
        // 101st read in the same window must fail.
        let err = limiter
            .check_method("chain_searchInsights", None)
            .unwrap_err();
        match err {
            RateLimitError::MethodBudgetExceeded {
                method,
                limit_rps,
                retry_after_ms,
            } => {
                assert_eq!(method, "chain_searchInsights");
                assert_eq!(limit_rps, 100);
                assert!(retry_after_ms >= 1);
            }
            other => panic!("expected MethodBudgetExceeded, got {other:?}"),
        }
    }

    #[test]
    fn write_budget_rejects_after_exhaustion() {
        let limiter = RateLimiter::new(tight_config());
        for i in 0..10 {
            limiter
                .check_method("chain_confirmInsight", Some(&format!("author{i}")))
                .expect("within write budget");
        }
        let err = limiter
            .check_method("chain_confirmInsight", Some("authorX"))
            .unwrap_err();
        assert!(matches!(err, RateLimitError::MethodBudgetExceeded { .. }));
    }

    #[test]
    fn per_author_quota_rejects_3601st_post() {
        // Use a very relaxed per-method RPS so the method bucket never fires.
        let mut cfg = tight_config();
        cfg.read_rps = 1_000_000;
        cfg.write_rps = 1_000_000;
        cfg.author_writes_per_hour = 5; // small for a fast test
        let limiter = RateLimiter::new(cfg);
        for _ in 0..5 {
            limiter
                .check_method("chain_postInsight", Some("alice"))
                .expect("within author quota");
        }
        let err = limiter
            .check_method("chain_postInsight", Some("alice"))
            .unwrap_err();
        match err {
            RateLimitError::AuthorQuotaExceeded {
                author,
                limit_per_hour,
                reset_at_ms,
            } => {
                assert_eq!(author, "alice");
                assert_eq!(limit_per_hour, 5);
                assert!(reset_at_ms > 0);
            }
            other => panic!("expected AuthorQuotaExceeded, got {other:?}"),
        }
        // A different author still has quota.
        limiter
            .check_method("chain_postInsight", Some("bob"))
            .expect("bob has a fresh quota");
    }

    #[test]
    fn per_method_override_honored() {
        let mut cfg = tight_config();
        cfg.per_method_overrides
            .insert("chain_searchInsights".to_owned(), 200);
        let limiter = RateLimiter::new(cfg);
        for _ in 0..200 {
            limiter
                .check_method("chain_searchInsights", None)
                .expect("override raises budget to 200");
        }
        assert!(limiter.check_method("chain_searchInsights", None).is_err());
    }

    #[test]
    fn method_classification_is_correct_for_all_twelve() {
        let reads = [
            "chain_searchInsights",
            "chain_queryPheromones",
            "chain_getInsight",
            "chain_stats",
            "chain_version",
            "chain_listKinds",
            "chain_methodSchema",
        ];
        for m in reads {
            assert_eq!(classify_method(m), MethodClass::Read, "{m} should be Read");
        }
        let writes = [
            "chain_postInsight",
            "chain_confirmInsight",
            "chain_challengeInsight",
            "chain_depositPheromone",
            "chain_applyDecay",
        ];
        for m in writes {
            assert_eq!(
                classify_method(m),
                MethodClass::Write,
                "{m} should be Write"
            );
        }
        let subs = [
            "chain_subscribePheromones",
            "chain_subscribeInsights",
            "chain_unsubscribe",
        ];
        for m in subs {
            assert_eq!(
                classify_method(m),
                MethodClass::Subscription,
                "{m} should be Subscription"
            );
        }
        // Unknown → Read (safe default).
        assert_eq!(classify_method("chain_unknownMethod"), MethodClass::Read);
    }

    #[test]
    fn subscription_methods_skip_rps_check() {
        let mut cfg = tight_config();
        cfg.read_rps = 1; // would trip immediately if subs were classified as reads
        cfg.write_rps = 1;
        let limiter = RateLimiter::new(cfg);
        // Call way more than any sane RPS cap would allow.
        for _ in 0..10_000 {
            limiter
                .check_method("chain_subscribePheromones", None)
                .expect("subscriptions skip RPS");
        }
        limiter
            .check_method("chain_unsubscribe", None)
            .expect("unsubscribe skips RPS");
    }

    #[test]
    fn stats_reports_accurate_counters() {
        let limiter = RateLimiter::new(tight_config());
        for _ in 0..3 {
            limiter.check_method("chain_searchInsights", None).unwrap();
        }
        for _ in 0..2 {
            limiter
                .check_method("chain_confirmInsight", Some("alice"))
                .unwrap();
        }
        let stats = limiter.stats();
        assert_eq!(stats.total_allowed, 5);
        assert_eq!(stats.total_denied, 0);
        assert!(
            stats
                .per_method_budget_remaining
                .contains_key("chain_searchInsights")
        );
        assert!(
            stats
                .per_method_budget_remaining
                .contains_key("chain_confirmInsight")
        );
        // chain_searchInsights: 100 capacity - 3 used ~= 97 remaining.
        let search_remaining = stats
            .per_method_budget_remaining
            .get("chain_searchInsights")
            .copied()
            .unwrap();
        assert!(
            (96.0..=100.0).contains(&search_remaining),
            "expected ~97 tokens remaining, got {search_remaining}"
        );
        // Per-method "current RPS" rough estimate: 3 for search.
        let search_current = stats
            .per_method_rps_current
            .get("chain_searchInsights")
            .copied()
            .unwrap();
        assert!((2.0..=4.0).contains(&search_current));
        // Author remaining bucket reflects 2 consumed.
        let alice_remaining = stats.per_author_remaining.get("alice").copied().unwrap();
        assert_eq!(alice_remaining, 3598);
    }

    #[test]
    fn to_rpc_error_uses_reserved_code() {
        let err = RateLimitError::MethodBudgetExceeded {
            method: "chain_postInsight".into(),
            limit_rps: 10,
            retry_after_ms: 42,
        };
        let rpc = to_rpc_error(err);
        assert_eq!(rpc.code(), RATE_LIMIT_ERROR_CODE);
        assert_eq!(rpc.code(), -32105);
        let data = rpc.data().expect("data attached").get();
        assert!(data.contains("retry_after_ms"));
        assert!(data.contains("chain_postInsight"));

        let err = RateLimitError::AuthorQuotaExceeded {
            author: "alice".into(),
            limit_per_hour: 3600,
            reset_at_ms: 99,
        };
        let rpc = to_rpc_error(err);
        assert_eq!(rpc.code(), RATE_LIMIT_ERROR_CODE);
        let data = rpc.data().expect("data attached").get();
        assert!(data.contains("reset_at_ms"));
        assert!(data.contains("alice"));
    }

    #[test]
    fn concurrent_checks_from_four_threads_yield_correct_totals() {
        let mut cfg = tight_config();
        cfg.read_rps = 1_000_000; // ensure no denials in this test
        let limiter = Arc::new(RateLimiter::new(cfg));
        let denied = Arc::new(AtomicU64::new(0));
        let per_thread = 500_u64;
        let mut handles = Vec::new();
        for _ in 0..4 {
            let l = Arc::clone(&limiter);
            let d = Arc::clone(&denied);
            handles.push(thread::spawn(move || {
                for _ in 0..per_thread {
                    if l.check_method("chain_searchInsights", None).is_err() {
                        d.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let stats = limiter.stats();
        let expected = 4 * per_thread;
        assert_eq!(
            stats.total_allowed + stats.total_denied,
            expected,
            "every call should be counted"
        );
        assert_eq!(stats.total_denied, denied.load(Ordering::Relaxed));
        // With 1M RPS budget, zero denials expected.
        assert_eq!(stats.total_denied, 0);
        assert_eq!(stats.total_allowed, expected);
    }

    #[test]
    fn refill_period_restores_budget() {
        // Use a bucket directly so the test runs fast and deterministically.
        let mut bucket = TokenBucket::new(2, Duration::from_millis(100));
        let t0 = Instant::now();
        bucket.try_consume(t0).unwrap();
        bucket.try_consume(t0).unwrap();
        assert!(bucket.try_consume(t0).is_err());
        // After the full refill period, both tokens should be back.
        let t1 = t0 + Duration::from_millis(120);
        bucket.try_consume(t1).expect("refilled");
        bucket.try_consume(t1).expect("refilled");
    }

    #[test]
    fn unknown_method_uses_read_budget() {
        let limiter = RateLimiter::new(tight_config());
        // Unknown method is classified as Read, so 100/s budget.
        for _ in 0..100 {
            limiter.check_method("chain_madeUp", None).unwrap();
        }
        assert!(limiter.check_method("chain_madeUp", None).is_err());
    }

    #[test]
    fn denied_method_request_does_not_consume_author_quota() {
        let mut cfg = tight_config();
        cfg.write_rps = 1;
        cfg.author_writes_per_hour = 3;
        let limiter = RateLimiter::new(cfg);
        // One successful write.
        limiter
            .check_method("chain_postInsight", Some("alice"))
            .unwrap();
        // Second write in the same second — method budget exhausted.
        let err = limiter
            .check_method("chain_postInsight", Some("alice"))
            .unwrap_err();
        assert!(matches!(err, RateLimitError::MethodBudgetExceeded { .. }));
        let stats = limiter.stats();
        // Alice should have spent ONE token, not two.
        assert_eq!(stats.per_author_remaining.get("alice").copied(), Some(2));
    }

    #[test]
    fn denied_author_request_refunds_method_token() {
        let mut cfg = tight_config();
        cfg.write_rps = 100; // method budget not a constraint here
        cfg.author_writes_per_hour = 1;
        let limiter = RateLimiter::new(cfg);
        limiter
            .check_method("chain_postInsight", Some("alice"))
            .unwrap();
        // Alice is out of quota — this should fail with AuthorQuotaExceeded.
        let err = limiter
            .check_method("chain_postInsight", Some("alice"))
            .unwrap_err();
        assert!(matches!(err, RateLimitError::AuthorQuotaExceeded { .. }));
        let stats = limiter.stats();
        // Method bucket should only reflect ONE successful debit because the
        // denied attempt refunded its method token.
        let remaining = stats
            .per_method_budget_remaining
            .get("chain_postInsight")
            .copied()
            .unwrap();
        assert!(
            (98.0..=100.0).contains(&remaining),
            "expected ~99 tokens remaining, got {remaining}"
        );
    }
}

//! Scoped, time-bounded authorization tokens for privileged actions.
//!
//! A [`Permit`] represents a conditional grant of authority: a principal
//! (the grantor) has authorized some action against a specific resource
//! ([`PermitScope`]) for a bounded duration (`ttl_ms` from `granted_at_ms`).
//! Before executing a privileged action, the orchestrator checks whether a
//! matching, non-expired permit exists via [`Permit::is_valid_for`].
//!
//! Roko parity reference: §28.4 of `MORI-PARITY-CHECKLIST.md`.
//!
//! # Design notes
//!
//! - Permits are pure data. They carry no state transitions and no store;
//!   higher-level safety modules (e.g. capability tokens, audit chain) are
//!   responsible for persistence and lifecycle.
//! - Time is expressed in milliseconds since the Unix epoch (`i64`) to match
//!   the rest of the `roko-core` safety surface (audit chain, signals).
//! - Scope matching is exact: a permit granted for `PermitScope::Path("/a")`
//!   does **not** authorize actions on `/a/b`. Callers that want prefix
//!   semantics should model that explicitly via a dedicated scope variant.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A resource-and-action pair this permit authorizes.
///
/// Each variant identifies exactly one kind of privileged operation. New
/// variants are additive; the enum is `#[non_exhaustive]` so callers must
/// use a wildcard arm when matching.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PermitScope {
    /// Filesystem path that may be read from or written to.
    Path(PathBuf),
    /// Shell command line (canonical form) that may be executed.
    Command(String),
    /// Network URL that may be fetched.
    Url(String),
    /// Environment variable that may be read or set.
    EnvVar(String),
    /// Git remote (e.g. `origin`) that may be pushed to or pulled from.
    GitRemote(String),
    /// Arbitrary free-form resource identifier for scopes that don't fit
    /// the structured variants above.
    Other(String),
}

impl PermitScope {
    /// Construct a filesystem scope from anything convertible to a [`Path`].
    pub fn path(p: impl AsRef<Path>) -> Self {
        Self::Path(p.as_ref().to_path_buf())
    }

    /// Construct a command scope.
    pub fn command(cmd: impl Into<String>) -> Self {
        Self::Command(cmd.into())
    }

    /// Construct a URL scope.
    pub fn url(u: impl Into<String>) -> Self {
        Self::Url(u.into())
    }

    /// Construct an environment-variable scope.
    pub fn env_var(name: impl Into<String>) -> Self {
        Self::EnvVar(name.into())
    }

    /// Construct a git-remote scope.
    pub fn git_remote(name: impl Into<String>) -> Self {
        Self::GitRemote(name.into())
    }

    /// Construct an arbitrary scope.
    pub fn other(label: impl Into<String>) -> Self {
        Self::Other(label.into())
    }

    /// Short human-readable discriminant name (stable).
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Path(_) => "path",
            Self::Command(_) => "command",
            Self::Url(_) => "url",
            Self::EnvVar(_) => "env_var",
            Self::GitRemote(_) => "git_remote",
            Self::Other(_) => "other",
        }
    }
}

/// A scoped, time-bounded authorization token.
///
/// A permit does **not** by itself perform any mutation; it only records
/// that some principal (`grantor`) authorized an action on a specific
/// [`PermitScope`] starting at `granted_at_ms` for `ttl_ms` milliseconds.
/// Consumers call [`Permit::is_valid_for`] at the moment of use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permit {
    /// Resource this permit authorizes action against.
    pub scope: PermitScope,
    /// Wall-clock time the permit was granted, in Unix epoch milliseconds.
    pub granted_at_ms: i64,
    /// Time-to-live in milliseconds after `granted_at_ms`.
    pub ttl_ms: u64,
    /// Identity of the principal that issued the permit (free-form, e.g.
    /// `"conductor"` or `"auto-policy:safe-writes"`).
    pub grantor: String,
}

impl Permit {
    /// Create a new permit with the given scope, grant timestamp, TTL, and
    /// grantor.
    pub fn new(
        scope: PermitScope,
        granted_at_ms: i64,
        ttl_ms: u64,
        grantor: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            granted_at_ms,
            ttl_ms,
            grantor: grantor.into(),
        }
    }

    /// Builder: replace the scope.
    #[must_use]
    pub fn with_scope(mut self, scope: PermitScope) -> Self {
        self.scope = scope;
        self
    }

    /// Builder: replace the grant timestamp (Unix epoch milliseconds).
    #[must_use]
    pub const fn with_granted_at_ms(mut self, granted_at_ms: i64) -> Self {
        self.granted_at_ms = granted_at_ms;
        self
    }

    /// Builder: replace the TTL in milliseconds.
    #[must_use]
    pub const fn with_ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = ttl_ms;
        self
    }

    /// Builder: replace the grantor identity.
    #[must_use]
    pub fn with_grantor(mut self, grantor: impl Into<String>) -> Self {
        self.grantor = grantor.into();
        self
    }

    /// Expiry time (exclusive) as Unix epoch milliseconds.
    ///
    /// Saturates to [`i64::MAX`] if the addition would overflow so that
    /// extreme TTLs still produce a well-defined answer for callers.
    #[must_use]
    pub const fn expires_at_ms(&self) -> i64 {
        // Split the `u64` TTL into an `i64`-sized half plus a remainder so
        // we can add step-by-step with `saturating_add`, never leaving
        // `i64` range and never needing a lossy cast.
        //
        // Any `u64 <= i64::MAX as u64` fits losslessly into `i64`; if
        // `ttl_ms` exceeds that, we add `i64::MAX` first (saturating) and
        // then add the leftover (also saturating). The second add can only
        // push us further past `i64::MAX` and thus saturates immediately.
        let i64_max_as_u64: u64 = i64::MAX as u64;
        if self.ttl_ms <= i64_max_as_u64 {
            // Safe: `ttl_ms` fits in `i64`. `as i64` is value-preserving.
            #[allow(clippy::cast_possible_wrap)]
            let ttl_signed = self.ttl_ms as i64;
            self.granted_at_ms.saturating_add(ttl_signed)
        } else {
            // `ttl_ms > i64::MAX`: adding i64::MAX alone already saturates
            // unless `granted_at_ms` is very negative, in which case we
            // add the leftover to reach saturation eventually.
            let remainder_u = self.ttl_ms - i64_max_as_u64;
            // `remainder_u <= u64::MAX - i64::MAX as u64 == i64::MAX as u64 + 1`,
            // so it fits in i64 except for the single boundary case where
            // `ttl_ms == u64::MAX`; split again to stay exact.
            if remainder_u <= i64_max_as_u64 {
                #[allow(clippy::cast_possible_wrap)]
                let remainder = remainder_u as i64;
                self.granted_at_ms
                    .saturating_add(i64::MAX)
                    .saturating_add(remainder)
            } else {
                // Only reachable when `ttl_ms == u64::MAX`: add i64::MAX
                // twice plus one, saturating each time.
                self.granted_at_ms
                    .saturating_add(i64::MAX)
                    .saturating_add(i64::MAX)
                    .saturating_add(1)
            }
        }
    }

    /// True if the permit has expired at `now_ms`.
    ///
    /// A permit is expired once `now_ms >= expires_at_ms()` — the expiry
    /// boundary is exclusive so a permit with `ttl_ms == 0` is expired
    /// immediately.
    #[must_use]
    pub const fn is_expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at_ms()
    }

    /// True if `now_ms` lies within `[granted_at_ms, expires_at_ms)`.
    ///
    /// Returns `false` for clocks that are earlier than the grant time
    /// (a not-yet-effective permit should not authorize anything).
    #[must_use]
    pub const fn is_active(&self, now_ms: i64) -> bool {
        now_ms >= self.granted_at_ms && !self.is_expired(now_ms)
    }

    /// True if this permit authorizes `requested` at the given time.
    ///
    /// Scope matching is exact equality; the permit must additionally be
    /// active (granted, not expired, not in the future).
    #[must_use]
    pub fn is_valid_for(&self, requested: &PermitScope, now_ms: i64) -> bool {
        self.is_active(now_ms) && &self.scope == requested
    }

    /// Remaining TTL in milliseconds at `now_ms`, or `0` if expired.
    #[must_use]
    pub const fn remaining_ms(&self, now_ms: i64) -> u64 {
        let expires = self.expires_at_ms();
        if now_ms >= expires {
            0
        } else if now_ms < self.granted_at_ms {
            // Not yet active: the whole TTL is still ahead.
            self.ttl_ms
        } else {
            // `now_ms >= granted_at_ms` and `now_ms < expires` here, so
            // `expires - now_ms` is strictly positive. Use saturating
            // subtraction and `unsigned_abs` to stay lossless without an
            // `as u64` cast.
            expires.saturating_sub(now_ms).unsigned_abs()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000_000;

    fn sample_permit() -> Permit {
        Permit::new(
            PermitScope::path("/tmp/work/out.txt"),
            T0,
            5_000,
            "conductor",
        )
    }

    #[test]
    fn new_populates_all_fields() {
        let p = sample_permit();
        assert_eq!(
            p.scope,
            PermitScope::Path(PathBuf::from("/tmp/work/out.txt"))
        );
        assert_eq!(p.granted_at_ms, T0);
        assert_eq!(p.ttl_ms, 5_000);
        assert_eq!(p.grantor, "conductor");
    }

    #[test]
    fn builder_methods_override_fields() {
        let p = Permit::new(PermitScope::other("seed"), 0, 1, "a")
            .with_scope(PermitScope::url("https://example.com"))
            .with_granted_at_ms(T0)
            .with_ttl_ms(10_000)
            .with_grantor("auto-policy");
        assert_eq!(p.granted_at_ms, T0);
        assert_eq!(p.ttl_ms, 10_000);
        assert_eq!(p.grantor, "auto-policy");
        assert_eq!(p.scope, PermitScope::Url("https://example.com".into()));
    }

    #[test]
    fn expires_at_is_grant_plus_ttl() {
        let p = sample_permit();
        assert_eq!(p.expires_at_ms(), T0 + 5_000);
    }

    #[test]
    fn is_expired_boundary_is_exclusive() {
        let p = sample_permit();
        // Strictly before expiry — still valid.
        assert!(!p.is_expired(T0 + 4_999));
        // Exactly at expiry — expired (boundary exclusive).
        assert!(p.is_expired(T0 + 5_000));
        // After expiry — expired.
        assert!(p.is_expired(T0 + 6_000));
    }

    #[test]
    fn zero_ttl_is_immediately_expired() {
        let p = Permit::new(PermitScope::command("ls"), T0, 0, "test");
        assert!(p.is_expired(T0));
        assert!(!p.is_active(T0));
    }

    #[test]
    fn not_yet_active_before_grant_time() {
        let p = sample_permit();
        assert!(!p.is_active(T0 - 1));
        // Not-yet-active permits also don't authorize a matching scope.
        assert!(!p.is_valid_for(&PermitScope::path("/tmp/work/out.txt"), T0 - 1));
    }

    #[test]
    fn is_active_during_window() {
        let p = sample_permit();
        assert!(p.is_active(T0));
        assert!(p.is_active(T0 + 2_500));
        assert!(p.is_active(T0 + 4_999));
        assert!(!p.is_active(T0 + 5_000));
    }

    #[test]
    fn is_valid_for_matching_scope_and_time() {
        let p = sample_permit();
        let requested = PermitScope::path("/tmp/work/out.txt");
        assert!(p.is_valid_for(&requested, T0 + 1));
    }

    #[test]
    fn is_valid_for_rejects_scope_mismatch() {
        let p = sample_permit();
        let other_path = PermitScope::path("/tmp/work/other.txt");
        assert!(!p.is_valid_for(&other_path, T0 + 1));

        let wrong_kind = PermitScope::command("rm -rf /tmp/work/out.txt");
        assert!(!p.is_valid_for(&wrong_kind, T0 + 1));
    }

    #[test]
    fn is_valid_for_rejects_expired_even_with_matching_scope() {
        let p = sample_permit();
        let requested = PermitScope::path("/tmp/work/out.txt");
        assert!(!p.is_valid_for(&requested, T0 + 5_000));
        assert!(!p.is_valid_for(&requested, T0 + 10_000));
    }

    #[test]
    fn exact_path_match_no_prefix_semantics() {
        let p = Permit::new(PermitScope::path("/a"), T0, 1_000, "g");
        // A child path is *not* covered by a parent-path permit.
        assert!(!p.is_valid_for(&PermitScope::path("/a/b"), T0));
        // But the exact path is.
        assert!(p.is_valid_for(&PermitScope::path("/a"), T0));
    }

    #[test]
    fn remaining_ms_counts_down_then_hits_zero() {
        let p = sample_permit();
        assert_eq!(p.remaining_ms(T0), 5_000);
        assert_eq!(p.remaining_ms(T0 + 1_500), 3_500);
        assert_eq!(p.remaining_ms(T0 + 4_999), 1);
        assert_eq!(p.remaining_ms(T0 + 5_000), 0);
        assert_eq!(p.remaining_ms(T0 + 10_000), 0);
    }

    #[test]
    fn remaining_ms_before_grant_is_full_ttl() {
        let p = sample_permit();
        assert_eq!(p.remaining_ms(T0 - 50), 5_000);
    }

    #[test]
    fn expires_at_saturates_on_overflow() {
        let p = Permit::new(PermitScope::other("x"), i64::MAX - 10, u64::MAX, "g");
        assert_eq!(p.expires_at_ms(), i64::MAX);
        // Saturation still yields a sensible "not expired" at reasonable times.
        assert!(!p.is_expired(0));
    }

    #[test]
    fn scope_constructors_cover_all_variants() {
        assert_eq!(
            PermitScope::path("/x"),
            PermitScope::Path(PathBuf::from("/x"))
        );
        assert_eq!(
            PermitScope::command("cargo test"),
            PermitScope::Command("cargo test".into())
        );
        assert_eq!(
            PermitScope::url("https://r.example"),
            PermitScope::Url("https://r.example".into())
        );
        assert_eq!(
            PermitScope::env_var("PATH"),
            PermitScope::EnvVar("PATH".into())
        );
        assert_eq!(
            PermitScope::git_remote("origin"),
            PermitScope::GitRemote("origin".into())
        );
        assert_eq!(
            PermitScope::other("custom"),
            PermitScope::Other("custom".into())
        );
    }

    #[test]
    fn scope_kind_names_are_stable() {
        assert_eq!(PermitScope::path("/x").kind_name(), "path");
        assert_eq!(PermitScope::command("x").kind_name(), "command");
        assert_eq!(PermitScope::url("x").kind_name(), "url");
        assert_eq!(PermitScope::env_var("x").kind_name(), "env_var");
        assert_eq!(PermitScope::git_remote("x").kind_name(), "git_remote");
        assert_eq!(PermitScope::other("x").kind_name(), "other");
    }

    #[test]
    fn permit_serde_roundtrip() {
        let p = sample_permit();
        let json = serde_json::to_string(&p).expect("serialize permit");
        let back: Permit = serde_json::from_str(&json).expect("deserialize permit");
        assert_eq!(back, p);
    }

    #[test]
    fn scope_serde_tagged_format_roundtrip() {
        let cases = [
            PermitScope::path("/tmp/x"),
            PermitScope::command("ls -la"),
            PermitScope::url("https://example.com/api"),
            PermitScope::env_var("HOME"),
            PermitScope::git_remote("origin"),
            PermitScope::other("custom"),
        ];
        for original in cases {
            let json = serde_json::to_string(&original).expect("serialize scope");
            let back: PermitScope = serde_json::from_str(&json).expect("deserialize scope");
            assert_eq!(back, original);
        }
    }
}

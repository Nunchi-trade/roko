//! Queries and budgets — the specs that drive substrate lookups and composer
//! constraints.
//!
//! A [`Query`] describes which signals a [`Store`](crate::Store) should
//! return. A [`Budget`] describes the resource limits a [`Compose`](crate::Compose)
//! must respect.

use crate::Kind;
use serde::{Deserialize, Serialize};

/// A query against a substrate.
///
/// All fields are optional filters. An empty `Query` matches everything.
/// Multiple filters AND together.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Query {
    /// Only signals of these kinds. None = any kind.
    pub kinds: Option<Vec<Kind>>,
    /// Only signals with this author (exact match). None = any author.
    pub author: Option<String>,
    /// Only signals with this session. None = any session.
    pub session: Option<String>,
    /// Only signals created at/after this Unix-ms timestamp.
    pub since_ms: Option<i64>,
    /// Only signals created at/before this Unix-ms timestamp.
    pub until_ms: Option<i64>,
    /// Only signals whose effective weight exceeds this threshold (after decay).
    pub min_weight: Option<f32>,
    /// Only signals matching all listed tag key=value pairs.
    pub tags: Vec<(String, String)>,
    /// Maximum number of signals to return. None = unbounded.
    pub limit: Option<usize>,
}

impl Query {
    /// An empty query matching all signals.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Query for one specific kind.
    #[must_use]
    pub fn of_kind(kind: Kind) -> Self {
        Self {
            kinds: Some(vec![kind]),
            ..Default::default()
        }
    }

    /// Query for any of several kinds.
    #[must_use]
    pub fn of_kinds(kinds: impl IntoIterator<Item = Kind>) -> Self {
        Self {
            kinds: Some(kinds.into_iter().collect()),
            ..Default::default()
        }
    }

    /// Filter to signals with this author.
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Filter to signals with this session id.
    #[must_use]
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }

    /// Filter to signals created at/after this Unix-ms timestamp.
    #[must_use]
    pub const fn since(mut self, ms: i64) -> Self {
        self.since_ms = Some(ms);
        self
    }

    /// Filter to signals created at/before this Unix-ms timestamp.
    #[must_use]
    pub const fn until(mut self, ms: i64) -> Self {
        self.until_ms = Some(ms);
        self
    }

    /// Filter to signals whose effective weight exceeds this threshold (decay applied).
    #[must_use]
    pub const fn with_min_weight(mut self, w: f32) -> Self {
        self.min_weight = Some(w);
        self
    }

    /// Require this tag key=value on matching signals.
    #[must_use]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }

    /// Cap the number of returned results.
    #[must_use]
    pub const fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }
}

/// Resource budget for a composer operation.
///
/// A composer must produce output that respects all set limits. When a limit
/// is `None`, that dimension is unbounded.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Budget {
    /// Maximum token count (rough estimate: ~4 bytes per token).
    pub max_tokens: Option<usize>,
    /// Maximum number of input pulses to include.
    pub max_pulses: Option<usize>,
    /// Maximum output byte size.
    pub max_bytes: Option<usize>,
    /// Maximum wall-clock milliseconds a composer may spend.
    pub max_wall_ms: Option<u64>,
}

impl Budget {
    /// An unlimited budget.
    #[must_use]
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Budget with a token cap only.
    #[must_use]
    pub fn tokens(n: usize) -> Self {
        Self {
            max_tokens: Some(n),
            ..Default::default()
        }
    }

    /// Budget with a pulse count cap only.
    #[must_use]
    pub fn pulses(n: usize) -> Self {
        Self {
            max_pulses: Some(n),
            ..Default::default()
        }
    }

    /// Set an output byte-size cap.
    #[must_use]
    pub const fn with_max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = Some(n);
        self
    }

    /// Set a wall-clock millisecond cap.
    #[must_use]
    pub const fn with_max_wall_ms(mut self, ms: u64) -> Self {
        self.max_wall_ms = Some(ms);
        self
    }

    /// Estimate token count from byte length (~4 bytes/token).
    #[must_use]
    pub const fn estimate_tokens(bytes: usize) -> usize {
        bytes.div_ceil(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_all() {
        let q = Query::all();
        assert!(q.kinds.is_none());
        assert!(q.limit.is_none());
    }

    #[test]
    fn of_kind_builds_filter() {
        let q = Query::of_kind(Kind::Task);
        assert_eq!(q.kinds.unwrap(), vec![Kind::Task]);
    }

    #[test]
    fn builder_chain() {
        let q = Query::of_kind(Kind::Episode)
            .with_author("agent:1")
            .since(1000)
            .until(2000)
            .with_tag("run", "42")
            .limit(10);
        assert_eq!(q.author.as_deref(), Some("agent:1"));
        assert_eq!(q.since_ms, Some(1000));
        assert_eq!(q.until_ms, Some(2000));
        assert_eq!(q.tags, vec![("run".to_string(), "42".to_string())]);
        assert_eq!(q.limit, Some(10));
    }

    #[test]
    fn budget_tokens_only() {
        let b = Budget::tokens(500);
        assert_eq!(b.max_tokens, Some(500));
        assert!(b.max_pulses.is_none());
    }

    #[test]
    fn estimate_tokens_rough_math() {
        assert_eq!(Budget::estimate_tokens(0), 0);
        assert_eq!(Budget::estimate_tokens(1), 1);
        assert_eq!(Budget::estimate_tokens(4), 1);
        assert_eq!(Budget::estimate_tokens(5), 2);
        assert_eq!(Budget::estimate_tokens(100), 25);
    }
}

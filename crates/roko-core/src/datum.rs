//! Polymorphic input surface — work over either medium without a new trait family.
//!
//! [`Datum`] unifies [`Engram`](crate::Engram) (persisted) and
//! [`Pulse`](crate::Pulse) (ephemeral) so operators like [`Score`](crate::traits::Score)
//! and [`Compose`](crate::Compose) can accept both without requiring callers
//! to persist first.

use crate::{Body, Engram, Kind, Pulse};
use std::collections::BTreeMap;

/// Polymorphic input surface over [`Engram`] and [`Pulse`].
///
/// Operators that need to inspect kind, body, or tags can accept a `Datum`
/// instead of requiring a concrete type. This avoids "persist first" friction
/// when scoring or composing ephemeral data.
///
/// # Examples
///
/// ```
/// use roko_core::{Body, Datum, Engram, Kind, Pulse, Topic};
///
/// let engram = Engram::builder(Kind::Task)
///     .body(Body::text("implement login"))
///     .created_at_ms(1000)
///     .build();
/// let d = Datum::Engram(&engram);
/// assert_eq!(d.kind(), &Kind::Task);
/// assert_eq!(d.body(), &Body::Text("implement login".into()));
///
/// let pulse = Pulse::new(1, Topic::new("gate"), Kind::GateVerdict, Body::text("pass"));
/// let d = Datum::Pulse(&pulse);
/// assert_eq!(d.kind(), &Kind::GateVerdict);
/// ```
#[derive(Clone, Copy, Debug)]
pub enum Datum<'a> {
    /// A persisted engram.
    Engram(&'a Engram),
    /// An ephemeral pulse.
    Pulse(&'a Pulse),
}

impl<'a> Datum<'a> {
    /// The kind of the underlying event.
    #[must_use]
    pub fn kind(&self) -> &Kind {
        match self {
            Self::Engram(e) => &e.kind,
            Self::Pulse(p) => &p.kind,
        }
    }

    /// The body (payload) of the underlying event.
    #[must_use]
    pub fn body(&self) -> &Body {
        match self {
            Self::Engram(e) => &e.body,
            Self::Pulse(p) => &p.body,
        }
    }

    /// Tags, if available.
    ///
    /// Both Engram and Pulse carry tags, so this always returns `Some`.
    #[must_use]
    pub fn tags(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Engram(e) => &e.tags,
            Self::Pulse(p) => &p.tags,
        }
    }

    /// Unix milliseconds when the underlying event was created.
    #[must_use]
    pub fn created_at_ms(&self) -> i64 {
        match self {
            Self::Engram(e) => e.created_at_ms,
            Self::Pulse(p) => p.created_at_ms,
        }
    }

    /// Whether the underlying event is an engram (persisted).
    #[must_use]
    pub fn is_engram(&self) -> bool {
        matches!(self, Self::Engram(_))
    }

    /// Whether the underlying event is a pulse (ephemeral).
    #[must_use]
    pub fn is_pulse(&self) -> bool {
        matches!(self, Self::Pulse(_))
    }
}

impl<'a> From<&'a Engram> for Datum<'a> {
    fn from(e: &'a Engram) -> Self {
        Self::Engram(e)
    }
}

impl<'a> From<&'a Pulse> for Datum<'a> {
    fn from(p: &'a Pulse) -> Self {
        Self::Pulse(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Topic;

    #[test]
    fn datum_from_engram() {
        let e = Engram::builder(Kind::Task)
            .body(Body::text("do thing"))
            .created_at_ms(5000)
            .tag("priority", "high")
            .build();
        let d = Datum::Engram(&e);

        assert_eq!(d.kind(), &Kind::Task);
        assert_eq!(d.body(), &Body::Text("do thing".into()));
        assert_eq!(d.created_at_ms(), 5000);
        assert_eq!(d.tags().get("priority").map(String::as_str), Some("high"));
        assert!(d.is_engram());
        assert!(!d.is_pulse());
    }

    #[test]
    fn datum_from_pulse() {
        let p = Pulse::builder(1, Topic::new("gate.verdict"), Kind::GateVerdict)
            .body(Body::text("pass"))
            .created_at_ms(9000)
            .tag("gate", "compile")
            .build();
        let d = Datum::Pulse(&p);

        assert_eq!(d.kind(), &Kind::GateVerdict);
        assert_eq!(d.body(), &Body::Text("pass".into()));
        assert_eq!(d.created_at_ms(), 9000);
        assert_eq!(d.tags().get("gate").map(String::as_str), Some("compile"));
        assert!(d.is_pulse());
        assert!(!d.is_engram());
    }

    #[test]
    fn datum_from_trait_impls() {
        let e = Engram::builder(Kind::Episode).created_at_ms(0).build();
        let d: Datum<'_> = (&e).into();
        assert!(d.is_engram());

        let p = Pulse::new(1, Topic::new("x"), Kind::Metric, Body::empty());
        let d: Datum<'_> = (&p).into();
        assert!(d.is_pulse());
    }

    #[test]
    fn datum_empty_tags() {
        let e = Engram::builder(Kind::Task).created_at_ms(0).build();
        let d = Datum::Engram(&e);
        assert!(d.tags().is_empty());

        let p = Pulse::new(1, Topic::new("x"), Kind::Task, Body::empty());
        let d = Datum::Pulse(&p);
        assert!(d.tags().is_empty());
    }
}

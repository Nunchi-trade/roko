//! `TradingAgent` — glue `Agent` impl that drives one strategy tick per
//! `Agent::run`.
//!
//! Composes `roko-venue::VenueAdapter` + `roko-strategy::Strategy` +
//! `roko-custody::CustodyGuard` so trading fits the existing universal
//! loop (compose → agent → gate → persist → policy).
//!
//! Scaffolded stub — per plans/P08-trading-surface.md T10.

#![allow(missing_docs)]

/// Placeholder until T10 lands.
pub fn _scaffold() {}

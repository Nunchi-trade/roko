//! APEX autonomous multi-slot orchestrator. Ports
//! `offchainservices-agent/modules/apex_config.py`, `apex_state.py`, and the
//! public surface of `apex_engine.py`.
//!
//! See `plans/P08-trading-surface.md` T8. The deeper signal-evaluation
//! logic from `apex_engine._evaluate_entries` lands in the T8 close-out
//! commit once Radar / Pulse / Guard are ported. This commit lands the
//! persistent state + config + minimal `tick()` + `enter()` / `close()`
//! surface so the orchestrator can be driven from the CLI today.

#![forbid(unsafe_code)]
#![allow(missing_docs)]

mod config;
mod engine;
mod state;

pub use config::{apex_presets, ApexConfig, ApexPreset, APEX_PRESETS};
pub use engine::ApexEngine;
pub use state::{ApexSlot, ApexState, ApexStateStore};

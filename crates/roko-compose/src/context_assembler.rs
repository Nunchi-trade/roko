//! Canonical context assembly lives in `roko-neuro`; `roko-compose` re-exports
//! the memory-facing assembly primitives for prompt construction.

pub use roko_neuro::{ContextAssembler, ContextChunk, PadState};

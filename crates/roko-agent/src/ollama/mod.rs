//! Ollama provider support.
//!
//! The `agent` module contains both the direct [`OllamaAgent`] adapter and the
//! structured [`OllamaLlmBackend`] used by the tool loop.

pub mod agent;
pub mod format;

pub use agent::{OllamaAgent, OllamaLlmBackend};

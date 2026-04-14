//! OpenAI-compatible HTTP backend for the tool loop.
//!
//! The concrete implementation already lives at the crate root and is used by
//! the existing provider adapter. This module exposes the canonical
//! `tool_loop::backends` path and backend name expected by the model-routing
//! plan without forking request/response logic.

pub use crate::openai_compat_backend::OpenAiCompatLlmBackend;

/// Canonical tool-loop backend name used by model-routing code.
pub type OpenAiCompatBackend = OpenAiCompatLlmBackend;

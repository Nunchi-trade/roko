//! Gemini provider support.

pub mod adapter;
pub mod types;

pub use adapter::{GeminiAdapter, GeminiCompatAgent, GeminiEmbedAgent, GeminiNativeAgent};
pub use types::*;

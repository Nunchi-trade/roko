//! Gemini provider support.

pub mod adapter;
pub mod native;
pub mod types;

pub use adapter::{GeminiAdapter, GeminiCompatAgent, GeminiEmbedAgent};
pub use native::GeminiNativeAgent;
pub use types::*;

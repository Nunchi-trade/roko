//! Gemini provider support.

pub mod adapter;
pub mod compat;
pub mod native;
pub mod types;

pub use adapter::{GeminiAdapter, GeminiEmbedAgent};
pub use compat::GeminiCompatAgent;
pub use native::GeminiNativeAgent;
pub use types::*;

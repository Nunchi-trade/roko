//! Gemini provider support.

pub mod adapter;
pub mod compat;
pub mod embed;
pub mod native;
pub mod types;

pub use adapter::GeminiAdapter;
pub use compat::GeminiCompatAgent;
pub use embed::{EmbedError, GeminiEmbedAgent};
pub use native::GeminiNativeAgent;
pub use types::*;

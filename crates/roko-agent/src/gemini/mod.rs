//! Gemini provider support.

pub mod adapter;
pub mod cache;
pub mod compat;
pub mod embed;
pub mod native;
pub mod types;
pub(crate) mod wire;

pub use adapter::GeminiAdapter;
pub use cache::{CacheError, GeminiCacheClient};
pub use compat::GeminiCompatAgent;
pub use embed::{EmbedError, GeminiEmbedAgent};
pub use native::GeminiNativeAgent;
pub use types::*;

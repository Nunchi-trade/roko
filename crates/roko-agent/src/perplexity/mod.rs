//! Perplexity Sonar-specific types and helpers.

pub mod adapter;
pub mod chat;
pub mod deep_research;
pub mod embed;
pub mod search;
pub mod tool_loop;
pub mod types;
pub(crate) mod wire;

pub use adapter::PerplexityAdapter;
pub use chat::PerplexityChatAgent;
pub use deep_research::PerplexityDeepResearchAgent;
pub use embed::{EmbedError, PerplexityEmbedAgent};
pub use search::{PerplexitySearchClient, SearchError, SearchQuery, SearchResponse};
pub use tool_loop::{PerplexityMetadataSource, PerplexityToolLoopAgent, PerplexityToolLoopBackend};
pub use types::{Annotation, PerplexityMetadata, SearchOptions, SearchResult};

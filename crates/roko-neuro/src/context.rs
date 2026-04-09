//! Context assembly primitives for composing knowledge and episode memory.

use std::sync::Arc;

use crate::KnowledgeStore;
use roko_learn::episode_logger::EpisodeLogger;

/// Existing episode persistence backend used by the context assembler.
pub type EpisodeStore = EpisodeLogger;

/// Assembles context from knowledge and episode memory under a token budget.
#[derive(Debug, Clone)]
pub struct ContextAssembler {
    knowledge_store: Arc<KnowledgeStore>,
    episode_store: Arc<EpisodeStore>,
    /// Budget for assembled context, in estimated tokens.
    max_context_tokens: usize,
}

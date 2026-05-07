//! Background distillation of completed episodes into durable knowledge.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use roko_core::foundation::{
    CachePolicy, ChatMessage, MessageRole, ModelCallRequest, ModelCaller, caller,
};
use roko_learn::episode_logger::Episode;
use tokio::task;

use crate::{DistillationBackend, Distiller, KnowledgeStore};

/// Spawn background distillation for one completed episode.
///
/// The work is intentionally detached from the caller so episode
/// persistence can finish without waiting on model inference or store
/// writes.
pub fn spawn_episode_distillation(
    workdir: PathBuf,
    episode: Episode,
    model_caller: Option<Arc<dyn ModelCaller>>,
) {
    tokio::spawn(async move {
        if let Err(error) = distill_episode(workdir, episode, model_caller).await {
            tracing::warn!(error = %error, "episode distillation failed");
        }
    });
}

async fn distill_episode(
    workdir: PathBuf,
    episode: Episode,
    model_caller: Option<Arc<dyn ModelCaller>>,
) -> Result<()> {
    let Some(model_caller) = model_caller else {
        tracing::debug!("no ModelCaller provided; skipping episode distillation");
        return Ok(());
    };

    // Use an empty model string so the ModelCaller resolves via its own
    // default — this respects whatever model the workspace has configured
    // instead of hardcoding a specific provider.
    let distiller =
        Distiller::with_backend(Arc::new(GatewayDistillationBackend::new(model_caller, "")));

    let episodes = [episode];
    let entries = distiller
        .distill(&episodes)
        .await
        .context("distill completed episode")?;

    let store = KnowledgeStore::for_workdir(&workdir);
    task::spawn_blocking(move || -> Result<()> {
        for entry in entries {
            store.add(entry)?;
        }
        Ok(())
    })
    .await
    .context("join knowledge-store writer")??;

    Ok(())
}

struct GatewayDistillationBackend {
    model_caller: Arc<dyn ModelCaller>,
    model: String,
}

impl std::fmt::Debug for GatewayDistillationBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayDistillationBackend")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl GatewayDistillationBackend {
    fn new(model_caller: Arc<dyn ModelCaller>, model: impl Into<String>) -> Self {
        Self {
            model_caller,
            model: model.into(),
        }
    }
}

#[async_trait]
impl DistillationBackend for GatewayDistillationBackend {
    async fn complete(&self, prompt: &str) -> Result<String> {
        let response = self
            .model_caller
            .call(ModelCallRequest {
                model: self.model.clone(),
                system: None,
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: prompt.to_string(),
                }],
                max_tokens: None,
                temperature: None,
                role: Some("episode-distiller".to_string()),
                caller: Some(caller::RESEARCH.to_string()),
                run_id: None,
                prompt_section_ids: Vec::new(),
                knowledge_ids: Vec::new(),
                budget: None,
                budget_remaining: None,
                routing_hints: Vec::new(),
                cache_policy: CachePolicy::Default,
                tools: Vec::new(),
            })
            .await
            .context("call gateway distillation model")?;
        Ok(response.content)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

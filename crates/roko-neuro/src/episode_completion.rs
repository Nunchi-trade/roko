//! Background distillation of completed episodes into durable knowledge.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use roko_core::foundation::{
    CachePolicy, CallerIdentity, ChatMessage, MessageRole, ModelCallRequest, ModelCaller,
};
use roko_learn::episode_logger::Episode;
use tokio::task;

use crate::{DistillationBackend, Distiller, KnowledgeStore};

const GATEWAY_DISTILLATION_MODEL: &str = "claude-haiku-3-5";

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
    let distiller = if let Some(model_caller) = model_caller {
        Distiller::with_backend(Arc::new(GatewayDistillationBackend::new(
            model_caller,
            GATEWAY_DISTILLATION_MODEL,
        )))
    } else {
        let Some(api_key) = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
        else {
            return Ok(());
        };

        tracing::warn!("distillation using direct API key; gateway not available");
        Distiller::with_claude(api_key)
    };

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
                caller: Some(CallerIdentity::Research),
                budget: None,
                cache_policy: CachePolicy::Default,
            })
            .await
            .context("call gateway distillation model")?;
        Ok(response.content)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

//! Background distillation of completed episodes into durable knowledge.

use std::path::PathBuf;

use anyhow::{Context, Result};
use roko_learn::episode_logger::Episode;
use tokio::task;

use crate::{Distiller, KnowledgeStore};

/// Spawn background distillation for one completed episode.
///
/// The work is intentionally detached from the caller so episode
/// persistence can finish without waiting on model inference or store
/// writes.
pub fn spawn_episode_distillation(workdir: PathBuf, episode: Episode) {
    tokio::spawn(async move {
        if let Err(error) = distill_episode(workdir, episode).await {
            tracing::warn!(error = %error, "episode distillation failed");
        }
    });
}

async fn distill_episode(workdir: PathBuf, episode: Episode) -> Result<()> {
    let Some(api_key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
    else {
        return Ok(());
    };

    let distiller = Distiller::with_claude(api_key);
    let episodes = [episode];
    let entries = distiller
        .distill(&episodes)
        .await
        .context("distill completed episode")?;
    if entries.is_empty() {
        return Ok(());
    }

    let store = KnowledgeStore::for_workdir(workdir);
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

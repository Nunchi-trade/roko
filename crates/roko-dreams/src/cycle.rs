//! Offline dream-cycle orchestration.
//!
//! The dream cycle batches completed episodes, mines recurring patterns,
//! promotes the resulting knowledge into the persistent stores, and gives an
//! agent a final review pass before the cycle is marked complete.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use roko_agent::{Agent, AgentResult};
use roko_core::{Body, Context as RokoContext, Kind, Signal};
use roko_learn::{
    episode_logger::EpisodeLogger,
    playbook::{Playbook, PlaybookStep, PlaybookStore},
};
use roko_neuro::{
    KnowledgeEntry, KnowledgeKind, KnowledgeStore,
    tier_progression::{TierProgression, TierProgressionReport},
};
use serde::{Deserialize, Serialize};

/// Agent hook used by the dream cycle to review a consolidation batch.
#[async_trait]
pub trait AgentDispatcher: Send + Sync {
    /// Dispatch a dream-review prompt through the configured agent.
    async fn dispatch(&self, input: &Signal, ctx: &RokoContext) -> AgentResult;
}

#[async_trait]
impl<T> AgentDispatcher for T
where
    T: Agent + Send + Sync,
{
    async fn dispatch(&self, input: &Signal, ctx: &RokoContext) -> AgentResult {
        self.run(input, ctx).await
    }
}

/// Summary of one completed dream cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamCycleReport {
    /// When the cycle started.
    pub started_at: DateTime<Utc>,
    /// When the cycle completed.
    pub completed_at: DateTime<Utc>,
    /// Number of episodes visible in the backing log.
    pub total_episodes: usize,
    /// Number of episodes included in this batch.
    pub processed_episodes: usize,
    /// Timestamp cutoff used to avoid reprocessing old episodes.
    pub processed_through: Option<DateTime<Utc>>,
    /// Batch analysis produced by the tier progression pipeline.
    pub analysis: TierProgressionReport,
    /// Number of knowledge entries written to the durable store.
    pub knowledge_entries_written: usize,
    /// Whether a playbook snapshot was written.
    pub playbook_written: bool,
    /// Optional review emitted by the dispatcher agent.
    pub agent_review: Option<String>,
}

/// Main offline learning process.
///
/// The cycle reads episode history, runs the existing tier progression
/// pipeline, persists the resulting knowledge, and closes the loop with an
/// agent review pass.
pub struct DreamCycle {
    episode_store: Arc<EpisodeLogger>,
    knowledge_store: Arc<KnowledgeStore>,
    playbook_store: Arc<PlaybookStore>,
    dispatcher: Arc<dyn AgentDispatcher>,
    last_dream_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for DreamCycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DreamCycle")
            .field("episode_store", &self.episode_store.path())
            .field("knowledge_store", &self.knowledge_store.path())
            .field("playbook_store", &self.playbook_store.root())
            .field("dispatcher", &"<dispatcher>")
            .field("last_dream_at", &self.last_dream_at)
            .finish()
    }
}

impl DreamCycle {
    /// Construct a dream cycle around the existing stores and dispatcher.
    #[must_use]
    pub fn new(
        episode_store: Arc<EpisodeLogger>,
        knowledge_store: Arc<KnowledgeStore>,
        playbook_store: Arc<PlaybookStore>,
        dispatcher: Arc<dyn AgentDispatcher>,
    ) -> Self {
        Self {
            episode_store,
            knowledge_store,
            playbook_store,
            dispatcher,
            last_dream_at: None,
        }
    }

    /// Last completed cycle time, if any.
    #[must_use]
    pub const fn last_dream_at(&self) -> Option<DateTime<Utc>> {
        self.last_dream_at
    }

    /// Run a full offline learning pass.
    ///
    /// # Errors
    ///
    /// Returns an error if the episode log cannot be read, the stores cannot
    /// be updated, or the review agent fails unexpectedly.
    pub async fn run(&mut self) -> Result<DreamCycleReport> {
        let started_at = Utc::now();
        let all_episodes = EpisodeLogger::read_all_lossy(self.episode_store.path())
            .await
            .with_context(|| {
                format!("read episode log from {}", self.episode_store.path().display())
            })?;
        let total_episodes = all_episodes.len();
        let cutoff = self.last_dream_at;
        let mut batch: Vec<_> = all_episodes
            .into_iter()
            .filter(|episode| cutoff.map(|ts| episode.timestamp > ts).unwrap_or(true))
            .collect();
        batch.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.id.cmp(&right.id))
        });

        let processed_through = batch.iter().map(|episode| episode.timestamp).max();
        let analysis = TierProgression::default().analyze(&batch);

        let mut knowledge_entries_written = 0usize;
        knowledge_entries_written += self.persist_analysis(&analysis)?;

        let playbook_written = self.persist_playbook(&analysis, started_at).await?;

        let agent_review = if batch.is_empty() {
            None
        } else {
            let review_prompt = build_review_prompt(&analysis, &batch, started_at);
            let signal = Signal::builder(Kind::Prompt)
                .body(Body::text(review_prompt))
                .build();
            let result = self.dispatcher.dispatch(&signal, &RokoContext::now()).await;
            let review_text = result.output.body.as_text().unwrap_or("").trim().to_string();
            if review_text.is_empty() {
                None
            } else {
                let entry = KnowledgeEntry {
                    id: format!("dream-review-{}", started_at.timestamp_millis()),
                    kind: KnowledgeKind::Insight,
                    content: review_text.clone(),
                    confidence: 0.35,
                    source_episodes: batch.iter().map(|episode| episode.id.clone()).collect(),
                    tags: vec![
                        "dream:agent-review".to_string(),
                        "dream:integration".to_string(),
                    ],
                    created_at: started_at,
                    half_life_days: KnowledgeKind::Insight.default_half_life_days(),
                    hdc_vector: None,
                };
                self.knowledge_store.add(entry)?;
                knowledge_entries_written += 1;
                Some(review_text)
            }
        };

        self.last_dream_at = Some(processed_through.unwrap_or(started_at));

        Ok(DreamCycleReport {
            started_at,
            completed_at: Utc::now(),
            total_episodes,
            processed_episodes: batch.len(),
            processed_through,
            analysis,
            knowledge_entries_written,
            playbook_written,
            agent_review,
        })
    }

    fn persist_analysis(&self, analysis: &TierProgressionReport) -> Result<usize> {
        let mut written = 0usize;
        for entry in analysis.insights.iter().map(KnowledgeEntry::from) {
            self.knowledge_store.add(entry)?;
            written += 1;
        }

        for entry in analysis.heuristics.iter().map(KnowledgeEntry::from) {
            self.knowledge_store.add(entry)?;
            written += 1;
        }

        if !analysis.playbook.rules.is_empty() {
            self.knowledge_store.add(KnowledgeEntry::from(&analysis.playbook))?;
            written += 1;
        }

        Ok(written)
    }

    async fn persist_playbook(
        &self,
        analysis: &TierProgressionReport,
        started_at: DateTime<Utc>,
    ) -> Result<bool> {
        if analysis.playbook.rules.is_empty() {
            return Ok(false);
        }

        let mut playbook = Playbook::new(
            format!("dream-cycle-{}", started_at.timestamp_millis()),
            format!("Dream consolidation from {} episodes", analysis.insights.len()),
        );
        playbook.name = format!("Dream cycle {}", started_at.format("%Y-%m-%d %H:%M:%S UTC"));
        playbook.steps = analysis
            .playbook
            .rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                PlaybookStep::new(
                    index as u32,
                    rule.summary(),
                    "dream_validation",
                    vec![
                        format!("confidence:{:.2}", rule.confidence),
                        format!("support:{}", rule.confirmations),
                    ],
                )
            })
            .collect();

        self.playbook_store
            .save(&playbook)
            .await
            .context("save dream playbook")?;
        Ok(true)
    }
}

fn build_review_prompt(
    analysis: &TierProgressionReport,
    batch: &[roko_learn::episode_logger::Episode],
    started_at: DateTime<Utc>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are reviewing an offline dream cycle.\n\n");
    prompt.push_str(&format!("Cycle start: {}\n", started_at.to_rfc3339()));
    prompt.push_str(&format!("Episodes processed: {}\n\n", batch.len()));

    prompt.push_str("Top insights:\n");
    if analysis.insights.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for insight in analysis.insights.iter().take(5) {
            prompt.push_str(&format!("- {}\n", insight.summary()));
        }
    }

    prompt.push_str("\nTop heuristics:\n");
    if analysis.heuristics.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for heuristic in analysis.heuristics.iter().take(5) {
            prompt.push_str(&format!("- {}\n", heuristic.summary()));
        }
    }

    prompt.push_str(
        "\nReturn one concise follow-up insight, warning, or experiment suggestion that builds on the batch.\n",
    );
    prompt
}

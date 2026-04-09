//! Main polling loop for `roko-chain-watcher`.
//!
//! The watcher repeatedly:
//! 1. queries pheromones + insights via HTTP JSON-RPC,
//! 2. runs the pattern-based reaction rules in [`crate::reactions`],
//! 3. applies a rate limit and executes the surviving reactions,
//! 4. sleeps for `poll_interval_ms` and repeats.
//!
//! The loop exits cleanly on `Ctrl+C` (handled by the caller) or when
//! `max_events` observations have been seen.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::{debug, info, warn};

use crate::config::WatcherCli;
use crate::reactions::{Reaction, ReactionKind, decide};
use crate::rpc_client::MirageRpcClient;

/// Converts an elapsed `Instant` duration to milliseconds, saturating at `u64::MAX`.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Long-running watcher handle.
pub struct Watcher {
    cli: WatcherCli,
    client: MirageRpcClient,
    reactions_this_min: AtomicU32,
    last_reset: Mutex<Instant>,
    events_seen: AtomicU64,
}

impl Watcher {
    /// Constructs a new `Watcher`.
    #[must_use]
    pub fn new(cli: WatcherCli, client: MirageRpcClient) -> Self {
        Self {
            cli,
            client,
            reactions_this_min: AtomicU32::new(0),
            last_reset: Mutex::new(Instant::now()),
            events_seen: AtomicU64::new(0),
        }
    }

    /// Reset the per-minute rate-limit counter if more than 60 seconds have elapsed.
    fn maybe_reset_window(&self) {
        let mut last = self.last_reset.lock();
        if last.elapsed() >= Duration::from_secs(60) {
            self.reactions_this_min.store(0, Ordering::Relaxed);
            *last = Instant::now();
        }
    }

    /// Returns `true` if a new reaction may fire, incrementing the counter.
    fn acquire_reaction_slot(&self) -> bool {
        self.maybe_reset_window();
        let current = self.reactions_this_min.load(Ordering::Relaxed);
        if current >= self.cli.max_reactions_per_min {
            return false;
        }
        self.reactions_this_min.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Executes one poll+react iteration. Returns the count of reactions executed.
    #[allow(clippy::cognitive_complexity)]
    async fn tick(&self) -> anyhow::Result<usize> {
        let pheromones = match self
            .client
            .chain_query_pheromones(&self.cli.query, self.cli.poll_k)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "chain_queryPheromones failed");
                Vec::new()
            }
        };
        let insights = match self
            .client
            .chain_search_insights(&self.cli.query, self.cli.poll_k, None)
            .await
        {
            Ok(i) => i,
            Err(e) => {
                warn!(error = %e, "chain_searchInsights failed");
                Vec::new()
            }
        };
        // chain_stats is fetched purely for observability — a single field per
        // tick tagged onto the debug log; failures are tolerated.
        if let Ok(stats) = self.client.chain_stats().await {
            debug!(
                insights_total = stats.insights,
                pheromones_total = stats.pheromones,
                "chain stats"
            );
        }

        let observed = pheromones.len() + insights.len();
        self.events_seen.fetch_add(
            u64::try_from(observed).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );

        debug!(
            pheromones = pheromones.len(),
            insights = insights.len(),
            "polled chain state"
        );

        let reactions = decide(&pheromones, &insights, &self.cli.watcher_id);
        let mut executed = 0usize;
        for reaction in reactions {
            if !self.acquire_reaction_slot() {
                warn!(
                    reason = %reaction.reason,
                    limit = self.cli.max_reactions_per_min,
                    "rate limit hit, dropping reaction"
                );
                continue;
            }
            if self.cli.dry_run {
                info!(
                    dry_run = true,
                    kind = ?reaction.kind,
                    reason = %reaction.reason,
                    "would have reacted"
                );
                continue;
            }
            match self.execute(&reaction).await {
                Ok(()) => {
                    executed += 1;
                    info!(
                        kind = ?reaction.kind,
                        reason = %reaction.reason,
                        "reaction executed"
                    );
                }
                Err(e) => {
                    warn!(
                        kind = ?reaction.kind,
                        error = %e,
                        "reaction failed"
                    );
                }
            }
        }
        Ok(executed)
    }

    /// Executes a single reaction against the RPC client.
    #[allow(clippy::cognitive_complexity)]
    async fn execute(&self, reaction: &Reaction) -> anyhow::Result<()> {
        let started = Instant::now();
        match reaction.kind {
            ReactionKind::PostInsight => {
                let kind = reaction.insight_kind.as_deref().unwrap_or("insight");
                let content = reaction.content.as_deref().unwrap_or("");
                let result = self
                    .client
                    .chain_post_insight(&self.cli.watcher_id, kind, content, 0)
                    .await?;
                info!(
                    method = "chain_postInsight",
                    outcome = %result.outcome,
                    id = %result.id,
                    similarity = result.similarity.unwrap_or(0.0),
                    latency_ms = elapsed_ms(started),
                    "post_insight ok"
                );
            }
            ReactionKind::DepositPheromone => {
                let kind = reaction.pheromone_kind.as_deref().unwrap_or("wisdom");
                let content = reaction.content.as_deref().unwrap_or("");
                let intensity = reaction.intensity.unwrap_or(0.2);
                let id = self
                    .client
                    .chain_deposit_pheromone(kind, content, intensity)
                    .await?;
                info!(
                    method = "chain_depositPheromone",
                    id = id.id,
                    latency_ms = elapsed_ms(started),
                    "deposit_pheromone ok"
                );
            }
            ReactionKind::ConfirmInsight => {
                let Some(target) = reaction.target_id.as_deref() else {
                    return Err(anyhow::anyhow!("confirm reaction missing target_id"));
                };
                self.client
                    .chain_confirm_insight(target, &self.cli.watcher_id)
                    .await?;
                info!(
                    method = "chain_confirmInsight",
                    target = target,
                    latency_ms = elapsed_ms(started),
                    "confirm ok"
                );
            }
            ReactionKind::ChallengeInsight => {
                let Some(target) = reaction.target_id.as_deref() else {
                    return Err(anyhow::anyhow!("challenge reaction missing target_id"));
                };
                self.client
                    .chain_challenge_insight(target, &self.cli.watcher_id)
                    .await?;
                info!(
                    method = "chain_challengeInsight",
                    target = target,
                    latency_ms = elapsed_ms(started),
                    "challenge ok"
                );
            }
        }
        Ok(())
    }

    /// Run the watcher loop until `max_events` is hit (if nonzero).
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(self) -> anyhow::Result<()> {
        let interval = Duration::from_millis(self.cli.poll_interval_ms);
        info!(
            poll_ms = self.cli.poll_interval_ms,
            dry_run = self.cli.dry_run,
            watcher_id = %self.cli.watcher_id,
            "watcher loop started"
        );
        loop {
            match self.tick().await {
                Ok(n) => debug!(executed = n, "tick complete"),
                Err(e) => warn!(error = %e, "tick failed"),
            }
            if self.cli.max_events > 0
                && self.events_seen.load(Ordering::Relaxed) >= self.cli.max_events
            {
                info!(
                    events = self.events_seen.load(Ordering::Relaxed),
                    max = self.cli.max_events,
                    "max_events reached, exiting"
                );
                break;
            }
            tokio::time::sleep(interval).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cli(max_rate: u32) -> WatcherCli {
        let mut cli = WatcherCli::with_defaults();
        cli.max_reactions_per_min = max_rate;
        cli
    }

    #[test]
    fn rate_limit_blocks_after_cap() {
        let cli = test_cli(2);
        let client = MirageRpcClient::new(cli.rpc_url.clone());
        let watcher = Watcher::new(cli, client);
        assert!(watcher.acquire_reaction_slot());
        assert!(watcher.acquire_reaction_slot());
        assert!(!watcher.acquire_reaction_slot());
    }

    #[test]
    fn rate_limit_zero_blocks_everything() {
        let cli = test_cli(0);
        let client = MirageRpcClient::new(cli.rpc_url.clone());
        let watcher = Watcher::new(cli, client);
        assert!(!watcher.acquire_reaction_slot());
    }

    #[test]
    fn events_seen_starts_at_zero() {
        let cli = test_cli(5);
        let client = MirageRpcClient::new(cli.rpc_url.clone());
        let watcher = Watcher::new(cli, client);
        assert_eq!(watcher.events_seen.load(Ordering::Relaxed), 0);
    }
}

//! Background dream-cycle bootstrap for daemon mode.
//!
//! The loop watches for idle periods in daemon mode, then runs the existing
//! `roko-dreams` batch processor when enough new episodes have accumulated.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::{DateTime, TimeZone, Utc};
use roko_core::ContentHash;
use roko_daimon::{
    AffectEngine as _, AffectEvent, DaimonState, EpisodeStrategyObservation, SomaticMarker,
    StrategyCoordinates, StrategySpaceDefinition,
};
use roko_dreams::cycle::{DreamCycleReport, DreamOutcome};
use roko_dreams::{DreamCycle, build_dream_review_dispatcher};
use roko_learn::{
    episode_logger::{Episode, EpisodeLogger},
    playbook::PlaybookStore,
};
use roko_neuro::KnowledgeStore;
use serde_json::Value;
use tokio::task::JoinHandle;
use tokio::time::{Instant as TokioInstant, interval_at};
use tracing::{info, warn};

use crate::state::AppState;

pub use roko_dreams::{DreamAgentConfig, DreamLoopConfig};

const DREAM_CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Start the dream cycle in the background.
#[must_use]
pub fn start_dream_loop(state: Arc<AppState>, config: DreamLoopConfig) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !config.auto_dream {
            return;
        }

        let mut cycle = match build_dream_cycle(&state, &config).await {
            Ok(cycle) => cycle,
            Err(err) => {
                warn!(error = %err, "dream cycle bootstrap failed");
                return;
            }
        };

        if let Err(err) = restore_last_dream_at(&state, &mut cycle) {
            warn!(error = %err, "failed to restore last dream checkpoint");
        }

        let idle_threshold = Duration::from_secs(config.idle_threshold_mins.saturating_mul(60));
        let mut idle_since: Option<TokioInstant> = None;
        let mut interval = interval_at(
            TokioInstant::now() + DREAM_CHECK_INTERVAL,
            DREAM_CHECK_INTERVAL,
        );

        loop {
            interval.tick().await;

            if state.cancel.is_cancelled() {
                break;
            }

            let active_agents = state.supervisor.count().await;
            if active_agents > 0 {
                idle_since = None;
                continue;
            }

            let now = TokioInstant::now();
            let started_idle = idle_since.get_or_insert(now);
            if now.duration_since(*started_idle) < idle_threshold {
                continue;
            }

            if let Err(err) =
                maybe_run_dream_cycle(&state, &mut cycle, config.min_episodes_for_dream).await
            {
                warn!(error = %err, "dream cycle failed");
            }
        }
    })
}

/// Run one dream cycle immediately using the existing stores and agent config.
///
/// This mirrors the daemon bootstrap path, but executes the batch once instead
/// of waiting for the idle scheduler.
pub async fn run_dream_cycle_now(
    state: Arc<AppState>,
    config: DreamLoopConfig,
) -> Result<DreamCycleReport> {
    let mut cycle = build_dream_cycle(&state, &config).await?;
    restore_last_dream_at(&state, &mut cycle)?;
    let report = cycle.run().await.context("run dream cycle")?;
    apply_dream_affect_feedback(&state, &report).await;
    Ok(report)
}

/// Load the latest persisted dream report from the report directory.
///
/// Returns `Ok(None)` when no report exists yet.
pub fn load_latest_dream_report(report_dir: &Path) -> Result<Option<DreamCycleReport>> {
    let Some(path) = latest_dream_report_path(report_dir)? else {
        return Ok(None);
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read dream report {}", path.display()))?;
    let report: DreamCycleReport = serde_json::from_str(&text)
        .with_context(|| format!("parse dream report {}", path.display()))?;
    Ok(Some(report))
}

async fn build_dream_cycle(state: &AppState, config: &DreamLoopConfig) -> Result<DreamCycle> {
    let episodes = Arc::new(EpisodeLogger::new(state.layout.episodes_path()));
    let knowledge = Arc::new(KnowledgeStore::for_layout(&state.layout));
    let playbooks_root = state.layout.root().join("learn").join("playbooks");
    let playbooks = Arc::new(PlaybookStore::new(playbooks_root));
    let dispatcher = build_dream_review_dispatcher(&state.workdir, &config.agent)?;
    Ok(DreamCycle::new(episodes, knowledge, playbooks, dispatcher))
}

fn restore_last_dream_at(state: &AppState, cycle: &mut DreamCycle) -> Result<()> {
    let report_dir = state.layout.root().join("dreams");
    let Some(path) = latest_dream_report_path(&report_dir)? else {
        return Ok(());
    };
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read dream report {}", path.display()))?;
    let report: DreamCycleReport = serde_json::from_str(&text)
        .with_context(|| format!("parse dream report {}", path.display()))?;
    cycle.set_last_dream_at(report.processed_through.or(Some(report.started_at)));
    Ok(())
}

fn latest_dream_report_path(report_dir: &Path) -> Result<Option<PathBuf>> {
    let Ok(entries) = fs::read_dir(report_dir) else {
        return Ok(None);
    };

    let mut latest: Option<(DateTime<Utc>, PathBuf)> = None;
    for entry in entries {
        let entry = entry.with_context(|| format!("scan {}", report_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some(ts) = stem.strip_prefix("dream-") else {
            continue;
        };
        let Ok(ts_ms) = ts.parse::<i64>() else {
            continue;
        };
        let Some(dt) = Utc.timestamp_millis_opt(ts_ms).single() else {
            continue;
        };

        let should_replace = latest.as_ref().is_none_or(|(current, _)| dt > *current);
        if should_replace {
            latest = Some((dt, path));
        }
    }

    Ok(latest.map(|(_, path)| path))
}

async fn maybe_run_dream_cycle(
    state: &AppState,
    cycle: &mut DreamCycle,
    min_episodes_for_dream: usize,
) -> Result<()> {
    let episodes_path = state.layout.episodes_path();
    let episodes = EpisodeLogger::read_all_lossy(&episodes_path)
        .await
        .with_context(|| format!("load episodes from {}", episodes_path.display()))?;
    let last_dream_at = cycle.last_dream_at();
    let new_episode_count = episodes
        .iter()
        .filter(|episode| {
            last_dream_at
                .map(|cutoff| episode.timestamp > cutoff)
                .unwrap_or(true)
        })
        .count();

    if new_episode_count < min_episodes_for_dream {
        return Ok(());
    }

    info!(
        new_episodes = new_episode_count,
        min_episodes_for_dream, "running dream cycle"
    );
    let report = cycle.run().await.context("run dream cycle")?;
    info!(
        processed_episodes = report.processed_episodes,
        knowledge_entries_written = report.knowledge_entries_written,
        playbooks_created = report.playbooks_created,
        "dream cycle completed"
    );
    apply_dream_affect_feedback(state, &report).await;
    Ok(())
}

async fn apply_dream_affect_feedback(state: &AppState, report: &DreamCycleReport) {
    let dream_episodes = match load_dream_feedback_episodes(state, report).await {
        Ok(episodes) => episodes,
        Err(err) => {
            warn!(error = %err, "failed to load dream episodes for somatic consolidation");
            Vec::new()
        }
    };
    let mut engine = state.affect_engine.lock();
    apply_dream_affect_feedback_to_engine(&mut engine, report, &dream_episodes);
}

async fn load_dream_feedback_episodes(
    state: &AppState,
    report: &DreamCycleReport,
) -> Result<Vec<Episode>> {
    let episode_ids: BTreeSet<&str> = report
        .clusters
        .iter()
        .flat_map(|cluster| cluster.episode_ids.iter().map(String::as_str))
        .filter(|episode_id| !episode_id.trim().is_empty())
        .collect();
    if episode_ids.is_empty() {
        return Ok(Vec::new());
    }

    let episodes_path = state.layout.episodes_path();
    let all = EpisodeLogger::read_all_lossy(&episodes_path)
        .await
        .context("load episodes for dream somatic feedback")?;
    Ok(all
        .into_iter()
        .filter(|episode| episode_ids.contains(episode_source_id(episode)))
        .collect())
}

fn apply_dream_affect_feedback_to_engine(
    engine: &mut DaimonState,
    report: &DreamCycleReport,
    dream_episodes: &[Episode],
) {
    let mut failing_task_types: BTreeMap<String, usize> = BTreeMap::new();
    for cluster in &report.clusters {
        if cluster.key.outcome != DreamOutcome::Failure || cluster.failure_count <= 2 {
            continue;
        }
        *failing_task_types
            .entry(cluster.key.task_type.clone())
            .or_insert(0) += cluster.failure_count;
    }

    for (task_type, failure_count) in failing_task_types {
        let _ = engine.appraise(AffectEvent::DreamFailure {
            task_type,
            failure_count,
        });
    }

    if report.processed_episodes > 0 {
        let _ = engine.apply_dream_depotentiation();
    }

    let strategy_space = engine.strategy_space().clone();
    for marker in synthesize_dream_somatic_markers(report, dream_episodes, &strategy_space) {
        engine.record_somatic_marker(marker);
    }
}

fn synthesize_dream_somatic_markers(
    report: &DreamCycleReport,
    dream_episodes: &[Episode],
    strategy_space: &StrategySpaceDefinition,
) -> Vec<SomaticMarker> {
    let episodes_by_id: BTreeMap<&str, &Episode> = dream_episodes
        .iter()
        .map(|episode| (episode_source_id(episode), episode))
        .collect();

    report
        .clusters
        .iter()
        .filter_map(|cluster| {
            let episodes = cluster
                .episode_ids
                .iter()
                .filter_map(|episode_id| episodes_by_id.get(episode_id.as_str()).copied())
                .collect::<Vec<_>>();
            build_cluster_somatic_marker(cluster, &episodes, strategy_space)
        })
        .collect()
}

fn build_cluster_somatic_marker(
    cluster: &roko_dreams::cycle::DreamClusterReport,
    episodes: &[&Episode],
    strategy_space: &StrategySpaceDefinition,
) -> Option<SomaticMarker> {
    if episodes.is_empty() {
        return None;
    }

    let has_strong_affect = episodes
        .iter()
        .any(|episode| episode_marker_intensity(episode) >= 0.55);
    if cluster.episode_count < 2 && !has_strong_affect {
        return None;
    }

    let strategy_coords = average_strategy_coordinates(
        episodes
            .iter()
            .map(|episode| strategy_coordinates_from_episode(episode, strategy_space)),
    );
    let average_valence = episodes
        .iter()
        .map(|episode| episode_marker_valence(episode, cluster.key.outcome))
        .sum::<f64>()
        / episodes.len() as f64;
    let repetition_boost = ((cluster.episode_count.saturating_sub(1)).min(4) as f64) * 0.05;
    let intensity = (episodes
        .iter()
        .map(|episode| episode_marker_intensity(episode))
        .sum::<f64>()
        / episodes.len() as f64
        + repetition_boost)
        .clamp(0.0, 1.0);

    if average_valence.abs() < 0.10 || intensity < 0.35 {
        return None;
    }

    let mut source_episodes = Vec::new();
    for episode in episodes {
        let hash = episode_content_hash(episode);
        if !source_episodes.contains(&hash) {
            source_episodes.push(hash);
        }
    }

    Some(SomaticMarker {
        strategy_coords,
        valence: average_valence.clamp(-1.0, 1.0),
        intensity,
        episodes: source_episodes,
        updated_at: cluster.last_seen_at,
    })
}

fn average_strategy_coordinates(
    strategies: impl IntoIterator<Item = StrategyCoordinates>,
) -> StrategyCoordinates {
    let mut count = 0.0;
    let mut total = [0.0; 8];
    for strategy in strategies {
        let values = strategy.as_array();
        for (slot, value) in total.iter_mut().zip(values) {
            *slot += value;
        }
        count += 1.0;
    }

    if count <= 0.0 {
        return StrategyCoordinates::neutral();
    }

    StrategyCoordinates::new(
        total[0] / count,
        total[1] / count,
        total[2] / count,
        total[3] / count,
        total[4] / count,
        total[5] / count,
        total[6] / count,
        total[7] / count,
    )
}

fn strategy_coordinates_from_episode(
    episode: &Episode,
    fallback_strategy_space: &StrategySpaceDefinition,
) -> StrategyCoordinates {
    if let Some(value) = episode.extra.get("strategy_coordinates")
        && let Ok(coords) = serde_json::from_value::<StrategyCoordinates>(value.clone())
    {
        return coords;
    }

    let strategy_space =
        strategy_space_from_episode(episode).unwrap_or_else(|| fallback_strategy_space.clone());

    let tier = extra_string(episode, "task_tier")
        .or_else(|| nested_extra_string(episode, "original_task", "tier"))
        .unwrap_or_else(|| "focused".to_string());
    let file_count = extra_usize(episode, "file_count")
        .or_else(|| extra_array_len(episode, "files"))
        .or_else(|| nested_extra_array_len(episode, "original_task", "files"))
        .unwrap_or(0) as f64;
    let verify_count = extra_usize(episode, "verify_count")
        .or_else(|| nested_extra_array_len(episode, "original_task", "verify"))
        .unwrap_or(0) as f64;
    let dependency_count = extra_usize(episode, "dependency_count")
        .or_else(|| nested_extra_array_len(episode, "original_task", "depends_on"))
        .unwrap_or(0) as f64;
    let max_loc = extra_f64(episode, "max_loc").unwrap_or(50.0);
    let familiarity = extra_f64(episode, "crate_familiarity").unwrap_or(0.5);
    let affect_confidence =
        extra_f64(episode, "affect_confidence").unwrap_or(if episode.success { 0.6 } else { 0.35 });
    let failure_pressure = f64::from(!episode.success);
    let observation = EpisodeStrategyObservation {
        task_tier: tier,
        file_count: file_count as usize,
        verification_count: verify_count as usize,
        dependency_count: dependency_count as usize,
        max_loc: max_loc.round().clamp(0.0, f64::from(u32::MAX)) as u32,
        familiarity,
        confidence: affect_confidence,
        failure_pressure,
        emotional_intensity: episode_marker_intensity(episode),
    };

    strategy_space.computer().episode_coords(&observation)
}

fn strategy_space_from_episode(episode: &Episode) -> Option<StrategySpaceDefinition> {
    let domain = extra_string(episode, "strategy_space_domain")?;
    let dimensions_value = episode.extra.get("strategy_space_dimensions")?.clone();
    let dimensions_vec = serde_json::from_value::<Vec<String>>(dimensions_value).ok()?;
    let dimensions: [String; 8] = dimensions_vec.try_into().ok()?;
    StrategySpaceDefinition { domain, dimensions }
        .validate()
        .ok()
}

fn episode_marker_valence(episode: &Episode, outcome: DreamOutcome) -> f64 {
    if let Some(tag) = episode.emotional_tag.as_ref() {
        return tag.pad.pleasure.clamp(-1.0, 1.0);
    }
    match outcome {
        DreamOutcome::Success => 0.35,
        DreamOutcome::Failure => -0.45,
    }
}

fn episode_marker_intensity(episode: &Episode) -> f64 {
    episode
        .emotional_tag
        .as_ref()
        .map(|tag| tag.pad.arousal.abs().max(f64::from(tag.intensity)))
        .unwrap_or_else(|| if episode.success { 0.35 } else { 0.60 })
        .clamp(0.0, 1.0)
}

fn episode_content_hash(episode: &Episode) -> ContentHash {
    ContentHash::from_hex(&episode.output_signal_hash)
        .or_else(|| ContentHash::from_hex(episode_source_id(episode)))
        .unwrap_or_else(|| ContentHash::of(episode_source_id(episode).as_bytes()))
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn nested_extra_string(episode: &Episode, object_key: &str, field_key: &str) -> Option<String> {
    episode
        .extra
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|object| object.get(field_key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn extra_usize(episode: &Episode, key: &str) -> Option<usize> {
    episode
        .extra
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn extra_f64(episode: &Episode, key: &str) -> Option<f64> {
    episode.extra.get(key).and_then(Value::as_f64)
}

fn extra_array_len(episode: &Episode, key: &str) -> Option<usize> {
    episode
        .extra
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
}

fn nested_extra_array_len(episode: &Episode, object_key: &str, field_key: &str) -> Option<usize> {
    episode
        .extra
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|object| object.get(field_key))
        .and_then(Value::as_array)
        .map(Vec::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Duration as ChronoDuration;
    use roko_core::{EmotionalTag, PadVector};
    use roko_daimon::{DaimonState, StrategyCoordinates};
    use roko_dreams::cycle::{DreamClusterKey, DreamClusterReport, DreamOutcome};
    use serde_json::json;

    #[test]
    fn dream_failures_reduce_confidence_by_task_type() {
        let mut engine = DaimonState::new();
        let report = DreamCycleReport {
            started_at: Utc::now() - ChronoDuration::minutes(10),
            completed_at: Utc::now(),
            total_episodes: 3,
            processed_episodes: 3,
            processed_through: None,
            analysis: roko_neuro::tier_progression::TierProgression::default().analyze(&[]),
            cfactor_regression: None,
            clusters: vec![DreamClusterReport {
                key: DreamClusterKey {
                    plan_id: "plan-a".to_string(),
                    task_type: "implementation".to_string(),
                    outcome: DreamOutcome::Failure,
                    model: "claude-haiku-4-5".to_string(),
                },
                episode_count: 3,
                success_count: 0,
                failure_count: 3,
                first_seen_at: Utc::now() - ChronoDuration::minutes(20),
                last_seen_at: Utc::now() - ChronoDuration::minutes(5),
                episode_ids: vec!["ep-1".to_string(), "ep-2".to_string(), "ep-3".to_string()],
                knowledge_entries: Vec::new(),
                playbook: None,
                regression_entries: Vec::new(),
                agent_review: None,
                warnings: Vec::new(),
            }],
            knowledge_entries_written: 0,
            playbooks_created: 0,
            regressions_detected: Vec::new(),
            strategy_hypotheses: Vec::new(),
            performance_notes: Vec::new(),
        };

        apply_dream_affect_feedback_to_engine(&mut engine, &report, &[]);

        let state = engine.query();
        assert!(state.confidence < 0.5);
        assert!(state.confidence > 0.25);
    }

    #[test]
    fn dream_feedback_depotentiates_arousal_and_somatic_markers() {
        let mut engine = DaimonState::new();
        engine.state.pad.arousal = 0.9;
        engine.somatic_landscape.record_outcome(
            roko_daimon::StrategyCoordinates::new(0.8, 0.7, 0.6, 0.5, 0.5, 0.8, 0.5, 0.7),
            -0.8,
            0.8,
            roko_core::ContentHash::of(b"dream-marker"),
            Utc::now(),
        );

        let report = DreamCycleReport {
            started_at: Utc::now() - ChronoDuration::minutes(10),
            completed_at: Utc::now(),
            total_episodes: 1,
            processed_episodes: 1,
            processed_through: None,
            analysis: roko_neuro::tier_progression::TierProgression::default().analyze(&[]),
            cfactor_regression: None,
            clusters: Vec::new(),
            knowledge_entries_written: 0,
            playbooks_created: 0,
            regressions_detected: Vec::new(),
            strategy_hypotheses: Vec::new(),
            performance_notes: Vec::new(),
        };

        apply_dream_affect_feedback_to_engine(&mut engine, &report, &[]);

        assert!(engine.query().pad.arousal < 0.9);
        assert!(engine.somatic_landscape.markers[0].intensity < 0.8);
    }

    #[test]
    fn dream_feedback_synthesizes_somatic_markers_from_replayed_episodes() {
        let mut engine = DaimonState::new();
        let report = DreamCycleReport {
            started_at: Utc::now() - ChronoDuration::minutes(10),
            completed_at: Utc::now(),
            total_episodes: 2,
            processed_episodes: 2,
            processed_through: None,
            analysis: roko_neuro::tier_progression::TierProgression::default().analyze(&[]),
            cfactor_regression: None,
            clusters: vec![DreamClusterReport {
                key: DreamClusterKey {
                    plan_id: "plan-a".to_string(),
                    task_type: "implementation".to_string(),
                    outcome: DreamOutcome::Failure,
                    model: "claude-sonnet-4-6".to_string(),
                },
                episode_count: 2,
                success_count: 0,
                failure_count: 2,
                first_seen_at: Utc::now() - ChronoDuration::minutes(20),
                last_seen_at: Utc::now() - ChronoDuration::minutes(5),
                episode_ids: vec!["ep-1".to_string(), "ep-2".to_string()],
                knowledge_entries: Vec::new(),
                playbook: None,
                regression_entries: Vec::new(),
                agent_review: None,
                warnings: Vec::new(),
            }],
            knowledge_entries_written: 0,
            playbooks_created: 0,
            regressions_detected: Vec::new(),
            strategy_hypotheses: Vec::new(),
            performance_notes: Vec::new(),
        };
        let episodes = vec![
            dreamed_episode(
                "ep-1",
                false,
                -0.7,
                0.8,
                StrategyCoordinates::new(0.8, 0.7, 0.6, 0.4, 0.7, 0.85, 0.35, 0.6),
            ),
            dreamed_episode(
                "ep-2",
                false,
                -0.5,
                0.7,
                StrategyCoordinates::new(0.75, 0.65, 0.55, 0.45, 0.65, 0.80, 0.40, 0.55),
            ),
        ];

        apply_dream_affect_feedback_to_engine(&mut engine, &report, &episodes);

        let synthesized = engine
            .somatic_landscape
            .markers
            .iter()
            .find(|marker| marker.episodes.len() == 2)
            .expect("synthesized marker");
        assert!(synthesized.valence < -0.3);
        assert!(synthesized.intensity > 0.6);
        assert!(synthesized.strategy_coords.complexity > 0.7);
    }

    #[test]
    fn dream_marker_synthesis_falls_back_to_episode_metadata_when_needed() {
        let report = DreamCycleReport {
            started_at: Utc::now() - ChronoDuration::minutes(10),
            completed_at: Utc::now(),
            total_episodes: 2,
            processed_episodes: 2,
            processed_through: None,
            analysis: roko_neuro::tier_progression::TierProgression::default().analyze(&[]),
            cfactor_regression: None,
            clusters: vec![DreamClusterReport {
                key: DreamClusterKey {
                    plan_id: "plan-a".to_string(),
                    task_type: "implementation".to_string(),
                    outcome: DreamOutcome::Success,
                    model: "claude-sonnet-4-6".to_string(),
                },
                episode_count: 2,
                success_count: 2,
                failure_count: 0,
                first_seen_at: Utc::now() - ChronoDuration::minutes(20),
                last_seen_at: Utc::now() - ChronoDuration::minutes(5),
                episode_ids: vec!["ep-3".to_string(), "ep-4".to_string()],
                knowledge_entries: Vec::new(),
                playbook: None,
                regression_entries: Vec::new(),
                agent_review: None,
                warnings: Vec::new(),
            }],
            knowledge_entries_written: 0,
            playbooks_created: 0,
            regressions_detected: Vec::new(),
            strategy_hypotheses: Vec::new(),
            performance_notes: Vec::new(),
        };
        let mut first = Episode::new("Implementer", "task-a").succeeded();
        first.episode_id = "ep-3".to_string();
        first.emotional_tag = Some(EmotionalTag::new(
            PadVector::new(0.6, 0.4, 0.2),
            0.6,
            "success",
            PadVector::new(0.4, 0.3, 0.2),
        ));
        first
            .extra
            .insert("task_tier".to_string(), json!("architectural"));
        first.extra.insert("file_count".to_string(), json!(6));
        first.extra.insert("verify_count".to_string(), json!(3));
        first.extra.insert("dependency_count".to_string(), json!(4));
        first.extra.insert("max_loc".to_string(), json!(250));
        first
            .extra
            .insert("crate_familiarity".to_string(), json!(0.2));

        let mut second = first.clone();
        second.episode_id = "ep-4".to_string();
        second.output_signal_hash = "not-a-hash".to_string();

        let markers = synthesize_dream_somatic_markers(
            &report,
            &[first, second],
            &StrategySpaceDefinition::coding(),
        );
        let marker = markers.first().expect("marker");
        assert!(marker.valence > 0.3);
        assert!(marker.strategy_coords.complexity > 0.8);
        assert!(marker.strategy_coords.scope > 0.6);
    }

    #[test]
    fn dream_marker_synthesis_prefers_recorded_strategy_space_definition() {
        let mut episode = Episode::new("Implementer", "task-a").succeeded();
        episode
            .extra
            .insert("strategy_space_domain".to_string(), json!("chain"));
        episode.extra.insert(
            "strategy_space_dimensions".to_string(),
            json!([
                "volatility",
                "liquidity",
                "correlation",
                "leverage",
                "time_horizon",
                "concentration",
                "counterparty_risk",
                "regulatory_exposure"
            ]),
        );

        let strategy_space = strategy_space_from_episode(&episode).expect("strategy space");

        assert_eq!(strategy_space.domain, "chain");
        assert_eq!(strategy_space.labels()[0], "volatility");
    }

    fn dreamed_episode(
        episode_id: &str,
        success: bool,
        pleasure: f64,
        arousal: f64,
        strategy: StrategyCoordinates,
    ) -> Episode {
        let mut episode = Episode::new("Implementer", "task-a");
        episode.success = success;
        episode.episode_id = episode_id.to_string();
        episode.output_signal_hash = ContentHash::of(episode_id.as_bytes()).to_hex();
        episode.emotional_tag = Some(EmotionalTag::new(
            PadVector::new(pleasure, arousal, 0.1),
            arousal.abs() as f32,
            if success {
                "task_success"
            } else {
                "task_failure"
            },
            PadVector::new(pleasure * 0.8, arousal * 0.8, 0.1),
        ));
        episode.extra.insert(
            "strategy_coordinates".to_string(),
            serde_json::to_value(strategy).expect("strategy coordinates"),
        );
        episode
    }
}

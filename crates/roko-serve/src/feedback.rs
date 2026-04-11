//! Background feedback collection for external actions recorded in episodes.
//!
//! The loop scans recent episodes, finds external actions in the last 7 days,
//! polls the corresponding external services on a 15m/6h schedule, and
//! persists the collected feedback as normal signals.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Utc};
use octocrab::{Octocrab, Page};
use reqwest::Client;
use roko_core::tool::ExternalAction;
use roko_core::{Body, Kind, Provenance, Signal};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::prompt_experiment::ExperimentStore;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::{Instant as TokioInstant, interval_at};
use tracing::{info, warn};

use super::dispatch::record_cascade_router_observation;
use crate::events::ServerEvent;
use crate::state::AppState;

const FEEDBACK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const FEEDBACK_SHORT_INTERVAL: Duration = Duration::from_secs(15 * 60);
const FEEDBACK_LONG_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const FEEDBACK_MAX_WINDOW: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const FEEDBACK_SOURCE: &str = "roko-serve:feedback";

/// Start the feedback collection loop as a detached background task.
#[must_use]
pub fn start_feedback_loop(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = interval_at(TokioInstant::now() + FEEDBACK_INTERVAL, FEEDBACK_INTERVAL);
        let mut completed_actions = HashSet::new();
        let mut last_polled_actions = std::collections::HashMap::new();

        loop {
            interval.tick().await;

            if state.cancel.is_cancelled() {
                break;
            }

            if let Err(err) =
                collect_feedback_cycle(&state, &mut completed_actions, &mut last_polled_actions)
                    .await
            {
                warn!(error = %err, "feedback collection failed");
            }
        }
    })
}

async fn collect_feedback_cycle(
    state: &AppState,
    completed_actions: &mut HashSet<String>,
    last_polled_actions: &mut std::collections::HashMap<String, DateTime<Utc>>,
) -> Result<()> {
    let episodes = load_recent_episodes(state).await?;
    let now = Utc::now();
    let client = Client::builder()
        .user_agent("roko-serve-feedback/0.1")
        .build()
        .context("build feedback HTTP client")?;

    let mut collected = 0usize;
    for episode in episodes {
        for action in &episode.external_actions {
            let Some(action) = parse_external_action(action.clone()) else {
                continue;
            };
            if !within_recent_window(&action.performed_at) {
                continue;
            }
            let key = action_key(&episode, &action);
            if completed_actions.contains(&key) {
                continue;
            }
            if !should_poll_action(&action, now, last_polled_actions.get(&key).copied()) {
                continue;
            }
            last_polled_actions.insert(key.clone(), now);

            let feedback = match action.service.as_str() {
                "github" => collect_github_feedback(&episode, &action).await?,
                "slack" => collect_slack_feedback(&client, &episode, &action).await?,
                _ => None,
            };

            if let Some(feedback) = feedback {
                persist_feedback_result(state, &episode, &action, feedback).await?;
                completed_actions.insert(key);
                collected += 1;
            }
        }
    }

    if collected > 0 {
        info!(count = collected, "collected feedback signals");
    }

    Ok(())
}

async fn load_recent_episodes(state: &AppState) -> Result<Vec<Episode>> {
    let path = state.layout.episodes_path();
    let episodes = EpisodeLogger::read_all_lossy(&path)
        .await
        .with_context(|| format!("load episodes from {}", path.display()))?;
    let cutoff = Utc::now()
        - chrono::Duration::from_std(FEEDBACK_MAX_WINDOW)
            .unwrap_or_else(|_| chrono::Duration::days(7));

    Ok(episodes
        .into_iter()
        .filter(|episode| episode.timestamp >= cutoff && !episode.external_actions.is_empty())
        .collect())
}

fn parse_external_action(value: Value) -> Option<ExternalAction> {
    serde_json::from_value(value).ok()
}

fn within_recent_window(performed_at: &DateTime<Utc>) -> bool {
    let cutoff = Utc::now()
        - chrono::Duration::from_std(FEEDBACK_MAX_WINDOW)
            .unwrap_or_else(|_| chrono::Duration::days(7));
    *performed_at >= cutoff
}

fn should_poll_action(
    action: &ExternalAction,
    now: DateTime<Utc>,
    last_polled_at: Option<DateTime<Utc>>,
) -> bool {
    let interval = match now.signed_duration_since(action.performed_at).to_std() {
        Ok(age) if age <= Duration::from_secs(24 * 60 * 60) => FEEDBACK_SHORT_INTERVAL,
        Ok(age) if age <= FEEDBACK_MAX_WINDOW => FEEDBACK_LONG_INTERVAL,
        _ => return false,
    };

    match last_polled_at {
        None => true,
        Some(last_polled_at) => now
            .signed_duration_since(last_polled_at)
            .to_std()
            .map_or(true, |elapsed| elapsed >= interval),
    }
}

fn action_key(episode: &Episode, action: &ExternalAction) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        episode.episode_id,
        action.service,
        action.action_type,
        action.resource_id,
        action
            .performed_at
            .timestamp_nanos_opt()
            .unwrap_or_default()
    )
}

fn github_client(token: &str) -> Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token.to_string())
        .build()
        .context("build GitHub client")
}

fn value_as_strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(stringish_value).collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn stringish_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("login")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn reaction_content(value: &Value) -> Option<&str> {
    value.get("content").and_then(Value::as_str)
}

fn reaction_sentiment(reactions: &[Value]) -> (u32, u32, f64) {
    let positive = ["+1", "heart", "hooray", "rocket", "eyes", "laugh"];
    let negative = ["-1", "confused"];
    let pos = reactions
        .iter()
        .filter(|reaction| reaction_content(reaction).is_some_and(|name| positive.contains(&name)))
        .count() as u32;
    let neg = reactions
        .iter()
        .filter(|reaction| reaction_content(reaction).is_some_and(|name| negative.contains(&name)))
        .count() as u32;
    let total = pos + neg;
    let sentiment = if total == 0 {
        0.0
    } else {
        (pos as f64 - neg as f64) / total as f64
    };
    (pos, neg, sentiment)
}

fn issue_labels_changed(current_labels: &[String], initial_labels: &[String]) -> bool {
    let current = current_labels.iter().collect::<HashSet<_>>();
    let initial = initial_labels.iter().collect::<HashSet<_>>();
    current != initial
}

#[derive(Debug, Clone)]
struct FeedbackObservation {
    success: bool,
    sentiment: f64,
    payload: Value,
}

async fn persist_feedback_result(
    state: &AppState,
    episode: &Episode,
    action: &ExternalAction,
    observation: FeedbackObservation,
) -> Result<()> {
    let feedback_kind = feedback_signal_kind(&action.service, &action.action_type);
    let signal = Signal::builder(feedback_kind)
        .body(Body::from_json(&json!({
            "episode_id": episode.episode_id,
            "episode_hash": episode.id,
            "service": action.service,
            "action_type": action.action_type,
            "resource_id": action.resource_id,
            "performed_at": action.performed_at,
            "collected_at": Utc::now(),
            "success": observation.success,
            "details": observation.payload,
        }))?)
        .provenance(Provenance::trusted(FEEDBACK_SOURCE))
        .build();

    state
        .signal_store
        .put(signal.clone())
        .await
        .with_context(|| format!("persist feedback signal for {}", action.resource_id))?;

    state.event_bus.publish(ServerEvent::OperationCompleted {
        op_id: format!("feedback:{}:{}", action.service, action.resource_id),
        kind: format!("feedback:{}", action.service),
        success: observation.success,
    });

    let experiment_name = {
        let templates = state.templates.read().await;
        templates
            .get(&episode.agent_template)
            .and_then(|template| template.experiment.as_ref())
            .map(|experiment| experiment.name.clone())
    };

    if let (Some(experiment_name), Some(variant_id)) =
        (experiment_name, episode_experiment_variant(episode))
    {
        if let Err(err) =
            record_experiment_outcome(&state.workdir, &variant_id, observation.success)
        {
            warn!(
                error = %err,
                episode_id = %episode.episode_id,
                experiment = %experiment_name,
                variant_id = %variant_id,
                "failed to record experiment outcome"
            );
        }

        if let Err(err) = record_experiment_metric(
            &state.workdir,
            &experiment_name,
            &variant_id,
            observation.sentiment,
        ) {
            warn!(
                error = %err,
                episode_id = %episode.episode_id,
                experiment = %experiment_name,
                variant_id = %variant_id,
                "failed to record experiment metric"
            );
        }
    }

    if observation.sentiment > 0.0
        && !episode.model.trim().is_empty()
        && let Err(err) = record_positive_cascade_feedback(&state.workdir, episode)
    {
        warn!(
            error = %err,
            episode_id = %episode.episode_id,
            model = %episode.model,
            "failed to record positive cascade router feedback"
        );
    }

    Ok(())
}

fn feedback_signal_kind(service: &str, action_type: &str) -> Kind {
    Kind::Custom(format!("feedback:{service}:{action_type}"))
}

fn record_positive_cascade_feedback(workdir: &Path, episode: &Episode) -> Result<()> {
    let model_slug = episode.model.trim();
    if model_slug.is_empty() {
        return Ok(());
    }

    record_cascade_router_observation(workdir, vec![model_slug.to_string()], model_slug, true)?;
    Ok(())
}

fn episode_experiment_variant(episode: &Episode) -> Option<String> {
    episode
        .extra
        .get("experiment_variant")
        .and_then(Value::as_str)
        .or_else(|| {
            episode
                .extra
                .get("experiment_variant_id")
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .filter(|variant| !variant.is_empty())
}

fn sentiment_to_metric_value(sentiment: f64) -> f64 {
    ((sentiment.clamp(-1.0, 1.0) + 1.0) / 2.0).clamp(0.0, 1.0)
}

fn record_experiment_outcome(workdir: &Path, variant_id: &str, success: bool) -> Result<()> {
    let path = workdir.join(".roko/learn/experiments.json");
    let mut store = ExperimentStore::load_or_new(&path);
    store.record_outcome(variant_id, success);
    store
        .save(&path)
        .with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

fn record_experiment_metric(
    workdir: &Path,
    experiment_name: &str,
    variant_id: &str,
    sentiment: f64,
) -> Result<()> {
    let path = workdir.join(".roko/learn/experiments.json");
    let mut store = ExperimentStore::load_or_new(&path);
    let metric_value = sentiment_to_metric_value(sentiment);
    store.record_metric(experiment_name, variant_id, metric_value);
    store
        .save(&path)
        .with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct GitHubResource {
    owner: String,
    repo: String,
    number: u64,
}

fn parse_github_resource(action: &ExternalAction) -> Result<GitHubResource> {
    if let (Some(owner), Some(repo), Some(number)) = (
        action.metadata.get("owner").and_then(Value::as_str),
        action.metadata.get("repo").and_then(Value::as_str),
        action.metadata.get("number").and_then(Value::as_u64),
    ) {
        return Ok(GitHubResource {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        });
    }

    let resource = action.resource_id.trim();
    let resource = resource
        .strip_prefix("https://github.com/")
        .unwrap_or(resource);
    let resource = resource
        .strip_prefix("http://github.com/")
        .unwrap_or(resource);

    if let Some((repo_part, number_part)) = resource.split_once('#') {
        let (owner, repo) = repo_part
            .split_once('/')
            .ok_or_else(|| anyhow!("invalid github resource id: {resource}"))?;
        return Ok(GitHubResource {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number: number_part
                .parse()
                .with_context(|| format!("parse github number from {resource}"))?,
        });
    }

    let parts: Vec<&str> = resource
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() >= 4 {
        let owner = parts[0];
        let repo = parts[1];
        let number = parts[3]
            .parse()
            .with_context(|| format!("parse github number from {resource}"))?;
        return Ok(GitHubResource {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        });
    }

    Err(anyhow!("invalid github resource id: {resource}"))
}

async fn collect_github_feedback(
    episode: &Episode,
    action: &ExternalAction,
) -> Result<Option<FeedbackObservation>> {
    let token = match std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GH_TOKEN")) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            warn!(resource = %action.resource_id, "github token not configured; skipping feedback poll");
            return Ok(None);
        }
    };

    let github = github_client(&token)?;
    let resource = parse_github_resource(action)?;
    match action.action_type.as_str() {
        "review_pr" => {
            let pr_path = format!(
                "/repos/{}/{}/pulls/{}",
                resource.owner, resource.repo, resource.number
            );
            let pr = github_get_json(&github, &pr_path).await?;
            let state = pr.get("state").and_then(Value::as_str).unwrap_or("unknown");
            let merged = pr.get("merged_at").and_then(Value::as_str).is_some();

            let reviews_path = format!(
                "/repos/{}/{}/pulls/{}/reviews",
                resource.owner, resource.repo, resource.number
            );
            let reviews = github_get_json_page(&github, &reviews_path).await?;
            let review_states: Vec<String> = reviews
                .iter()
                .filter_map(|review| {
                    review
                        .get("state")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect();
            let dismissed = review_states
                .iter()
                .any(|review_state| review_state == "DISMISSED");

            if state == "open" && !merged && !dismissed {
                return Ok(None);
            }

            let sentiment = if merged {
                1.0
            } else if dismissed {
                -0.5
            } else {
                0.0
            };

            Ok(Some(FeedbackObservation {
                success: merged || state == "closed",
                sentiment,
                payload: json!({
                    "resource": {
                        "owner": resource.owner,
                        "repo": resource.repo,
                        "number": resource.number,
                    },
                    "episode_id": episode.episode_id,
                    "pr_state": state,
                    "merged": merged,
                    "dismissed": dismissed,
                    "sentiment": sentiment,
                    "review_states": review_states,
                    "review_count": reviews.len(),
                }),
            }))
        }
        "comment_issue" | "comment_pr" => {
            let comment_id = match action.metadata.get("comment_id").and_then(Value::as_u64) {
                Some(comment_id) => comment_id,
                None => {
                    warn!(resource = %action.resource_id, "github comment id missing; skipping feedback poll");
                    return Ok(None);
                }
            };

            let comment_path = format!(
                "/repos/{}/{}/issues/comments/{}",
                resource.owner, resource.repo, comment_id
            );
            let comment = github_get_json(&github, &comment_path).await?;
            let comment_created_at = comment
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                .map(|ts| ts.with_timezone(&Utc));

            let reactions_path = format!(
                "/repos/{}/{}/issues/comments/{comment_id}/reactions",
                resource.owner, resource.repo
            );
            let reactions = github_get_json_page(&github, &reactions_path).await?;
            let (positive_reactions, negative_reactions, sentiment) =
                reaction_sentiment(&reactions);

            let replies_path = format!(
                "/repos/{}/{}/issues/{}/comments",
                resource.owner, resource.repo, resource.number
            );
            let comments = github_get_json_page(&github, &replies_path).await?;
            let reply_count = comments
                .iter()
                .filter(|reply| {
                    let reply_created_at = reply
                        .get("created_at")
                        .and_then(Value::as_str)
                        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                        .map(|ts| ts.with_timezone(&Utc));

                    match (comment_created_at.as_ref(), reply_created_at.as_ref()) {
                        (Some(comment_created_at), Some(reply_created_at)) => {
                            reply_created_at > comment_created_at
                        }
                        _ => false,
                    }
                })
                .count();

            if positive_reactions == 0 && negative_reactions == 0 && reply_count == 0 {
                return Ok(None);
            }

            Ok(Some(FeedbackObservation {
                success: sentiment > 0.0 || reply_count > 0,
                sentiment,
                payload: json!({
                    "resource": {
                        "owner": resource.owner,
                        "repo": resource.repo,
                        "number": resource.number,
                    },
                    "episode_id": episode.episode_id,
                    "comment_id": comment_id,
                    "reply_count": reply_count,
                    "positive_reactions": positive_reactions,
                    "negative_reactions": negative_reactions,
                    "sentiment": sentiment,
                }),
            }))
        }
        "create_issue" => {
            let issue_path = format!(
                "/repos/{}/{}/issues/{}",
                resource.owner, resource.repo, resource.number
            );
            let issue = github_get_json(&github, &issue_path).await?;
            let issue_state = issue
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let assignees = value_as_strings(issue.get("assignees"));
            let labels = value_as_strings(issue.get("labels"));
            let initial_labels = value_as_strings(action.metadata.get("labels"));
            let labels_changed = issue_labels_changed(&labels, &initial_labels);
            let assigned = !assignees.is_empty();
            let closed = issue_state == "closed";

            if !closed && !labels_changed && !assigned {
                return Ok(None);
            }

            let sentiment = if closed {
                1.0
            } else if labels_changed || assigned {
                0.5
            } else {
                0.0
            };

            Ok(Some(FeedbackObservation {
                success: closed || labels_changed || assigned,
                sentiment,
                payload: json!({
                    "resource": {
                        "owner": resource.owner,
                        "repo": resource.repo,
                        "number": resource.number,
                    },
                    "episode_id": episode.episode_id,
                    "issue_state": issue_state,
                    "assignees": assignees,
                    "labels": labels,
                    "labels_changed": labels_changed,
                    "assigned": assigned,
                    "sentiment": sentiment,
                }),
            }))
        }
        _ => Ok(None),
    }
}

async fn github_get_json(client: &Octocrab, route: &str) -> Result<Value> {
    client
        .get::<Value, _, _>(route, None::<&()>)
        .await
        .with_context(|| format!("GET {route}"))
}

async fn github_get_json_page(client: &Octocrab, route: &str) -> Result<Vec<Value>> {
    let params = json!({ "per_page": 100 });
    let page: Page<Value> = client
        .get::<Page<Value>, _, _>(route, Some(&params))
        .await
        .with_context(|| format!("GET {route}"))?;
    client
        .all_pages(page)
        .await
        .with_context(|| format!("page through {route}"))
}

fn parse_slack_resource(action: &ExternalAction) -> Result<(String, String)> {
    if let (Some(channel), Some(ts)) = (
        action.metadata.get("channel").and_then(Value::as_str),
        action.metadata.get("ts").and_then(Value::as_str),
    ) {
        return Ok((channel.to_string(), ts.to_string()));
    }

    let resource = action.resource_id.trim();
    let (channel, ts) = resource
        .split_once(':')
        .ok_or_else(|| anyhow!("invalid slack resource id: {resource}"))?;
    Ok((channel.to_string(), ts.to_string()))
}

fn slack_reaction_name(reaction: &Value) -> Option<&str> {
    reaction
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| reaction.get("content").and_then(Value::as_str))
}

fn is_positive_slack_reaction(name: &str) -> bool {
    matches!(
        name,
        "+1" | "thumbsup"
            | "white_check_mark"
            | "tada"
            | "heart"
            | "fire"
            | "rocket"
            | "👍"
            | "✅"
            | "🎉"
            | "❤️"
            | "🔥"
            | "🚀"
    )
}

fn is_negative_slack_reaction(name: &str) -> bool {
    matches!(
        name,
        "-1" | "thumbsdown" | "x" | "no_entry" | "👎" | "❌" | "🚫"
    )
}

fn slack_reaction_sentiment(reactions: &[Value]) -> (u32, u32, f64) {
    let positive = reactions
        .iter()
        .filter(|reaction| slack_reaction_name(reaction).is_some_and(is_positive_slack_reaction))
        .map(|reaction| reaction.get("count").and_then(Value::as_u64).unwrap_or(1) as u32)
        .sum();
    let negative = reactions
        .iter()
        .filter(|reaction| slack_reaction_name(reaction).is_some_and(is_negative_slack_reaction))
        .map(|reaction| reaction.get("count").and_then(Value::as_u64).unwrap_or(1) as u32)
        .sum();
    let total = positive + negative;
    let sentiment = if total == 0 {
        0.0
    } else {
        (positive as f64 - negative as f64) / total as f64
    };

    (positive, negative, sentiment)
}

fn slack_replier_id(message: &Value) -> Option<String> {
    message
        .get("user")
        .and_then(Value::as_str)
        .or_else(|| message.get("bot_id").and_then(Value::as_str))
        .or_else(|| message.get("username").and_then(Value::as_str))
        .map(str::to_string)
}

fn slack_unique_replier_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .skip(1)
        .filter_map(slack_replier_id)
        .collect::<HashSet<_>>()
        .len()
}

async fn collect_slack_feedback(
    client: &Client,
    episode: &Episode,
    action: &ExternalAction,
) -> Result<Option<FeedbackObservation>> {
    let token = match std::env::var("SLACK_BOT_TOKEN").or_else(|_| std::env::var("SLACK_TOKEN")) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            warn!(resource = %action.resource_id, "slack token not configured; skipping feedback poll");
            return Ok(None);
        }
    };

    let (channel, ts) = parse_slack_resource(action)?;

    match action.action_type.as_str() {
        "post_message" | "reply_thread" => {
            let reactions_url = "https://slack.com/api/reactions.get";
            let reactions_resp = client
                .get(reactions_url)
                .bearer_auth(&token)
                .query(&[
                    ("channel", channel.as_str()),
                    ("timestamp", ts.as_str()),
                    ("full", "true"),
                ])
                .send()
                .await
                .with_context(|| format!("GET {reactions_url}"))?
                .error_for_status()
                .with_context(|| format!("GET {reactions_url} returned error"))?
                .json::<Value>()
                .await
                .context("decode reactions.get response")?;

            let reactions: Vec<(String, u32)> = reactions_resp
                .pointer("/message/reactions")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let name = item.get("name")?.as_str()?.to_string();
                            let count = item.get("count")?.as_u64()? as u32;
                            Some((name, count))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let reaction_values = reactions_resp
                .pointer("/message/reactions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            let replies_url = "https://slack.com/api/conversations.replies";
            let replies_resp = client
                .get(replies_url)
                .bearer_auth(&token)
                .query(&[
                    ("channel", channel.as_str()),
                    ("ts", ts.as_str()),
                    ("limit", "100"),
                ])
                .send()
                .await
                .with_context(|| format!("GET {replies_url}"))?
                .error_for_status()
                .with_context(|| format!("GET {replies_url} returned error"))?
                .json::<Value>()
                .await
                .context("decode conversations.replies response")?;

            let messages = replies_resp
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let unique_repliers = slack_unique_replier_count(&messages);

            if reactions.is_empty() && unique_repliers == 0 {
                return Ok(None);
            }

            let (positive_reactions, negative_reactions, sentiment) =
                slack_reaction_sentiment(&reaction_values);

            Ok(Some(FeedbackObservation {
                success: unique_repliers > 0 || positive_reactions > negative_reactions,
                sentiment,
                payload: json!({
                    "resource": {
                        "channel": channel,
                        "ts": ts,
                    },
                    "episode_id": episode.episode_id,
                    "reply_count": unique_repliers,
                    "unique_repliers": unique_repliers,
                    "reactions": reactions.iter().map(|(name, count)| json!({
                        "name": name,
                        "count": count,
                    })).collect::<Vec<_>>(),
                    "positive_reactions": positive_reactions,
                    "negative_reactions": negative_reactions,
                    "sentiment": sentiment,
                }),
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_learn::cascade_router::CascadeRouter;
    use roko_learn::prompt_experiment::{ExperimentStore, PromptExperiment, PromptVariant};
    use serde_json::json;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn action_key_is_stable() {
        let episode = Episode::new("agent", "task");
        let action = ExternalAction {
            service: "github".into(),
            action_type: "review_pr".into(),
            resource_id: "owner/repo#12".into(),
            metadata: json!({}),
            performed_at: Utc::now(),
        };

        let key = action_key(&episode, &action);
        assert!(key.contains("github"));
        assert!(key.contains("review_pr"));
    }

    #[test]
    fn schedules_feedback_polls_by_age() {
        let now = Utc::now();

        let fresh_action = ExternalAction {
            service: "github".into(),
            action_type: "review_pr".into(),
            resource_id: "owner/repo#12".into(),
            metadata: json!({}),
            performed_at: now - chrono::Duration::minutes(30),
        };
        assert!(should_poll_action(&fresh_action, now, None));
        assert!(!should_poll_action(
            &fresh_action,
            now,
            Some(now - chrono::Duration::minutes(14))
        ));
        assert!(should_poll_action(
            &fresh_action,
            now,
            Some(now - chrono::Duration::minutes(15))
        ));

        let older_action = ExternalAction {
            service: "slack".into(),
            action_type: "post_message".into(),
            resource_id: "C12345:1712345678.123456".into(),
            metadata: json!({}),
            performed_at: now - chrono::Duration::days(3),
        };
        assert!(should_poll_action(&older_action, now, None));
        assert!(!should_poll_action(
            &older_action,
            now,
            Some(now - chrono::Duration::hours(5))
        ));
        assert!(should_poll_action(
            &older_action,
            now,
            Some(now - chrono::Duration::hours(6))
        ));

        let expired_action = ExternalAction {
            service: "github".into(),
            action_type: "create_issue".into(),
            resource_id: "owner/repo#13".into(),
            metadata: json!({}),
            performed_at: now - chrono::Duration::days(8),
        };
        assert!(!should_poll_action(&expired_action, now, None));
    }

    #[test]
    fn parses_slack_resource_from_plain_id() {
        let action = ExternalAction {
            service: "slack".into(),
            action_type: "post_message".into(),
            resource_id: "C12345:1712345678.123456".into(),
            metadata: json!({}),
            performed_at: Utc::now(),
        };

        let (channel, ts) = parse_slack_resource(&action).expect("parse slack resource");
        assert_eq!(channel, "C12345");
        assert_eq!(ts, "1712345678.123456");
    }

    #[test]
    fn classifies_slack_reactions_by_sentiment() {
        let reactions = vec![
            json!({"name": "thumbsup", "count": 2}),
            json!({"name": "white_check_mark", "count": 1}),
            json!({"name": "thumbsdown", "count": 3}),
        ];

        let (positive, negative, sentiment) = slack_reaction_sentiment(&reactions);
        assert_eq!(positive, 3);
        assert_eq!(negative, 3);
        assert_eq!(sentiment, 0.0);
    }

    #[test]
    fn counts_unique_slack_repliers() {
        let messages = vec![
            json!({"user": "Uroot"}),
            json!({"user": "U1"}),
            json!({"user": "U2"}),
            json!({"user": "U1"}),
            json!({"bot_id": "B1"}),
        ];

        assert_eq!(slack_unique_replier_count(&messages), 3);
    }

    #[test]
    fn positive_feedback_updates_cascade_router_for_used_model() {
        let root = std::env::temp_dir().join(format!("roko-feedback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create temp root");

        let path = root.join(".roko/learn/cascade-router.json");
        let router = CascadeRouter::new(vec![
            "claude-haiku-3-5".to_string(),
            "claude-sonnet-4-5".to_string(),
        ]);
        router.save(&path).expect("seed router state");

        let updated = record_cascade_router_observation(
            &root,
            vec!["claude-sonnet-4-5".to_string()],
            "claude-sonnet-4-5",
            true,
        )
        .expect("record positive feedback");
        assert!(updated);

        let reloaded = CascadeRouter::load_or_new(
            &path,
            vec![
                "claude-haiku-3-5".to_string(),
                "claude-sonnet-4-5".to_string(),
            ],
        );
        assert_eq!(
            reloaded.confidence_snapshot().get("claude-sonnet-4-5"),
            Some(&(1, 1))
        );
    }

    #[test]
    fn experiment_metric_records_against_named_experiment() {
        let root = std::env::temp_dir().join(format!("roko-feedback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create temp root");

        let path = root.join(".roko/learn/experiments.json");
        let mut store = ExperimentStore::new();
        store.register(PromptExperiment::new(
            "template-experiment",
            "template-section",
            vec![PromptVariant {
                id: "variant-a".into(),
                name: "Variant A".into(),
                section_name: "template-section".into(),
                content: "Use the short form.".into(),
                active: true,
            }],
        ));
        store.save(&path).expect("seed experiment store");

        record_experiment_metric(&root, "template-experiment", "variant-a", 0.5)
            .expect("record experiment metric");

        let saved = std::fs::read_to_string(&path).expect("read experiment store");
        let json: serde_json::Value = serde_json::from_str(&saved).expect("parse experiment store");
        let metric =
            json["experiments"]["template-experiment"]["metric_stats"]["variant-a"]["samples"]
                .as_u64()
                .expect("metric samples");
        assert_eq!(metric, 1);
    }

    #[test]
    fn experiment_outcome_updates_variant_trials_and_successes() {
        let root = std::env::temp_dir().join(format!("roko-feedback-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create temp root");

        let path = root.join(".roko/learn/experiments.json");
        let mut store = ExperimentStore::new();
        store.register(PromptExperiment::new(
            "outcome-experiment",
            "outcome-section",
            vec![
                PromptVariant {
                    id: "variant-a".into(),
                    name: "Variant A".into(),
                    section_name: "outcome-section".into(),
                    content: "Be concise.".into(),
                    active: true,
                },
                PromptVariant {
                    id: "variant-b".into(),
                    name: "Variant B".into(),
                    section_name: "outcome-section".into(),
                    content: "Be thorough.".into(),
                    active: true,
                },
            ],
        ));
        store.save(&path).expect("seed experiment store");

        record_experiment_outcome(&root, "variant-a", true).expect("record success outcome");
        record_experiment_outcome(&root, "variant-a", false).expect("record failure outcome");
        record_experiment_outcome(&root, "variant-b", true).expect("record success outcome b");

        let reloaded = ExperimentStore::load_or_new(&path);
        let experiment = reloaded
            .get("outcome-experiment")
            .expect("experiment exists");
        let stats_a = experiment.stats.get("variant-a").expect("variant-a stats");
        assert_eq!(stats_a.trials, 2);
        assert_eq!(stats_a.successes, 1);

        let stats_b = experiment.stats.get("variant-b").expect("variant-b stats");
        assert_eq!(stats_b.trials, 1);
        assert_eq!(stats_b.successes, 1);
    }

    #[test]
    fn feedback_signal_kind_uses_colon_separated_namespace() {
        let kind = feedback_signal_kind("github", "review_pr");
        assert_eq!(kind.as_str(), "feedback:github:review_pr");
    }
}

//! Background feedback collection for external actions recorded in episodes.
//!
//! The loop scans recent episodes, finds external actions performed in the
//! last 24 hours, polls the corresponding external services for outcomes, and
//! persists the collected feedback as normal signals.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Utc};
use reqwest::Client;
use roko_core::tool::ExternalAction;
use roko_core::{Body, Kind, Provenance, Signal};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::{Instant as TokioInstant, interval_at};
use tracing::{info, warn};

use crate::events::ServerEvent;
use crate::state::AppState;

const FEEDBACK_INTERVAL: Duration = Duration::from_secs(15 * 60);
const RECENT_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
const FEEDBACK_SOURCE: &str = "roko-serve:feedback";

/// Start the feedback collection loop as a detached background task.
#[must_use]
pub fn start_feedback_loop(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = interval_at(TokioInstant::now() + FEEDBACK_INTERVAL, FEEDBACK_INTERVAL);
        let mut completed_actions = HashSet::new();

        loop {
            interval.tick().await;

            if state.cancel.is_cancelled() {
                break;
            }

            if let Err(err) = collect_feedback_cycle(&state, &mut completed_actions).await {
                warn!(error = %err, "feedback collection failed");
            }
        }
    })
}

async fn collect_feedback_cycle(
    state: &AppState,
    completed_actions: &mut HashSet<String>,
) -> Result<()> {
    let episodes = load_recent_episodes(state).await?;
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

            let feedback = match action.service.as_str() {
                "github" => collect_github_feedback(&client, &episode, &action).await?,
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
    let cutoff = Utc::now() - chrono::Duration::from_std(RECENT_WINDOW).unwrap_or_else(|_| chrono::Duration::hours(24));

    Ok(episodes
        .into_iter()
        .filter(|episode| episode.timestamp >= cutoff && !episode.external_actions.is_empty())
        .collect())
}

fn parse_external_action(value: Value) -> Option<ExternalAction> {
    serde_json::from_value(value).ok()
}

fn within_recent_window(performed_at: &DateTime<Utc>) -> bool {
    let cutoff = Utc::now() - chrono::Duration::from_std(RECENT_WINDOW).unwrap_or_else(|_| chrono::Duration::hours(24));
    *performed_at >= cutoff
}

fn action_key(episode: &Episode, action: &ExternalAction) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        episode.episode_id,
        action.service,
        action.action_type,
        action.resource_id,
        action.performed_at.timestamp_nanos_opt().unwrap_or_default()
    )
}

#[derive(Debug, Clone)]
struct FeedbackObservation {
    success: bool,
    payload: Value,
}

async fn persist_feedback_result(
    state: &AppState,
    episode: &Episode,
    action: &ExternalAction,
    observation: FeedbackObservation,
) -> Result<()> {
    let feedback_kind = Kind::Custom(format!("feedback.{}.{}", action.service, action.action_type));
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
    let resource = resource.strip_prefix("https://github.com/").unwrap_or(resource);
    let resource = resource.strip_prefix("http://github.com/").unwrap_or(resource);

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

    let parts: Vec<&str> = resource.split('/').filter(|part| !part.is_empty()).collect();
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
    client: &Client,
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

    let resource = parse_github_resource(action)?;
    match action.action_type.as_str() {
        "review_pr" | "merge_pr" | "create_pr" => {
            let pr_url = format!(
                "https://api.github.com/repos/{}/{}/pulls/{}",
                resource.owner, resource.repo, resource.number
            );
            let pr = github_get_json(client, &token, &pr_url).await?;
            let merged_at = pr.get("merged_at").and_then(Value::as_str);
            let state = pr
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            if merged_at.is_none() && state == "open" {
                return Ok(None);
            }

            let reviews_url = format!(
                "https://api.github.com/repos/{}/{}/pulls/{}/reviews",
                resource.owner, resource.repo, resource.number
            );
            let reviews = github_get_json_array(client, &token, &reviews_url).await?;
            let review_states: Vec<String> = reviews
                .iter()
                .filter_map(|review| review.get("state").and_then(Value::as_str).map(str::to_string))
                .collect();

            let success = merged_at.is_some() || review_states.iter().any(|state| state == "APPROVED");
            Ok(Some(FeedbackObservation {
                success,
                payload: json!({
                    "resource": {
                        "owner": resource.owner,
                        "repo": resource.repo,
                        "number": resource.number,
                    },
                    "episode_id": episode.episode_id,
                    "pr_state": state,
                    "merged": merged_at.is_some(),
                    "review_states": review_states,
                    "review_count": reviews.len(),
                }),
            }))
        }
        "comment_issue" | "comment_pr" => {
            let issue_url = format!(
                "https://api.github.com/repos/{}/{}/issues/{}",
                resource.owner, resource.repo, resource.number
            );
            let issue = github_get_json(client, &token, &issue_url).await?;
            let issue_state = issue
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown");

            let comment_id = action.metadata.get("comment_id").and_then(Value::as_u64);
            let reactions = if let Some(comment_id) = comment_id {
                let reactions_url = format!(
                    "https://api.github.com/repos/{}/{}/issues/comments/{comment_id}/reactions",
                    resource.owner, resource.repo
                );
                github_get_json_array(client, &token, &reactions_url).await?
            } else {
                Vec::new()
            };
            let reaction_names: Vec<String> = reactions
                .iter()
                .filter_map(|reaction| reaction.get("content").and_then(Value::as_str).map(str::to_string))
                .collect();

            let replies_url = format!(
                "https://api.github.com/repos/{}/{}/issues/{}/comments",
                resource.owner, resource.repo, resource.number
            );
            let comments = github_get_json_array(client, &token, &replies_url).await?;

            if issue_state == "open" && reaction_names.is_empty() && comments.len() <= 1 {
                return Ok(None);
            }

            Ok(Some(FeedbackObservation {
                success: issue_state == "closed" || !reaction_names.is_empty() || comments.len() > 1,
                payload: json!({
                    "resource": {
                        "owner": resource.owner,
                        "repo": resource.repo,
                        "number": resource.number,
                    },
                    "episode_id": episode.episode_id,
                    "issue_state": issue_state,
                    "reaction_names": reaction_names,
                    "comment_count": comments.len(),
                    "comment_id": comment_id,
                }),
            }))
        }
        "create_issue" => {
            let issue_url = format!(
                "https://api.github.com/repos/{}/{}/issues/{}",
                resource.owner, resource.repo, resource.number
            );
            let issue = github_get_json(client, &token, &issue_url).await?;
            let issue_state = issue
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let assignees = issue
                .get("assignees")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("login").and_then(Value::as_str).map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let labels = issue
                .get("labels")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            Ok(Some(FeedbackObservation {
                success: issue_state == "closed" || !assignees.is_empty() || !labels.is_empty(),
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
                }),
            }))
        }
        _ => Ok(None),
    }
}

async fn github_get_json(client: &Client, token: &str, url: &str) -> Result<Value> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("GET {url} returned error"))?;

    response
        .json::<Value>()
        .await
        .with_context(|| format!("decode github response from {url}"))
}

async fn github_get_json_array(client: &Client, token: &str, url: &str) -> Result<Vec<Value>> {
    let response = github_get_json(client, token, url).await?;
    Ok(response.as_array().cloned().unwrap_or_default())
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
                .query(&[("channel", channel.as_str()), ("timestamp", ts.as_str()), ("full", "true")])
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

            let replies_url = "https://slack.com/api/conversations.replies";
            let replies_resp = client
                .get(replies_url)
                .bearer_auth(&token)
                .query(&[("channel", channel.as_str()), ("ts", ts.as_str()), ("limit", "100")])
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
            let reply_count = messages.len().saturating_sub(1);

            if reactions.is_empty() && reply_count == 0 {
                return Ok(None);
            }

            let positive = ["+1", "thumbsup", "white_check_mark", "tada", "heart", "fire", "rocket"];
            let negative = ["-1", "thumbsdown", "x", "no_entry"];
            let pos: u32 = reactions
                .iter()
                .filter(|(name, _)| positive.contains(&name.as_str()))
                .map(|(_, count)| *count)
                .sum();
            let neg: u32 = reactions
                .iter()
                .filter(|(name, _)| negative.contains(&name.as_str()))
                .map(|(_, count)| *count)
                .sum();
            let total = pos + neg;
            let sentiment = if total == 0 {
                0.0
            } else {
                (pos as f64 - neg as f64) / total as f64
            };

            Ok(Some(FeedbackObservation {
                success: reply_count > 0 || pos > neg,
                payload: json!({
                    "resource": {
                        "channel": channel,
                        "ts": ts,
                    },
                    "episode_id": episode.episode_id,
                    "reply_count": reply_count,
                    "reactions": reactions.iter().map(|(name, count)| json!({
                        "name": name,
                        "count": count,
                    })).collect::<Vec<_>>(),
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
    use serde_json::json;

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
}

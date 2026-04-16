//! Prediction session and claim tracking for the dashboard backend.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type SessionId = String;
pub type ClaimId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Dispatching,
    Collecting,
    Registered,
    Pending,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    Registered,
    Pending,
    Resolved,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionSession {
    pub id: SessionId,
    pub question: String,
    pub creator: String,
    pub staked_points: u64,
    pub target_block: u64,
    pub category: String,
    pub context: String,
    pub metric: String,
    pub state: SessionState,
    pub claims: Vec<ClaimId>,
    pub consensus_value: Option<f64>,
    pub consensus_confidence: Option<f64>,
    pub outcome: Option<f64>,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionClaim {
    pub id: ClaimId,
    pub agent_id: String,
    pub session_id: SessionId,
    pub predicted_value: f64,
    pub interval_width: f64,
    pub confidence: f64,
    #[serde(default)]
    pub entries_in_context: Vec<String>,
    pub registered_block: u64,
    pub state: ClaimState,
    pub actual_value: Option<f64>,
    pub residual: Option<f64>,
    pub covered: Option<bool>,
    pub difficulty_weight: f64,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveResult {
    pub session_id: SessionId,
    pub claim_count: usize,
    pub consensus_value: f64,
    pub consensus_confidence: f64,
    pub mean_residual: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationSummary {
    pub agent_id: String,
    #[serde(default)]
    pub categories: Vec<CalibrationCategorySummary>,
    pub total_claims: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationCategorySummary {
    pub category: String,
    pub mean_bias: f64,
    pub coverage_rate: f64,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PredictionEvent {
    SessionCreated {
        session_id: String,
        question: String,
    },
    ClaimSubmitted {
        session_id: String,
        agent_id: String,
        confidence: f64,
    },
    SessionRegistered {
        session_id: String,
        claim_count: usize,
    },
    SessionResolved {
        session_id: String,
        consensus_residual: f64,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PredictionError {
    #[error("prediction session not found: {0}")]
    SessionNotFound(SessionId),
    #[error("prediction claim not found: {0}")]
    ClaimNotFound(ClaimId),
    #[error("prediction session {id} is in state {current:?}, expected {expected:?}")]
    InvalidSessionState {
        id: SessionId,
        current: SessionState,
        expected: SessionState,
    },
    #[error("agent '{agent_id}' already has a claim for session {session_id}")]
    DuplicateClaim {
        session_id: SessionId,
        agent_id: String,
    },
    #[error("{0}")]
    Validation(String),
}

#[derive(Debug, Default)]
pub struct PredictionStore {
    sessions: HashMap<SessionId, PredictionSession>,
    claims: HashMap<ClaimId, PredictionClaim>,
    next_session_id: u64,
    next_claim_id: u64,
}

impl PredictionStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    #[must_use]
    pub fn claim_count(&self) -> usize {
        self.claims.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &mut self,
        question: String,
        creator: String,
        staked_points: u64,
        target_block: u64,
        category: String,
        context: String,
        metric: String,
        now: u64,
    ) -> SessionId {
        let id = next_hex_id(&mut self.next_session_id);
        let session = PredictionSession {
            id: id.clone(),
            question,
            creator,
            staked_points,
            target_block,
            category,
            context,
            metric,
            state: SessionState::Dispatching,
            claims: Vec::new(),
            consensus_value: None,
            consensus_confidence: None,
            outcome: None,
            created_at: now,
            resolved_at: None,
        };
        self.sessions.insert(id.clone(), session);
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_claim(
        &mut self,
        session_id: &str,
        agent_id: String,
        predicted_value: f64,
        interval_width: f64,
        confidence: f64,
        entries_in_context: Vec<String>,
        block: u64,
        now: u64,
    ) -> Result<ClaimId, PredictionError> {
        if !predicted_value.is_finite() {
            return Err(PredictionError::Validation(
                "predicted_value must be finite".to_string(),
            ));
        }
        if !interval_width.is_finite() || interval_width <= 0.0 {
            return Err(PredictionError::Validation(
                "interval_width must be > 0".to_string(),
            ));
        }
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(PredictionError::Validation(
                "confidence must be between 0.0 and 1.0".to_string(),
            ));
        }

        let category = {
            let session = self
                .sessions
                .get(session_id)
                .ok_or_else(|| PredictionError::SessionNotFound(session_id.to_string()))?;
            if session.state == SessionState::Resolved {
                return Err(PredictionError::InvalidSessionState {
                    id: session_id.to_string(),
                    current: session.state,
                    expected: SessionState::Collecting,
                });
            }
            if session.claims.iter().any(|claim_id| {
                self.claims
                    .get(claim_id)
                    .is_some_and(|claim| claim.agent_id == agent_id)
            }) {
                return Err(PredictionError::DuplicateClaim {
                    session_id: session_id.to_string(),
                    agent_id,
                });
            }
            session.category.clone()
        };

        let sample_count = self
            .claims
            .values()
            .filter(|claim| claim.agent_id == agent_id)
            .filter(|claim| claim.state == ClaimState::Resolved)
            .filter(|claim| {
                self.sessions
                    .get(&claim.session_id)
                    .is_some_and(|session| session.category == category)
            })
            .count() as u64;
        let domain_stddev = self.domain_stddev_for_category(&category);
        let difficulty_weight = compute_difficulty(interval_width, sample_count, domain_stddev);

        let claim_id = next_hex_id(&mut self.next_claim_id);
        let claim = PredictionClaim {
            id: claim_id.clone(),
            agent_id,
            session_id: session_id.to_string(),
            predicted_value,
            interval_width,
            confidence,
            entries_in_context,
            registered_block: block,
            state: ClaimState::Registered,
            actual_value: None,
            residual: None,
            covered: None,
            difficulty_weight,
            created_at: now,
        };
        self.claims.insert(claim_id.clone(), claim);

        if let Some(session) = self.sessions.get_mut(session_id) {
            session.claims.push(claim_id.clone());
            session.state = if session.claims.len() >= 2 {
                SessionState::Registered
            } else {
                SessionState::Collecting
            };
        }

        Ok(claim_id)
    }

    pub fn resolve_session(
        &mut self,
        session_id: &str,
        actual_value: f64,
        now: u64,
    ) -> Result<ResolveResult, PredictionError> {
        if !actual_value.is_finite() {
            return Err(PredictionError::Validation(
                "actual_value must be finite".to_string(),
            ));
        }

        let claim_ids = {
            let session = self
                .sessions
                .get(session_id)
                .ok_or_else(|| PredictionError::SessionNotFound(session_id.to_string()))?;
            if session.state == SessionState::Resolved {
                return Err(PredictionError::InvalidSessionState {
                    id: session_id.to_string(),
                    current: session.state,
                    expected: SessionState::Registered,
                });
            }
            session.claims.clone()
        };

        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;
        let mut confidence_sum = 0.0;
        let mut residual_sum = 0.0;
        let mut resolved_count = 0usize;

        for claim_id in &claim_ids {
            let claim = self
                .claims
                .get_mut(claim_id)
                .ok_or_else(|| PredictionError::ClaimNotFound(claim_id.clone()))?;
            let residual = claim.predicted_value - actual_value;
            let half_width = claim.interval_width / 2.0;
            claim.actual_value = Some(actual_value);
            claim.residual = Some(residual);
            claim.covered = Some(residual.abs() <= half_width);
            claim.state = ClaimState::Resolved;
            weighted_sum += claim.predicted_value * claim.confidence.max(0.01);
            weight_total += claim.confidence.max(0.01);
            confidence_sum += claim.confidence;
            residual_sum += residual;
            resolved_count += 1;
        }

        let consensus_value = if weight_total > 0.0 {
            weighted_sum / weight_total
        } else {
            actual_value
        };
        let consensus_confidence = if resolved_count > 0 {
            confidence_sum / resolved_count as f64
        } else {
            0.0
        };
        let mean_residual = if resolved_count > 0 {
            residual_sum / resolved_count as f64
        } else {
            0.0
        };

        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| PredictionError::SessionNotFound(session_id.to_string()))?;
        session.state = SessionState::Resolved;
        session.consensus_value = Some(consensus_value);
        session.consensus_confidence = Some(consensus_confidence);
        session.outcome = Some(actual_value);
        session.resolved_at = Some(now);

        Ok(ResolveResult {
            session_id: session_id.to_string(),
            claim_count: resolved_count,
            consensus_value,
            consensus_confidence,
            mean_residual,
        })
    }

    pub fn get_session(&self, id: &str) -> Option<&PredictionSession> {
        self.sessions.get(id)
    }

    pub fn list_sessions(
        &self,
        state_filter: Option<SessionState>,
        creator_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> (Vec<&PredictionSession>, usize) {
        let mut sessions: Vec<&PredictionSession> = self
            .sessions
            .values()
            .filter(|session| state_filter.is_none() || Some(session.state) == state_filter)
            .filter(|session| {
                creator_filter.is_none() || Some(session.creator.as_str()) == creator_filter
            })
            .collect();
        sessions.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let total = sessions.len();
        let items = sessions.into_iter().skip(offset).take(limit).collect();
        (items, total)
    }

    pub fn get_claim(&self, id: &str) -> Option<&PredictionClaim> {
        self.claims.get(id)
    }

    pub fn list_claims(
        &self,
        session_filter: Option<&str>,
        agent_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> (Vec<&PredictionClaim>, usize) {
        let mut claims: Vec<&PredictionClaim> = self
            .claims
            .values()
            .filter(|claim| {
                session_filter.is_none() || Some(claim.session_id.as_str()) == session_filter
            })
            .filter(|claim| agent_filter.is_none() || Some(claim.agent_id.as_str()) == agent_filter)
            .collect();
        claims.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let total = claims.len();
        let items = claims.into_iter().skip(offset).take(limit).collect();
        (items, total)
    }

    #[must_use]
    pub fn calibration_summary(&self, agent_id: &str) -> CalibrationSummary {
        let mut grouped: HashMap<String, Vec<&PredictionClaim>> = HashMap::new();
        for claim in self.claims.values() {
            if claim.agent_id != agent_id || claim.state != ClaimState::Resolved {
                continue;
            }
            let Some(session) = self.sessions.get(&claim.session_id) else {
                continue;
            };
            grouped
                .entry(session.category.clone())
                .or_default()
                .push(claim);
        }

        let mut categories: Vec<CalibrationCategorySummary> = grouped
            .into_iter()
            .map(|(category, claims)| {
                let sample_count = claims.len();
                let mean_bias = if sample_count > 0 {
                    claims
                        .iter()
                        .filter_map(|claim| claim.residual)
                        .sum::<f64>()
                        / sample_count as f64
                } else {
                    0.0
                };
                let coverage_rate = if sample_count > 0 {
                    claims
                        .iter()
                        .filter(|claim| claim.covered == Some(true))
                        .count() as f64
                        / sample_count as f64
                } else {
                    0.0
                };
                CalibrationCategorySummary {
                    category,
                    mean_bias,
                    coverage_rate,
                    sample_count,
                }
            })
            .collect();
        categories.sort_by(|a, b| a.category.cmp(&b.category));
        let total_claims = categories.iter().map(|entry| entry.sample_count).sum();
        CalibrationSummary {
            agent_id: agent_id.to_string(),
            categories,
            total_claims,
        }
    }

    fn domain_stddev_for_category(&self, category: &str) -> f64 {
        let values: Vec<f64> = self
            .claims
            .values()
            .filter_map(|claim| {
                let session = self.sessions.get(&claim.session_id)?;
                if session.category != category || claim.state != ClaimState::Resolved {
                    return None;
                }
                claim.actual_value
            })
            .collect();
        if values.len() < 2 {
            return 0.25;
        }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|value| {
                let delta = value - mean;
                delta * delta
            })
            .sum::<f64>()
            / values.len() as f64;
        variance.sqrt().max(0.05)
    }
}

fn next_hex_id(counter: &mut u64) -> String {
    *counter += 1;
    format!("{counter:032x}")
}

fn compute_difficulty(interval_width: f64, sample_count: u64, domain_stddev: f64) -> f64 {
    let category_variance = domain_stddev.clamp(0.05, 1.0);
    let novelty = (10.0 / sample_count.max(1) as f64).clamp(0.1, 1.0);
    let tightness = (1.0 - interval_width / (3.0 * domain_stddev.max(0.05))).clamp(0.0, 1.0);
    category_variance * novelty * tightness
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_submit_and_resolve_prediction_session() {
        let mut store = PredictionStore::new();
        let session_id = store.create_session(
            "Will ETH finish above 4k?".into(),
            "creator-1".into(),
            50,
            1_000,
            "macro".into(),
            "market context".into(),
            "price".into(),
            10,
        );
        let claim_a = store
            .submit_claim(
                &session_id,
                "agent-a".into(),
                4_100.0,
                300.0,
                0.7,
                vec!["insight-1".into()],
                900,
                20,
            )
            .unwrap();
        let claim_b = store
            .submit_claim(
                &session_id,
                "agent-b".into(),
                3_950.0,
                250.0,
                0.6,
                vec!["insight-2".into()],
                901,
                21,
            )
            .unwrap();

        assert_ne!(claim_a, claim_b);
        assert_eq!(
            store.get_session(&session_id).unwrap().state,
            SessionState::Registered
        );

        let result = store.resolve_session(&session_id, 4_020.0, 30).unwrap();
        assert_eq!(result.claim_count, 2);
        assert_eq!(
            store.get_session(&session_id).unwrap().state,
            SessionState::Resolved
        );
        assert!(store.get_claim(&claim_a).unwrap().residual.is_some());
        assert!(store.get_claim(&claim_b).unwrap().covered.is_some());
    }

    #[test]
    fn duplicate_agent_claim_is_rejected() {
        let mut store = PredictionStore::new();
        let session_id = store.create_session(
            "Will rates fall?".into(),
            "creator-1".into(),
            10,
            10,
            "rates".into(),
            String::new(),
            "percent".into(),
            1,
        );
        store
            .submit_claim(
                &session_id,
                "agent-a".into(),
                1.0,
                0.2,
                0.5,
                Vec::new(),
                1,
                2,
            )
            .unwrap();

        let err = store
            .submit_claim(
                &session_id,
                "agent-a".into(),
                1.1,
                0.2,
                0.6,
                Vec::new(),
                2,
                3,
            )
            .unwrap_err();
        assert!(matches!(err, PredictionError::DuplicateClaim { .. }));
    }

    #[test]
    fn calibration_summary_groups_by_category() {
        let mut store = PredictionStore::new();
        let session_id = store.create_session(
            "Question".into(),
            "creator".into(),
            1,
            1,
            "defi".into(),
            String::new(),
            "score".into(),
            1,
        );
        store
            .submit_claim(
                &session_id,
                "agent-z".into(),
                10.0,
                4.0,
                0.8,
                Vec::new(),
                1,
                2,
            )
            .unwrap();
        store.resolve_session(&session_id, 11.0, 3).unwrap();

        let summary = store.calibration_summary("agent-z");
        assert_eq!(summary.total_claims, 1);
        assert_eq!(summary.categories[0].category, "defi");
    }
}

//! Structured audit object for one agent invocation.
//!
//! The workspace is intentionally provider- and orchestrator-neutral so it can
//! be serialized, attached to event logs, and enriched later by gate, reward,
//! or learning subsystems without scraping prompt logs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::{ContextPolicyRef, GateExpectation, PromptPolicy, RoleProfile, Task};

/// Schema version for [`CognitiveWorkspace`].
pub const COGNITIVE_WORKSPACE_SCHEMA_VERSION: u32 = 1;

/// Audit object for one agent invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CognitiveWorkspace {
    /// Audit schema version.
    pub schema_version: u32,
    /// Stable workspace id for this invocation.
    pub workspace_id: String,
    /// Provider/run invocation id when available.
    pub invocation_id: String,
    /// Task contract the agent was asked to satisfy.
    pub task_contract: TaskInvocationContract,
    /// Selected role profile.
    pub role_profile: PolicyVersionRef,
    /// Selected prompt policy.
    pub prompt_policy: PolicyVersionRef,
    /// Selected context policy, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_policy: Option<ContextPolicyAuditRef>,
    /// Context sections included in the prompt, without raw content.
    #[serde(default)]
    pub included_context_sections: Vec<ContextSectionAudit>,
    /// Context candidates rejected before prompt assembly.
    #[serde(default)]
    pub rejected_context_candidates: Vec<ContextRejectionAudit>,
    /// Prompt sections considered during assembly, without raw content.
    #[serde(default)]
    pub prompt_sections: Vec<PromptSectionAudit>,
    /// Provider/model selection used for the invocation.
    pub model_choice: ModelChoice,
    /// Tools and higher-level capabilities granted to the invocation.
    #[serde(default)]
    pub capability_grants: Vec<CapabilityGrant>,
    /// Structured output parse result, attached once output is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_parse_result: Option<OutputParseResult>,
    /// Gate outcomes associated with this invocation.
    #[serde(default)]
    pub gate_outcomes: Vec<InvocationGateOutcome>,
    /// Structured review verdicts associated with this invocation.
    #[serde(default)]
    pub review_verdicts: Vec<InvocationReviewVerdictOutcome>,
    /// Reward observations derived from gates, latency, cost, retries, or review.
    #[serde(default)]
    pub reward_observations: Vec<RewardObservation>,
    /// Unix millisecond timestamp when the audit object was created.
    pub created_at_ms: i64,
}

impl CognitiveWorkspace {
    /// Create a workspace with empty context, output, gate, and reward sections.
    #[must_use]
    pub fn new(
        workspace_id: impl Into<String>,
        invocation_id: impl Into<String>,
        task_contract: TaskInvocationContract,
        role_profile: PolicyVersionRef,
        prompt_policy: PolicyVersionRef,
        model_choice: ModelChoice,
    ) -> Self {
        Self {
            schema_version: COGNITIVE_WORKSPACE_SCHEMA_VERSION,
            workspace_id: workspace_id.into(),
            invocation_id: invocation_id.into(),
            task_contract,
            role_profile,
            prompt_policy,
            context_policy: None,
            included_context_sections: Vec::new(),
            rejected_context_candidates: Vec::new(),
            prompt_sections: Vec::new(),
            model_choice,
            capability_grants: Vec::new(),
            output_parse_result: None,
            gate_outcomes: Vec::new(),
            review_verdicts: Vec::new(),
            reward_observations: Vec::new(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Attach context policy metadata.
    #[must_use]
    pub fn with_context_policy(mut self, policy: Option<ContextPolicyAuditRef>) -> Self {
        self.context_policy = policy;
        self
    }

    /// Attach included and rejected context audit rows.
    #[must_use]
    pub fn with_context_audit(
        mut self,
        included: Vec<ContextSectionAudit>,
        rejected: Vec<ContextRejectionAudit>,
    ) -> Self {
        self.included_context_sections = included;
        self.rejected_context_candidates = rejected;
        self
    }

    /// Attach prompt-section audit rows.
    #[must_use]
    pub fn with_prompt_section_audit(mut self, sections: Vec<PromptSectionAudit>) -> Self {
        self.prompt_sections = sections;
        self
    }

    /// Attach capability grants.
    #[must_use]
    pub fn with_capability_grants(mut self, grants: Vec<CapabilityGrant>) -> Self {
        self.capability_grants = grants;
        self
    }
}

/// Versioned reference to a selected policy object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVersionRef {
    /// Stable policy/profile identifier.
    pub id: String,
    /// Selected version.
    pub version: String,
}

impl PolicyVersionRef {
    /// Create a policy reference.
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
        }
    }
}

impl From<&RoleProfile> for PolicyVersionRef {
    fn from(value: &RoleProfile) -> Self {
        Self::new(value.role_id.clone(), value.version.clone())
    }
}

impl From<&PromptPolicy> for PolicyVersionRef {
    fn from(value: &PromptPolicy) -> Self {
        Self::new(value.policy_id.clone(), value.version.clone())
    }
}

/// Context policy reference plus bounded dispatch metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicyAuditRef {
    /// Stable context policy id.
    pub id: String,
    /// Optional context budget carried by the policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    /// Context bidders allowed by the policy.
    #[serde(default)]
    pub bidders: Vec<String>,
}

impl From<&ContextPolicyRef> for ContextPolicyAuditRef {
    fn from(value: &ContextPolicyRef) -> Self {
        Self {
            id: value.policy_id.clone(),
            budget_tokens: value.budget_tokens,
            bidders: value.bidders.clone(),
        }
    }
}

/// Compact task contract captured at dispatch time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskInvocationContract {
    /// Plan identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Task identifier.
    pub task_id: String,
    /// Task title or summary.
    pub title: String,
    /// Files the task is expected to touch.
    #[serde(default)]
    pub files: Vec<String>,
    /// Acceptance criteria.
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Role requested by the task contract, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_role: Option<String>,
}

impl TaskInvocationContract {
    /// Build a compact contract from a task plus optional plan id.
    #[must_use]
    pub fn from_task(plan_id: Option<String>, task: &Task) -> Self {
        Self {
            plan_id,
            task_id: task.id.clone(),
            title: task.title.clone(),
            files: task.files.clone(),
            acceptance: task.acceptance.clone(),
            requested_role: task.role.clone(),
        }
    }

    /// Build a compact contract from prompt-level task text.
    #[must_use]
    pub fn from_prompt_context(
        plan_id: Option<String>,
        task_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            plan_id,
            task_id: task_id.into(),
            title: title.into(),
            files: Vec::new(),
            acceptance: Vec::new(),
            requested_role: None,
        }
    }
}

/// Provider/model selection for the invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// Provider id, for example `openai`, `anthropic`, `codex`, or `local`.
    pub provider: String,
    /// Model id/slug used by the provider.
    pub model: String,
    /// Optional routing policy or experiment id that selected this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_policy_id: Option<String>,
}

impl ModelChoice {
    /// Create a provider/model choice.
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            routing_policy_id: None,
        }
    }
}

/// Capability or tool granted for one invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    /// Tool or capability identifier.
    pub id: String,
    /// `tool`, `capability`, `mcp_server`, or another stable grant kind.
    pub kind: String,
    /// Grant source, for example a role profile or task override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Included context section audit row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSectionAudit {
    /// Stable context section id.
    #[serde(default)]
    pub section_id: String,
    /// Included section name.
    pub section_name: String,
    /// Stable action id for future section-level bandits.
    #[serde(default)]
    pub action_id: String,
    /// Stable source type key.
    pub source_type: String,
    /// Stable source identifier when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Section purpose.
    pub purpose: String,
    /// Visibility boundary for the section.
    pub scope: ContextScopeAudit,
    /// Human-readable inclusion reason.
    pub inclusion_reason: String,
    /// Estimated section token count.
    pub estimated_tokens: usize,
    /// Section token budget, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<usize>,
    /// Experiment id that caused this section to be included, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Structured selector or bidder decision metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub decision_metadata: BTreeMap<String, String>,
}

/// Rejected context candidate audit row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRejectionAudit {
    /// Stable context section id.
    #[serde(default)]
    pub section_id: String,
    /// Rejected section name.
    pub section_name: String,
    /// Stable action id for future section-level bandits.
    #[serde(default)]
    pub action_id: String,
    /// Stable source type key.
    pub source_type: String,
    /// Stable source identifier when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Section purpose.
    pub purpose: String,
    /// Visibility boundary for the section.
    pub scope: ContextScopeAudit,
    /// Estimated section token count.
    pub estimated_tokens: usize,
    /// Experiment id that caused this section to be considered, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Structured selector or bidder decision metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub decision_metadata: BTreeMap<String, String>,
    /// Structured rejection reason.
    pub reason: ContextRejectionAuditReason,
}

/// Prompt section audit row emitted by composition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSectionAudit {
    /// Stable prompt section id.
    pub section_id: String,
    /// Human-readable section name.
    pub section_name: String,
    /// Stable action id for future section-level bandits.
    pub action_id: String,
    /// Whether the section was included in the final prompt.
    pub included: bool,
    /// Estimated tokens before final prompt rendering.
    pub estimated_tokens: usize,
    /// Tokens used in the final prompt. Dropped sections use zero.
    pub tokens_used: usize,
    /// Section token budget, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<usize>,
    /// Prompt priority label.
    pub priority: String,
    /// Prompt cache layer label.
    pub cache_layer: String,
    /// Prompt placement label.
    pub placement: String,
    /// Context or subsystem bidder label.
    pub bidder: String,
    /// Stable source type key, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    /// Stable source identifier, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Compact provenance label, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    /// Experiment id that caused this section to be considered, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment_id: Option<String>,
    /// Structured inclusion or rejection reason.
    pub reason: String,
}

/// Serializable context visibility boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextScopeAudit {
    /// Only valid for one task in one plan.
    Task {
        /// Plan identifier.
        plan_id: String,
        /// Task identifier.
        task_id: String,
    },
    /// Valid for any task in one plan.
    Plan {
        /// Plan identifier.
        plan_id: String,
    },
    /// Context from another task.
    CrossTask {
        /// Plan identifier.
        plan_id: String,
        /// Task identifier, when known.
        task_id: Option<String>,
        /// Why crossing the task boundary is allowed.
        reason: String,
    },
    /// Context from another plan.
    CrossPlan {
        /// Why crossing the plan boundary is allowed.
        reason: String,
    },
    /// Globally visible context.
    Global {
        /// Why global visibility is allowed.
        reason: String,
    },
}

/// Serializable context rejection reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextRejectionAuditReason {
    /// Candidate relevance was below the policy floor.
    Irrelevant {
        /// Candidate relevance score.
        relevance_microunits: u32,
        /// Required relevance floor.
        min_relevance_microunits: u32,
    },
    /// Section exceeded the policy or section token ceiling.
    Oversized {
        /// Estimated section tokens.
        estimated_tokens: usize,
        /// Maximum allowed tokens.
        max_tokens: usize,
    },
    /// Cross-task, cross-plan, or global context omitted an explicit reason.
    MissingScopeReason,
    /// Section was dropped to fit the dispatch budget.
    BudgetExceeded {
        /// Dispatch context budget.
        budget_tokens: usize,
    },
}

/// Structured output parse result for an invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OutputParseResult {
    /// Output parsed against the expected schema.
    Parsed {
        /// Schema id that parsed successfully.
        schema_id: String,
        /// Parsed output reference or compact parsed object.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parsed_ref: Option<Value>,
    },
    /// Output was missing, ambiguous, or failed to parse.
    Failed {
        /// Schema id that was expected.
        schema_id: String,
        /// Machine-readable parse error.
        error: String,
    },
}

/// Gate outcome attached to an invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationGateOutcome {
    /// Gate identifier.
    pub gate_id: String,
    /// Outcome state such as `passed`, `failed`, `blocked`, or `needs_replan`.
    pub outcome: String,
    /// Whether this gate blocks completion.
    pub required: bool,
    /// Optional command associated with the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Optional short summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Structured review verdict attached to an invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvocationReviewVerdictOutcome {
    /// Stable verdict id.
    pub verdict_id: String,
    /// Reviewer role id.
    pub reviewer_role_id: String,
    /// Verdict status such as `passed`, `failed`, `needs_retry`, or `needs_replan`.
    pub status: String,
    /// Confidence in `[0.0, 1.0]` when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Blocking finding identifiers or compact summaries.
    #[serde(default)]
    pub blocking_findings: Vec<String>,
    /// Non-blocking finding identifiers or compact summaries.
    #[serde(default)]
    pub non_blocking_findings: Vec<String>,
    /// Required next action.
    pub required_next_action: String,
    /// Evidence paths or refs.
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

impl From<&GateExpectation> for InvocationGateOutcome {
    fn from(value: &GateExpectation) -> Self {
        Self {
            gate_id: value.gate_id.clone(),
            outcome: value.outcome.clone(),
            required: value.required,
            command: value.command.clone(),
            summary: None,
        }
    }
}

/// Reward observation suitable for later policy learning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RewardObservation {
    /// Stable action id being observed.
    pub action_id: String,
    /// Reward metric name.
    pub metric: String,
    /// Numeric reward or cost signal.
    pub value: f64,
    /// Evidence or source of the reward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn workspace_serializes_without_raw_context_content() {
        let workspace = CognitiveWorkspace::new(
            "cw-1",
            "invoke-1",
            TaskInvocationContract::from_prompt_context(Some("P1".into()), "T1", "Fix parser"),
            PolicyVersionRef::new("implementer", "1.0.0"),
            PolicyVersionRef::new("roko.builtin.prompt.implementer", "1.0.0"),
            ModelChoice::new("codex", "gpt-5.5"),
        )
        .with_context_audit(
            vec![ContextSectionAudit {
                section_id: "context:source:src/lib.rs".into(),
                section_name: "source".into(),
                action_id: "context_section:source:file:src/lib.rs".into(),
                source_type: "file".into(),
                source_id: Some("src/lib.rs".into()),
                purpose: "source_evidence".into(),
                scope: ContextScopeAudit::Task {
                    plan_id: "P1".into(),
                    task_id: "T1".into(),
                },
                inclusion_reason: "needed public API".into(),
                estimated_tokens: 42,
                token_budget: Some(128),
                experiment_id: None,
                decision_metadata: BTreeMap::new(),
            }],
            Vec::new(),
        );

        let encoded = serde_json::to_value(&workspace).expect("serialize workspace");
        assert_eq!(
            encoded["schema_version"],
            json!(COGNITIVE_WORKSPACE_SCHEMA_VERSION)
        );
        assert_eq!(
            encoded["included_context_sections"][0]["source_id"],
            "src/lib.rs"
        );
        assert!(!encoded.to_string().contains("secret file contents"));
    }
}

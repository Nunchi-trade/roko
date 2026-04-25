//! Compose-side assembly of [`CognitiveWorkspace`](roko_core::CognitiveWorkspace).
//!
//! This module sits at the prompt/context boundary: it records the selected
//! role and policies plus context decisions, but stores only provenance,
//! budgets, and reasons rather than raw prompt/context text.

use roko_core::{
    CapabilityGrant, CognitiveWorkspace, ContextPolicyAuditRef, ContextRejectionAudit,
    ContextRejectionAuditReason, ContextScopeAudit, ContextSectionAudit, InvocationGateOutcome,
    ModelChoice, PolicyVersionRef, PromptPolicy, RoleProfile, TaskInvocationContract,
};

use crate::{
    ContextPurpose, ContextRejection, ContextRejectionReason, ContextScope, ResolvedContext,
    TaskContext,
};

/// Inputs needed to build a cognitive workspace audit object at dispatch time.
#[derive(Clone, Debug)]
pub struct CognitiveWorkspaceInput<'a> {
    /// Stable workspace id for this invocation.
    pub workspace_id: String,
    /// Provider/run invocation id when available.
    pub invocation_id: String,
    /// Task contract passed to the agent.
    pub task_contract: TaskInvocationContract,
    /// Selected role profile.
    pub role_profile: &'a RoleProfile,
    /// Selected prompt policy.
    pub prompt_policy: &'a PromptPolicy,
    /// Resolved context decisions for this dispatch.
    pub resolved_context: Option<&'a ResolvedContext>,
    /// Provider/model choice.
    pub model_choice: ModelChoice,
}

/// Build a serializable invocation audit object from compose-time decisions.
#[must_use]
pub fn build_cognitive_workspace(input: CognitiveWorkspaceInput<'_>) -> CognitiveWorkspace {
    let (included, rejected) = input.resolved_context.map_or_else(
        || (Vec::new(), Vec::new()),
        |context| {
            (
                included_context_sections(context),
                rejected_context_candidates(&context.rejected_sections),
            )
        },
    );

    let grants = capability_grants(input.role_profile);
    let gates = input
        .role_profile
        .gate_expectations
        .iter()
        .map(InvocationGateOutcome::from)
        .collect();

    let mut workspace = CognitiveWorkspace::new(
        input.workspace_id,
        input.invocation_id,
        input.task_contract,
        PolicyVersionRef::from(input.role_profile),
        PolicyVersionRef::from(input.prompt_policy),
        input.model_choice,
    )
    .with_context_policy(Some(ContextPolicyAuditRef::from(
        &input.role_profile.context_policy,
    )))
    .with_context_audit(included, rejected)
    .with_capability_grants(grants);
    workspace.gate_outcomes = gates;
    workspace
}

/// Build a compact task contract from role prompt context.
#[must_use]
pub fn task_contract_from_prompt_context(
    context: &TaskContext,
    task_id: impl Into<String>,
) -> TaskInvocationContract {
    TaskInvocationContract::from_prompt_context(
        context.plan_id.clone(),
        task_id,
        context.task.clone(),
    )
}

fn included_context_sections(context: &ResolvedContext) -> Vec<ContextSectionAudit> {
    context
        .injection_manifest()
        .into_iter()
        .map(|record| ContextSectionAudit {
            section_name: record.section_name,
            source_type: record.source_type.to_string(),
            source_id: record.source_id,
            purpose: purpose_label(record.purpose).to_string(),
            scope: scope_audit(record.scope),
            inclusion_reason: record.inclusion_reason,
            estimated_tokens: record.estimated_tokens,
            token_budget: record.token_budget,
        })
        .collect()
}

fn rejected_context_candidates(rejections: &[ContextRejection]) -> Vec<ContextRejectionAudit> {
    rejections
        .iter()
        .map(|rejection| ContextRejectionAudit {
            section_name: rejection.section_name.clone(),
            source_type: rejection.source_type.to_string(),
            source_id: rejection.source_id.clone(),
            purpose: purpose_label(rejection.purpose).to_string(),
            scope: scope_audit(rejection.scope.clone()),
            estimated_tokens: rejection.estimated_tokens,
            reason: rejection_reason_audit(&rejection.reason),
        })
        .collect()
}

fn capability_grants(role: &RoleProfile) -> Vec<CapabilityGrant> {
    let source = Some(format!("role_profile:{}", role.role_id));
    let mut grants = Vec::new();
    grants.extend(role.tools.allowed_tools.iter().map(|tool| CapabilityGrant {
        id: tool.clone(),
        kind: "tool".to_string(),
        source: source.clone(),
    }));
    grants.extend(
        role.tools
            .capabilities
            .iter()
            .map(|capability| CapabilityGrant {
                id: capability.clone(),
                kind: "capability".to_string(),
                source: source.clone(),
            }),
    );
    grants
}

fn purpose_label(purpose: ContextPurpose) -> &'static str {
    match purpose {
        ContextPurpose::SourceEvidence => "source_evidence",
        ContextPurpose::TaskGuidance => "task_guidance",
        ContextPurpose::SafetyConstraint => "safety_constraint",
        ContextPurpose::Verification => "verification",
        ContextPurpose::DependencyMemory => "dependency_memory",
        ContextPurpose::PlanOrientation => "plan_orientation",
        ContextPurpose::CrossPlanMemory => "cross_plan_memory",
        ContextPurpose::ResearchEvidence => "research_evidence",
        ContextPurpose::AmbientSignal => "ambient_signal",
    }
}

fn scope_audit(scope: ContextScope) -> ContextScopeAudit {
    match scope {
        ContextScope::Task { plan_id, task_id } => ContextScopeAudit::Task { plan_id, task_id },
        ContextScope::Plan { plan_id } => ContextScopeAudit::Plan { plan_id },
        ContextScope::CrossTask {
            plan_id,
            task_id,
            reason,
        } => ContextScopeAudit::CrossTask {
            plan_id,
            task_id,
            reason,
        },
        ContextScope::CrossPlan { reason } => ContextScopeAudit::CrossPlan { reason },
        ContextScope::Global { reason } => ContextScopeAudit::Global { reason },
    }
}

fn rejection_reason_audit(reason: &ContextRejectionReason) -> ContextRejectionAuditReason {
    match reason {
        ContextRejectionReason::Irrelevant {
            relevance,
            min_relevance,
        } => ContextRejectionAuditReason::Irrelevant {
            relevance_microunits: relevance_to_microunits(*relevance),
            min_relevance_microunits: relevance_to_microunits(*min_relevance),
        },
        ContextRejectionReason::Oversized {
            estimated_tokens,
            max_tokens,
        } => ContextRejectionAuditReason::Oversized {
            estimated_tokens: *estimated_tokens,
            max_tokens: *max_tokens,
        },
        ContextRejectionReason::MissingScopeReason => {
            ContextRejectionAuditReason::MissingScopeReason
        }
        ContextRejectionReason::BudgetExceeded { budget_tokens } => {
            ContextRejectionAuditReason::BudgetExceeded {
                budget_tokens: *budget_tokens,
            }
        }
    }
}

fn relevance_to_microunits(value: f32) -> u32 {
    (value.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roko_core::{AgentRole, ModelChoice};

    use super::*;
    use crate::{
        ContextInjectionPolicy, ContextProvider, ContextRequest, ContextSection, ContextSource,
        PromptSection,
    };

    #[test]
    fn cognitive_workspace_retains_context_provenance_without_raw_content() {
        let provider = ContextProvider::new(PathBuf::from("/tmp/test"));
        let request = ContextRequest {
            tier: crate::ContextTier::Focused,
            budget_tokens: 1_000,
            plan_id: "P1".into(),
            task_id: "T1".into(),
            task_files: vec!["src/lib.rs".into()],
        };
        let context = provider.select_candidates(
            &request,
            vec![crate::ContextCandidate {
                section: ContextSection::scoped(
                    PromptSection::new("source", "secret file contents should not be audited")
                        .with_hard_cap(100),
                    ContextSource::InlineFile {
                        path: "src/lib.rs".into(),
                        lines: Some("1-20".into()),
                    },
                    ContextPurpose::SourceEvidence,
                    ContextScope::task("P1", "T1"),
                    "inspect public API",
                ),
                relevance: 1.0,
                bidder: crate::AttentionBidder::CodeIntelligence,
            }],
            ContextInjectionPolicy::default(),
        );

        let role = crate::builtin_role_profile_for(AgentRole::Implementer);
        let prompt = crate::builtin_prompt_policy_for(AgentRole::Implementer);
        let workspace = build_cognitive_workspace(CognitiveWorkspaceInput {
            workspace_id: "cw-P1-T1".into(),
            invocation_id: "invoke-1".into(),
            task_contract: TaskInvocationContract::from_prompt_context(
                Some("P1".into()),
                "T1",
                "Audit context",
            ),
            role_profile: &role,
            prompt_policy: &prompt,
            resolved_context: Some(&context),
            model_choice: ModelChoice::new("codex", "gpt-5.5"),
        });

        assert_eq!(workspace.included_context_sections.len(), 1);
        let section = &workspace.included_context_sections[0];
        assert_eq!(section.source_type, "file");
        assert_eq!(section.source_id.as_deref(), Some("src/lib.rs:1-20"));
        assert_eq!(section.token_budget, Some(100));
        let encoded = serde_json::to_string(&workspace).expect("workspace serializes");
        assert!(!encoded.contains("secret file contents"));
        assert!(encoded.contains("inspect public API"));
    }

    #[test]
    fn cognitive_workspace_records_rejected_context_reason() {
        let provider = ContextProvider::new(PathBuf::from("/tmp/test"));
        let request = ContextRequest {
            tier: crate::ContextTier::Focused,
            budget_tokens: 1_000,
            plan_id: "P1".into(),
            task_id: "T1".into(),
            task_files: Vec::new(),
        };
        let context = provider.select_candidates(
            &request,
            vec![crate::ContextCandidate {
                section: ContextSection::scoped(
                    PromptSection::new("global", "global note"),
                    ContextSource::ResearchMemo,
                    ContextPurpose::ResearchEvidence,
                    ContextScope::Global {
                        reason: String::new(),
                    },
                    "maybe useful",
                ),
                relevance: 0.9,
                bidder: crate::AttentionBidder::Research,
            }],
            ContextInjectionPolicy::default(),
        );

        let role = crate::builtin_role_profile_for(AgentRole::Architect);
        let prompt = crate::builtin_prompt_policy_for(AgentRole::Architect);
        let workspace = build_cognitive_workspace(CognitiveWorkspaceInput {
            workspace_id: "cw-P1-T1".into(),
            invocation_id: "invoke-1".into(),
            task_contract: TaskInvocationContract::from_prompt_context(
                Some("P1".into()),
                "T1",
                "Audit rejected context",
            ),
            role_profile: &role,
            prompt_policy: &prompt,
            resolved_context: Some(&context),
            model_choice: ModelChoice::new("codex", "gpt-5.5"),
        });

        assert!(workspace.included_context_sections.is_empty());
        assert!(matches!(
            workspace.rejected_context_candidates[0].reason,
            ContextRejectionAuditReason::MissingScopeReason
        ));
    }
}

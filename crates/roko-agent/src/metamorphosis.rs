//! Agent metamorphosis and role switching.

use std::collections::HashMap;

use async_trait::async_trait;
use roko_core::{AgentRole, Body, Context, Engram, Provenance};

use crate::agent::{Agent, AgentResult, derived_output};
use crate::introspection::AgentIdentity;

/// A role-shaping profile that can be updated at runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct RoleProfile {
    /// Role this profile applies to.
    pub role: AgentRole,
    /// How clearly the role should expose its reasoning.
    pub clarity: f32,
    /// How differentiated the role should be from adjacent roles.
    pub differentiation: f32,
    /// How aligned the role is with the current plan.
    pub alignment: f32,
}

impl RoleProfile {
    /// Build a default profile for a role.
    #[must_use]
    pub const fn new(role: AgentRole) -> Self {
        Self {
            role,
            clarity: 0.5,
            differentiation: 0.5,
            alignment: 0.5,
        }
    }
}

/// Error returned when a morph is not allowed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MorphError {
    /// The role transition is not present in the allowlist.
    #[error("role transition {from} -> {to} is not allowed")]
    TransitionDenied { from: AgentRole, to: AgentRole },
}

/// An [`Agent`] wrapper that can change roles during a run.
pub struct MorphableAgent {
    inner: Box<dyn Agent>,
    identity: AgentIdentity,
    profile: RoleProfile,
    allowed_transitions: HashMap<AgentRole, Vec<AgentRole>>,
    system_prompt: String,
    name: String,
}

impl std::fmt::Debug for MorphableAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MorphableAgent")
            .field("identity", &self.identity)
            .field("profile", &self.profile)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl MorphableAgent {
    /// Create a new morphable wrapper.
    #[must_use]
    pub fn new(inner: Box<dyn Agent>, identity: AgentIdentity) -> Self {
        let profile = RoleProfile::new(identity.role);
        let system_prompt = system_prompt_for(identity.role, &profile);
        let name = format!("{}[{}]", inner.name(), identity.role.label());
        Self {
            inner,
            identity,
            profile,
            allowed_transitions: default_transition_matrix(),
            system_prompt,
            name,
        }
    }

    /// Override the allowlist of transitions.
    #[must_use]
    pub fn with_transitions(
        mut self,
        allowed_transitions: HashMap<AgentRole, Vec<AgentRole>>,
    ) -> Self {
        self.allowed_transitions = allowed_transitions;
        self
    }

    /// Current role.
    #[must_use]
    pub const fn role(&self) -> AgentRole {
        self.identity.role
    }

    /// Current identity snapshot.
    #[must_use]
    pub const fn identity(&self) -> &AgentIdentity {
        &self.identity
    }

    /// Current role profile.
    #[must_use]
    pub const fn profile(&self) -> &RoleProfile {
        &self.profile
    }

    /// The current system-prompt augmentation used for morphing.
    #[must_use]
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    /// Attempt to morph into a new role.
    pub fn morph(&mut self, new_role: AgentRole) -> Result<(), MorphError> {
        if !self.can_morph_to(new_role) {
            return Err(MorphError::TransitionDenied {
                from: self.identity.role,
                to: new_role,
            });
        }

        self.identity.role = new_role;
        self.identity.model_tier = new_role.model_tier();
        self.identity.capabilities = new_role.tool_permissions();
        self.profile.role = new_role;
        self.system_prompt = system_prompt_for(new_role, &self.profile);
        self.name = format!("{}[{}]", self.inner.name(), new_role.label());
        Ok(())
    }

    fn can_morph_to(&self, new_role: AgentRole) -> bool {
        if new_role == self.identity.role {
            return true;
        }
        self.allowed_transitions
            .get(&self.identity.role)
            .is_some_and(|roles| roles.contains(&new_role))
    }

    fn augment_input(&self, input: &Engram) -> Engram {
        if self.system_prompt.is_empty() {
            return input.clone();
        }

        let text = input
            .body
            .as_text()
            .ok()
            .map(|body| format!("{}\n\n{}", self.system_prompt, body))
            .unwrap_or_else(|| self.system_prompt.clone());
        derived_output(input, input.kind.clone(), Body::text(text))
            .provenance(Provenance::agent(self.name()))
            .tag("role", self.identity.role.label())
            .build()
    }
}

#[async_trait]
impl Agent for MorphableAgent {
    async fn run(&self, input: &Engram, ctx: &Context) -> AgentResult {
        let wrapped_input = self.augment_input(input);
        let mut result = self.inner.run(&wrapped_input, ctx).await;
        let output_kind = result.output.kind.clone();
        let output_body = result.output.body.clone();
        result.output = derived_output(&result.output, output_kind, output_body)
            .provenance(Provenance::agent(self.name()))
            .tag("role", self.identity.role.label())
            .tag("temperament", self.identity.temperament.to_string())
            .build();
        result
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_streaming(&self) -> bool {
        self.inner.supports_streaming()
    }
}

fn system_prompt_for(role: AgentRole, profile: &RoleProfile) -> String {
    format!(
        "You are now acting as {}. clarity={:.2}, differentiation={:.2}, alignment={:.2}.",
        role.label(),
        profile.clarity,
        profile.differentiation,
        profile.alignment
    )
}

fn default_transition_matrix() -> HashMap<AgentRole, Vec<AgentRole>> {
    use AgentRole::*;

    HashMap::from([
        (Implementer, vec![QuickReviewer, Auditor, Refactorer]),
        (QuickReviewer, vec![Auditor]),
        (Auditor, vec![Implementer, Critic]),
        (Strategist, vec![Implementer, Architect, Researcher]),
        (Researcher, vec![Strategist, Implementer, Auditor]),
        (Conductor, vec![Strategist, Implementer, Auditor]),
        (Refactorer, vec![Auditor, Implementer]),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAgent;
    use roko_core::{Body, Context, Engram, Kind, Temperament};

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    #[tokio::test]
    async fn morphable_agent_applies_role_tag() {
        let identity = AgentIdentity::new(AgentRole::Implementer, Temperament::Balanced);
        let agent = MorphableAgent::new(Box::new(MockAgent::reply("ok")), identity);
        let result = agent.run(&prompt("hi"), &Context::at(0)).await;
        assert_eq!(result.output.tag("role"), Some("implementer"));
    }

    #[test]
    fn morph_rejects_disallowed_transition() {
        let identity = AgentIdentity::new(AgentRole::Implementer, Temperament::Balanced);
        let mut agent =
            MorphableAgent::new(Box::new(MockAgent::reply("ok")), identity).with_transitions(
                HashMap::from([(AgentRole::Implementer, vec![AgentRole::Auditor])]),
            );
        let err = agent.morph(AgentRole::Strategist).unwrap_err();
        assert!(matches!(err, MorphError::TransitionDenied { .. }));
    }
}

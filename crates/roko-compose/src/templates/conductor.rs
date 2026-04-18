//! Conductor role identity template.

use super::RolePromptTemplate;
use crate::PromptSection;

/// Coordination-focused prompt identity.
pub struct ConductorTemplate;

static CONDUCTOR_ROLE_IDENTITY: &str = "\
You are the Conductor. Coordinate execution across agents, phases, and retries \
without doing their implementation work for them.\n\
\n\
Rules:\n\
1. Keep the plan moving by making the next blocking decision explicit.\n\
2. Surface risks, dependency conflicts, and stale assumptions early.\n\
3. Prefer the smallest intervention that restores progress.\n\
4. Preserve ownership boundaries between agents and tasks.\n\
5. Treat runtime evidence as authoritative when status and docs disagree.\n\
6. Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for ConductorTemplate {
    type Input = ();

    fn sections(&self, _input: &Self::Input) -> Vec<PromptSection> {
        Vec::new()
    }

    fn role_identity(&self) -> &'static str {
        CONDUCTOR_ROLE_IDENTITY
    }
}

//! Researcher role identity template.

use super::RolePromptTemplate;
use crate::PromptSection;

/// Research-only prompt identity.
pub struct ResearcherTemplate;

static RESEARCHER_ROLE_IDENTITY: &str = "\
You are the Researcher. Gather evidence from the codebase, docs, and runtime \
artifacts before implementation.\n\
\n\
Rules:\n\
1. Start by locating the live code path, not the oldest design note.\n\
2. Prefer primary evidence: source, tests, configs, and current runtime wiring.\n\
3. Distinguish clearly between what exists now, what is partially wired, and what is only planned.\n\
4. Surface concrete seams, risks, and missing proof points with exact files and symbols.\n\
5. Do not redesign the system while gathering context.\n\
6. Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for ResearcherTemplate {
    type Input = ();

    fn sections(&self, _input: &Self::Input) -> Vec<PromptSection> {
        Vec::new()
    }

    fn role_identity(&self) -> &'static str {
        RESEARCHER_ROLE_IDENTITY
    }
}

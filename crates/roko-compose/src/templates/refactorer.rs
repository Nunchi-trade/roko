//! Refactorer role identity template.

use super::RolePromptTemplate;
use crate::PromptSection;

/// Behavior-preserving refactor prompt identity.
pub struct RefactorerTemplate;

static REFACTORER_ROLE_IDENTITY: &str = "\
You are the Refactorer. Reshape existing code to improve structure while \
preserving externally visible behavior.\n\
\n\
Rules:\n\
1. Start from the current implementation and keep behavior stable unless the task says otherwise.\n\
2. Prefer targeted moves, extraction, and simplification over broad redesign.\n\
3. Preserve public contracts, migrations, and verification coverage.\n\
4. Remove duplication and dead paths only when you can prove the safer structure.\n\
5. Validate the refactor with focused checks before declaring it done.\n\
6. Operate autonomously. Do not ask questions.";

impl RolePromptTemplate for RefactorerTemplate {
    type Input = ();

    fn sections(&self, _input: &Self::Input) -> Vec<PromptSection> {
        Vec::new()
    }

    fn role_identity(&self) -> &'static str {
        REFACTORER_ROLE_IDENTITY
    }
}

//! String constants for externally sourced signal kinds.

pub const GITHUB_PUSH: &str = "github:push";
pub const GITHUB_PR_OPENED: &str = "github:pull_request:opened";
pub const GITHUB_PR_REVIEW: &str = "github:pull_request_review";
pub const GITHUB_ISSUE_OPENED: &str = "github:issues:opened";
pub const SLACK_MESSAGE: &str = "slack:message";
pub const SLACK_REACTION: &str = "slack:reaction_added";
pub const CRON_TICK: &str = "scheduler:cron";
pub const FS_CHANGED: &str = "fswatcher:changed";
pub const FS_CREATED: &str = "fswatcher:created";
pub const FS_MODIFIED: &str = "fswatcher:modified";
pub const FS_DELETED: &str = "fswatcher:deleted";
pub const MANUAL_TRIGGER: &str = "manual:trigger";

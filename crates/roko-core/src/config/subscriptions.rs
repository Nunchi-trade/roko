//! Subscription and file-system watcher configuration sections.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::agent::default_true;

/// Custom deserializer that accepts either a string or a list of strings.
fn deserialize_glob_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::de::{SeqAccess, Visitor};
    use std::fmt;

    struct GlobListVisitor;

    impl<'de> Visitor<'de> for GlobListVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string or a list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value.to_owned()])
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![value])
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut items = Vec::new();
            while let Some(value) = seq.next_element::<String>()? {
                items.push(value);
            }
            Ok(items)
        }
    }

    deserializer.deserialize_any(GlobListVisitor)
}

// ---- subscriptions -------------------------------------------------------

/// Subscription configuration loaded from `roko.toml` and `.roko/subscriptions/*.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionConfig {
    /// Agent template name associated with this subscription.
    pub template: String,
    /// Engram kind glob used to match webhook signals.
    pub trigger: String,
    /// Typed trigger configuration (cron schedule, file-watch paths, or webhook URL).
    ///
    /// When set, this takes precedence over the plain `trigger` string for
    /// determining how the subscription fires. The `trigger` field is still
    /// used for signal matching in the dispatch loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_config: Option<SubscriptionTrigger>,
    /// Optional repo / branch / path filters.
    #[serde(default, skip_serializing_if = "SubscriptionFilterConfig::is_empty")]
    pub filter: SubscriptionFilterConfig,
    /// Maximum number of concurrent dispatches for this subscription.
    #[serde(default = "default_subscription_concurrency_limit")]
    pub concurrency_limit: usize,
    /// Minimum interval between dispatches, in seconds.
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Debounce window in milliseconds. Events arriving within this window
    /// after the first event are coalesced into a single dispatch.
    #[serde(default)]
    pub debounce_ms: u64,
    /// Whether the subscription is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            template: String::new(),
            trigger: String::new(),
            trigger_config: None,
            filter: SubscriptionFilterConfig::default(),
            concurrency_limit: default_subscription_concurrency_limit(),
            cooldown_secs: 0,
            debounce_ms: 0,
            enabled: default_true(),
        }
    }
}

fn default_subscription_concurrency_limit() -> usize {
    1
}

/// Typed trigger configuration for subscriptions.
///
/// Each variant corresponds to a distinct firing mechanism:
/// - `Cron` fires on a cron schedule (e.g., `*/30 * * * *`).
/// - `FileWatch` fires when watched paths change on disk.
/// - `Webhook` fires when a matching webhook payload arrives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubscriptionTrigger {
    /// Cron-schedule trigger.
    Cron {
        /// Standard cron expression (5 or 6 fields).
        schedule: String,
    },
    /// File-system watch trigger.
    FileWatch {
        /// Directories or file globs to watch.
        paths: Vec<String>,
        /// File-extension filter (e.g., `["rs", "toml"]`). Empty means all.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extensions: Vec<String>,
        /// Whether to watch recursively (default `true`).
        #[serde(default = "default_true")]
        recursive: bool,
    },
    /// Webhook trigger (matched against incoming webhook signals).
    Webhook {
        /// URL pattern or event type glob to match.
        event: String,
    },
}

impl SubscriptionTrigger {
    /// Return the trigger type as a string label.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Cron { .. } => "cron",
            Self::FileWatch { .. } => "file_watch",
            Self::Webhook { .. } => "webhook",
        }
    }
}

/// Optional filter applied after the trigger pattern matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionFilterConfig {
    /// Repo glob(s) to match against webhook payload repository fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "repos"
    )]
    pub repo: Vec<String>,
    /// Branch glob(s) to match against webhook payload branch/ref fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "branches"
    )]
    pub branch: Vec<String>,
    /// Path glob(s) to match against changed file paths.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "paths"
    )]
    pub path: Vec<String>,
    /// Label names to match against webhook payload label fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "labels"
    )]
    pub label: Vec<String>,
    /// Author logins to match against webhook payload author fields.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty",
        alias = "authors"
    )]
    pub author: Vec<String>,
}

impl SubscriptionFilterConfig {
    /// Returns `true` when no filter criteria are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.repo.is_empty()
            && self.branch.is_empty()
            && self.path.is_empty()
            && self.label.is_empty()
            && self.author.is_empty()
    }
}

// ---- [watcher] -----------------------------------------------------------

/// File-system watcher configuration.
///
/// Each watch path can narrow the observed file set with include/exclude
/// glob patterns.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherConfig {
    /// Watch roots configured by the user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<WatcherPathConfig>,
}

impl WatcherConfig {
    /// Returns `true` when no watch paths are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

/// One watched directory and its path filters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatcherPathConfig {
    /// Directory to watch recursively.
    pub directory: PathBuf,
    /// Glob patterns that opt paths into emission.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub include: Vec<String>,
    /// Glob patterns that suppress paths even if they match `include`.
    #[serde(
        default,
        deserialize_with = "deserialize_glob_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub exclude: Vec<String>,
}

impl Default for WatcherPathConfig {
    fn default() -> Self {
        Self {
            directory: PathBuf::from("."),
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

impl WatcherPathConfig {
    /// Returns `true` when no include/exclude filters are configured.
    #[must_use]
    pub fn filters_are_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }
}

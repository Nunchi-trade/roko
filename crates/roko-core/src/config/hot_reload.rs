//! Configuration hot-reload support (LIFE-07).
//!
//! Provides selective hot-reload of `roko.toml` fields that can safely
//! be changed at runtime without requiring a full agent restart.
//!
//! # Hot-reloadable sections
//!
//! | Section | Hot-reload | Notes |
//! |---|---|---|
//! | `[budget]` | Yes | Thresholds and degradation mode |
//! | `[tools]` | Yes | Tool profile allow/deny lists |
//! | `[learning]` | Yes | Learning subsystem toggles |
//! | `[demurrage]` | Yes | Knowledge decay rates |
//! | `[gates]` | Yes | Verify thresholds |
//! | `[conductor]` | Yes | Conductor settings |
//! | `[agent]` | **No** | Requires restart (model, provider) |
//! | `[providers]` | **No** | Requires restart |
//! | `[models]` | **No** | Requires restart |
//! | `[serve]` | **No** | Requires restart (bind address) |
//!
//! # Usage
//!
//! ```ignore
//! let old_config = current_config.clone();
//! let new_config = load_config(workdir)?;
//! let diff = config_diff(&old_config, &new_config);
//! if !diff.is_empty() {
//!     let applied = apply_hot_reload(&mut current_config, &new_config, &diff);
//!     // Log what was applied and what requires restart.
//! }
//! ```

use serde::{Deserialize, Serialize};

use super::schema::RokoConfig;

/// Which config section changed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSection {
    /// `[budget]` — spending limits, thresholds, degradation mode.
    Budget,
    /// `[tools]` — tool profiles and allow/deny lists.
    Tools,
    /// `[learning]` — learning subsystem toggles.
    Learning,
    /// `[demurrage]` — knowledge decay configuration.
    Demurrage,
    /// `[gates]` — gate thresholds and pipeline settings.
    Gates,
    /// `[conductor]` — conductor meta-orchestrator settings.
    Conductor,
    /// `[agent]` — agent/model configuration (requires restart).
    Agent,
    /// `[providers]` — provider registry (requires restart).
    Providers,
    /// `[models]` — model registry (requires restart).
    Models,
    /// `[serve]` — HTTP server settings (requires restart).
    Serve,
    /// `[routing]` — model routing (partial hot-reload).
    Routing,
    /// `[scheduler]` — cron scheduling.
    Scheduler,
    /// `[watcher]` — filesystem watchers.
    Watcher,
    /// Other unrecognized section.
    Other(String),
}

impl ConfigSection {
    /// Whether this section supports hot-reload.
    pub const fn is_hot_reloadable(&self) -> bool {
        matches!(
            self,
            Self::Budget
                | Self::Tools
                | Self::Learning
                | Self::Demurrage
                | Self::Gates
                | Self::Conductor
                | Self::Routing
        )
    }
}

/// A detected difference between two configurations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigChange {
    /// Which section changed.
    pub section: ConfigSection,
    /// Human-readable summary of the change.
    pub summary: String,
}

/// Result of applying hot-reload changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HotReloadResult {
    /// Changes that were successfully applied.
    pub applied: Vec<ConfigChange>,
    /// Changes that require a restart to take effect.
    pub needs_restart: Vec<ConfigChange>,
}

impl HotReloadResult {
    /// True if there are changes that require a restart.
    pub fn has_restart_required(&self) -> bool {
        !self.needs_restart.is_empty()
    }

    /// True if no changes were detected.
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty() && self.needs_restart.is_empty()
    }
}

/// Compare two configs and return a list of changed sections.
pub fn config_diff(old: &RokoConfig, new: &RokoConfig) -> Vec<ConfigChange> {
    let mut changes = Vec::new();

    if old.budget != new.budget {
        changes.push(ConfigChange {
            section: ConfigSection::Budget,
            summary: format!(
                "budget: plan ${:.2} -> ${:.2}, turn ${:.2} -> ${:.2}",
                old.budget.max_plan_usd,
                new.budget.max_plan_usd,
                old.budget.max_turn_usd,
                new.budget.max_turn_usd,
            ),
        });
    }

    if old.tools != new.tools {
        changes.push(ConfigChange {
            section: ConfigSection::Tools,
            summary: "tool profile configuration changed".into(),
        });
    }

    if old.learning != new.learning {
        changes.push(ConfigChange {
            section: ConfigSection::Learning,
            summary: "learning subsystem toggles changed".into(),
        });
    }

    if old.demurrage != new.demurrage {
        changes.push(ConfigChange {
            section: ConfigSection::Demurrage,
            summary: format!(
                "demurrage: rate_per_hour {:.4} -> {:.4}, min_balance {:.3} -> {:.3}",
                old.demurrage.rate_per_hour,
                new.demurrage.rate_per_hour,
                old.demurrage.min_balance,
                new.demurrage.min_balance,
            ),
        });
    }

    if old.gates != new.gates {
        changes.push(ConfigChange {
            section: ConfigSection::Gates,
            summary: "gate thresholds changed".into(),
        });
    }

    if old.conductor != new.conductor {
        changes.push(ConfigChange {
            section: ConfigSection::Conductor,
            summary: "conductor settings changed".into(),
        });
    }

    if old.routing != new.routing {
        changes.push(ConfigChange {
            section: ConfigSection::Routing,
            summary: "routing configuration changed".into(),
        });
    }

    // Non-hot-reloadable sections.
    if old.agent != new.agent {
        changes.push(ConfigChange {
            section: ConfigSection::Agent,
            summary: "agent configuration changed (requires restart)".into(),
        });
    }

    if old.providers != new.providers {
        changes.push(ConfigChange {
            section: ConfigSection::Providers,
            summary: "provider registry changed (requires restart)".into(),
        });
    }

    if old.models != new.models {
        changes.push(ConfigChange {
            section: ConfigSection::Models,
            summary: "model registry changed (requires restart)".into(),
        });
    }

    if old.serve != new.serve {
        changes.push(ConfigChange {
            section: ConfigSection::Serve,
            summary: "serve configuration changed (requires restart)".into(),
        });
    }

    if old.server != new.server {
        changes.push(ConfigChange {
            section: ConfigSection::Other("server".into()),
            summary: "server configuration changed (requires restart)".into(),
        });
    }

    changes
}

/// Apply hot-reloadable changes from `new_config` into `current_config`.
///
/// Returns a report of what was applied and what requires a restart.
/// Non-hot-reloadable changes are logged as warnings but not applied.
pub fn apply_hot_reload(
    current: &mut RokoConfig,
    new_config: &RokoConfig,
    changes: &[ConfigChange],
) -> HotReloadResult {
    let mut applied = Vec::new();
    let mut needs_restart = Vec::new();

    for change in changes {
        if change.section.is_hot_reloadable() {
            match &change.section {
                ConfigSection::Budget => current.budget = new_config.budget.clone(),
                ConfigSection::Tools => current.tools = new_config.tools.clone(),
                ConfigSection::Learning => current.learning = new_config.learning.clone(),
                ConfigSection::Demurrage => current.demurrage = new_config.demurrage.clone(),
                ConfigSection::Gates => current.gates = new_config.gates.clone(),
                ConfigSection::Conductor => current.conductor = new_config.conductor.clone(),
                ConfigSection::Routing => current.routing = new_config.routing.clone(),
                _ => {} // unreachable given is_hot_reloadable()
            }
            tracing::info!(
                section = %format!("{:?}", change.section),
                summary = %change.summary,
                "config hot-reloaded"
            );
            applied.push(change.clone());
        } else {
            tracing::warn!(
                section = %format!("{:?}", change.section),
                summary = %change.summary,
                "config change requires restart to take effect"
            );
            needs_restart.push(change.clone());
        }
    }

    HotReloadResult {
        applied,
        needs_restart,
    }
}

/// Parse a STRATEGY.md file into structured strategy fields.
///
/// The strategy file uses a simple Markdown format:
/// ```markdown
/// # Goals
/// - Primary: achieve self-hosting
/// - Secondary: improve test coverage
///
/// # Tactics
/// - Prefer small, incremental changes
/// - Run tests before committing
///
/// # Risk Bounds
/// - max_cost_per_task: $5.00
/// - max_concurrent_agents: 3
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyDocument {
    /// Parsed goal lines from the `# Goals` section.
    pub goals: Vec<String>,
    /// Parsed tactic lines from the `# Tactics` section.
    pub tactics: Vec<String>,
    /// Parsed risk-bound lines from the `# Risk Bounds` section.
    pub risk_bounds: Vec<String>,
    /// Raw content of the strategy file.
    pub raw: String,
}

/// Parse a STRATEGY.md file into structured fields.
pub fn parse_strategy_md(content: &str) -> StrategyDocument {
    let mut doc = StrategyDocument {
        raw: content.to_string(),
        ..Default::default()
    };

    let mut current_section: Option<&str> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect section headers.
        if let Some(header) = trimmed.strip_prefix("# ") {
            let header_lower = header.to_lowercase();
            current_section = if header_lower.contains("goal") {
                Some("goals")
            } else if header_lower.contains("tactic") || header_lower.contains("approach") {
                Some("tactics")
            } else if header_lower.contains("risk")
                || header_lower.contains("bound")
                || header_lower.contains("constraint")
            {
                Some("risk_bounds")
            } else {
                None
            };
            continue;
        }

        // Extract bullet items.
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !item.is_empty() {
                match current_section {
                    Some("goals") => doc.goals.push(item.to_string()),
                    Some("tactics") => doc.tactics.push(item.to_string()),
                    Some("risk_bounds") => doc.risk_bounds.push(item.to_string()),
                    _ => {}
                }
            }
        }
    }

    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_section_hot_reload_classification() {
        assert!(ConfigSection::Budget.is_hot_reloadable());
        assert!(ConfigSection::Tools.is_hot_reloadable());
        assert!(ConfigSection::Learning.is_hot_reloadable());
        assert!(ConfigSection::Demurrage.is_hot_reloadable());
        assert!(ConfigSection::Gates.is_hot_reloadable());
        assert!(ConfigSection::Conductor.is_hot_reloadable());
        assert!(ConfigSection::Routing.is_hot_reloadable());

        assert!(!ConfigSection::Agent.is_hot_reloadable());
        assert!(!ConfigSection::Providers.is_hot_reloadable());
        assert!(!ConfigSection::Models.is_hot_reloadable());
        assert!(!ConfigSection::Serve.is_hot_reloadable());
    }

    #[test]
    fn config_diff_detects_budget_change() {
        let old = RokoConfig::default();
        let mut new = old.clone();
        new.budget.max_plan_usd = 50.0;

        let changes = config_diff(&old, &new);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].section, ConfigSection::Budget);
    }

    #[test]
    fn config_diff_empty_for_identical() {
        let config = RokoConfig::default();
        let changes = config_diff(&config, &config);
        assert!(changes.is_empty());
    }

    #[test]
    fn apply_hot_reload_updates_reloadable_sections() {
        let mut current = RokoConfig::default();
        let mut new_config = current.clone();
        new_config.budget.max_plan_usd = 99.0;

        let changes = config_diff(&current, &new_config);
        let result = apply_hot_reload(&mut current, &new_config, &changes);

        assert_eq!(result.applied.len(), 1);
        assert!(result.needs_restart.is_empty());
        assert!((current.budget.max_plan_usd - 99.0).abs() < 1e-6);
    }

    #[test]
    fn apply_hot_reload_flags_restart_required() {
        let mut current = RokoConfig::default();
        let new_config = {
            let mut c = current.clone();
            c.agent.default_model = "claude-sonnet".into();
            c
        };
        let original_model = current.agent.default_model.clone();

        let changes = config_diff(&current, &new_config);
        let result = apply_hot_reload(&mut current, &new_config, &changes);

        assert!(result.applied.is_empty());
        assert_eq!(result.needs_restart.len(), 1);
        assert!(result.has_restart_required());
        // Agent config should NOT have been modified.
        assert_eq!(current.agent.default_model, original_model);
    }

    #[test]
    fn hot_reload_result_is_empty() {
        let result = HotReloadResult {
            applied: vec![],
            needs_restart: vec![],
        };
        assert!(result.is_empty());
    }

    #[test]
    fn parse_strategy_md_extracts_sections() {
        let content = r#"# Goals
- Primary: achieve self-hosting
- Secondary: improve test coverage

# Tactics
- Prefer small, incremental changes
- Run tests before committing

# Risk Bounds
- max_cost_per_task: $5.00
- max_concurrent_agents: 3
"#;

        let doc = parse_strategy_md(content);
        assert_eq!(doc.goals.len(), 2);
        assert_eq!(doc.goals[0], "Primary: achieve self-hosting");
        assert_eq!(doc.tactics.len(), 2);
        assert_eq!(doc.tactics[0], "Prefer small, incremental changes");
        assert_eq!(doc.risk_bounds.len(), 2);
        assert_eq!(doc.risk_bounds[0], "max_cost_per_task: $5.00");
    }

    #[test]
    fn parse_strategy_md_handles_empty_content() {
        let doc = parse_strategy_md("");
        assert!(doc.goals.is_empty());
        assert!(doc.tactics.is_empty());
        assert!(doc.risk_bounds.is_empty());
    }

    #[test]
    fn parse_strategy_md_ignores_non_matching_sections() {
        let content = r#"# Notes
- This should not appear anywhere

# Goals
- This is a goal
"#;

        let doc = parse_strategy_md(content);
        assert_eq!(doc.goals.len(), 1);
        assert!(doc.tactics.is_empty());
        assert!(doc.risk_bounds.is_empty());
    }
}

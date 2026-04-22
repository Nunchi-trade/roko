//! Cross-surface parity matrix and widget state semantics.
//!
//! Documents the correspondence between dashboard pages, TUI tabs/subviews,
//! CLI fallback commands, and backend data sources. Also defines the shared
//! [`WidgetState`] enum that describes the visual health of any widget across
//! all rendering surfaces.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Widget state semantics (SURF-GAP-02)
// ---------------------------------------------------------------------------

/// Visual state for any widget across all surfaces.
///
/// Every widget — whether rendered in the web dashboard, TUI, or CLI fallback —
/// should report one of these states so that operators get a consistent signal
/// about data freshness regardless of surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum WidgetState {
    /// Data is fresh and valid.
    Live,
    /// Data is from cache and may be outdated.
    Stale {
        /// How many seconds since the data was last refreshed.
        age_secs: u64,
    },
    /// Data is being fetched; the widget should show a spinner or skeleton.
    Loading,
    /// The data source is reachable but returning partial or incomplete results.
    Degraded {
        /// Human-readable explanation (e.g. "relay timeout", "partial response").
        reason: String,
    },
    /// The data source is unavailable and no cached fallback exists.
    Error {
        /// Human-readable error description.
        message: String,
    },
    /// The feature is not available in this context (e.g. git info outside a repo).
    Unavailable,
}

impl WidgetState {
    /// Whether the widget is in a state where user interaction makes sense.
    ///
    /// Returns `true` for `Live`, `Stale`, and `Degraded` — the three states
    /// where at least some data is present. Returns `false` for `Loading`,
    /// `Error`, and `Unavailable`.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Live | Self::Stale { .. } | Self::Degraded { .. })
    }

    /// Short human-readable label for the state (suitable for a badge or tag).
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Stale { .. } => "stale",
            Self::Loading => "loading",
            Self::Degraded { .. } => "degraded",
            Self::Error { .. } => "error",
            Self::Unavailable => "n/a",
        }
    }

    /// Accessibility-friendly text that describes both the state and its detail.
    #[must_use]
    pub fn accessibility_text(&self) -> String {
        match self {
            Self::Live => "Data is live and up to date".to_string(),
            Self::Stale { age_secs } => {
                format!("Data is stale, last refreshed {age_secs} seconds ago")
            }
            Self::Loading => "Data is loading".to_string(),
            Self::Degraded { reason } => format!("Data is degraded: {reason}"),
            Self::Error { message } => format!("Error: {message}"),
            Self::Unavailable => "Feature is not available in this context".to_string(),
        }
    }

    /// Returns `true` when data is fully fresh.
    #[must_use]
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }

    /// Returns `true` when the widget is in an error state.
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

impl Default for WidgetState {
    fn default() -> Self {
        Self::Loading
    }
}

impl std::fmt::Display for WidgetState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ---------------------------------------------------------------------------
// Cross-surface parity matrix (SURF-GAP-01)
// ---------------------------------------------------------------------------

/// Parity status for a feature across rendering targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParityStatus {
    /// All surfaces render equivalent data.
    Full,
    /// Some surfaces have more detail than others.
    Partial,
    /// Only some surfaces implement this feature.
    Incomplete,
    /// Not yet implemented anywhere.
    Missing,
}

impl ParityStatus {
    /// Short human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::Incomplete => "incomplete",
            Self::Missing => "missing",
        }
    }
}

impl std::fmt::Display for ParityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Surface parity entry for a single feature across rendering targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityEntry {
    /// Feature name (e.g. "Health / status overview").
    pub feature: String,
    /// Dashboard HTTP route that serves this feature, if any.
    pub dashboard_route: Option<String>,
    /// TUI tab that displays this feature, if any.
    pub tui_tab: Option<String>,
    /// TUI subview within the tab, if applicable.
    pub tui_subview: Option<String>,
    /// CLI fallback command that exposes this data, if any.
    pub cli_fallback: Option<String>,
    /// Backend data source (file path, StateHub key, runtime call, etc.).
    pub backend_source: String,
    /// Current parity status across surfaces.
    pub status: ParityStatus,
}

/// Summary statistics for the parity matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParitySummary {
    /// Total number of tracked features.
    pub total: usize,
    /// Features with full parity across all surfaces.
    pub full: usize,
    /// Features with partial parity (some surfaces have more detail).
    pub partial: usize,
    /// Features only implemented on some surfaces.
    pub incomplete: usize,
    /// Features not yet implemented anywhere.
    pub missing: usize,
}

/// Complete parity matrix response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParityMatrix {
    /// Aggregate counts by parity status.
    pub summary: ParitySummary,
    /// Per-feature parity entries.
    pub entries: Vec<ParityEntry>,
}

/// Build the full cross-surface parity matrix.
///
/// This captures the current state of feature coverage across the four
/// rendering surfaces: web dashboard, TUI, CLI fallback, and backend source.
#[must_use]
pub fn build_parity_matrix() -> ParityMatrix {
    let entries = vec![
        // -- Health / Status --
        ParityEntry {
            feature: "Health / status overview".into(),
            dashboard_route: Some("GET /api/health".into()),
            tui_tab: Some("F1 Dashboard".into()),
            tui_subview: Some("Health gauges".into()),
            cli_fallback: Some("roko status".into()),
            backend_source: "AppState (uptime, active_plans, active_agents)".into(),
            status: ParityStatus::Full,
        },
        // -- Plan list --
        ParityEntry {
            feature: "Plan list".into(),
            dashboard_route: Some("GET /api/plans".into()),
            tui_tab: Some("F2 Plans".into()),
            tui_subview: Some("Plan tree".into()),
            cli_fallback: Some("roko plan list".into()),
            backend_source: ".roko/plans/*.toml + .roko/plans/*.json".into(),
            status: ParityStatus::Full,
        },
        // -- Plan detail --
        ParityEntry {
            feature: "Plan detail".into(),
            dashboard_route: Some("GET /api/plans/{id}".into()),
            tui_tab: Some("F2 Plans".into()),
            tui_subview: Some("Task detail panel".into()),
            cli_fallback: Some("roko plan show <id>".into()),
            backend_source: ".roko/plans/{id}.toml".into(),
            status: ParityStatus::Full,
        },
        // -- Plan execution progress --
        ParityEntry {
            feature: "Plan execution progress".into(),
            dashboard_route: Some("GET /api/plans/{id}/status".into()),
            tui_tab: Some("F2 Plans".into()),
            tui_subview: Some("Wave progress bars".into()),
            cli_fallback: Some("roko plan run <dir> (live output)".into()),
            backend_source: "AppState::active_plans + executor snapshots".into(),
            status: ParityStatus::Partial,
        },
        // -- Agent roster --
        ParityEntry {
            feature: "Agent roster and output".into(),
            dashboard_route: Some("GET /api/managed-agents".into()),
            tui_tab: Some("F3 Agents".into()),
            tui_subview: Some("Agent list + output pane".into()),
            cli_fallback: Some("roko status (agent count)".into()),
            backend_source: "ProcessSupervisor + agent registry".into(),
            status: ParityStatus::Partial,
        },
        // -- Agent detail --
        ParityEntry {
            feature: "Agent detail (logs, episodes)".into(),
            dashboard_route: Some("GET /api/agents/{id}, GET /api/agents/{id}/logs".into()),
            tui_tab: Some("F3 Agents".into()),
            tui_subview: Some("Agent detail panel".into()),
            cli_fallback: Some("roko chat --agent <id>".into()),
            backend_source: "Agent sidecar proxy + episode logger".into(),
            status: ParityStatus::Partial,
        },
        // -- Gate results --
        ParityEntry {
            feature: "Gate results".into(),
            dashboard_route: Some("GET /api/gates/summary, GET /api/gates/history".into()),
            tui_tab: Some("F2 Plans".into()),
            tui_subview: Some("Gate result badges".into()),
            cli_fallback: Some("roko plan run <dir> (gate output in log)".into()),
            backend_source: ".roko/episodes.jsonl (gate events) + adaptive thresholds".into(),
            status: ParityStatus::Full,
        },
        // -- Job list --
        ParityEntry {
            feature: "Job list".into(),
            dashboard_route: Some("GET /api/jobs".into()),
            tui_tab: Some("F8 Marketplace".into()),
            tui_subview: Some("Job board table".into()),
            cli_fallback: None,
            backend_source: ".roko/jobs/*.json + StateHub marketplace_jobs".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Job detail --
        ParityEntry {
            feature: "Job detail".into(),
            dashboard_route: Some("GET /api/jobs/{id}".into()),
            tui_tab: Some("F8 Marketplace".into()),
            tui_subview: Some("Job detail panel".into()),
            cli_fallback: None,
            backend_source: ".roko/jobs/{id}.json".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Job creation --
        ParityEntry {
            feature: "Job creation".into(),
            dashboard_route: Some("POST /api/jobs".into()),
            tui_tab: Some("F8 Marketplace".into()),
            tui_subview: Some("Create job form".into()),
            cli_fallback: None,
            backend_source: ".roko/jobs/ (write)".into(),
            status: ParityStatus::Incomplete,
        },
        // -- PRD list --
        ParityEntry {
            feature: "PRD list".into(),
            dashboard_route: Some("GET /api/prds".into()),
            tui_tab: Some("F9 Atelier".into()),
            tui_subview: Some("PRD tree".into()),
            cli_fallback: Some("roko prd list".into()),
            backend_source: ".roko/prd/**/*.md".into(),
            status: ParityStatus::Full,
        },
        // -- PRD detail --
        ParityEntry {
            feature: "PRD detail".into(),
            dashboard_route: Some("GET /api/prds/{slug}".into()),
            tui_tab: Some("F9 Atelier".into()),
            tui_subview: Some("PRD content viewer".into()),
            cli_fallback: Some("roko prd list (summary only)".into()),
            backend_source: ".roko/prd/{stage}/{slug}.md".into(),
            status: ParityStatus::Partial,
        },
        // -- PRD transitions --
        ParityEntry {
            feature: "PRD transitions (idea/draft/promote/plan)".into(),
            dashboard_route: Some(
                "POST /api/prds/ideas, POST /api/prds/{slug}/draft, \
                 POST /api/prds/{slug}/promote, POST /api/prds/{slug}/plan"
                    .into(),
            ),
            tui_tab: Some("F9 Atelier".into()),
            tui_subview: Some("PRD action buttons".into()),
            cli_fallback: Some(
                "roko prd idea, roko prd draft new, roko prd draft promote, roko prd plan".into(),
            ),
            backend_source: "CLI runtime + .roko/prd/ lifecycle".into(),
            status: ParityStatus::Full,
        },
        // -- PRD coverage / status --
        ParityEntry {
            feature: "PRD coverage report".into(),
            dashboard_route: Some("GET /api/prds/status".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: Some("roko prd status".into()),
            backend_source: ".roko/prd/ scan + plan cross-reference".into(),
            status: ParityStatus::Partial,
        },
        // -- Config view --
        ParityEntry {
            feature: "Config view".into(),
            dashboard_route: Some("GET /api/config".into()),
            tui_tab: Some("F6 Config".into()),
            tui_subview: Some("Config tree".into()),
            cli_fallback: Some("roko config show".into()),
            backend_source: "roko.toml (ArcSwap<RokoConfig>)".into(),
            status: ParityStatus::Full,
        },
        // -- Git information --
        ParityEntry {
            feature: "Git information".into(),
            dashboard_route: None,
            tui_tab: Some("F4 Git".into()),
            tui_subview: Some("Branch tree, commit graph, worktree list".into()),
            cli_fallback: None,
            backend_source: "git CLI / libgit2 (local repo)".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Logs --
        ParityEntry {
            feature: "Logs".into(),
            dashboard_route: Some("GET /api/agents/{id}/logs".into()),
            tui_tab: Some("F5 Logs".into()),
            tui_subview: Some("Scrollable log viewer".into()),
            cli_fallback: None,
            backend_source: "tracing subscriber + agent sidecar logs".into(),
            status: ParityStatus::Partial,
        },
        // -- C-Factor metrics --
        ParityEntry {
            feature: "C-Factor metrics".into(),
            dashboard_route: Some("GET /api/metrics/c_factor".into()),
            tui_tab: Some("F1 Dashboard".into()),
            tui_subview: Some("C-Factor gauge".into()),
            cli_fallback: Some("roko status (summary)".into()),
            backend_source: ".roko/learn/efficiency.jsonl + cfactor computation".into(),
            status: ParityStatus::Partial,
        },
        // -- Efficiency metrics --
        ParityEntry {
            feature: "Efficiency metrics".into(),
            dashboard_route: Some("GET /api/learn/efficiency".into()),
            tui_tab: Some("F1 Dashboard".into()),
            tui_subview: Some("Cost / token sparklines".into()),
            cli_fallback: None,
            backend_source: ".roko/learn/efficiency.jsonl".into(),
            status: ParityStatus::Partial,
        },
        // -- Cost tier / model routing --
        ParityEntry {
            feature: "Cost tiers / model routing".into(),
            dashboard_route: Some("GET /api/learn/cost-tiers, GET /api/learn/cascade-router".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: None,
            backend_source: ".roko/learn/cascade-router.json".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Episodes --
        ParityEntry {
            feature: "Episodes".into(),
            dashboard_route: Some("GET /api/episodes".into()),
            tui_tab: Some("F7 Inspect".into()),
            tui_subview: Some("Episode list + replay".into()),
            cli_fallback: Some("roko replay".into()),
            backend_source: ".roko/episodes.jsonl".into(),
            status: ParityStatus::Partial,
        },
        // -- Signals --
        ParityEntry {
            feature: "Signals".into(),
            dashboard_route: Some("GET /api/signals".into()),
            tui_tab: Some("F7 Inspect".into()),
            tui_subview: Some("Signal DAG inspector".into()),
            cli_fallback: Some("roko replay (walks signal DAG)".into()),
            backend_source: ".roko/signals.jsonl".into(),
            status: ParityStatus::Partial,
        },
        // -- Knowledge / Engram browse --
        ParityEntry {
            feature: "Knowledge / Engram browse".into(),
            dashboard_route: None,
            tui_tab: Some("F7 Inspect".into()),
            tui_subview: Some("Engram DAG inspector".into()),
            cli_fallback: None,
            backend_source: "roko-neuro NeuroStore".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Provider health --
        ParityEntry {
            feature: "Provider health".into(),
            dashboard_route: Some("GET /api/providers/health".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: Some("roko doctor".into()),
            backend_source: "Provider health checks (roko-agent backends)".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Deployment status --
        ParityEntry {
            feature: "Deployment status".into(),
            dashboard_route: Some("GET /api/deployments".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: None,
            backend_source: "DeployBackend (Railway / manual)".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Experiments --
        ParityEntry {
            feature: "Prompt experiments (A/B)".into(),
            dashboard_route: Some("GET /api/learn/experiments".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: None,
            backend_source: ".roko/learn/experiments.json".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Adaptive gate thresholds --
        ParityEntry {
            feature: "Adaptive gate thresholds".into(),
            dashboard_route: Some("GET /api/learn/adaptive-thresholds".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: None,
            backend_source: ".roko/learn/gate-thresholds.json".into(),
            status: ParityStatus::Incomplete,
        },
        // -- Research --
        ParityEntry {
            feature: "Research topics".into(),
            dashboard_route: Some("GET /api/research".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: Some("roko research topic".into()),
            backend_source: ".roko/research/".into(),
            status: ParityStatus::Partial,
        },
        // -- Metrics summary --
        ParityEntry {
            feature: "Metrics summary (velocity, coverage, success rate)".into(),
            dashboard_route: Some(
                "GET /api/metrics/summary, GET /api/metrics/velocity, \
                 GET /api/metrics/coverage, GET /api/metrics/success_rate"
                    .into(),
            ),
            tui_tab: Some("F1 Dashboard".into()),
            tui_subview: Some("Summary cards".into()),
            cli_fallback: Some("roko status".into()),
            backend_source: "Aggregated from episodes + plans + efficiency events".into(),
            status: ParityStatus::Partial,
        },
        // -- SSE event stream --
        ParityEntry {
            feature: "Real-time event stream (SSE)".into(),
            dashboard_route: Some("GET /api/events/stream".into()),
            tui_tab: Some("F1 Dashboard (live updates)".into()),
            tui_subview: None,
            cli_fallback: None,
            backend_source: "ServerEvent broadcast bus".into(),
            status: ParityStatus::Partial,
        },
        // -- WebSocket subscriptions --
        ParityEntry {
            feature: "WebSocket subscriptions".into(),
            dashboard_route: Some("WS /ws".into()),
            tui_tab: None,
            tui_subview: None,
            cli_fallback: None,
            backend_source: "ServerEvent broadcast bus (filtered)".into(),
            status: ParityStatus::Incomplete,
        },
    ];

    let summary = ParitySummary {
        total: entries.len(),
        full: entries.iter().filter(|e| e.status == ParityStatus::Full).count(),
        partial: entries
            .iter()
            .filter(|e| e.status == ParityStatus::Partial)
            .count(),
        incomplete: entries
            .iter()
            .filter(|e| e.status == ParityStatus::Incomplete)
            .count(),
        missing: entries
            .iter()
            .filter(|e| e.status == ParityStatus::Missing)
            .count(),
    };

    ParityMatrix { summary, entries }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_state_labels() {
        assert_eq!(WidgetState::Live.label(), "live");
        assert_eq!(
            WidgetState::Stale { age_secs: 30 }.label(),
            "stale"
        );
        assert_eq!(WidgetState::Loading.label(), "loading");
        assert_eq!(
            WidgetState::Degraded {
                reason: "timeout".into()
            }
            .label(),
            "degraded"
        );
        assert_eq!(
            WidgetState::Error {
                message: "gone".into()
            }
            .label(),
            "error"
        );
        assert_eq!(WidgetState::Unavailable.label(), "n/a");
    }

    #[test]
    fn widget_state_actionable() {
        assert!(WidgetState::Live.is_actionable());
        assert!(WidgetState::Stale { age_secs: 10 }.is_actionable());
        assert!(WidgetState::Degraded {
            reason: "slow".into()
        }
        .is_actionable());
        assert!(!WidgetState::Loading.is_actionable());
        assert!(!WidgetState::Error {
            message: "fail".into()
        }
        .is_actionable());
        assert!(!WidgetState::Unavailable.is_actionable());
    }

    #[test]
    fn widget_state_accessibility() {
        let live = WidgetState::Live;
        assert!(live.accessibility_text().contains("live"));

        let stale = WidgetState::Stale { age_secs: 42 };
        assert!(stale.accessibility_text().contains("42"));

        let degraded = WidgetState::Degraded {
            reason: "relay timeout".into(),
        };
        assert!(degraded.accessibility_text().contains("relay timeout"));

        let err = WidgetState::Error {
            message: "connection refused".into(),
        };
        assert!(err.accessibility_text().contains("connection refused"));
    }

    #[test]
    fn widget_state_default_is_loading() {
        assert_eq!(WidgetState::default(), WidgetState::Loading);
    }

    #[test]
    fn widget_state_serde_roundtrip() {
        let states = vec![
            WidgetState::Live,
            WidgetState::Stale { age_secs: 120 },
            WidgetState::Loading,
            WidgetState::Degraded {
                reason: "partial".into(),
            },
            WidgetState::Error {
                message: "boom".into(),
            },
            WidgetState::Unavailable,
        ];
        for state in &states {
            let json = serde_json::to_string(state).expect("serialize");
            let back: WidgetState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(&back, state);
        }
    }

    #[test]
    fn parity_status_labels() {
        assert_eq!(ParityStatus::Full.label(), "full");
        assert_eq!(ParityStatus::Partial.label(), "partial");
        assert_eq!(ParityStatus::Incomplete.label(), "incomplete");
        assert_eq!(ParityStatus::Missing.label(), "missing");
    }

    #[test]
    fn parity_matrix_is_populated() {
        let matrix = build_parity_matrix();
        assert!(matrix.entries.len() >= 17, "expected at least 17 features");
        assert_eq!(
            matrix.summary.total,
            matrix.entries.len(),
            "summary.total must match entries.len()"
        );
        assert_eq!(
            matrix.summary.full + matrix.summary.partial
                + matrix.summary.incomplete + matrix.summary.missing,
            matrix.summary.total,
            "status counts must sum to total"
        );
    }

    #[test]
    fn parity_matrix_no_empty_features() {
        let matrix = build_parity_matrix();
        for entry in &matrix.entries {
            assert!(!entry.feature.is_empty(), "feature name must not be empty");
            assert!(
                !entry.backend_source.is_empty(),
                "backend_source must not be empty for '{}'",
                entry.feature
            );
        }
    }

    #[test]
    fn parity_entry_serde_roundtrip() {
        let matrix = build_parity_matrix();
        let json = serde_json::to_string(&matrix).expect("serialize");
        let back: ParityMatrix = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.entries.len(), matrix.entries.len());
        assert_eq!(back.summary.total, matrix.summary.total);
    }

    #[test]
    fn full_parity_entries_have_all_surfaces() {
        let matrix = build_parity_matrix();
        for entry in matrix.entries.iter().filter(|e| e.status == ParityStatus::Full) {
            assert!(
                entry.dashboard_route.is_some(),
                "'{}' is Full but missing dashboard_route",
                entry.feature
            );
            assert!(
                entry.cli_fallback.is_some(),
                "'{}' is Full but missing cli_fallback",
                entry.feature
            );
            assert!(
                entry.tui_tab.is_some(),
                "'{}' is Full but missing tui_tab",
                entry.feature
            );
        }
    }
}

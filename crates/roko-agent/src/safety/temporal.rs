//! Temporal logic (LTL) runtime monitor for safety and liveness properties.
//!
//! Provides [`TemporalMonitor`] which tracks [`LtlProperty`] instances across
//! tool calls. Safety properties (`Never`, `Always`) are checked incrementally
//! on each event; the first violation short-circuits. Liveness properties
//! (`Eventually`) are checked against a turn deadline — violation fires after
//! N turns without progress.
//!
//! # Configuration
//!
//! Properties are declared in `roko.toml`:
//!
//! ```toml
//! [safety]
//! never = ["rm -rf /", "force-push main"]
//! eventually = [{ predicate = "run_tests", deadline_turns = 20 }]
//! ```
//!
//! # Integration
//!
//! The monitor is wired into [`SafetyLayer::check_pre_execution`] and checked
//! on every tool call. `Never` violations return `ToolError::PermissionDenied`
//! immediately. `Eventually` violations fire after the deadline elapses.

use std::fmt;

use roko_core::tool::{ToolCall, ToolError};
use serde::{Deserialize, Serialize};

// ─── Property definitions ────────────────────────────────────────────

/// A temporal logic property monitored at runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LtlProperty {
    /// Safety: the pattern must never appear in any tool call.
    ///
    /// Checked against tool name and command arguments on each call.
    /// Violation is immediate and non-recoverable.
    Never {
        /// Substring pattern to match against tool name or command argument.
        pattern: String,
        /// Human-readable description of why this is forbidden.
        #[serde(default)]
        description: String,
    },
    /// Safety: a predicate that must hold on every tool call.
    ///
    /// Typically used for structural invariants (e.g., "every edit must
    /// be preceded by a read").
    Always {
        /// Predicate evaluated on each call (matched against tool names).
        predicate: String,
        /// Human-readable description.
        #[serde(default)]
        description: String,
    },
    /// Liveness: a predicate that must become true within N turns.
    ///
    /// Tracks turns since the property was registered. If the predicate
    /// has not been satisfied after `deadline_turns`, the monitor reports
    /// a violation.
    Eventually {
        /// Predicate that must match a tool call name to satisfy the property.
        predicate: String,
        /// Maximum turns before the property is considered violated.
        deadline_turns: u32,
        /// Human-readable description.
        #[serde(default)]
        description: String,
    },
}

// ─── Monitor state ───────────────────────────────────────────────────

/// Per-property state tracked by the monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorState {
    /// The property is satisfied (or has not yet been violated).
    Satisfied,
    /// A liveness property is pending — it has not yet been satisfied.
    Pending {
        /// Turn at which the property was registered.
        since_turn: u64,
    },
    /// The property has been violated.
    Violated {
        /// Turn at which the violation occurred.
        turn: u64,
        /// Human-readable detail.
        detail: String,
    },
}

/// A concrete violation detected by the temporal monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// Index of the property in the monitor's property list.
    pub property_index: usize,
    /// The property that was violated.
    pub property: LtlProperty,
    /// Turn at which the violation was detected.
    pub turn: u64,
    /// Human-readable detail.
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "temporal violation at turn {}: {}",
            self.turn, self.detail
        )
    }
}

// ─── TemporalMonitor ─────────────────────────────────────────────────

/// Runtime monitor for temporal logic properties.
///
/// Tracks a list of [`LtlProperty`] instances, maintaining per-property
/// state across tool calls. Call [`TemporalMonitor::check`] on each
/// tool call to detect violations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalMonitor {
    /// Registered properties.
    pub properties: Vec<LtlProperty>,
    /// Per-property state, indexed parallel to `properties`.
    pub states: Vec<MonitorState>,
    /// Global turn counter, incremented on each `check()` call.
    pub turn_count: u64,
}

impl TemporalMonitor {
    /// Create an empty monitor with no properties.
    #[must_use]
    pub fn new() -> Self {
        Self {
            properties: Vec::new(),
            states: Vec::new(),
            turn_count: 0,
        }
    }

    /// Create a monitor from a list of properties.
    #[must_use]
    pub fn with_properties(properties: Vec<LtlProperty>) -> Self {
        let states = properties
            .iter()
            .map(|prop| match prop {
                LtlProperty::Never { .. } | LtlProperty::Always { .. } => {
                    MonitorState::Satisfied
                }
                LtlProperty::Eventually { .. } => MonitorState::Pending { since_turn: 0 },
            })
            .collect();
        Self {
            properties,
            states,
            turn_count: 0,
        }
    }

    /// Register an additional property.
    pub fn add_property(&mut self, property: LtlProperty) {
        let state = match &property {
            LtlProperty::Never { .. } | LtlProperty::Always { .. } => MonitorState::Satisfied,
            LtlProperty::Eventually { .. } => MonitorState::Pending {
                since_turn: self.turn_count,
            },
        };
        self.properties.push(property);
        self.states.push(state);
    }

    /// Check all properties against a tool call event.
    ///
    /// Returns a list of violations detected during this check. The turn
    /// counter is incremented regardless of violations.
    pub fn check(&mut self, call: &ToolCall) -> Vec<Violation> {
        self.turn_count += 1;
        let turn = self.turn_count;
        let mut violations = Vec::new();

        for (i, property) in self.properties.iter().enumerate() {
            // Skip already-violated properties.
            if matches!(self.states[i], MonitorState::Violated { .. }) {
                continue;
            }

            match property {
                LtlProperty::Never {
                    pattern,
                    description,
                } => {
                    if matches_pattern(call, pattern) {
                        let detail = if description.is_empty() {
                            format!("Never property violated: pattern `{pattern}` matched tool call `{}`", call.name)
                        } else {
                            format!("Never: {description} (pattern `{pattern}` matched `{}`)", call.name)
                        };
                        self.states[i] = MonitorState::Violated { turn, detail: detail.clone() };
                        violations.push(Violation {
                            property_index: i,
                            property: property.clone(),
                            turn,
                            detail,
                        });
                    }
                }
                LtlProperty::Always {
                    predicate,
                    description,
                } => {
                    // Always properties are checked as "the predicate must
                    // hold on every call". If the call does NOT match the
                    // predicate, that is a violation.
                    if !matches_pattern(call, predicate) {
                        let detail = if description.is_empty() {
                            format!(
                                "Always property violated: predicate `{predicate}` not satisfied by `{}`",
                                call.name
                            )
                        } else {
                            format!("Always: {description} (not satisfied by `{}`)", call.name)
                        };
                        self.states[i] =
                            MonitorState::Violated {
                                turn,
                                detail: detail.clone(),
                            };
                        violations.push(Violation {
                            property_index: i,
                            property: property.clone(),
                            turn,
                            detail,
                        });
                    }
                }
                LtlProperty::Eventually {
                    predicate,
                    deadline_turns,
                    description,
                } => {
                    if matches_pattern(call, predicate) {
                        // Satisfied! Transition to Satisfied state.
                        self.states[i] = MonitorState::Satisfied;
                    } else if let MonitorState::Pending { since_turn } = self.states[i] {
                        // Check deadline.
                        if turn - since_turn >= u64::from(*deadline_turns) {
                            let detail = if description.is_empty() {
                                format!(
                                    "Eventually property violated: predicate `{predicate}` not satisfied within {deadline_turns} turns"
                                )
                            } else {
                                format!(
                                    "Eventually: {description} (not satisfied within {deadline_turns} turns)"
                                )
                            };
                            self.states[i] = MonitorState::Violated {
                                turn,
                                detail: detail.clone(),
                            };
                            violations.push(Violation {
                                property_index: i,
                                property: property.clone(),
                                turn,
                                detail,
                            });
                        }
                    }
                }
            }
        }

        violations
    }

    /// Convert any violations from the last check into a `ToolError`.
    ///
    /// Returns `Ok(())` if no violations were found, or
    /// `Err(ToolError::PermissionDenied)` with the violation details.
    pub fn check_as_tool_error(&mut self, call: &ToolCall) -> Result<(), ToolError> {
        let violations = self.check(call);
        if violations.is_empty() {
            return Ok(());
        }
        let messages: Vec<String> = violations.iter().map(|v| v.detail.clone()).collect();
        Err(ToolError::PermissionDenied(format!(
            "temporal property violation: {}",
            messages.join("; ")
        )))
    }

    /// Return the number of registered properties.
    #[must_use]
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Return `true` if any property is in a violated state.
    #[must_use]
    pub fn has_violations(&self) -> bool {
        self.states
            .iter()
            .any(|s| matches!(s, MonitorState::Violated { .. }))
    }
}

impl Default for TemporalMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a tool call matches a pattern string.
///
/// Matching rules (checked in order):
/// 1. Tool name equals the pattern.
/// 2. Tool name contains the pattern as a substring.
/// 3. The `command` argument (for bash tools) contains the pattern.
fn matches_pattern(call: &ToolCall, pattern: &str) -> bool {
    // Exact tool name match.
    if call.name == pattern {
        return true;
    }

    // Tool name contains pattern.
    if call.name.contains(pattern) {
        return true;
    }

    // Command argument contains pattern (for bash/exec tools).
    if let Some(command) = call
        .arguments
        .get("command")
        .and_then(|v| v.as_str())
    {
        let lower_command = command.to_ascii_lowercase();
        let lower_pattern = pattern.to_ascii_lowercase();
        if lower_command.contains(&lower_pattern) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash_call(cmd: &str) -> ToolCall {
        ToolCall::new("test-id", "bash", serde_json::json!({ "command": cmd }))
    }

    fn tool_call(name: &str) -> ToolCall {
        ToolCall::new("test-id", name, serde_json::json!({}))
    }

    #[test]
    fn never_property_fires_on_pattern_match() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Never {
            pattern: "rm -rf /".into(),
            description: "never delete root".into(),
        }]);

        // Safe call: no violation.
        let violations = monitor.check(&bash_call("echo hi"));
        assert!(violations.is_empty());

        // Dangerous call: violation.
        let violations = monitor.check(&bash_call("rm -rf /"));
        assert_eq!(violations.len(), 1);
        assert!(violations[0].detail.contains("never delete root"));
    }

    #[test]
    fn never_property_matches_tool_name() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Never {
            pattern: "web_fetch".into(),
            description: "no network".into(),
        }]);
        let violations = monitor.check(&tool_call("web_fetch"));
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn eventually_satisfied_before_deadline() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Eventually {
            predicate: "run_tests".into(),
            deadline_turns: 5,
            description: "must run tests".into(),
        }]);

        // 3 turns without tests.
        for _ in 0..3 {
            let v = monitor.check(&bash_call("echo hi"));
            assert!(v.is_empty());
        }

        // Satisfy on turn 4.
        let v = monitor.check(&tool_call("run_tests"));
        assert!(v.is_empty());
        assert!(matches!(monitor.states[0], MonitorState::Satisfied));
    }

    #[test]
    fn eventually_violated_after_deadline() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Eventually {
            predicate: "run_tests".into(),
            deadline_turns: 3,
            description: "must run tests".into(),
        }]);

        // 3 turns without tests -> deadline reached.
        for i in 0..3 {
            let v = monitor.check(&bash_call("echo hi"));
            if i < 2 {
                assert!(v.is_empty(), "turn {i} should not violate yet");
            } else {
                assert_eq!(v.len(), 1, "turn {i} should fire violation");
                assert!(v[0].detail.contains("must run tests"));
            }
        }
    }

    #[test]
    fn check_as_tool_error_returns_permission_denied() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Never {
            pattern: "force-push main".into(),
            description: "never force-push main".into(),
        }]);
        let result = monitor.check_as_tool_error(&bash_call("git push --force-push main"));
        assert!(result.is_err());
        if let Err(ToolError::PermissionDenied(msg)) = result {
            assert!(msg.contains("temporal property violation"));
        }
    }

    #[test]
    fn empty_monitor_always_passes() {
        let mut monitor = TemporalMonitor::new();
        assert!(monitor.check(&bash_call("anything")).is_empty());
        assert!(!monitor.has_violations());
    }

    #[test]
    fn violated_property_stays_violated() {
        let mut monitor = TemporalMonitor::with_properties(vec![LtlProperty::Never {
            pattern: "bad".into(),
            description: String::new(),
        }]);
        monitor.check(&tool_call("bad"));
        assert!(monitor.has_violations());

        // Subsequent checks skip already-violated properties.
        let v = monitor.check(&tool_call("good"));
        assert!(v.is_empty());
        assert!(monitor.has_violations());
    }

    #[test]
    fn monitor_round_trips_through_serde() {
        let monitor = TemporalMonitor::with_properties(vec![
            LtlProperty::Never {
                pattern: "rm -rf".into(),
                description: "no rm".into(),
            },
            LtlProperty::Eventually {
                predicate: "test".into(),
                deadline_turns: 10,
                description: "run tests".into(),
            },
        ]);
        let json = serde_json::to_string(&monitor).unwrap();
        let decoded: TemporalMonitor = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.properties.len(), 2);
        assert_eq!(decoded.states.len(), 2);
    }
}

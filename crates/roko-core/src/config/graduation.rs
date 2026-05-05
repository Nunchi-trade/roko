//! Graduation policy configuration -- which Pulses get promoted to Signals.
//!
//! Graduation is the controlled promotion of ephemeral [`Pulse`](crate::Pulse)s
//! to durable [`Signal`](crate::Signal)s in the Store. Without graduation
//! policies, the system either logs everything (wasteful) or loses important
//! events (silently). Graduation policies formalize the decision: each policy
//! declares which Bus topics should always/never/sometimes graduate.
//!
//! # Policy precedence
//!
//! When multiple policies match a pulse's topic, the evaluation collects all
//! matches and then applies:
//!
//! 1. **No match** -- do not graduate (default).
//! 2. **`never`** wins over `always` -- prevents noisy streams from being
//!    persisted accidentally.
//! 3. **`always`** -- graduate unconditionally.
//! 4. **`sample_every`** -- graduate every Nth matching pulse by sequence number.
//!
//! # Example TOML
//!
//! ```toml
//! [[graduation.policies]]
//! watch = { Prefix = "gate.verdict." }
//! always = true
//!
//! [[graduation.policies]]
//! watch = { Prefix = "heartbeat." }
//! never = true
//! ```

use serde::{Deserialize, Serialize};

use crate::pulse::{Topic, TopicFilter};

/// A single graduation policy: watch these topics, apply these criteria.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraduationPolicy {
    /// Bus topic(s) to watch.
    pub watch: TopicFilter,

    /// Always graduate Pulses matching this policy. `never` still wins when
    /// multiple matching policies apply.
    #[serde(default)]
    pub always: bool,

    /// Never graduate Pulses matching this policy.
    #[serde(default)]
    pub never: bool,

    /// Graduate every Nth matching Pulse (1 = every pulse, 10 = 10%).
    /// Ignored when `always` or `never` is set.
    #[serde(default = "default_sample_every")]
    pub sample_every: usize,
}

fn default_sample_every() -> usize {
    1
}

impl GraduationPolicy {
    /// Does this policy's watch filter match the given topic?
    #[must_use]
    pub fn matches_topic(&self, topic: &Topic) -> bool {
        self.watch.matches(topic)
    }
}

/// Top-level graduation config section in `roko.toml`.
///
/// See module-level docs for policy precedence rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraduationConfig {
    /// The list of graduation policies, evaluated in order.
    #[serde(default)]
    pub policies: Vec<GraduationPolicy>,
}

impl GraduationConfig {
    /// Return the default graduation policies from the v2 spec.
    ///
    /// These match the "always" and "never" tables in `09-GRADUATION.md`.
    /// Agent wildcard policies (e.g. `agent.*.turn.completed`) are deferred
    /// until `TopicFilter` supports glob/wildcard matching.
    #[must_use]
    pub fn default_policies() -> Self {
        Self {
            policies: vec![
                // Always graduate these:
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.verdict.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("safety.approval.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("conductor.circuit.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Exact(Topic::new("cost.charged")),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                // Never graduate these:
                GraduationPolicy {
                    watch: TopicFilter::Prefix("heartbeat.".into()),
                    always: false,
                    never: true,
                    sample_every: 1,
                },
            ],
        }
    }

    /// Evaluate all policies against a pulse's topic and sequence number.
    ///
    /// Returns `true` if the pulse should be graduated. The evaluation
    /// collects all matching policies and then applies the precedence
    /// rules: `never` > `always` > `sample_every`. If no policy matches,
    /// the pulse is not graduated.
    #[must_use]
    pub fn should_graduate(&self, topic: &Topic, seq: u64) -> bool {
        let mut any_match = false;
        let mut any_never = false;
        let mut any_always = false;
        let mut min_sample: Option<usize> = None;

        for policy in &self.policies {
            if !policy.watch.matches(topic) {
                continue;
            }
            any_match = true;

            if policy.never {
                any_never = true;
            }
            if policy.always {
                any_always = true;
            }
            if !policy.always && !policy.never {
                let s = policy.sample_every.max(1);
                min_sample = Some(match min_sample {
                    Some(prev) => prev.min(s),
                    None => s,
                });
            }
        }

        if !any_match {
            return false;
        }
        // never wins over always
        if any_never {
            return false;
        }
        if any_always {
            return true;
        }
        // Sampling: graduate every Nth pulse (deterministic on seq)
        match min_sample {
            Some(n) => seq % (n as u64) == 0,
            None => false,
        }
    }
}

impl Default for GraduationConfig {
    fn default() -> Self {
        Self::default_policies()
    }
}

// ---- Tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Policy precedence tests ----

    #[test]
    fn never_overrides_always_for_same_topic() {
        let config = GraduationConfig {
            policies: vec![
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.".into()),
                    always: false,
                    never: true,
                    sample_every: 1,
                },
            ],
        };
        // never must win over always
        assert!(!config.should_graduate(&Topic::new("gate.verdict.emitted"), 1));
    }

    #[test]
    fn always_graduates_matching_topic() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("gate.verdict.".into()),
                always: true,
                never: false,
                sample_every: 1,
            }],
        };
        assert!(config.should_graduate(&Topic::new("gate.verdict.emitted"), 1));
        assert!(config.should_graduate(&Topic::new("gate.verdict.emitted"), 999));
    }

    #[test]
    fn never_blocks_matching_topic() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("heartbeat.".into()),
                always: false,
                never: true,
                sample_every: 1,
            }],
        };
        assert!(!config.should_graduate(&Topic::new("heartbeat.tick"), 1));
    }

    #[test]
    fn no_matching_policy_does_not_graduate() {
        let config = GraduationConfig::default_policies();
        assert!(!config.should_graduate(&Topic::new("agent.output.token"), 1));
    }

    #[test]
    fn sample_every_graduates_every_nth_pulse() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::Prefix("metric.".into()),
                always: false,
                never: false,
                sample_every: 3,
            }],
        };
        // seq 0 % 3 == 0 -> graduate
        assert!(config.should_graduate(&Topic::new("metric.cpu"), 0));
        // seq 1 % 3 != 0 -> skip
        assert!(!config.should_graduate(&Topic::new("metric.cpu"), 1));
        // seq 2 % 3 != 0 -> skip
        assert!(!config.should_graduate(&Topic::new("metric.cpu"), 2));
        // seq 3 % 3 == 0 -> graduate
        assert!(config.should_graduate(&Topic::new("metric.cpu"), 3));
    }

    #[test]
    fn sample_every_zero_treated_as_one() {
        let config = GraduationConfig {
            policies: vec![GraduationPolicy {
                watch: TopicFilter::All,
                always: false,
                never: false,
                sample_every: 0,
            }],
        };
        // sample_every=0 clamped to 1, so every pulse graduates
        assert!(config.should_graduate(&Topic::new("anything"), 0));
        assert!(config.should_graduate(&Topic::new("anything"), 1));
    }

    // ---- Default policies tests ----

    #[test]
    fn default_policies_loaded() {
        let cfg = GraduationConfig::default_policies();
        assert!(!cfg.policies.is_empty());
        // At least one always-graduate policy.
        assert!(cfg.policies.iter().any(|p| p.always));
        // At least one never-graduate policy.
        assert!(cfg.policies.iter().any(|p| p.never));
    }

    #[test]
    fn default_policies_graduate_gate_verdicts() {
        let cfg = GraduationConfig::default_policies();
        assert!(cfg.should_graduate(&Topic::new("gate.verdict.emitted"), 1));
    }

    #[test]
    fn default_policies_graduate_safety_approvals() {
        let cfg = GraduationConfig::default_policies();
        assert!(cfg.should_graduate(&Topic::new("safety.approval.granted"), 1));
    }

    #[test]
    fn default_policies_graduate_conductor_circuit() {
        let cfg = GraduationConfig::default_policies();
        assert!(cfg.should_graduate(&Topic::new("conductor.circuit.opened"), 1));
    }

    #[test]
    fn default_policies_graduate_cost_charged() {
        let cfg = GraduationConfig::default_policies();
        assert!(cfg.should_graduate(&Topic::new("cost.charged"), 1));
    }

    #[test]
    fn default_policies_block_heartbeat() {
        let cfg = GraduationConfig::default_policies();
        assert!(!cfg.should_graduate(&Topic::new("heartbeat.tick"), 1));
    }

    // ---- Config parsing tests ----

    #[test]
    fn graduation_config_parses_from_toml() {
        let toml_str = r#"
            [[policies]]
            watch = { Prefix = "gate.verdict." }
            always = true

            [[policies]]
            watch = { Prefix = "heartbeat." }
            never = true
        "#;

        let cfg: GraduationConfig = toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.policies.len(), 2);
        assert!(cfg.policies[0].always);
        assert!(cfg.policies[1].never);
    }

    #[test]
    fn graduation_config_parses_exact_topic() {
        let toml_str = r#"
            [[policies]]
            watch = { Exact = "cost.charged" }
            always = true
        "#;

        let cfg: GraduationConfig = toml::from_str(toml_str).expect("should parse exact topic");
        assert_eq!(cfg.policies.len(), 1);
        assert!(cfg.policies[0]
            .watch
            .matches(&Topic::new("cost.charged")));
    }

    #[test]
    fn roko_config_graduation_section_parses() {
        let toml_str = r#"
            [[graduation.policies]]
            watch = { Prefix = "gate.verdict." }
            always = true

            [[graduation.policies]]
            watch = { Prefix = "heartbeat." }
            never = true
        "#;

        let cfg: crate::config::schema::RokoConfig =
            toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.graduation.policies.len(), 2);
    }

    #[test]
    fn graduation_config_roundtrip_toml() {
        let cfg = GraduationConfig::default_policies();
        let serialized = toml::to_string_pretty(&cfg).expect("serialize");
        let parsed: GraduationConfig = toml::from_str(&serialized).expect("re-parse");
        assert_eq!(cfg.policies.len(), parsed.policies.len());
    }
}

//! Format-selection bandit trait + simple concrete impls (§36.l, parity
//! items 36.81–36.86).
//!
//! **Why a bandit, not a static table**: Meta-Harness demonstrated a 6×
//! performance gap from harness changes alone. TensorZero's Track-and-Stop
//! bandit (Garivier & Kaufmann 2016) handles ~1% of global LLM API spend.
//! HAL benchmark: 42% → 78% by swapping scaffolds. These findings
//! establish that format selection must be **adaptive and empirical**,
//! not static-per-model.
//!
//! This module ships the runtime trait + two simple, deterministic
//! concrete implementations:
//!
//! - [`ProfileBandit`] — follows the [`ToolFormatProfile::preferred`] arm,
//!   with deterministic fallback-chain demotion after N failures. Useful
//!   as a day-one baseline that still feeds trace data into the system.
//! - [`EpsilonGreedyBandit`] — exploration + exploitation with a simple
//!   ε-greedy policy. Still deterministic given a fixed PRNG seed so
//!   tests are reproducible.
//!
//! The full Track-and-Stop bandit (§36.83, anytime-valid, asymmetric
//! exploration budget) lives in `roko-learn::format_bandit`.

#![allow(clippy::significant_drop_tightening)] // lock guards held for full method bodies are intentional
#![allow(clippy::unnecessary_literal_bound)] // trait default impls returning &str literals

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::format::{ToolFormat, ToolFormatProfile};
use super::trace::ToolOutcome;
use crate::{AgentRole, TaskComplexityBand};

// ─── BanditKey ────────────────────────────────────────────────────────────

/// Partitioning key for bandit arms.
///
/// Rationale for each dimension:
/// - **`model`**: format preference is model-specific (ToolHop, WildToolBench).
/// - **`role`**: different roles produce different tool-call patterns
///   (Implementer fires `write_file`/`bash`; Auditor uses `grep`/`read_file`).
/// - **`tool_count_bucket`**: Qwen3-coder format-switches above 5 tools;
///   smaller models degrade on large menus.
/// - **`complexity_band`**: `Complex` tasks benefit from NL-to-Format
///   two-pass (Let Me Speak Freely + CRANE); `Fast` tasks don't.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BanditKey {
    /// Model slug (as-recorded).
    pub model: String,
    /// Agent role emitting the call.
    pub role: AgentRole,
    /// Tool-count bucket (0 / 1–2 / 3–5 / 6–10 / 11+ → buckets 0..=4).
    pub tool_count_bucket: u8,
    /// Task complexity band.
    pub complexity_band: TaskComplexityBand,
}

impl BanditKey {
    /// Compute the canonical tool-count bucket (0, 1, 2, 3, 4).
    #[must_use]
    pub const fn bucket_for_tool_count(count: u8) -> u8 {
        match count {
            0 => 0,
            1..=2 => 1,
            3..=5 => 2,
            6..=10 => 3,
            _ => 4,
        }
    }

    /// Construct a key from raw fields.
    #[must_use]
    pub fn new(
        model: impl Into<String>,
        role: AgentRole,
        tool_count: u8,
        complexity_band: TaskComplexityBand,
    ) -> Self {
        Self {
            model: model.into(),
            role,
            tool_count_bucket: Self::bucket_for_tool_count(tool_count),
            complexity_band,
        }
    }
}

// ─── ArmEntry ─────────────────────────────────────────────────────────────

/// One arm's cumulative statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmEntry {
    /// Which format this arm represents.
    pub format: ToolFormat,
    /// Number of times this arm was pulled.
    pub pulls: u32,
    /// Sum of rewards received.
    pub cumulative_reward: f32,
    /// Wall-clock of the most recent pull (ms since epoch).
    pub last_pulled_ms: i64,
    /// Consecutive failures on this arm (resets on any success).
    pub consecutive_failures: u32,
}

impl ArmEntry {
    /// Fresh arm with zero statistics.
    #[must_use]
    pub const fn new(format: ToolFormat) -> Self {
        Self {
            format,
            pulls: 0,
            cumulative_reward: 0.0,
            last_pulled_ms: 0,
            consecutive_failures: 0,
        }
    }

    /// Mean reward (0 for unpulled arms).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn mean_reward(&self) -> f32 {
        if self.pulls == 0 {
            0.0
        } else {
            self.cumulative_reward / self.pulls as f32
        }
    }
}

// ─── FormatBandit trait ───────────────────────────────────────────────────

/// Runtime-agnostic format-selection bandit.
///
/// Implementors may be deterministic (`ProfileBandit`), ε-greedy, UCB,
/// Thompson, Track-and-Stop, etc. The trait's minimum contract:
/// `select` returns a format; `feedback` updates internal state;
/// `arm_table` exposes the current per-arm stats for introspection.
pub trait FormatBandit: Send + Sync {
    /// Select the format to use for a call identified by `key`.
    fn select(&self, key: &BanditKey) -> ToolFormat;

    /// Apply feedback from a completed call.
    fn feedback(&self, key: &BanditKey, chosen: ToolFormat, outcome: &ToolOutcome);

    /// Return a snapshot of the arm table for introspection/TUI display.
    fn arm_table(&self, key: &BanditKey) -> Vec<ArmEntry>;

    /// Human-readable name for logs.
    fn name(&self) -> &str {
        "unnamed_bandit"
    }
}

// ─── ProfileBandit ────────────────────────────────────────────────────────

/// Deterministic bandit that follows a [`ToolFormatProfile`]'s preferred
/// format with fallback-chain demotion after N consecutive failures.
///
/// This is the **day-one default** — it produces trace data and populates
/// the arm table without any exploration overhead. A smarter bandit can
/// replace it later by reading its arm table as warm-start state.
pub struct ProfileBandit {
    /// Factory function: given a key's `model`, produce a profile.
    profile_of: fn(&str) -> ToolFormatProfile,
    arms: RwLock<HashMap<BanditKey, Vec<ArmEntry>>>,
}

impl ProfileBandit {
    /// Construct using the static `profile_for_model` lookup.
    #[must_use]
    pub fn with_static_profiles() -> Self {
        Self {
            profile_of: super::format::profile_for_model,
            arms: RwLock::new(HashMap::new()),
        }
    }

    /// Construct with a custom profile resolver (for tests).
    #[must_use]
    pub fn with_resolver(profile_of: fn(&str) -> ToolFormatProfile) -> Self {
        Self {
            profile_of,
            arms: RwLock::new(HashMap::new()),
        }
    }

    /// Select via walk of the preferred → fallback_chain, demoting past
    /// arms with ≥ `demotion_after_failures` consecutive failures.
    fn pick_format(profile: &ToolFormatProfile, arms: &[ArmEntry]) -> ToolFormat {
        let chain: Vec<&ToolFormat> = std::iter::once(&profile.preferred)
            .chain(profile.fallback_chain.iter())
            .collect();
        for f in &chain {
            let demoted = arms.iter().find(|a| a.format == **f).is_some_and(|a| {
                a.consecutive_failures >= u32::from(profile.demotion_after_failures)
            });
            if !demoted {
                return (*f).clone();
            }
        }
        // Everything demoted — return the last arm (ReAct by convention).
        chain.last().map_or(ToolFormat::ReActText, |f| (*f).clone())
    }

    /// Ensure an arm entry exists for each format in the profile's chain.
    fn ensure_arms(profile: &ToolFormatProfile, arms: &mut Vec<ArmEntry>) {
        for f in std::iter::once(&profile.preferred).chain(profile.fallback_chain.iter()) {
            if !arms.iter().any(|a| &a.format == f) {
                arms.push(ArmEntry::new(f.clone()));
            }
        }
    }
}

impl FormatBandit for ProfileBandit {
    fn select(&self, key: &BanditKey) -> ToolFormat {
        let profile = (self.profile_of)(&key.model);
        let mut arms = self.arms.write();
        let entry = arms.entry(key.clone()).or_default();
        Self::ensure_arms(&profile, entry);
        Self::pick_format(&profile, entry)
    }

    fn feedback(&self, key: &BanditKey, chosen: ToolFormat, outcome: &ToolOutcome) {
        let profile = (self.profile_of)(&key.model);
        let mut arms = self.arms.write();
        let entry = arms.entry(key.clone()).or_default();
        Self::ensure_arms(&profile, entry);
        if let Some(arm) = entry.iter_mut().find(|a| a.format == chosen) {
            arm.pulls = arm.pulls.saturating_add(1);
            arm.cumulative_reward += outcome.reward;
            arm.last_pulled_ms = chrono::Utc::now().timestamp_millis();
            if outcome.success {
                arm.consecutive_failures = 0;
            } else {
                arm.consecutive_failures = arm.consecutive_failures.saturating_add(1);
            }
        }
    }

    fn arm_table(&self, key: &BanditKey) -> Vec<ArmEntry> {
        let arms = self.arms.read();
        arms.get(key).cloned().unwrap_or_default()
    }

    fn name(&self) -> &str {
        "profile_bandit"
    }
}

// ─── EpsilonGreedyBandit ──────────────────────────────────────────────────

/// ε-greedy bandit over the arms initialized from a profile's chain.
///
/// With probability ε, picks a random arm; otherwise, picks the
/// highest-mean-reward arm. Deterministic given a fixed PRNG seed.
/// Useful as a quick learning baseline before the Track-and-Stop bandit
/// lands in `roko-learn`.
pub struct EpsilonGreedyBandit {
    profile_of: fn(&str) -> ToolFormatProfile,
    epsilon: f32,
    seed: RwLock<u64>,
    arms: RwLock<HashMap<BanditKey, Vec<ArmEntry>>>,
}

impl EpsilonGreedyBandit {
    /// Construct with the static profile table and an exploration rate.
    ///
    /// Typical values: `0.1` for production, `0.3` for warm-up phases.
    ///
    /// # Panics
    ///
    /// Panics if `epsilon` is outside `[0, 1]`.
    #[must_use]
    pub fn new(epsilon: f32, seed: u64) -> Self {
        assert!(
            (0.0..=1.0).contains(&epsilon),
            "epsilon must be in [0, 1], got {epsilon}"
        );
        Self {
            profile_of: super::format::profile_for_model,
            epsilon,
            seed: RwLock::new(seed),
            arms: RwLock::new(HashMap::new()),
        }
    }

    /// Deterministic xorshift PRNG for tests.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn next_random(&self) -> f32 {
        let mut seed = self.seed.write();
        let mut x = *seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *seed = x;
        // map to [0, 1)
        (((x >> 11) as f64) / (1u64 << 53) as f64) as f32
    }
}

impl FormatBandit for EpsilonGreedyBandit {
    fn select(&self, key: &BanditKey) -> ToolFormat {
        let profile = (self.profile_of)(&key.model);
        let mut arms_map = self.arms.write();
        let entry = arms_map.entry(key.clone()).or_default();
        ProfileBandit::ensure_arms(&profile, entry);
        let r = self.next_random();
        // Explore.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        if r < self.epsilon {
            let idx = (self.next_random() * entry.len() as f32) as usize;
            let idx = idx.min(entry.len() - 1);
            return entry[idx].format.clone();
        }
        // Exploit: pick the arm with max mean reward; preferred breaks ties.
        let mut best_idx = 0;
        let mut best_mean = f32::NEG_INFINITY;
        for (i, arm) in entry.iter().enumerate() {
            let m = arm.mean_reward();
            if m > best_mean {
                best_mean = m;
                best_idx = i;
            }
        }
        entry[best_idx].format.clone()
    }

    fn feedback(&self, key: &BanditKey, chosen: ToolFormat, outcome: &ToolOutcome) {
        let profile = (self.profile_of)(&key.model);
        let mut arms_map = self.arms.write();
        let entry = arms_map.entry(key.clone()).or_default();
        ProfileBandit::ensure_arms(&profile, entry);
        if let Some(arm) = entry.iter_mut().find(|a| a.format == chosen) {
            arm.pulls = arm.pulls.saturating_add(1);
            arm.cumulative_reward += outcome.reward;
            arm.last_pulled_ms = chrono::Utc::now().timestamp_millis();
            if outcome.success {
                arm.consecutive_failures = 0;
            } else {
                arm.consecutive_failures = arm.consecutive_failures.saturating_add(1);
            }
        }
    }

    fn arm_table(&self, key: &BanditKey) -> Vec<ArmEntry> {
        let arms = self.arms.read();
        arms.get(key).cloned().unwrap_or_default()
    }

    fn name(&self) -> &str {
        "epsilon_greedy_bandit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentRole;

    fn key_claude() -> BanditKey {
        BanditKey::new(
            "claude-sonnet-4-5",
            AgentRole::Implementer,
            4,
            TaskComplexityBand::Standard,
        )
    }

    fn key_qwen() -> BanditKey {
        BanditKey::new(
            "qwen3-32b",
            AgentRole::Implementer,
            4,
            TaskComplexityBand::Standard,
        )
    }

    #[test]
    fn bucket_boundaries() {
        assert_eq!(BanditKey::bucket_for_tool_count(0), 0);
        assert_eq!(BanditKey::bucket_for_tool_count(1), 1);
        assert_eq!(BanditKey::bucket_for_tool_count(2), 1);
        assert_eq!(BanditKey::bucket_for_tool_count(3), 2);
        assert_eq!(BanditKey::bucket_for_tool_count(5), 2);
        assert_eq!(BanditKey::bucket_for_tool_count(6), 3);
        assert_eq!(BanditKey::bucket_for_tool_count(10), 3);
        assert_eq!(BanditKey::bucket_for_tool_count(11), 4);
        assert_eq!(BanditKey::bucket_for_tool_count(255), 4);
    }

    #[test]
    fn arm_entry_mean_zero_on_unpulled() {
        let arm = ArmEntry::new(ToolFormat::HermesJson);
        assert!(arm.mean_reward().abs() < f32::EPSILON);
    }

    #[test]
    fn arm_entry_mean_averages_rewards() {
        let arm = ArmEntry {
            format: ToolFormat::HermesJson,
            pulls: 3,
            cumulative_reward: 2.4,
            last_pulled_ms: 0,
            consecutive_failures: 0,
        };
        assert!((arm.mean_reward() - 0.8).abs() < 0.01);
    }

    #[test]
    fn profile_bandit_returns_preferred_format() {
        let b = ProfileBandit::with_static_profiles();
        assert_eq!(b.select(&key_claude()), ToolFormat::AnthropicBlocks);
        assert_eq!(b.select(&key_qwen()), ToolFormat::HermesJson);
    }

    #[test]
    fn profile_bandit_demotes_after_consecutive_failures() {
        let b = ProfileBandit::with_static_profiles();
        let k = key_qwen();
        // Fail the preferred arm 3 times.
        for _ in 0..3 {
            let chosen = b.select(&k);
            assert_eq!(chosen, ToolFormat::HermesJson);
            b.feedback(
                &k,
                chosen,
                &ToolOutcome::failure(super::super::trace::FailureKind::MalformedJson, 100, 0.001),
            );
        }
        // Now the preferred is demoted → next fallback (JsonMode for qwen3).
        let next = b.select(&k);
        assert_eq!(next, ToolFormat::JsonMode);
    }

    #[test]
    fn profile_bandit_resets_failures_on_success() {
        let b = ProfileBandit::with_static_profiles();
        let k = key_qwen();
        // 2 failures.
        for _ in 0..2 {
            let f = b.select(&k);
            b.feedback(
                &k,
                f,
                &ToolOutcome::failure(super::super::trace::FailureKind::MalformedJson, 100, 0.0),
            );
        }
        // One success resets.
        let f = b.select(&k);
        b.feedback(&k, f, &ToolOutcome::success(50, 0.001));
        // Two more failures — still not demoted (was reset).
        for _ in 0..2 {
            let f = b.select(&k);
            b.feedback(
                &k,
                f,
                &ToolOutcome::failure(super::super::trace::FailureKind::MalformedJson, 100, 0.0),
            );
        }
        assert_eq!(b.select(&k), ToolFormat::HermesJson);
    }

    #[test]
    fn profile_bandit_arm_table_populates_after_feedback() {
        let b = ProfileBandit::with_static_profiles();
        let k = key_claude();
        let f = b.select(&k);
        b.feedback(&k, f, &ToolOutcome::success(100, 0.001).with_reward(0.95));
        let table = b.arm_table(&k);
        assert!(!table.is_empty());
        let preferred = table
            .iter()
            .find(|a| a.format == ToolFormat::AnthropicBlocks)
            .unwrap();
        assert_eq!(preferred.pulls, 1);
        assert!((preferred.cumulative_reward - 0.95).abs() < 0.01);
    }

    #[test]
    fn epsilon_greedy_with_epsilon_zero_is_deterministic() {
        let b = EpsilonGreedyBandit::new(0.0, 42);
        let k = key_claude();
        // With ε=0, always exploit; with all arms unpulled, tie on 0 → first arm (preferred).
        assert_eq!(b.select(&k), ToolFormat::AnthropicBlocks);
        assert_eq!(b.select(&k), ToolFormat::AnthropicBlocks);
    }

    #[test]
    fn epsilon_greedy_favors_high_reward_arm() {
        let b = EpsilonGreedyBandit::new(0.0, 1);
        let k = key_claude();
        // Train the fallback arm (OpenAiJson) to win.
        for _ in 0..5 {
            b.feedback(
                &k,
                ToolFormat::OpenAiJson,
                &ToolOutcome::success(100, 0.001).with_reward(0.9),
            );
        }
        for _ in 0..5 {
            b.feedback(
                &k,
                ToolFormat::AnthropicBlocks,
                &ToolOutcome::failure(super::super::trace::FailureKind::Timeout, 5000, 0.05),
            );
        }
        // OpenAiJson now has higher mean → should be selected.
        assert_eq!(b.select(&k), ToolFormat::OpenAiJson);
    }

    #[test]
    fn epsilon_greedy_random_in_unit_interval() {
        let b = EpsilonGreedyBandit::new(0.1, 12345);
        for _ in 0..1000 {
            let r = b.next_random();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    #[should_panic(expected = "epsilon must be in [0, 1]")]
    fn epsilon_greedy_rejects_invalid_epsilon() {
        let _ = EpsilonGreedyBandit::new(1.5, 1);
    }

    #[test]
    fn bandit_key_serde_roundtrip() {
        let k = key_claude();
        let json = serde_json::to_string(&k).unwrap();
        let decoded: BanditKey = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, k);
    }

    #[test]
    fn arm_entry_serde_roundtrip() {
        let arm = ArmEntry {
            format: ToolFormat::HermesJson,
            pulls: 5,
            cumulative_reward: 4.2,
            last_pulled_ms: 1_700_000_000_000,
            consecutive_failures: 0,
        };
        let json = serde_json::to_string(&arm).unwrap();
        let decoded: ArmEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, arm);
    }
}

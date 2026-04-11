//! Prompt-section effectiveness tracking for gate-to-scaffold feedback.
//!
//! This module tracks whether including a prompt section for a given role
//! correlates with higher gate pass rates. The registry is keyed by
//! `(section_name, role)` and persisted as JSON so prompt assembly can later
//! adjust section priorities using the learned lift.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Default relative path used to persist section-effect snapshots.
pub const DEFAULT_SECTION_EFFECTS_PATH: &str = ".roko/learn/section-effects.json";

/// Suggested priority adjustment for a prompt section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriorityChange {
    /// Increase the section's priority because it improves pass rate.
    Increase,
    /// Decrease the section's priority because it appears to hurt pass rate.
    Decrease,
    /// Keep the section's current priority.
    NoChange,
    /// Not enough trials have accumulated to make a recommendation.
    InsufficientData,
}

/// Inclusion/exclusion outcome statistics for one prompt section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionEffect {
    /// Prompt section name, such as `"workspace_map"`.
    pub section_name: String,
    /// Number of trials where the section was included.
    pub included_trials: u64,
    /// Number of successful trials where the section was included.
    pub included_passes: u64,
    /// Number of trials where the section was excluded.
    pub excluded_trials: u64,
    /// Number of successful trials where the section was excluded.
    pub excluded_passes: u64,
}

impl SectionEffect {
    /// Create a new zeroed tracker for `section_name`.
    #[must_use]
    pub fn new(section_name: impl Into<String>) -> Self {
        Self {
            section_name: section_name.into(),
            included_trials: 0,
            included_passes: 0,
            excluded_trials: 0,
            excluded_passes: 0,
        }
    }

    /// Record one task outcome for the section.
    pub fn record(&mut self, included: bool, passed: bool) {
        if included {
            self.included_trials = self.included_trials.saturating_add(1);
            if passed {
                self.included_passes = self.included_passes.saturating_add(1);
            }
        } else {
            self.excluded_trials = self.excluded_trials.saturating_add(1);
            if passed {
                self.excluded_passes = self.excluded_passes.saturating_add(1);
            }
        }
    }

    /// Pass-rate lift when the section is included versus excluded.
    #[must_use]
    pub fn lift(&self) -> f64 {
        let included_rate = self.included_passes as f64 / self.included_trials.max(1) as f64;
        let excluded_rate = self.excluded_passes as f64 / self.excluded_trials.max(1) as f64;
        included_rate - excluded_rate
    }

    /// Recommend whether the section priority should change.
    #[must_use]
    pub fn recommend_priority_change(&self) -> PriorityChange {
        if self.included_trials < 20 || self.excluded_trials < 5 {
            return PriorityChange::InsufficientData;
        }

        let lift = self.lift();
        if lift > 0.05 {
            PriorityChange::Increase
        } else if lift < -0.02 {
            PriorityChange::Decrease
        } else {
            PriorityChange::NoChange
        }
    }
}

/// Thread-agnostic registry of section effectiveness keyed by `(section, role)`.
#[derive(Debug, Clone, Default)]
pub struct SectionEffectivenessRegistry {
    effects: HashMap<(String, String), SectionEffect>,
}

impl SectionEffectivenessRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the registry from `path`, or return an empty registry if missing
    /// or invalid.
    #[must_use]
    pub fn load_or_new(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| {
                serde_json::from_str::<SectionEffectivenessSnapshot>(&contents).ok()
            })
            .map(Self::from_snapshot)
            .unwrap_or_default()
    }

    /// Save the registry to `path` using an atomic rename.
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let snapshot = self.snapshot();
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Record one included/excluded outcome for `section_name` scoped to `role`.
    pub fn record_outcome(
        &mut self,
        section_name: impl Into<String>,
        role: impl Into<String>,
        included: bool,
        passed: bool,
    ) {
        let section_name = section_name.into();
        let role = role.into();
        let key = (section_name.clone(), role);
        let effect = self
            .effects
            .entry(key)
            .or_insert_with(|| SectionEffect::new(section_name));
        effect.record(included, passed);
    }

    /// Return the tracked effect for one `(section, role)` pair.
    #[must_use]
    pub fn get(&self, section_name: &str, role: &str) -> Option<&SectionEffect> {
        self.effects
            .get(&(section_name.to_owned(), role.to_owned()))
    }

    /// Recommend how prompt assembly should adjust this section's priority.
    #[must_use]
    pub fn recommend_priority_change(&self, section_name: &str, role: &str) -> PriorityChange {
        self.get(section_name, role).map_or(
            PriorityChange::InsufficientData,
            SectionEffect::recommend_priority_change,
        )
    }

    /// Return sections for `role` whose inclusion currently shows positive lift.
    ///
    /// Results are sorted by descending lift, then by section name.
    #[must_use]
    pub fn positive_lift_sections(&self, role: &str) -> Vec<SectionEffect> {
        let mut sections: Vec<_> = self
            .effects
            .iter()
            .filter(|((_, effect_role), effect)| {
                effect_role == role && effect.lift().is_sign_positive() && effect.lift() > 0.0
            })
            .map(|(_, effect)| effect.clone())
            .collect();

        sections.sort_by(|a, b| {
            b.lift()
                .total_cmp(&a.lift())
                .then(a.section_name.cmp(&b.section_name))
        });
        sections
    }

    fn snapshot(&self) -> SectionEffectivenessSnapshot {
        let mut entries: Vec<_> = self
            .effects
            .iter()
            .map(|((section, role), effect)| SectionEffectivenessEntry {
                section_name: section.clone(),
                role: role.clone(),
                effect: effect.clone(),
            })
            .collect();
        entries.sort_by(|a, b| {
            a.role
                .cmp(&b.role)
                .then(a.section_name.cmp(&b.section_name))
        });
        SectionEffectivenessSnapshot { entries }
    }

    fn from_snapshot(snapshot: SectionEffectivenessSnapshot) -> Self {
        let effects = snapshot
            .entries
            .into_iter()
            .map(|entry| ((entry.section_name, entry.role), entry.effect))
            .collect();
        Self { effects }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SectionEffectivenessSnapshot {
    entries: Vec<SectionEffectivenessEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SectionEffectivenessEntry {
    section_name: String,
    role: String,
    effect: SectionEffect,
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SECTION_EFFECTS_PATH, PriorityChange, SectionEffect, SectionEffectivenessRegistry,
    };

    #[test]
    fn section_effectiveness_lift_and_priority_change_follow_thresholds() {
        let mut effect = SectionEffect::new("workspace_map");
        for _ in 0..24 {
            effect.record(true, true);
        }
        for _ in 0..6 {
            effect.record(true, false);
        }
        for _ in 0..2 {
            effect.record(false, true);
        }
        for _ in 0..8 {
            effect.record(false, false);
        }

        assert!((effect.lift() - 0.6).abs() < 1e-12);
        assert_eq!(effect.recommend_priority_change(), PriorityChange::Increase);
    }

    #[test]
    fn section_effectiveness_returns_insufficient_data_until_thresholds_are_met() {
        let mut effect = SectionEffect::new("prd_extract");
        for _ in 0..19 {
            effect.record(true, true);
        }
        for _ in 0..5 {
            effect.record(false, false);
        }

        assert_eq!(
            effect.recommend_priority_change(),
            PriorityChange::InsufficientData
        );
    }

    #[test]
    fn section_effectiveness_registry_identifies_positive_lift_after_50_events() {
        let mut registry = SectionEffectivenessRegistry::new();

        for _ in 0..30 {
            registry.record_outcome("workspace_map", "Implementer", true, true);
        }
        for _ in 0..10 {
            registry.record_outcome("workspace_map", "Implementer", true, false);
        }
        for _ in 0..5 {
            registry.record_outcome("workspace_map", "Implementer", false, true);
        }
        for _ in 0..5 {
            registry.record_outcome("workspace_map", "Implementer", false, false);
        }

        for _ in 0..20 {
            registry.record_outcome("prd_extract", "Implementer", true, false);
        }
        for _ in 0..5 {
            registry.record_outcome("prd_extract", "Implementer", false, true);
        }
        for _ in 0..5 {
            registry.record_outcome("prd_extract", "Implementer", false, false);
        }

        let positive = registry.positive_lift_sections("Implementer");
        assert_eq!(positive.len(), 1);
        assert_eq!(positive[0].section_name, "workspace_map");
        assert!(positive[0].lift() > 0.0);
        assert_eq!(
            registry.recommend_priority_change("workspace_map", "Implementer"),
            PriorityChange::Increase
        );
        assert_eq!(
            registry.recommend_priority_change("prd_extract", "Implementer"),
            PriorityChange::Decrease
        );
    }

    #[test]
    fn section_effectiveness_registry_persists_and_loads() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join(DEFAULT_SECTION_EFFECTS_PATH);

        let mut registry = SectionEffectivenessRegistry::new();
        registry.record_outcome("workspace_map", "Reviewer", true, true);
        registry.record_outcome("workspace_map", "Reviewer", false, false);
        registry.save(&path).expect("save");

        let loaded = SectionEffectivenessRegistry::load_or_new(&path);
        let effect = loaded
            .get("workspace_map", "Reviewer")
            .expect("persisted effect");

        assert_eq!(effect.included_trials, 1);
        assert_eq!(effect.included_passes, 1);
        assert_eq!(effect.excluded_trials, 1);
        assert_eq!(effect.excluded_passes, 0);
    }
}

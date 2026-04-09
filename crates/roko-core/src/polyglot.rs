//! Polyglot project support — multi-language workspace detection.
//!
//! Many real projects span multiple languages (e.g. Rust backend + `TypeScript`
//! frontend, Solidity contracts + `TypeScript` tests). [`detect_polyglot`]
//! identifies all languages present and picks the primary one.

use crate::project::{DetectedBuildSystem, Language};
use serde::{Deserialize, Serialize};

// ─── PolyglotProject ─────────────────────────────────────────────────────

/// A project that may contain multiple languages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolyglotProject {
    /// The primary (most important) language. Determined by detection order
    /// priority.
    pub primary: Language,
    /// Secondary languages detected in the project.
    pub secondary: Vec<Language>,
    /// All build systems detected.
    pub build_systems: Vec<DetectedBuildSystem>,
}

impl PolyglotProject {
    /// Whether this project uses more than one language.
    #[must_use]
    pub fn is_polyglot(&self) -> bool {
        !self.secondary.is_empty()
    }

    /// All languages present (primary + secondary).
    #[must_use]
    pub fn all_languages(&self) -> Vec<Language> {
        let mut all = vec![self.primary];
        all.extend_from_slice(&self.secondary);
        all
    }
}

// ─── Detection ───────────────────────────────────────────────────────────

/// A detection rule mapping marker file -> language + build system.
struct PolyRule {
    marker: &'static str,
    language: Language,
    build_system: DetectedBuildSystem,
}

/// Ordered priority list of detection rules. First match becomes primary.
const POLY_RULES: &[PolyRule] = &[
    PolyRule {
        marker: "Cargo.toml",
        language: Language::Rust,
        build_system: DetectedBuildSystem::Cargo,
    },
    PolyRule {
        marker: "go.mod",
        language: Language::Go,
        build_system: DetectedBuildSystem::Go,
    },
    PolyRule {
        marker: "foundry.toml",
        language: Language::Solidity,
        build_system: DetectedBuildSystem::Forge,
    },
    PolyRule {
        marker: "pyproject.toml",
        language: Language::Python,
        build_system: DetectedBuildSystem::Python,
    },
    PolyRule {
        marker: "setup.py",
        language: Language::Python,
        build_system: DetectedBuildSystem::Python,
    },
    PolyRule {
        marker: "package.json",
        language: Language::TypeScript,
        build_system: DetectedBuildSystem::Npm,
    },
];

/// Detect a polyglot project from a list of file names in the project root.
///
/// Returns a [`PolyglotProject`] with primary language (first matching rule)
/// and all other detected languages as secondary. Duplicate languages are
/// deduplicated (e.g. `setup.py` and `pyproject.toml` both -> Python once).
#[must_use]
pub fn detect_polyglot(file_names: &[&str]) -> PolyglotProject {
    let mut primary = Language::Unknown;
    let mut languages = Vec::new();
    let mut build_systems = Vec::new();

    for rule in POLY_RULES {
        if file_names.contains(&rule.marker) {
            // Deduplicate: skip if we already have this language.
            if !languages.contains(&rule.language) {
                languages.push(rule.language);
            }
            if !build_systems.contains(&rule.build_system) {
                build_systems.push(rule.build_system);
            }
            if primary == Language::Unknown {
                primary = rule.language;
            }
        }
    }

    // Separate primary from secondary.
    let secondary: Vec<Language> = languages.into_iter().filter(|l| *l != primary).collect();

    PolyglotProject {
        primary,
        secondary,
        build_systems,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_language_not_polyglot() {
        let p = detect_polyglot(&["Cargo.toml", "src"]);
        assert_eq!(p.primary, Language::Rust);
        assert!(!p.is_polyglot());
        assert!(p.secondary.is_empty());
        assert_eq!(p.build_systems, vec![DetectedBuildSystem::Cargo]);
    }

    #[test]
    fn rust_plus_typescript() {
        let p = detect_polyglot(&["Cargo.toml", "package.json"]);
        assert_eq!(p.primary, Language::Rust);
        assert!(p.is_polyglot());
        assert!(p.secondary.contains(&Language::TypeScript));
        assert!(p.build_systems.contains(&DetectedBuildSystem::Cargo));
        assert!(p.build_systems.contains(&DetectedBuildSystem::Npm));
    }

    #[test]
    fn solidity_plus_typescript() {
        let p = detect_polyglot(&["foundry.toml", "package.json"]);
        assert_eq!(p.primary, Language::Solidity);
        assert!(p.secondary.contains(&Language::TypeScript));
    }

    #[test]
    fn all_languages_includes_both() {
        let p = detect_polyglot(&["Cargo.toml", "package.json"]);
        let all = p.all_languages();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], Language::Rust);
        assert_eq!(all[1], Language::TypeScript);
    }

    #[test]
    fn empty_file_list() {
        let p = detect_polyglot(&[]);
        assert_eq!(p.primary, Language::Unknown);
        assert!(!p.is_polyglot());
        assert!(p.build_systems.is_empty());
    }

    #[test]
    fn python_deduplication() {
        // Both pyproject.toml and setup.py -> only one Python entry.
        let p = detect_polyglot(&["pyproject.toml", "setup.py"]);
        assert_eq!(p.primary, Language::Python);
        assert!(!p.is_polyglot());
        assert_eq!(p.build_systems, vec![DetectedBuildSystem::Python]);
    }

    #[test]
    fn three_languages() {
        let p = detect_polyglot(&["Cargo.toml", "go.mod", "package.json"]);
        assert_eq!(p.primary, Language::Rust);
        assert_eq!(p.secondary.len(), 2);
        assert!(p.secondary.contains(&Language::Go));
        assert!(p.secondary.contains(&Language::TypeScript));
    }

    #[test]
    fn priority_order_rust_first() {
        // Even if package.json appears first in the slice, Rust wins by rule priority.
        let p = detect_polyglot(&["package.json", "Cargo.toml"]);
        assert_eq!(p.primary, Language::Rust);
    }
}

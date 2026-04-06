//! Project detection — identify language and build system from file names.
//!
//! [`ProjectDetector`] works without I/O: callers provide a list of file names
//! present in the project root and get back a [`ProjectInfo`] describing the
//! detected language and build system.

use serde::{Deserialize, Serialize};

// ─── Language enum ───────────────────────────────────────────────────────

/// Programming language detected from project markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Language {
    /// Rust (marker: `Cargo.toml`).
    Rust,
    /// `TypeScript` / `JavaScript` (marker: `package.json`).
    TypeScript,
    /// Go (marker: `go.mod`).
    Go,
    /// Python (marker: `pyproject.toml` or `setup.py`).
    Python,
    /// Solidity (marker: `foundry.toml`).
    Solidity,
    /// Unable to determine the language.
    Unknown,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Go => "go",
            Self::Python => "python",
            Self::Solidity => "solidity",
            Self::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

// ─── DetectedBuildSystem ─────────────────────────────────────────────────

/// Build system tag detected from project markers.
///
/// This is a simple enum (distinct from the `build::BuildSystem` trait).
/// It tells callers *which* build system was detected; the trait provides
/// the runnable commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DetectedBuildSystem {
    /// Cargo (Rust).
    Cargo,
    /// npm / yarn / pnpm (Node).
    Npm,
    /// Go toolchain.
    Go,
    /// Python (pip / poetry / uv).
    Python,
    /// Foundry (forge).
    Forge,
    /// No recognisable build system.
    Unknown,
}

impl std::fmt::Display for DetectedBuildSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Go => "go",
            Self::Python => "python",
            Self::Forge => "forge",
            Self::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

// ─── ProjectInfo ─────────────────────────────────────────────────────────

/// Information about a detected project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Primary language of the project.
    pub language: Language,
    /// Detected build system.
    pub build_system: DetectedBuildSystem,
    /// Whether the project appears to be a workspace / monorepo.
    pub has_workspace: bool,
}

// ─── Detection rules ─────────────────────────────────────────────────────

/// A detection rule: marker file name -> language + build system.
struct Rule {
    marker: &'static str,
    language: Language,
    build_system: DetectedBuildSystem,
}

/// Ordered list of detection rules. First match wins for primary language.
const RULES: &[Rule] = &[
    Rule {
        marker: "Cargo.toml",
        language: Language::Rust,
        build_system: DetectedBuildSystem::Cargo,
    },
    Rule {
        marker: "go.mod",
        language: Language::Go,
        build_system: DetectedBuildSystem::Go,
    },
    Rule {
        marker: "foundry.toml",
        language: Language::Solidity,
        build_system: DetectedBuildSystem::Forge,
    },
    Rule {
        marker: "pyproject.toml",
        language: Language::Python,
        build_system: DetectedBuildSystem::Python,
    },
    Rule {
        marker: "setup.py",
        language: Language::Python,
        build_system: DetectedBuildSystem::Python,
    },
    Rule {
        marker: "package.json",
        language: Language::TypeScript,
        build_system: DetectedBuildSystem::Npm,
    },
];

/// Workspace marker files per language.
const WORKSPACE_MARKERS: &[(&str, Language)] = &[
    ("Cargo.toml", Language::Rust),     // checked for [workspace] below
    ("pnpm-workspace.yaml", Language::TypeScript),
    ("lerna.json", Language::TypeScript),
    ("go.work", Language::Go),
];

/// Detect project information from a list of file names in the project root.
///
/// This function is I/O-free: it pattern-matches known marker file names.
///
/// # Workspace detection
///
/// For most languages, the presence of specific files (e.g. `pnpm-workspace.yaml`,
/// `go.work`) indicates a workspace. For Rust, `Cargo.toml` is always present;
/// use [`detect_from_files_with_cargo_toml`] if you have the file contents and
/// want accurate workspace detection.
#[must_use]
pub fn detect_from_files(file_names: &[&str]) -> ProjectInfo {
    let mut language = Language::Unknown;
    let mut build_system = DetectedBuildSystem::Unknown;

    for rule in RULES {
        if file_names.contains(&rule.marker) {
            language = rule.language;
            build_system = rule.build_system;
            break;
        }
    }

    let has_workspace = detect_workspace(file_names, language);

    ProjectInfo {
        language,
        build_system,
        has_workspace,
    }
}

/// Like [`detect_from_files`] but accepts optional Cargo.toml contents to
/// check for `[workspace]` section. For non-Rust projects the `cargo_toml`
/// parameter is ignored.
#[must_use]
pub fn detect_from_files_with_cargo_toml(
    file_names: &[&str],
    cargo_toml: Option<&str>,
) -> ProjectInfo {
    let mut info = detect_from_files(file_names);

    // Refine workspace detection for Rust: look for [workspace] section.
    if info.language == Language::Rust {
        if let Some(contents) = cargo_toml {
            info.has_workspace = contents.contains("[workspace]");
        }
    }

    info
}

/// Heuristic workspace detection from file names.
fn detect_workspace(file_names: &[&str], language: Language) -> bool {
    for &(marker, lang) in WORKSPACE_MARKERS {
        if lang == language && file_names.contains(&marker) {
            // For Rust, Cargo.toml is always there; we can't tell workspace
            // from the file list alone. Default to false (use the _with_cargo_toml
            // variant for accuracy).
            if lang == Language::Rust {
                continue;
            }
            return true;
        }
    }
    false
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_rust_project() {
        let info = detect_from_files(&["Cargo.toml", "src", "README.md"]);
        assert_eq!(info.language, Language::Rust);
        assert_eq!(info.build_system, DetectedBuildSystem::Cargo);
    }

    #[test]
    fn detect_typescript_project() {
        let info = detect_from_files(&["package.json", "tsconfig.json", "src"]);
        assert_eq!(info.language, Language::TypeScript);
        assert_eq!(info.build_system, DetectedBuildSystem::Npm);
    }

    #[test]
    fn detect_go_project() {
        let info = detect_from_files(&["go.mod", "go.sum", "main.go"]);
        assert_eq!(info.language, Language::Go);
        assert_eq!(info.build_system, DetectedBuildSystem::Go);
    }

    #[test]
    fn detect_python_project() {
        let info = detect_from_files(&["pyproject.toml", "src"]);
        assert_eq!(info.language, Language::Python);
        assert_eq!(info.build_system, DetectedBuildSystem::Python);
    }

    #[test]
    fn detect_python_setup_py() {
        let info = detect_from_files(&["setup.py", "mypackage"]);
        assert_eq!(info.language, Language::Python);
    }

    #[test]
    fn detect_solidity_project() {
        let info = detect_from_files(&["foundry.toml", "src", "test"]);
        assert_eq!(info.language, Language::Solidity);
        assert_eq!(info.build_system, DetectedBuildSystem::Forge);
    }

    #[test]
    fn detect_unknown_project() {
        let info = detect_from_files(&["README.md", "LICENSE"]);
        assert_eq!(info.language, Language::Unknown);
        assert_eq!(info.build_system, DetectedBuildSystem::Unknown);
    }

    #[test]
    fn detect_empty_file_list() {
        let info = detect_from_files(&[]);
        assert_eq!(info.language, Language::Unknown);
    }

    #[test]
    fn first_match_wins_cargo_over_package_json() {
        // Project has both Cargo.toml and package.json (e.g. Rust + wasm-pack).
        let info = detect_from_files(&["Cargo.toml", "package.json"]);
        assert_eq!(info.language, Language::Rust);
    }

    #[test]
    fn workspace_detection_typescript() {
        let info = detect_from_files(&["package.json", "pnpm-workspace.yaml"]);
        assert_eq!(info.language, Language::TypeScript);
        assert!(info.has_workspace);
    }

    #[test]
    fn workspace_detection_go() {
        let info = detect_from_files(&["go.mod", "go.work"]);
        assert_eq!(info.language, Language::Go);
        assert!(info.has_workspace);
    }

    #[test]
    fn workspace_detection_rust_needs_cargo_toml_contents() {
        // Without contents, Rust workspace defaults to false.
        let info = detect_from_files(&["Cargo.toml"]);
        assert!(!info.has_workspace);

        // With contents containing [workspace]:
        let info2 = detect_from_files_with_cargo_toml(
            &["Cargo.toml"],
            Some("[workspace]\nmembers = [\"crates/*\"]\n"),
        );
        assert!(info2.has_workspace);
    }

    #[test]
    fn workspace_detection_rust_no_workspace_section() {
        let info = detect_from_files_with_cargo_toml(
            &["Cargo.toml"],
            Some("[package]\nname = \"my-app\"\n"),
        );
        assert!(!info.has_workspace);
    }

    #[test]
    fn language_display() {
        assert_eq!(Language::Rust.to_string(), "rust");
        assert_eq!(Language::Unknown.to_string(), "unknown");
    }

    #[test]
    fn build_system_display() {
        assert_eq!(DetectedBuildSystem::Cargo.to_string(), "cargo");
        assert_eq!(DetectedBuildSystem::Unknown.to_string(), "unknown");
    }
}

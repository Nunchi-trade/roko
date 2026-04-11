//! Types for gates generated from acceptance criteria rather than hand-written.
//!
//! These checks are consumed by gate orchestration only. The implementer agent
//! never sees the generated symbols or test bodies.

use crate::symbol_gate::{
    SymbolKind, Visibility, first_identifier, normalize_path, parse_visibility, rust_module_path,
    skip_modifiers,
};
use roko_core::Verdict;
use std::path::{Path, PathBuf};

const GENERATED_SYMBOL_GATE: &str = "generated_symbol";

/// Gate-generation errors reuse the crate's canonical core error type.
pub type GateError = roko_core::RokoError;

/// Produces verifier artifacts from acceptance criteria and task context.
pub trait GateGenerator: Send + Sync {
    /// Generate verification artifacts from acceptance criteria.
    fn generate(
        &self,
        acceptance_criteria: &str,
        task_context: &str,
    ) -> Result<Vec<GeneratedCheck>, GateError>;
}

/// A gate artifact synthesized from plan acceptance criteria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedCheck {
    /// Assert that a symbol exists with the expected kind, visibility, and module.
    SymbolExists {
        /// Symbol identifier, e.g. `RateLimiter`.
        name: String,
        /// Rust item kind, e.g. `struct`, `fn`, `trait`, `enum`.
        kind: String,
        /// Expected visibility, e.g. `pub`, `pub(crate)`, or empty for private.
        visibility: String,
        /// Canonical Rust module path where the symbol should live.
        module_path: String,
    },
    /// A complete generated test case to run at a specific verification rung.
    TestCase {
        /// Stable test function name.
        name: String,
        /// Complete test source, including `#[test]`.
        code: String,
        /// Verification rung for this test, e.g. 3 for behavioral, 4 for property.
        rung: u32,
    },
}

/// Verify that a generated symbol expectation exists in the workspace.
#[must_use]
pub fn check_symbol_exists(check: &GeneratedCheck, workspace: &Path) -> Verdict {
    match check {
        GeneratedCheck::SymbolExists {
            name,
            kind,
            visibility,
            module_path,
        } => {
            let Some(expected_kind) = SymbolKind::from_keyword(kind.trim()) else {
                return Verdict::fail(
                    GENERATED_SYMBOL_GATE,
                    format!("unsupported symbol kind `{kind}`"),
                );
            };
            let Some(expected_visibility) = parse_expected_visibility(visibility) else {
                return Verdict::fail(
                    GENERATED_SYMBOL_GATE,
                    format!("unsupported visibility `{visibility}`"),
                );
            };

            let expected_path = normalize_path(module_path);
            let expected_head = render_symbol_head(expected_visibility, expected_kind, name);
            let symbols = discover_workspace_symbols(workspace);
            let exact_matches: Vec<&WorkspaceSymbol> = symbols
                .iter()
                .filter(|symbol| symbol.name == *name && symbol.module_path == expected_path)
                .collect();

            if exact_matches.len() > 1 {
                let locations = exact_matches
                    .iter()
                    .map(|symbol| symbol.location())
                    .collect::<Vec<_>>()
                    .join(", ");
                let reason = format!("ambiguous {expected_head} at {expected_path} ({locations})");
                return Verdict::fail(GENERATED_SYMBOL_GATE, &reason)
                    .with_error_digest(reason.clone())
                    .with_detail(reason);
            }

            if let Some(found) = exact_matches.first() {
                if found.kind != expected_kind {
                    let reason = format!(
                        "wrong kind for {expected_head} at {expected_path}: found {} at {}",
                        found.kind.as_str(),
                        found.location()
                    );
                    return Verdict::fail(GENERATED_SYMBOL_GATE, &reason)
                        .with_error_digest(reason.clone())
                        .with_detail(reason);
                }

                if found.visibility != expected_visibility {
                    let reason = format!(
                        "wrong visibility for {expected_head} at {expected_path}: found {} at {}",
                        found.visibility.as_str(),
                        found.location()
                    );
                    return Verdict::fail(GENERATED_SYMBOL_GATE, &reason)
                        .with_error_digest(reason.clone())
                        .with_detail(reason);
                }

                let detail = format!(
                    "found {expected_head} at {expected_path} ({})",
                    found.location()
                );
                Verdict::pass(GENERATED_SYMBOL_GATE).with_detail(detail)
            } else if let Some(found) = symbols.iter().find(|symbol| symbol.name == *name) {
                let reason = format!(
                    "wrong path for {expected_head}: expected {expected_path}, found {} at {}",
                    found.module_path,
                    found.location()
                );
                Verdict::fail(GENERATED_SYMBOL_GATE, &reason)
                    .with_error_digest(reason.clone())
                    .with_detail(reason)
            } else {
                let reason = format!("missing {expected_head} at {expected_path}");
                Verdict::fail(GENERATED_SYMBOL_GATE, &reason)
                    .with_error_digest(reason.clone())
                    .with_detail(reason)
            }
        }
        GeneratedCheck::TestCase { .. } => {
            Verdict::pass(GENERATED_SYMBOL_GATE).with_detail("not a symbol check")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSymbol {
    name: String,
    kind: SymbolKind,
    visibility: Visibility,
    module_path: String,
    file: PathBuf,
    line: usize,
}

impl WorkspaceSymbol {
    fn location(&self) -> String {
        format!("{}:{}", self.file.display(), self.line)
    }
}

fn parse_expected_visibility(visibility: &str) -> Option<Visibility> {
    match visibility.trim() {
        "" | "private" => Some(Visibility::Private),
        "pub" => Some(Visibility::Pub),
        "pub(crate)" | "pub(super)" => Some(Visibility::PubCrate),
        value if value.starts_with("pub(in ") && value.ends_with(')') => Some(Visibility::PubCrate),
        _ => None,
    }
}

fn render_symbol_head(visibility: Visibility, kind: SymbolKind, name: &str) -> String {
    match visibility {
        Visibility::Pub => format!("pub {} {name}", kind.as_str()),
        Visibility::PubCrate => format!("pub(crate) {} {name}", kind.as_str()),
        Visibility::Private => format!("{} {name}", kind.as_str()),
    }
}

fn discover_workspace_symbols(workspace: &Path) -> Vec<WorkspaceSymbol> {
    let mut out = Vec::new();
    let files = collect_rust_files(workspace);

    for file in files {
        let Some(module_path) = workspace_module_path(workspace, &file) else {
            continue;
        };
        let Ok(source) = std::fs::read_to_string(&file) else {
            continue;
        };
        let display_path = file
            .strip_prefix(workspace)
            .map_or_else(|_| file.clone(), Path::to_path_buf);

        for (index, raw) in source.lines().enumerate() {
            let line = raw.trim_start();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let (visibility, rest) = parse_visibility(line);
            let rest = skip_modifiers(rest);
            let Some((kind_kw, tail)) = rest.split_once(char::is_whitespace) else {
                continue;
            };
            let Some(kind) = SymbolKind::from_keyword(kind_kw) else {
                continue;
            };
            let Some(name) = first_identifier(tail) else {
                continue;
            };

            out.push(WorkspaceSymbol {
                name: name.to_string(),
                kind,
                visibility,
                module_path: module_path.clone(),
                file: display_path.clone(),
                line: index + 1,
            });
        }
    }

    out
}

fn collect_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            if path.is_dir() {
                if matches!(name, "target" | ".git" | "node_modules") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }

    out
}

fn workspace_module_path(workspace: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(workspace).ok()?;
    let components = relative
        .iter()
        .map(|component| component.to_str())
        .collect::<Option<Vec<_>>>()?;

    match components.as_slice() {
        ["crates", crate_dir, "src", ..] | ["apps", crate_dir, "src", ..] => {
            let crate_name = crate_dir.replace('-', "_");
            let root = workspace.join(components[0]).join(crate_dir).join("src");
            let suffix = rust_module_path(file, &root)?;
            Some(prefix_module_path(&crate_name, &suffix))
        }
        ["src", ..] => {
            let crate_name = workspace
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.replace('-', "_"))?;
            let suffix = rust_module_path(file, &workspace.join("src"))?;
            Some(prefix_module_path(&crate_name, &suffix))
        }
        _ => None,
    }
}

fn prefix_module_path(prefix: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}::{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, body).expect("write source");
    }

    struct StaticGenerator;

    impl GateGenerator for StaticGenerator {
        fn generate(
            &self,
            acceptance_criteria: &str,
            task_context: &str,
        ) -> Result<Vec<GeneratedCheck>, GateError> {
            let _ = (acceptance_criteria, task_context);
            Ok(vec![
                GeneratedCheck::SymbolExists {
                    name: "RateLimiter".into(),
                    kind: "struct".into(),
                    visibility: "pub".into(),
                    module_path: "roko_gate::rate_limit".into(),
                },
                GeneratedCheck::TestCase {
                    name: "gen_rate_limiter_allows_first_call".into(),
                    code: "#[test]\nfn gen_rate_limiter_allows_first_call() {}".into(),
                    rung: 3,
                },
            ])
        }
    }

    #[test]
    fn generated_check_types() {
        let generator: &dyn GateGenerator = &StaticGenerator;
        let checks = generator
            .generate("must expose a rate limiter", "task 2K.31")
            .unwrap();

        assert_eq!(checks.len(), 2);

        match &checks[0] {
            GeneratedCheck::SymbolExists {
                name,
                kind,
                visibility,
                module_path,
            } => {
                assert_eq!(name, "RateLimiter");
                assert_eq!(kind, "struct");
                assert_eq!(visibility, "pub");
                assert_eq!(module_path, "roko_gate::rate_limit");
            }
            GeneratedCheck::TestCase { .. } => panic!("expected symbol check"),
        }

        match &checks[1] {
            GeneratedCheck::TestCase { name, code, rung } => {
                assert_eq!(name, "gen_rate_limiter_allows_first_call");
                assert!(code.starts_with("#[test]"));
                assert_eq!(*rung, 3);
            }
            GeneratedCheck::SymbolExists { .. } => panic!("expected test case"),
        }
    }

    #[test]
    fn symbol_existence_gate_passes_for_matching_symbol() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "crates/demo-crate/src/rate_limit.rs",
            "pub struct Foo {}\n",
        );
        let check = GeneratedCheck::SymbolExists {
            name: "Foo".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            module_path: "demo_crate::rate_limit".into(),
        };

        let verdict = check_symbol_exists(&check, tmp.path());

        assert!(verdict.passed, "expected pass, got {verdict:?}");
        let detail = verdict.detail.expect("detail");
        assert!(detail.contains("pub struct Foo"));
        assert!(detail.contains("demo_crate::rate_limit"));
        assert!(detail.contains("crates/demo-crate/src/rate_limit.rs:1"));
    }

    #[test]
    fn symbol_existence_gate_fails_for_missing_symbol() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "crates/demo-crate/src/lib.rs",
            "pub struct Other {}\n",
        );
        let check = GeneratedCheck::SymbolExists {
            name: "Foo".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            module_path: "demo_crate::rate_limit".into(),
        };

        let verdict = check_symbol_exists(&check, tmp.path());

        assert!(!verdict.passed, "expected fail, got {verdict:?}");
        assert!(verdict.reason.contains("missing pub struct Foo"));
        assert!(verdict.reason.contains("demo_crate::rate_limit"));
        assert_eq!(
            verdict.error_digest.as_deref(),
            Some(verdict.reason.as_str())
        );
    }

    #[test]
    fn symbol_existence_gate_fails_for_wrong_visibility() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "crates/demo-crate/src/rate_limit.rs",
            "struct Foo {}\n",
        );
        let check = GeneratedCheck::SymbolExists {
            name: "Foo".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            module_path: "demo_crate::rate_limit".into(),
        };

        let verdict = check_symbol_exists(&check, tmp.path());

        assert!(!verdict.passed, "expected fail, got {verdict:?}");
        let detail = verdict.detail.expect("detail");
        assert!(detail.contains("wrong visibility"));
        assert!(detail.contains("private"));
        assert!(detail.contains("crates/demo-crate/src/rate_limit.rs:1"));
    }

    #[test]
    fn symbol_existence_gate_reports_found_location_for_wrong_path() {
        let tmp = TempDir::new().expect("tempdir");
        write_file(
            tmp.path(),
            "crates/demo-crate/src/elsewhere.rs",
            "pub struct Foo {}\n",
        );
        let check = GeneratedCheck::SymbolExists {
            name: "Foo".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            module_path: "demo_crate::rate_limit".into(),
        };

        let verdict = check_symbol_exists(&check, tmp.path());

        assert!(!verdict.passed, "expected fail, got {verdict:?}");
        let detail = verdict.detail.expect("detail");
        assert!(detail.contains("wrong path"));
        assert!(detail.contains("demo_crate::elsewhere"));
        assert!(detail.contains("crates/demo-crate/src/elsewhere.rs:1"));
    }
}

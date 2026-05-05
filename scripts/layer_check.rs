//! Layer dependency and architecture negative-pattern checker.

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A single layer violation: crate at layer N depends on crate at layer M where M > N.
#[derive(Debug)]
#[allow(missing_docs)]
pub struct LayerViolation {
    pub from_crate: String,
    pub from_layer: u32,
    pub to_crate: String,
    pub to_layer: u32,
}

impl std::fmt::Display for LayerViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "L{} {} -> L{} {} (higher layer dependency)",
            self.from_layer, self.from_crate, self.to_layer, self.to_crate
        )
    }
}

#[derive(Debug)]
struct ArchitectureFinding {
    path: PathBuf,
    line: Option<usize>,
    message: String,
}

/// Extract layer assignments from workspace metadata.
fn extract_layers(metadata: &cargo_metadata::Metadata) -> HashMap<String, u32> {
    let mut layers = HashMap::new();
    for package in &metadata.packages {
        if let Some(roko_meta) = package.metadata.get("roko") {
            if let Some(layer) = roko_meta
                .get("layer")
                .and_then(serde_json::value::Value::as_u64)
                .and_then(|layer| u32::try_from(layer).ok())
            {
                layers.insert(package.name.as_ref().to_string(), layer);
            }
        }
    }
    layers
}

/// Check all workspace dependency edges for layer violations.
fn check_layers(metadata: &cargo_metadata::Metadata) -> Vec<LayerViolation> {
    let layers = extract_layers(metadata);
    let workspace_members: HashSet<_> = metadata
        .workspace_members
        .iter()
        .map(ToString::to_string)
        .collect();

    let mut violations = Vec::new();

    for package in &metadata.packages {
        if !workspace_members.contains(&package.id.to_string()) {
            continue;
        }

        let from_crate = package.name.as_ref();
        let Some(&from_layer) = layers.get(from_crate) else {
            continue;
        };

        for dep in &package.dependencies {
            let Some(&to_layer) = layers.get(dep.name.as_str()) else {
                continue;
            };
            if from_layer < to_layer {
                violations.push(LayerViolation {
                    from_crate: from_crate.to_string(),
                    from_layer,
                    to_crate: dep.name.clone(),
                    to_layer,
                });
            }
        }
    }

    violations.sort_by(|a, b| {
        a.from_layer
            .cmp(&b.from_layer)
            .then(a.from_crate.cmp(&b.from_crate))
            .then(a.to_crate.cmp(&b.to_crate))
    });

    violations
}

fn rust_files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).with_context(|| format!("read {}", path.display()))? {
            let entry = entry?;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                stack.push(entry_path);
            } else if entry_path.extension().is_some_and(|ext| ext == "rs") {
                files.push(entry_path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn line_location(path: &Path, line: Option<usize>) -> String {
    line.map_or_else(
        || path.display().to_string(),
        |line| format!("{}:{line}", path.display()),
    )
}

fn push_finding(
    findings: &mut Vec<ArchitectureFinding>,
    path: &Path,
    line: Option<usize>,
    message: impl Into<String>,
) {
    findings.push(ArchitectureFinding {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    });
}

fn check_duplicate_foundation_traits(
    root: &Path,
    findings: &mut Vec<ArchitectureFinding>,
) -> Result<()> {
    let patterns = [
        "pub trait AffectPolicy",
        "pub trait DispatchModulation",
        "pub struct DispatchModulation",
        "pub trait AffectContext",
    ];
    let crates_dir = root.join("crates");
    let entries =
        fs::read_dir(&crates_dir).with_context(|| format!("read {}", crates_dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let crate_path = entry.path();
        if !crate_path.is_dir() {
            continue;
        }
        // Skip roko-core — it is the canonical source for these definitions.
        if crate_path.file_name().is_some_and(|n| n == "roko-core") {
            continue;
        }
        for path in rust_files_under(&crate_path.join("src"))? {
            let contents =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            for (idx, line) in contents.lines().enumerate() {
                for needle in patterns {
                    if line.contains(needle) {
                        push_finding(
                            findings,
                            &path,
                            Some(idx + 1),
                            format!(
                                "duplicate foundation type `{needle}` found outside roko-core; import the canonical type from `roko_core::foundation` instead"
                            ),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn check_debug_event_logging(root: &Path, findings: &mut Vec<ArchitectureFinding>) -> Result<()> {
    let path = root.join("crates/roko-runtime/src/jsonl_logger.rs");
    let contents = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let debug_marker = ['{', ':', '?', '}'].iter().collect::<String>();
    let debug_format_call = format!("format!(\"{debug_marker}\"");
    let event_debug_marker = ['"', '{', 'e', 'v', 'e', 'n', 't', ':', '?', '}', '"']
        .iter()
        .collect::<String>();
    for (idx, line) in contents.lines().enumerate() {
        let debug_format = line.contains(&debug_format_call)
            || line.contains(&event_debug_marker)
            || ((line.contains("write!(") || line.contains("writeln!(")) && line.contains(":?"));
        if debug_format {
            push_finding(
                findings,
                &path,
                Some(idx + 1),
                "debug formatting used for runtime event logging; serialize RuntimeEventEnvelope with `serde_json::to_string` instead",
            );
        }
    }
    Ok(())
}

fn is_legacy_cfg_line(line: &str) -> bool {
    line.contains("cfg") && line.contains("feature") && line.contains("legacy-orchestrate")
}

fn legacy_gated_lines(contents: &str) -> HashSet<usize> {
    let mut gated = HashSet::new();

    // Detect a file-level `#![cfg(feature = "legacy-orchestrate")]` attribute in the
    // first 10 lines.  When present the entire file is conditionally compiled as a
    // unit, so every line counts as gated.
    let is_file_level_gated = contents
        .lines()
        .take(10)
        .any(|l| l.trim().starts_with("#![cfg(") && is_legacy_cfg_line(l));

    if is_file_level_gated {
        for line_no in 1..=contents.lines().count() {
            gated.insert(line_no);
        }
        return gated;
    }

    let mut pending = false;
    let mut active = false;
    let mut depth: i32 = 0;
    let mut seen_open = false;

    for (idx, line) in contents.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        if is_legacy_cfg_line(line) {
            pending = true;
            gated.insert(line_no);
            continue;
        }

        if pending {
            gated.insert(line_no);
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }
            active = true;
            pending = false;
        } else if active {
            gated.insert(line_no);
        }

        if active {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        seen_open = true;
                        depth += 1;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }

            if seen_open && depth <= 0 {
                active = false;
                depth = 0;
                seen_open = false;
            }
        }
    }

    gated
}

fn check_direct_model_subprocess(
    root: &Path,
    findings: &mut Vec<ArchitectureFinding>,
) -> Result<()> {
    let needles = ["Command::new(\"claude\")", "Command::new(\"codex\")"];
    for path in rust_files_under(&root.join("crates"))? {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let gated = legacy_gated_lines(&contents);
        for (idx, line) in contents.lines().enumerate() {
            let line_no = idx + 1;
            if gated.contains(&line_no) {
                continue;
            }
            for needle in needles {
                if line.contains(needle) {
                    push_finding(
                        findings,
                        &path,
                        Some(line_no),
                        format!(
                            "direct model subprocess dispatch `{needle}` found in un-gated code; use ModelCallService or gate legacy CLI subprocess code behind `legacy-orchestrate`"
                        ),
                    );
                }
            }
        }
    }
    Ok(())
}

fn check_noop_gates(root: &Path, findings: &mut Vec<ArchitectureFinding>) -> Result<()> {
    let path = root.join("crates/roko-gate/src/gate_service.rs");
    let contents = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let lines: Vec<_> = contents.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        if !line.contains("passed: true") {
            continue;
        }

        let start = idx.saturating_sub(5);
        let end = usize::min(idx + 6, lines.len());
        let nearby = lines[start..end].join("\n").to_lowercase();
        if nearby.contains("stub")
            || nearby.contains("noop")
            || nearby.contains("no-op")
            || nearby.contains("always")
        {
            push_finding(
                findings,
                &path,
                Some(idx + 1),
                "`passed: true` appears near stub/noop/always gate language; gates must execute real checks or return an explicit error",
            );
        }
    }
    Ok(())
}

fn check_empty_event_fields(root: &Path, findings: &mut Vec<ArchitectureFinding>) -> Result<()> {
    let needles = ["agent_id: String::new()", "model: String::new()"];
    for path in rust_files_under(&root.join("crates/roko-runtime/src"))? {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        for (idx, line) in contents.lines().enumerate() {
            for needle in needles {
                if line.contains(needle) {
                    push_finding(
                        findings,
                        &path,
                        Some(idx + 1),
                        format!(
                            "empty placeholder event field `{needle}` found; emit the real value or change the event field to `Option<String>`"
                        ),
                    );
                }
            }
        }
    }
    Ok(())
}

fn path_attrs(path: &Path) -> Result<Vec<String>> {
    let contents = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut attrs = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("#[path") {
            continue;
        }
        if let Some(start) = trimmed.find('"') {
            if let Some(end) = trimmed[start + 1..].find('"') {
                attrs.push(trimmed[start + 1..start + 1 + end].to_string());
            }
        }
    }
    Ok(attrs)
}

fn check_path_shared_modules(root: &Path, warnings: &mut Vec<ArchitectureFinding>) -> Result<()> {
    let cli = root.join("crates/roko-cli/src/lib.rs");
    let serve = root.join("crates/roko-serve/src/lib.rs");
    let cli_attrs: HashSet<_> = path_attrs(&cli)?.into_iter().collect();
    let serve_attrs: HashSet<_> = path_attrs(&serve)?.into_iter().collect();

    let mut shared: Vec<_> = cli_attrs.intersection(&serve_attrs).cloned().collect();
    shared.sort();
    for target in shared {
        push_finding(
            warnings,
            &cli,
            None,
            format!(
                "`roko-cli` and `roko-serve` both include `{target}` with `#[path = ...]`; shared modules should be exposed through a crate API instead"
            ),
        );
    }
    Ok(())
}

fn check_architecture_patterns(
    root: &Path,
) -> Result<(Vec<ArchitectureFinding>, Vec<ArchitectureFinding>)> {
    let mut findings = Vec::new();
    let mut warnings = Vec::new();

    check_duplicate_foundation_traits(root, &mut findings)?;
    check_debug_event_logging(root, &mut findings)?;
    check_direct_model_subprocess(root, &mut findings)?;
    check_noop_gates(root, &mut findings)?;
    check_empty_event_fields(root, &mut findings)?;
    check_path_shared_modules(root, &mut warnings)?;

    findings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    warnings.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    Ok((findings, warnings))
}

/// Run the layer check and return the process exit code (0 = pass, 1 = violations found).
pub fn run_layer_check() -> Result<i32> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("failed to run `cargo metadata`")?;

    let layers = extract_layers(&metadata);
    let workspace_count = metadata.workspace_members.len();
    let labeled_count = metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
        .filter(|p| layers.contains_key(p.name.as_ref()))
        .count();

    println!("Layer check: {labeled_count}/{workspace_count} crates have layer metadata");

    if labeled_count == 0 {
        println!("WARNING: No crates have [package.metadata.roko].layer set. Run L01 first.");
        return Ok(1);
    }

    let mut by_layer: HashMap<u32, Vec<&str>> = HashMap::new();
    for (name, &layer) in &layers {
        by_layer.entry(layer).or_default().push(name.as_str());
    }
    for layer in 0..=4 {
        if let Some(crates) = by_layer.get(&layer) {
            let mut names = crates.clone();
            names.sort_unstable();
            println!("  L{layer}: {}", names.join(", "));
        }
    }

    let violations = check_layers(&metadata);
    let workspace_root = metadata.workspace_root.as_std_path();
    let (architecture_findings, architecture_warnings) =
        check_architecture_patterns(workspace_root)?;

    for warning in &architecture_warnings {
        println!(
            "  WARNING: {}: {}",
            line_location(&warning.path, warning.line),
            warning.message
        );
    }

    if violations.is_empty() && architecture_findings.is_empty() {
        println!("\nNo layer or architecture negative-pattern violations found.");
        Ok(0)
    } else {
        if !violations.is_empty() {
            println!("\nFound {} layer violation(s):\n", violations.len());
            for violation in &violations {
                println!("  ERROR: {violation}");
            }
        }

        if !architecture_findings.is_empty() {
            println!(
                "\nFound {} architecture negative-pattern violation(s):\n",
                architecture_findings.len()
            );
            for finding in &architecture_findings {
                println!(
                    "  ERROR: {}: {}",
                    line_location(&finding.path, finding.line),
                    finding.message
                );
            }
        }

        Ok(1)
    }
}

fn main() -> Result<std::process::ExitCode> {
    let code = u8::try_from(run_layer_check()?).context("layer check exit code out of range")?;
    Ok(std::process::ExitCode::from(code))
}

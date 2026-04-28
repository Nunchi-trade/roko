//! Layer dependency checker.
//!
//! Reads `[package.metadata.roko].layer` from each workspace crate and
//! verifies that dependency edges only point from higher layers to equal
//! or lower layers (layer(dependent) >= layer(dependency)).

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

/// A single layer violation: crate at layer N depends on crate at layer M where M > N.
#[derive(Debug)]
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

/// Extract layer assignments from workspace metadata.
fn extract_layers(metadata: &cargo_metadata::Metadata) -> HashMap<String, u32> {
    let mut layers = HashMap::new();
    for package in &metadata.packages {
        if let Some(roko_meta) = package.metadata.get("roko") {
            if let Some(layer) = roko_meta.get("layer").and_then(|v| v.as_u64()) {
                layers.insert(package.name.as_ref().to_string(), layer as u32);
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

/// Run the layer check and return the process exit code (0 = pass, 1 = violations found).
pub fn run_layer_check() -> Result<i32> {
    let metadata = cargo_metadata::MetadataCommand::new()
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
            let mut names = crates.to_vec();
            names.sort();
            println!("  L{layer}: {}", names.join(", "));
        }
    }

    let violations = check_layers(&metadata);

    if violations.is_empty() {
        println!("\nNo layer violations found.");
        Ok(0)
    } else {
        println!("\nFound {} layer violation(s):\n", violations.len());
        for violation in &violations {
            println!("  ERROR: {violation}");
        }
        Ok(1)
    }
}

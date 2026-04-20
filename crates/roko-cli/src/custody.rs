//! CLI subcommands for inspecting the custody audit chain.
//!
//! Provides `list`, `show`, and `verify` commands for operators to inspect
//! and validate the append-only custody record chain.

use std::path::Path;

use anyhow::{Result, anyhow};
use roko_agent::safety::provenance::{Custody, CustodyLogger};
use roko_fs::RokoLayout;

/// List recent custody records, optionally limited to `limit` entries.
pub fn cmd_custody_list(workdir: &Path, limit: Option<usize>) -> Result<()> {
    let layout = RokoLayout::for_project(workdir);
    let logger = CustodyLogger::new(layout.custody_log());
    let records = logger
        .read_all()
        .map_err(|e| anyhow!("failed to read custody log: {e}"))?;

    if records.is_empty() {
        eprintln!(
            "No custody records found at {}",
            layout.custody_log().display()
        );
        return Ok(());
    }

    let display_records: Vec<&Custody> = if let Some(limit) = limit {
        records
            .iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        records.iter().collect()
    };

    println!(
        "{:<6} {:<20} {:<24} {:<14} {:<12} {}",
        "#", "ACTION", "PRINCIPAL", "TAINT", "GATES", "WHEN"
    );
    println!("{}", "-".repeat(90));

    for (idx, record) in records.iter().enumerate() {
        if limit.is_some() && !display_records.contains(&record) {
            continue;
        }
        let taint = record
            .taint
            .as_ref()
            .map(|t| format!("{t:?}"))
            .unwrap_or_else(|| "none".to_string());
        let taint_display = if taint.len() > 12 {
            format!("{}...", &taint[..9])
        } else {
            taint
        };
        let gates = if record.gates_passed.is_empty() {
            "-".to_string()
        } else {
            format!("{}", record.gates_passed.len())
        };
        let when = chrono_format(record.when);
        let action_display = if record.action.len() > 18 {
            format!("{}...", &record.action[..15])
        } else {
            record.action.clone()
        };
        let principal_display = if record.principal.len() > 22 {
            format!("{}...", &record.principal[..19])
        } else {
            record.principal.clone()
        };
        println!(
            "{:<6} {:<20} {:<24} {:<14} {:<12} {}",
            idx, action_display, principal_display, taint_display, gates, when
        );
    }

    println!("\nTotal: {} records", records.len());
    Ok(())
}

/// Show a single custody record by index.
pub fn cmd_custody_show(workdir: &Path, index: usize) -> Result<()> {
    let layout = RokoLayout::for_project(workdir);
    let logger = CustodyLogger::new(layout.custody_log());
    let records = logger
        .read_all()
        .map_err(|e| anyhow!("failed to read custody log: {e}"))?;

    if records.is_empty() {
        eprintln!(
            "No custody records found at {}",
            layout.custody_log().display()
        );
        std::process::exit(1);
    }

    let record = records
        .get(index)
        .ok_or_else(|| anyhow!("record index {index} out of range (0..{})", records.len()))?;

    println!("Custody Record #{index}");
    println!("{}", "=".repeat(50));
    println!("Action:      {}", record.action);
    println!("Principal:   {}", record.principal);
    println!(
        "When:        {} ({})",
        chrono_format(record.when),
        record.when
    );
    println!(
        "Taint:       {}",
        record
            .taint
            .as_ref()
            .map(|t| format!("{t:?}"))
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "Attestation: {}",
        record
            .attestation
            .as_ref()
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "Result:      {}",
        record.result.as_deref().unwrap_or("none")
    );
    println!(
        "Witness:     {}",
        record.witness.as_deref().unwrap_or("none")
    );
    println!(
        "Simulation:  {}",
        record.simulation.as_deref().unwrap_or("none")
    );

    if !record.gates_passed.is_empty() {
        println!("\nGates passed:");
        for gate in &record.gates_passed {
            println!("  - {gate}");
        }
    }

    if !record.authorized.is_empty() {
        println!("\nAuthorization evidence:");
        for ev in &record.authorized {
            println!("  - [{:?}] {}", ev.source, ev.detail);
        }
    }

    if !record.why_heuristics.is_empty() {
        println!("\nHeuristics:");
        for h in &record.why_heuristics {
            println!("  - {h}");
        }
    }

    if !record.why_claims.is_empty() {
        println!("\nClaims:");
        for c in &record.why_claims {
            println!("  - {c}");
        }
    }

    Ok(())
}

/// Verify integrity of the custody chain.
///
/// Checks for:
/// - Empty custody log
/// - Parse errors (corrupted lines)
/// - Monotonic timestamps
/// - Missing required fields
pub fn cmd_custody_verify(workdir: &Path) -> Result<()> {
    let layout = RokoLayout::for_project(workdir);
    let log_path = layout.custody_log();

    if !log_path.exists() {
        eprintln!("No custody log found at {}", log_path.display());
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&log_path)
        .map_err(|e| anyhow!("failed to read custody log: {e}"))?;

    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        eprintln!("Custody log is empty.");
        std::process::exit(1);
    }

    let mut violations = Vec::new();
    let mut prev_when: Option<i64> = None;
    let mut valid_count = 0usize;
    let mut parse_errors = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        match serde_json::from_str::<Custody>(line) {
            Ok(record) => {
                valid_count += 1;

                // Check monotonic timestamps.
                if let Some(prev) = prev_when {
                    if record.when < prev {
                        violations.push(format!(
                            "line {idx}: timestamp regression ({} < {prev})",
                            record.when
                        ));
                    }
                }
                prev_when = Some(record.when);

                // Check required fields are non-empty.
                if record.action.is_empty() {
                    violations.push(format!("line {idx}: empty action field"));
                }
                if record.principal.is_empty() {
                    violations.push(format!("line {idx}: empty principal field"));
                }
            }
            Err(e) => {
                parse_errors += 1;
                violations.push(format!("line {idx}: parse error: {e}"));
            }
        }
    }

    println!("Custody chain verification");
    println!("{}", "=".repeat(40));
    println!("Total lines:    {}", lines.len());
    println!("Valid records:  {valid_count}");
    println!("Parse errors:   {parse_errors}");
    println!("Violations:     {}", violations.len());

    if violations.is_empty() {
        println!("\nChain integrity: OK");
        Ok(())
    } else {
        println!("\nViolations:");
        for v in &violations {
            println!("  - {v}");
        }
        Err(anyhow!("{} integrity violation(s) found", violations.len()))
    }
}

/// Format a Unix-millis timestamp as a human-readable string.
fn chrono_format(millis: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let secs = (millis / 1000) as u64;
    let nanos = ((millis % 1000) * 1_000_000) as u32;
    match UNIX_EPOCH.checked_add(Duration::new(secs, nanos)) {
        Some(time) => {
            let datetime: std::time::SystemTime = time;
            // Use a simple ISO-ish format without pulling in chrono.
            let elapsed = datetime
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO);
            let total_secs = elapsed.as_secs();
            let days = total_secs / 86400;
            let day_secs = total_secs % 86400;
            let hours = day_secs / 3600;
            let mins = (day_secs % 3600) / 60;
            let secs = day_secs % 60;
            // Approximate year/month/day from days since epoch.
            // Good enough for display; not calendar-accurate for leap seconds.
            format!("epoch+{days}d {hours:02}:{mins:02}:{secs:02}",)
        }
        None => format!("{millis}ms"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_agent::safety::provenance::{AttestationLevel, Taint};
    use tempfile::TempDir;

    fn setup_custody_log(tmp: &TempDir) -> CustodyLogger {
        let layout = RokoLayout::for_project(tmp.path());
        let logger = CustodyLogger::new(layout.custody_log());
        logger
    }

    #[test]
    fn list_empty_log_succeeds() {
        let tmp = TempDir::new().unwrap();
        // No custody log file exists; cmd_custody_list should not error.
        assert!(cmd_custody_list(tmp.path(), None).is_ok());
    }

    #[test]
    fn list_with_records() {
        let tmp = TempDir::new().unwrap();
        let logger = setup_custody_log(&tmp);
        logger
            .log(&Custody::new("write_file", "agent-1", 1000, vec![]))
            .unwrap();
        logger
            .log(&Custody::new("bash", "agent-2", 2000, vec![]).with_taint(Taint::UserInput))
            .unwrap();

        assert!(cmd_custody_list(tmp.path(), None).is_ok());
        assert!(cmd_custody_list(tmp.path(), Some(1)).is_ok());
    }

    #[test]
    fn show_record_by_index() {
        let tmp = TempDir::new().unwrap();
        let logger = setup_custody_log(&tmp);
        logger
            .log(
                &Custody::new("edit_file", "agent-3", 3000, vec![])
                    .with_attestation(AttestationLevel::LocalAgent)
                    .with_gates_passed(vec!["compile".into(), "test".into()]),
            )
            .unwrap();

        assert!(cmd_custody_show(tmp.path(), 0).is_ok());
    }

    #[test]
    fn show_out_of_range_fails() {
        let tmp = TempDir::new().unwrap();
        let logger = setup_custody_log(&tmp);
        logger.log(&Custody::new("test", "p", 100, vec![])).unwrap();

        assert!(cmd_custody_show(tmp.path(), 99).is_err());
    }

    #[test]
    fn verify_valid_chain_succeeds() {
        let tmp = TempDir::new().unwrap();
        let logger = setup_custody_log(&tmp);
        logger
            .log(&Custody::new("action1", "agent-1", 1000, vec![]))
            .unwrap();
        logger
            .log(&Custody::new("action2", "agent-1", 2000, vec![]))
            .unwrap();

        assert!(cmd_custody_verify(tmp.path()).is_ok());
    }

    #[test]
    fn verify_detects_timestamp_regression() {
        let tmp = TempDir::new().unwrap();
        let logger = setup_custody_log(&tmp);
        // Second record has an earlier timestamp.
        logger
            .log(&Custody::new("action1", "agent-1", 2000, vec![]))
            .unwrap();
        logger
            .log(&Custody::new("action2", "agent-1", 1000, vec![]))
            .unwrap();

        assert!(cmd_custody_verify(tmp.path()).is_err());
    }

    #[test]
    fn chrono_format_produces_readable_output() {
        // 2024-01-01T00:00:00Z in millis = 1704067200000
        let result = chrono_format(1704067200000);
        assert!(result.contains(':'));
    }
}

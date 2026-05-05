//! Shared helper functions for status submodules.

use serde_json::Value;

use crate::error::ApiError;
use roko_learn::cfactor::CFactor;
use roko_learn::efficiency::AgentEfficiencyEvent;

pub const MAX_JSONL_RESULTS: usize = 10_000;

/// Read a JSONL file and return each line as a parsed `serde_json::Value`.
pub async fn read_jsonl_entries(path: &std::path::Path) -> Result<Vec<Value>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };
    let mut entries = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str::<Value>(line).map_err(|e| {
            ApiError::internal(format!(
                "parse {} line {}: {e}",
                path.display(),
                line_no + 1
            ))
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Read a JSONL file and return the entries as a `Json<Value::Array>`.
#[allow(dead_code)]
pub async fn read_jsonl_array(path: &std::path::Path) -> Result<axum::Json<Value>, ApiError> {
    let entries = read_jsonl_entries(path).await?;
    Ok(axum::Json(Value::Array(entries)))
}

pub async fn read_cfactor_history(path: &std::path::Path) -> Result<Vec<CFactor>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    let mut history = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(snapshot) = serde_json::from_str::<CFactor>(trimmed) {
            history.push(snapshot);
        }
    }
    Ok(history)
}

pub async fn read_efficiency_events(
    path: &std::path::Path,
) -> Result<Vec<AgentEfficiencyEvent>, ApiError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(ApiError::internal(format!("read {}: {e}", path.display()))),
    };

    let mut events = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<AgentEfficiencyEvent>(line).map_err(|e| {
            ApiError::internal(format!(
                "parse {} line {}: {e}",
                path.display(),
                line_no + 1
            ))
        })?;
        events.push(event);
    }
    Ok(events)
}

// ── signal extraction helpers ────────────────────────────────────────

pub fn is_gate_result_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
}

pub fn extract_gate_name(entry: &Value) -> Option<String> {
    entry
        .pointer("/tags/gate")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .pointer("/body/data/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .pointer("/body/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .get("kind")
                .and_then(Value::as_str)
                .and_then(|kind| kind.strip_prefix("gate:").or(kind.strip_prefix("gate_")))
                .map(ToOwned::to_owned)
        })
}

pub fn extract_gate_passed(entry: &Value) -> Option<bool> {
    entry
        .pointer("/tags/passed")
        .and_then(Value::as_str)
        .and_then(|s| match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .or_else(|| entry.pointer("/body/data/passed").and_then(Value::as_bool))
        .or_else(|| entry.pointer("/body/passed").and_then(Value::as_bool))
}

pub fn extract_gate_duration_ms(entry: &Value) -> Option<u64> {
    entry
        .pointer("/body/data/duration_ms")
        .and_then(Value::as_u64)
        .or_else(|| entry.pointer("/body/duration_ms").and_then(Value::as_u64))
        .or_else(|| entry.pointer("/tags/duration_ms").and_then(Value::as_u64))
}

pub fn extract_gate_rung(entry: &Value) -> Option<u32> {
    let raw = entry
        .pointer("/tags/rung")
        .or_else(|| entry.pointer("/body/data/rung"))
        .or_else(|| entry.pointer("/body/rung"))?;

    raw.as_u64()
        .map(|rung| rung as u32)
        .or_else(|| raw.as_str().and_then(|text| text.parse::<u32>().ok()))
}

pub fn signal_kind(entry: &Value) -> Option<String> {
    entry
        .get("kind")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn signal_id(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub fn entry_timestamp_ms(entry: &Value) -> Option<i64> {
    entry
        .get("created_at_ms")
        .and_then(Value::as_i64)
        .or_else(|| {
            entry
                .get("created_at_ms")
                .and_then(Value::as_u64)
                .map(|ts| ts as i64)
        })
}

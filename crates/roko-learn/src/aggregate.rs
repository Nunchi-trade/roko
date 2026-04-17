//! Efficiency and C-Factor trend aggregation helpers for JSONL telemetry.
//!
//! This module folds `.roko/learn/efficiency.jsonl` and
//! `.roko/learn/c-factor.jsonl` into fixed-width time buckets suitable for
//! dashboards and lightweight sparklines.

use std::fs::File;
use std::io::{self, BufRead, BufReader, ErrorKind, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::efficiency::AgentEfficiencyEvent;

/// Maximum number of buckets the aggregator will emit.
pub const MAX_EFFICIENCY_BUCKETS: usize = 168;

/// Incremental cursor for append-only JSONL files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonlCursor {
    path: PathBuf,
    offset: u64,
    last_line_number: usize,
}

impl JsonlCursor {
    /// Create a cursor for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            last_line_number: 0,
        }
    }

    /// Read and return lines appended since the last successful tick.
    ///
    /// Missing files and truncation reset the cursor to the beginning.
    ///
    /// # Errors
    ///
    /// Returns an error if the path metadata, open, seek, or read operation
    /// fails for reasons other than the file being missing.
    pub fn read_new_lines(&mut self) -> io::Result<Vec<String>> {
        self.read_new_lines_with_status().map(|read| read.lines)
    }

    /// Byte offset of the next unread line.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Count of committed lines read since the last reset.
    #[must_use]
    pub const fn last_line_number(&self) -> usize {
        self.last_line_number
    }

    /// JSONL path tracked by this cursor.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_new_lines_with_status(&mut self) -> io::Result<CursorRead> {
        let mut reset = false;
        let len = match std::fs::metadata(&self.path) {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.reset();
                return Ok(CursorRead {
                    lines: Vec::new(),
                    reset: true,
                });
            }
            Err(err) => return Err(err),
        };

        if len < self.offset {
            self.reset();
            reset = true;
        }

        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.reset();
                return Ok(CursorRead {
                    lines: Vec::new(),
                    reset: true,
                });
            }
            Err(err) => return Err(err),
        };

        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(self.offset))?;

        let mut lines = Vec::new();
        let mut buf = String::new();

        loop {
            buf.clear();
            let read = reader.read_line(&mut buf)?;
            if read == 0 {
                break;
            }
            if !buf.ends_with('\n') {
                break;
            }

            self.offset += read as u64;
            self.last_line_number += 1;
            lines.push(trim_line_ending(&buf));
        }

        Ok(CursorRead { lines, reset })
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.last_line_number = 0;
    }
}

#[derive(Debug)]
struct CursorRead {
    lines: Vec<String>,
    reset: bool,
}

/// One bucket of aggregated efficiency telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EfficiencyBucket {
    /// Bucket start timestamp in UTC.
    pub start: DateTime<Utc>,
    /// Number of turns recorded in the bucket.
    pub turns: u64,
    /// Sum of input tokens across the bucket.
    pub tokens_in: u64,
    /// Sum of output tokens across the bucket.
    pub tokens_out: u64,
    /// Sum of recorded cost in USD cents.
    pub cost_usd_cents: u64,
    /// Average latency in milliseconds for turns in the bucket.
    pub latency_ms_avg: f64,
}

impl Default for EfficiencyBucket {
    fn default() -> Self {
        Self {
            start: default_bucket_start(),
            turns: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd_cents: 0,
            latency_ms_avg: 0.0,
        }
    }
}

/// One bucket of aggregated c-factor telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorBucket {
    /// Bucket start timestamp in UTC.
    pub start: DateTime<Utc>,
    /// Number of C-Factor snapshots recorded in the bucket.
    pub samples: u32,
    /// Average overall C-Factor score across snapshots in the bucket.
    pub avg: f64,
    /// Median overall C-Factor score across snapshots in the bucket.
    pub p50: f64,
    /// p95 overall C-Factor score across snapshots in the bucket.
    pub p95: f64,
}

impl Default for CFactorBucket {
    fn default() -> Self {
        Self {
            start: default_bucket_start(),
            samples: 0,
            avg: 0.0,
            p50: 0.0,
            p95: 0.0,
        }
    }
}

/// Aggregate the full efficiency JSONL file into a fixed-width time series.
///
/// Missing files return an empty-count series of `n_buckets` buckets. Malformed
/// lines are skipped.
///
/// # Errors
///
/// Returns an error if `bucket` is not positive or if the file cannot be
/// read.
pub fn efficiency_trend(
    path: &Path,
    bucket: Duration,
    n_buckets: usize,
) -> io::Result<Vec<EfficiencyBucket>> {
    efficiency_trend_at(path, bucket, n_buckets, Utc::now())
}

/// Aggregate the full c-factor JSONL file into a fixed-width time series.
///
/// Missing files return an empty-count series of `n_buckets` buckets.
/// Malformed lines are skipped.
///
/// # Errors
///
/// Returns an error if `bucket` is not positive or if the file cannot be
/// read.
pub fn cfactor_trend(
    path: &Path,
    bucket: Duration,
    n_buckets: usize,
) -> io::Result<Vec<CFactorBucket>> {
    cfactor_trend_at(path, bucket, n_buckets, Utc::now())
}

/// Incrementally update a time series using only newly appended JSONL lines.
///
/// The caller supplies the prior `existing` bucket series so the function can
/// preserve in-window data without rescanning the full file. If the file was
/// truncated or recreated, the function falls back to a full rescan.
///
/// # Errors
///
/// Returns an error if the cursor cannot read the tracked file or if `bucket`
/// is not positive.
pub fn efficiency_trend_with_cursor(
    cursor: &mut JsonlCursor,
    existing: &[EfficiencyBucket],
    bucket: Duration,
    n_buckets: usize,
) -> io::Result<Vec<EfficiencyBucket>> {
    efficiency_trend_with_cursor_at(cursor, existing, bucket, n_buckets, Utc::now())
}

fn efficiency_trend_with_cursor_at(
    cursor: &mut JsonlCursor,
    existing: &[EfficiencyBucket],
    bucket: Duration,
    n_buckets: usize,
    now: DateTime<Utc>,
) -> io::Result<Vec<EfficiencyBucket>> {
    let bucket_ms = validate_bucket(bucket)?;
    let normalized = normalize_bucket_count(n_buckets);
    if normalized == 0 {
        return Ok(Vec::new());
    }

    let read = cursor.read_new_lines_with_status()?;
    if read.reset || existing.len() != normalized {
        return efficiency_trend_at(cursor.path(), bucket, normalized, now);
    }

    let oldest_start_ms = oldest_bucket_start_ms(now, bucket_ms, normalized);
    let mut buckets = empty_buckets(now, bucket_ms, normalized);
    overlay_existing_buckets(&mut buckets, existing);

    for event in parse_events(read.lines) {
        apply_event_to_buckets(&mut buckets, &event, oldest_start_ms, bucket_ms);
    }

    Ok(buckets)
}

fn efficiency_trend_at(
    path: &Path,
    bucket: Duration,
    n_buckets: usize,
    now: DateTime<Utc>,
) -> io::Result<Vec<EfficiencyBucket>> {
    let bucket_ms = validate_bucket(bucket)?;
    let normalized = normalize_bucket_count(n_buckets);
    if normalized == 0 {
        return Ok(Vec::new());
    }

    let lines = match std::fs::read_to_string(path) {
        Ok(text) => text.lines().map(str::to_owned).collect::<Vec<_>>(),
        Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err),
    };

    let oldest_start_ms = oldest_bucket_start_ms(now, bucket_ms, normalized);
    let mut buckets = empty_buckets(now, bucket_ms, normalized);
    for event in parse_events(lines) {
        apply_event_to_buckets(&mut buckets, &event, oldest_start_ms, bucket_ms);
    }

    Ok(buckets)
}

fn cfactor_trend_at(
    path: &Path,
    bucket: Duration,
    n_buckets: usize,
    now: DateTime<Utc>,
) -> io::Result<Vec<CFactorBucket>> {
    let bucket_ms = validate_bucket(bucket)?;
    let normalized = normalize_bucket_count(n_buckets);
    if normalized == 0 {
        return Ok(Vec::new());
    }

    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(empty_cfactor_buckets(now, bucket_ms, normalized));
        }
        Err(err) => return Err(err),
    };

    let oldest_start_ms = oldest_bucket_start_ms(now, bucket_ms, normalized);
    let mut buckets = empty_cfactor_buckets(now, bucket_ms, normalized);
    let mut bucket_samples = vec![Vec::new(); normalized];
    let mut reader = BufReader::new(file);
    let mut buf = String::new();

    loop {
        buf.clear();
        let read = reader.read_line(&mut buf)?;
        if read == 0 {
            break;
        }

        let trimmed = buf.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(snapshot) = serde_json::from_str::<ParsedCFactorSnapshot>(trimmed) else {
            continue;
        };
        apply_cfactor_snapshot_to_buckets(
            &mut bucket_samples,
            &snapshot,
            oldest_start_ms,
            bucket_ms,
        );
    }

    for (bucket, values) in buckets.iter_mut().zip(bucket_samples.iter_mut()) {
        finalize_cfactor_bucket(bucket, values);
    }

    Ok(buckets)
}

#[derive(Debug, Clone)]
struct ParsedEfficiencyEvent {
    timestamp: DateTime<Utc>,
    tokens_in: u64,
    tokens_out: u64,
    cost_usd_cents: u64,
    latency_ms: u64,
}

fn parse_events(lines: Vec<String>) -> Vec<ParsedEfficiencyEvent> {
    lines
        .into_iter()
        .filter_map(|line| parse_event(&line))
        .collect()
}

fn parse_event(line: &str) -> Option<ParsedEfficiencyEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let event = serde_json::from_str::<AgentEfficiencyEvent>(trimmed).ok()?;
    let timestamp = DateTime::parse_from_rfc3339(&event.timestamp)
        .ok()?
        .with_timezone(&Utc);
    Some(ParsedEfficiencyEvent {
        timestamp,
        tokens_in: event.input_tokens,
        tokens_out: event.output_tokens,
        cost_usd_cents: usd_to_cents(event.cost_usd),
        latency_ms: event_latency_ms(&event),
    })
}

fn apply_event_to_buckets(
    buckets: &mut [EfficiencyBucket],
    event: &ParsedEfficiencyEvent,
    oldest_start_ms: i64,
    bucket_ms: i64,
) {
    let event_bucket_ms = floor_bucket_start_ms(event.timestamp.timestamp_millis(), bucket_ms);
    if event_bucket_ms < oldest_start_ms {
        return;
    }

    let idx =
        usize::try_from((event_bucket_ms - oldest_start_ms) / bucket_ms).unwrap_or(usize::MAX);
    let Some(bucket) = buckets.get_mut(idx) else {
        return;
    };

    let turns_before = bucket.turns;
    let total_latency_before = bucket.latency_ms_avg.mul_add(turns_before as f64, 0.0);
    let turns_after = turns_before.saturating_add(1);

    bucket.turns = turns_after;
    bucket.tokens_in = bucket.tokens_in.saturating_add(event.tokens_in);
    bucket.tokens_out = bucket.tokens_out.saturating_add(event.tokens_out);
    bucket.cost_usd_cents = bucket.cost_usd_cents.saturating_add(event.cost_usd_cents);
    bucket.latency_ms_avg = (total_latency_before + event.latency_ms as f64) / turns_after as f64;
}

#[derive(Debug, Clone, Deserialize)]
struct ParsedCFactorSnapshot {
    computed_at: DateTime<Utc>,
    overall: f64,
}

fn apply_cfactor_snapshot_to_buckets(
    buckets: &mut [Vec<f64>],
    snapshot: &ParsedCFactorSnapshot,
    oldest_start_ms: i64,
    bucket_ms: i64,
) {
    let snapshot_bucket_ms =
        floor_bucket_start_ms(snapshot.computed_at.timestamp_millis(), bucket_ms);
    if snapshot_bucket_ms < oldest_start_ms {
        return;
    }

    let idx =
        usize::try_from((snapshot_bucket_ms - oldest_start_ms) / bucket_ms).unwrap_or(usize::MAX);
    let Some(bucket) = buckets.get_mut(idx) else {
        return;
    };

    bucket.push(snapshot.overall);
}

fn finalize_cfactor_bucket(bucket: &mut CFactorBucket, values: &mut [f64]) {
    if values.is_empty() {
        return;
    }

    values.sort_by(|lhs, rhs| lhs.total_cmp(rhs));
    let sample_count = values.len();
    let sum = values.iter().sum::<f64>();

    bucket.samples = u32::try_from(sample_count).unwrap_or(u32::MAX);
    bucket.avg = sum / sample_count as f64;
    bucket.p50 = quantile(values, 0.50);
    bucket.p95 = quantile(values, 0.95);
}

fn quantile(sorted_values: &[f64], quantile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }

    let clamped = quantile.clamp(0.0, 1.0);
    let last_idx = sorted_values.len().saturating_sub(1);
    let position = clamped * last_idx as f64;
    let lower_idx = position.floor() as usize;
    let upper_idx = position.ceil() as usize;

    if lower_idx == upper_idx {
        return sorted_values[lower_idx];
    }

    let lower = sorted_values[lower_idx];
    let upper = sorted_values[upper_idx];
    let weight = position - lower_idx as f64;
    lower + (upper - lower) * weight
}

fn overlay_existing_buckets(target: &mut [EfficiencyBucket], existing: &[EfficiencyBucket]) {
    for bucket in existing {
        if let Some(target_bucket) = target.iter_mut().find(|item| item.start == bucket.start) {
            *target_bucket = bucket.clone();
        }
    }
}

fn empty_cfactor_buckets(
    now: DateTime<Utc>,
    bucket_ms: i64,
    n_buckets: usize,
) -> Vec<CFactorBucket> {
    let oldest_start_ms = oldest_bucket_start_ms(now, bucket_ms, n_buckets);
    (0..n_buckets)
        .map(|idx| CFactorBucket {
            start: timestamp_from_millis(
                oldest_start_ms + i64::try_from(idx).unwrap_or(0) * bucket_ms,
            ),
            ..CFactorBucket::default()
        })
        .collect()
}

fn empty_buckets(now: DateTime<Utc>, bucket_ms: i64, n_buckets: usize) -> Vec<EfficiencyBucket> {
    let oldest_start_ms = oldest_bucket_start_ms(now, bucket_ms, n_buckets);
    (0..n_buckets)
        .map(|idx| EfficiencyBucket {
            start: timestamp_from_millis(
                oldest_start_ms + i64::try_from(idx).unwrap_or(0) * bucket_ms,
            ),
            ..EfficiencyBucket::default()
        })
        .collect()
}

fn oldest_bucket_start_ms(now: DateTime<Utc>, bucket_ms: i64, n_buckets: usize) -> i64 {
    let latest_start_ms = floor_bucket_start_ms(now.timestamp_millis(), bucket_ms);
    latest_start_ms - i64::try_from(n_buckets.saturating_sub(1)).unwrap_or(0) * bucket_ms
}

fn floor_bucket_start_ms(timestamp_ms: i64, bucket_ms: i64) -> i64 {
    timestamp_ms.div_euclid(bucket_ms) * bucket_ms
}

fn timestamp_from_millis(timestamp_ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .unwrap_or_else(default_bucket_start)
}

fn validate_bucket(bucket: Duration) -> io::Result<i64> {
    let bucket_ms = bucket.num_milliseconds();
    if bucket_ms <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "efficiency trend bucket must be positive",
        ));
    }
    Ok(bucket_ms)
}

fn normalize_bucket_count(n_buckets: usize) -> usize {
    n_buckets.min(MAX_EFFICIENCY_BUCKETS)
}

fn usd_to_cents(cost_usd: f64) -> u64 {
    if !cost_usd.is_finite() || cost_usd <= 0.0 {
        return 0;
    }

    let cents = (cost_usd * 100.0).round();
    if cents <= 0.0 {
        0
    } else if cents >= u64::MAX as f64 {
        u64::MAX
    } else {
        cents as u64
    }
}

fn event_latency_ms(event: &AgentEfficiencyEvent) -> u64 {
    if event.duration_ms > 0 {
        event.duration_ms
    } else {
        event.wall_time_ms
    }
}

fn default_bucket_start() -> DateTime<Utc> {
    Utc.timestamp_opt(0, 0).single().unwrap_or_else(Utc::now)
}

fn trim_line_ending(line: &str) -> String {
    line.trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use tempfile::tempdir;

    fn sample_event(timestamp: DateTime<Utc>, idx: usize) -> AgentEfficiencyEvent {
        AgentEfficiencyEvent {
            agent_id: format!("agent-{idx}"),
            role: "Implementer".to_string(),
            backend: "mock".to_string(),
            model: "claude-opus-4-6".to_string(),
            plan_id: "plan-1".to_string(),
            task_id: format!("task-{idx:03}"),
            input_tokens: 100,
            output_tokens: 40,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.12,
            cost_usd_without_cache: 0.12,
            prompt_sections: Vec::new(),
            total_prompt_tokens: 0,
            system_prompt_tokens: 0,
            tools_available: 4,
            tools_used: 2,
            tool_calls: Vec::new(),
            wall_time_ms: 2_000,
            duration_ms: 2_000,
            time_to_first_token_ms: 100,
            was_warm_start: true,
            iteration: 1,
            gate_passed: true,
            outcome: "ok".to_string(),
            gate_errors: Vec::new(),
            model_used: "claude-opus-4-6".to_string(),
            frequency: roko_core::OperatingFrequency::Theta,
            strategy_attempted: String::new(),
            timestamp: timestamp.to_rfc3339(),
        }
    }

    fn append_cfactor_snapshot(path: &Path, timestamp: DateTime<Utc>, overall: f64) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open file for append");
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "computed_at": timestamp.to_rfc3339(),
                "overall": overall,
            })
        )
        .expect("write c-factor snapshot");
    }

    fn append_event(path: &Path, event: &AgentEfficiencyEvent) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open file for append");
        writeln!(
            file,
            "{}",
            serde_json::to_string(event).expect("serialize event")
        )
        .expect("write event");
    }

    #[test]
    fn efficiency_trend_groups_synthetic_100_event_jsonl_into_hourly_buckets() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("efficiency.jsonl");
        let start = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .expect("parse start")
            .with_timezone(&Utc);

        for hour in 0..10 {
            for turn in 0..10 {
                let timestamp = start + Duration::hours(hour) + Duration::minutes(turn * 6);
                append_event(
                    &path,
                    &sample_event(timestamp, hour as usize * 10 + turn as usize),
                );
            }
        }

        let now = start + Duration::hours(9) + Duration::minutes(59);
        let buckets = efficiency_trend_at(&path, Duration::hours(1), 10, now).expect("trend");
        assert_eq!(buckets.len(), 10);
        assert!(buckets.iter().all(|bucket| bucket.turns == 10));
        assert_eq!(buckets[0].start, start);
        assert_eq!(buckets[0].tokens_in, 1_000);
        assert_eq!(buckets[0].tokens_out, 400);
        assert_eq!(buckets[0].cost_usd_cents, 120);
        assert!((buckets[0].latency_ms_avg - 2_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn efficiency_trend_caps_bucket_count_at_168() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("efficiency.jsonl");
        let start = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .expect("parse start")
            .with_timezone(&Utc);

        for idx in 0..200 {
            append_event(
                &path,
                &sample_event(start + Duration::minutes(idx as i64), idx),
            );
        }

        let buckets = efficiency_trend(&path, Duration::minutes(1), 1_000).expect("trend");
        assert_eq!(buckets.len(), MAX_EFFICIENCY_BUCKETS);
    }

    #[test]
    fn efficiency_trend_with_cursor_updates_without_rescanning() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("efficiency.jsonl");
        let start = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .expect("parse start")
            .with_timezone(&Utc);

        for idx in 0..12 {
            append_event(
                &path,
                &sample_event(start + Duration::minutes(idx as i64 * 5), idx),
            );
        }

        let mut cursor = JsonlCursor::new(&path);
        let first = efficiency_trend_with_cursor_at(
            &mut cursor,
            &[],
            Duration::hours(1),
            4,
            start + Duration::hours(1) + Duration::minutes(30),
        )
        .expect("first trend");
        assert_eq!(cursor.last_line_number(), 12);
        assert_eq!(first.iter().map(|bucket| bucket.turns).sum::<u64>(), 12);

        for idx in 12..24 {
            append_event(
                &path,
                &sample_event(start + Duration::minutes(idx as i64 * 5), idx),
            );
        }

        let second = efficiency_trend_with_cursor_at(
            &mut cursor,
            &first,
            Duration::hours(1),
            4,
            start + Duration::hours(2),
        )
        .expect("second trend");
        assert_eq!(cursor.last_line_number(), 24);
        assert_eq!(second.iter().map(|bucket| bucket.turns).sum::<u64>(), 24);
    }

    #[test]
    fn jsonl_cursor_resets_on_truncation_and_missing_files() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("efficiency.jsonl");
        fs::write(&path, "one\ntwo\n").expect("seed file");

        let mut cursor = JsonlCursor::new(&path);
        assert_eq!(
            cursor.read_new_lines().expect("initial read"),
            vec!["one", "two"]
        );
        assert_eq!(cursor.last_line_number(), 2);

        fs::write(&path, "reset\n").expect("truncate file");
        assert_eq!(
            cursor.read_new_lines().expect("after truncation"),
            vec!["reset"]
        );
        assert_eq!(cursor.last_line_number(), 1);

        fs::remove_file(&path).expect("remove file");
        assert_eq!(
            cursor.read_new_lines().expect("after removal"),
            Vec::<String>::new()
        );
        assert_eq!(cursor.offset(), 0);
        assert_eq!(cursor.last_line_number(), 0);
    }

    #[test]
    fn cfactor_trend_groups_snapshots_and_averages_overall() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".roko/learn/c-factor.jsonl");
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");

        let start = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .expect("parse start")
            .with_timezone(&Utc);
        append_cfactor_snapshot(&path, start + Duration::minutes(10), 0.20);
        append_cfactor_snapshot(&path, start + Duration::minutes(40), 0.60);
        append_cfactor_snapshot(
            &path,
            start + Duration::hours(1) + Duration::minutes(5),
            0.90,
        );
        append_cfactor_snapshot(
            &path,
            start + Duration::hours(1) + Duration::minutes(35),
            0.30,
        );

        let buckets = cfactor_trend_at(
            &path,
            Duration::hours(1),
            4,
            start + Duration::hours(3) + Duration::minutes(30),
        )
        .expect("trend");

        assert_eq!(buckets.len(), 4);
        assert_eq!(buckets[0].samples, 2);
        assert!((buckets[0].avg - 0.40).abs() < f64::EPSILON);
        assert!((buckets[0].p50 - 0.40).abs() < f64::EPSILON);
        assert!((buckets[0].p95 - 0.58).abs() < f64::EPSILON);
        assert_eq!(buckets[1].samples, 2);
        assert!((buckets[1].avg - 0.60).abs() < f64::EPSILON);
        assert!((buckets[1].p50 - 0.60).abs() < f64::EPSILON);
        assert!((buckets[1].p95 - 0.87).abs() < 1e-9);
        assert_eq!(buckets[2].samples, 0);
        assert_eq!(buckets[3].samples, 0);
    }

    #[test]
    fn cfactor_trend_skips_malformed_lines() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".roko/learn/c-factor.jsonl");
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");

        let start = DateTime::parse_from_rfc3339("2026-04-01T00:00:00Z")
            .expect("parse start")
            .with_timezone(&Utc);
        append_cfactor_snapshot(&path, start + Duration::minutes(5), 0.25);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open snapshot file")
            .write_all(b"not-json\n")
            .expect("write malformed line");
        append_cfactor_snapshot(
            &path,
            start + Duration::hours(1) + Duration::minutes(5),
            0.75,
        );

        let buckets = cfactor_trend_at(
            &path,
            Duration::hours(1),
            4,
            start + Duration::hours(3) + Duration::minutes(30),
        )
        .expect("trend");

        assert_eq!(buckets.iter().map(|bucket| bucket.samples).sum::<u32>(), 2);
        assert!((buckets[0].avg - 0.25).abs() < f64::EPSILON);
        assert!((buckets[1].avg - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn cfactor_trend_returns_empty_buckets_for_missing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(".roko/learn/c-factor.jsonl");

        let buckets = cfactor_trend(&path, Duration::hours(1), 3).expect("trend");

        assert_eq!(buckets.len(), 3);
        assert!(buckets.iter().all(|bucket| bucket.samples == 0));
        assert!(
            buckets
                .iter()
                .all(|bucket| (bucket.avg - 0.0).abs() < f64::EPSILON)
        );
        assert!(
            buckets
                .iter()
                .all(|bucket| (bucket.p50 - 0.0).abs() < f64::EPSILON)
        );
        assert!(
            buckets
                .iter()
                .all(|bucket| (bucket.p95 - 0.0).abs() < f64::EPSILON)
        );
    }
}

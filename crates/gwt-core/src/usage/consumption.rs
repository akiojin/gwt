//! Daily / weekly token-consumption aggregation (SPEC-2970 extension).
//!
//! Separate axis from the rate-limit windows: this reads local session history
//! (Codex rollouts, Claude transcripts) and sums **actual token consumption**
//! per local day. Each provider rollup carries a Today total, a This-week total
//! (aligned to the weekly rate-limit window via `week_start`), and a 7-day
//! series for a mini bar chart. Token counts are split into
//! `input` (uncached) / `output` / `cached` so the cache re-read volume does
//! not drown out the real new-work numbers.
//!
//! Per-event timestamps make day bucketing accurate:
//! - Codex `token_count` events carry a top-level `timestamp` and
//!   `info.last_token_usage` (the per-turn delta).
//! - Claude `assistant` lines carry a `timestamp` and `message.usage`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration as StdDuration, SystemTime};

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::claude;
use super::codex;
use super::types::UsageProvider;

/// How many days of files to scan / chart.
pub const RANGE_DAYS: i64 = 8;
/// Number of days shown in the mini bar chart (including today).
pub const CHART_DAYS: i64 = 7;
/// Upper bound on files scanned per provider (cost guard). When hit, the caller
/// should log that older history was skipped.
pub const MAX_FILES: usize = 400;

/// Token consumption split so cache re-reads don't dominate the real numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumptionBreakdown {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

impl ConsumptionBreakdown {
    pub fn add(&mut self, other: &ConsumptionBreakdown) {
        self.input += other.input;
        self.output += other.output;
        self.cached += other.cached;
    }

    /// Grand total across all three buckets (used for the chart bar height).
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cached
    }
}

/// One day's consumption bucket for the chart series.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayConsumption {
    /// Local date, `YYYY-MM-DD`.
    pub date: String,
    pub breakdown: ConsumptionBreakdown,
}

/// Per-provider consumption rollup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConsumption {
    pub provider: UsageProvider,
    pub today: ConsumptionBreakdown,
    pub this_week: ConsumptionBreakdown,
    /// Oldest-first series of the last [`CHART_DAYS`] local days (today last).
    pub days: Vec<DayConsumption>,
}

impl ProviderConsumption {
    pub fn empty(provider: UsageProvider, now: DateTime<Utc>) -> Self {
        aggregate(provider, Vec::new(), now, now)
    }
}

/// A single timestamped consumption delta.
pub type ConsumptionEvent = (DateTime<Utc>, ConsumptionBreakdown);

fn parse_ts(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let text = value?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Extract consumption events from Codex rollout JSONL. `input` excludes the
/// cached portion (`input_tokens - cached_input_tokens`).
pub fn parse_codex_events(jsonl: &str) -> Vec<ConsumptionEvent> {
    let mut events = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = obj.get("payload").unwrap_or(&obj);
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(ts) = parse_ts(obj.get("timestamp")) else {
            continue;
        };
        let last = payload.get("info").and_then(|i| i.get("last_token_usage"));
        let Some(last) = last else { continue };
        let input_total = last
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cached = last
            .get("cached_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = last
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        events.push((
            ts,
            ConsumptionBreakdown {
                input: input_total.saturating_sub(cached),
                output,
                cached,
            },
        ));
    }
    events
}

/// Extract consumption events from a Claude transcript JSONL. Claude's
/// `input_tokens` is already the uncached input; cache fields are separate.
pub fn parse_claude_events(jsonl: &str) -> Vec<ConsumptionEvent> {
    let mut events = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(ts) = parse_ts(obj.get("timestamp")) else {
            continue;
        };
        let Some(usage) = obj.get("message").and_then(|m| m.get("usage")) else {
            continue;
        };
        let input = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_create = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        events.push((
            ts,
            ConsumptionBreakdown {
                input,
                output,
                cached: cache_read + cache_create,
            },
        ));
    }
    events
}

/// Aggregate events into Today / This-week / 7-day-series, bucketing by the
/// **local** date of each event. `week_start` is the weekly rate-limit window
/// start (sum events at/after it); `now` defines "today".
pub fn aggregate(
    provider: UsageProvider,
    events: Vec<ConsumptionEvent>,
    week_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ProviderConsumption {
    let today_date = now.with_timezone(&Local).date_naive();
    let mut today = ConsumptionBreakdown::default();
    let mut this_week = ConsumptionBreakdown::default();
    let mut by_day: HashMap<NaiveDate, ConsumptionBreakdown> = HashMap::new();

    for (ts, breakdown) in &events {
        let local_date = ts.with_timezone(&Local).date_naive();
        if local_date == today_date {
            today.add(breakdown);
        }
        if *ts >= week_start {
            this_week.add(breakdown);
        }
        by_day.entry(local_date).or_default().add(breakdown);
    }

    let mut days = Vec::with_capacity(CHART_DAYS as usize);
    for offset in (0..CHART_DAYS).rev() {
        let date = today_date - Duration::days(offset);
        days.push(DayConsumption {
            date: date.format("%Y-%m-%d").to_string(),
            breakdown: by_day.get(&date).copied().unwrap_or_default(),
        });
    }

    ProviderConsumption {
        provider,
        today,
        this_week,
        days,
    }
}

fn scan_cutoff(now: DateTime<Utc>) -> SystemTime {
    let secs = (RANGE_DAYS as u64) * 24 * 60 * 60;
    // Anchor to wall-clock SystemTime; `now` is only used for date math.
    let _ = now;
    SystemTime::now()
        .checked_sub(StdDuration::from_secs(secs))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Read Codex consumption from recent rollouts under `home`.
pub fn read_codex_consumption(
    home: &Path,
    week_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ProviderConsumption {
    let paths = codex::rollouts_modified_since(home, scan_cutoff(now), MAX_FILES);
    let mut events = Vec::new();
    for path in &paths {
        if let Ok(text) = fs::read_to_string(path) {
            events.extend(parse_codex_events(&text));
        }
    }
    aggregate(UsageProvider::Codex, events, week_start, now)
}

/// Read Claude consumption from recent transcripts under `home`.
pub fn read_claude_consumption(
    home: &Path,
    week_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ProviderConsumption {
    let paths = claude::transcripts_modified_since(home, scan_cutoff(now), MAX_FILES);
    let mut events = Vec::new();
    for path in &paths {
        if let Ok(text) = fs::read_to_string(path) {
            events.extend(parse_claude_events(&text));
        }
    }
    aggregate(UsageProvider::ClaudeCode, events, week_start, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn codex_events_subtract_cached_from_input() {
        let jsonl = concat!(
            r#"{"timestamp":"2026-06-02T09:52:15.989Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":25228,"cached_input_tokens":9600,"output_tokens":78}}}}"#,
            "\n",
            r#"{"timestamp":"x","payload":{"type":"event_msg"}}"#,
        );
        let ev = parse_codex_events(jsonl);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].1.input, 25228 - 9600);
        assert_eq!(ev[0].1.cached, 9600);
        assert_eq!(ev[0].1.output, 78);
    }

    #[test]
    fn claude_events_sum_cache_fields() {
        let jsonl = r#"{"type":"assistant","timestamp":"2026-06-02T09:00:00Z","message":{"usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":140,"cache_creation_input_tokens":30}}}"#;
        let ev = parse_claude_events(jsonl);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].1.input, 100);
        assert_eq!(ev[0].1.output, 20);
        assert_eq!(ev[0].1.cached, 170);
    }

    #[test]
    fn aggregate_buckets_today_week_and_series() {
        // now = today 12:00Z. Use a wide week_start so all events count weekly.
        let now = ts("2026-06-02T12:00:00Z");
        let week_start = ts("2026-05-30T00:00:00Z");
        let b = |i, o, c| ConsumptionBreakdown {
            input: i,
            output: o,
            cached: c,
        };
        let events = vec![
            (ts("2026-06-02T01:00:00Z"), b(10, 1, 100)), // maybe today (tz dependent)
            (ts("2026-06-01T10:00:00Z"), b(20, 2, 200)), // yesterday-ish
            (ts("2026-05-20T10:00:00Z"), b(99, 9, 900)), // before week_start
        ];
        let pc = aggregate(UsageProvider::Codex, events, week_start, now);
        assert_eq!(pc.provider, UsageProvider::Codex);
        // 7-day series present, oldest first, ending today.
        assert_eq!(pc.days.len(), CHART_DAYS as usize);
        let today_local = now.with_timezone(&Local).date_naive();
        assert_eq!(
            pc.days.last().unwrap().date,
            today_local.format("%Y-%m-%d").to_string()
        );
        // The pre-week event must not be in this_week.
        assert!(pc.this_week.total() < b(99, 9, 900).total() + 1000);
        assert_eq!(pc.this_week.input, 30); // 10 + 20 (both >= week_start)
    }

    #[test]
    fn week_start_boundary_excludes_older() {
        let now = ts("2026-06-02T12:00:00Z");
        let week_start = ts("2026-06-02T00:00:00Z");
        let b = ConsumptionBreakdown {
            input: 5,
            output: 0,
            cached: 0,
        };
        let events = vec![
            (ts("2026-06-02T01:00:00Z"), b), // after week_start
            (ts("2026-06-01T23:00:00Z"), b), // before week_start
        ];
        let pc = aggregate(UsageProvider::ClaudeCode, events, week_start, now);
        assert_eq!(pc.this_week.input, 5);
    }

    #[test]
    fn empty_has_seven_zero_days() {
        let pc = ProviderConsumption::empty(UsageProvider::Codex, ts("2026-06-02T12:00:00Z"));
        assert_eq!(pc.days.len(), CHART_DAYS as usize);
        assert_eq!(pc.today, ConsumptionBreakdown::default());
        assert!(pc.days.iter().all(|d| d.breakdown.total() == 0));
    }

    #[test]
    fn breakdown_add_and_total_accumulate() {
        let mut acc = ConsumptionBreakdown::default();
        acc.add(&ConsumptionBreakdown {
            input: 3,
            output: 4,
            cached: 5,
        });
        acc.add(&ConsumptionBreakdown {
            input: 1,
            output: 1,
            cached: 1,
        });
        assert_eq!(acc.input, 4);
        assert_eq!(acc.output, 5);
        assert_eq!(acc.cached, 6);
        assert_eq!(acc.total(), 15);
    }

    #[test]
    fn read_codex_consumption_walks_rollout_tree() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/05/28");
        fs::create_dir_all(&day).unwrap();
        let now = ts("2026-05-28T12:00:00Z");
        // `input` excludes the cached portion: 300 - 100 = 200.
        let line = format!(
            r#"{{"timestamp":"{}","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":300,"cached_input_tokens":100,"output_tokens":50}}}}}}}}"#,
            now.to_rfc3339()
        );
        fs::write(day.join("rollout-2026-05-28T12-00-00-aaaa.jsonl"), line).unwrap();
        let week_start = now - Duration::days(1);
        let pc = read_codex_consumption(dir.path(), week_start, now);
        assert_eq!(pc.provider, UsageProvider::Codex);
        assert_eq!(pc.days.len(), CHART_DAYS as usize);
        assert_eq!(pc.this_week.input, 200);
        assert_eq!(pc.this_week.cached, 100);
        assert_eq!(pc.this_week.output, 50);
        // Event instant == `now`, so it buckets into "today".
        assert_eq!(pc.today.total(), 350);
    }

    #[test]
    fn read_claude_consumption_walks_project_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects/-Users-x");
        fs::create_dir_all(&proj).unwrap();
        let now = ts("2026-05-28T12:00:00Z");
        let line = format!(
            r#"{{"type":"assistant","timestamp":"{}","message":{{"usage":{{"input_tokens":80,"output_tokens":12,"cache_read_input_tokens":40,"cache_creation_input_tokens":5}}}}}}"#,
            now.to_rfc3339()
        );
        fs::write(proj.join("sid.jsonl"), line).unwrap();
        let week_start = now - Duration::days(1);
        let pc = read_claude_consumption(dir.path(), week_start, now);
        assert_eq!(pc.provider, UsageProvider::ClaudeCode);
        assert_eq!(pc.days.len(), CHART_DAYS as usize);
        assert_eq!(pc.this_week.input, 80);
        assert_eq!(pc.this_week.output, 12);
        assert_eq!(pc.this_week.cached, 45);
        assert_eq!(pc.today.total(), 80 + 12 + 45);
    }

    #[test]
    fn read_consumption_handles_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let now = ts("2026-05-28T12:00:00Z");
        let week_start = now - Duration::days(1);
        // No sessions/ or projects/ subtree → empty rollups, still 7 zero days.
        let codex = read_codex_consumption(dir.path(), week_start, now);
        assert_eq!(codex.days.len(), CHART_DAYS as usize);
        assert_eq!(codex.this_week.total(), 0);
        let claude = read_claude_consumption(dir.path(), week_start, now);
        assert_eq!(claude.days.len(), CHART_DAYS as usize);
        assert_eq!(claude.this_week.total(), 0);
    }
}

//! Read model and aggregation over the perf log (SPEC #3700 FR-007).
//!
//! The writer side ([`super::record::PerfRecord`]) is serialize-only by design:
//! it must never be able to read a record back into the hot path. Reading is a
//! separate, tolerant model that skips malformed lines rather than failing a
//! whole day of history.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use gwt_core::paths::gwt_logs_dir;
use serde::{Deserialize, Serialize};

use super::{budget::PerfBudgets, route::PerfRoute, OPERATION_ROLE_MUTATION, OPERATION_ROLE_READ};

/// Schema version of the `perf.summary` / `perf.violations` payloads.
pub const PERF_SUMMARY_SCHEMA_VERSION: u32 = 1;

const RECORD_TYPE_SAMPLE: &str = "sample";
const RECORD_TYPE_VIOLATION: &str = "violation";

/// Directory holding the daily perf logs.
pub fn perf_log_dir() -> PathBuf {
    gwt_logs_dir().join("perf")
}

/// One line of the perf log, read back tolerantly.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct PerfLogRecord {
    /// Writer schema version.
    pub schema_version: u32,
    /// `sample` or `violation`.
    #[serde(rename = "type")]
    pub record_type: String,
    /// When the measurement was taken.
    pub timestamp: DateTime<Utc>,
    /// `ui`, `op` or `resource`.
    pub stream: String,
    /// Sanitized target name.
    pub target: String,
    /// Measured value.
    pub value: f64,
    /// `ms`, `percent` or `bytes`.
    pub unit: String,
    /// Optional sub-classification, e.g. the read/mutation role of a gwtd op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Budget that was exceeded (violations only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
    /// Length of the over-budget run (violations only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consecutive_count: Option<u32>,
    /// Wall-clock length of the over-budget run (violations only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    /// Startup correlation and phase boundaries, absent on older samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<super::startup::StartupSample>,
}

impl PerfLogRecord {
    /// Whether this record is a sustained budget violation.
    pub fn is_violation(&self) -> bool {
        self.record_type == RECORD_TYPE_VIOLATION
    }

    /// Whether this record is a plain measurement.
    pub fn is_sample(&self) -> bool {
        self.record_type == RECORD_TYPE_SAMPLE
    }
}

/// Period, stream and target selection applied while reading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PerfFilter {
    /// Drop records older than this instant.
    pub since: Option<DateTime<Utc>>,
    /// Keep only this stream (`ui` / `op` / `resource`).
    pub stream: Option<String>,
    /// Keep only targets containing this substring.
    pub target: Option<String>,
}

impl PerfFilter {
    fn matches(&self, record: &PerfLogRecord) -> bool {
        if let Some(since) = self.since {
            if record.timestamp < since {
                return false;
            }
        }
        if let Some(stream) = self.stream.as_deref() {
            if record.stream != stream {
                return false;
            }
        }
        if let Some(target) = self.target.as_deref() {
            if !record.target.contains(target) {
                return false;
            }
        }
        true
    }
}

/// Read every perf record in `dir` that satisfies `filter`.
///
/// Unreadable files and malformed lines are skipped: a truncated tail from a
/// process that was killed mid-write must not hide the rest of the history.
/// The result is ordered oldest first.
pub fn read_records_from_dir(dir: &Path, filter: &PerfFilter) -> io::Result<Vec<PerfLogRecord>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("perf-") && name.ends_with(".jsonl"))
        })
        .collect();
    paths.sort();

    let mut records = Vec::new();
    for path in paths {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        for line in contents.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(record) = serde_json::from_str::<PerfLogRecord>(line) else {
                continue;
            };
            if filter.matches(&record) {
                records.push(record);
            }
        }
    }

    records.sort_by_key(|record| record.timestamp);
    Ok(records)
}

/// Read every perf record from the default perf log directory.
pub fn read_records(filter: &PerfFilter) -> io::Result<Vec<PerfLogRecord>> {
    read_records_from_dir(&perf_log_dir(), filter)
}

/// Aggregated latency for one stream/target pair.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerfTargetSummary {
    /// Stream the target belongs to.
    pub stream: String,
    /// Target name as written to the perf log.
    pub target: String,
    /// Unit shared by the aggregated samples.
    pub unit: String,
    /// Number of samples aggregated.
    pub count: usize,
    /// Median sample.
    pub p50: f64,
    /// 95th percentile sample.
    pub p95: f64,
    /// Worst sample observed.
    pub worst: f64,
    /// Budget in force for this target, when one is defined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<f64>,
    /// Whether `p95` exceeds `budget`.
    pub over_budget: bool,
    /// Sustained violations recorded for this target in the same period.
    pub violations: usize,
}

/// Full `perf.summary` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerfSummary {
    /// Payload schema version.
    pub schema_version: u32,
    /// Lower bound of the aggregated period, when one was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    /// Total samples aggregated.
    pub sample_count: usize,
    /// Total violations in the same period.
    pub violation_count: usize,
    /// Routes named by AC-1 that produced no sample in this period.
    pub missing_routes: Vec<String>,
    /// Per-target aggregates, worst p95 first.
    pub targets: Vec<PerfTargetSummary>,
}

/// Full `perf.violations` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerfViolationsReport {
    /// Payload schema version.
    pub schema_version: u32,
    /// Lower bound of the reported period, when one was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    /// Number of violations reported.
    pub count: usize,
    /// The violations themselves, oldest first.
    pub violations: Vec<PerfLogRecord>,
}

/// Budget in force for a target, or `None` when the target is unbudgeted.
pub fn budget_for_target(target: &str, role: Option<&str>, budgets: &PerfBudgets) -> Option<f64> {
    if let Some(route) = PerfRoute::from_target(target) {
        return Some(route.budget_ms(budgets));
    }
    match role {
        Some(OPERATION_ROLE_READ) => Some(budgets.gwtd_read_p95_ms),
        Some(OPERATION_ROLE_MUTATION) => Some(budgets.gwtd_mutation_p95_ms),
        _ => None,
    }
}

/// Nearest-rank percentile over an unsorted slice.
///
/// Returns `0.0` for an empty slice; `quantile` is clamped to `0.0..=1.0`.
pub fn percentile(values: &[f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let quantile = quantile.clamp(0.0, 1.0);
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

/// Aggregate records into the `perf.summary` payload.
pub fn summarize(
    records: &[PerfLogRecord],
    budgets: &PerfBudgets,
    since: Option<DateTime<Utc>>,
) -> PerfSummary {
    // Indexed by stream+target so aggregation stays linear in the record count
    // regardless of how many distinct targets a long period accumulated.
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut buckets: Vec<Bucket> = Vec::new();

    for record in records.iter().filter(|record| record.is_sample()) {
        let key = (record.stream.clone(), record.target.clone());
        let slot = *index.entry(key).or_insert_with(|| {
            buckets.push(Bucket::new(record));
            buckets.len() - 1
        });
        buckets[slot].values.push(record.value);
    }

    for record in records.iter().filter(|record| record.is_violation()) {
        let key = (record.stream.clone(), record.target.clone());
        let slot = *index.entry(key).or_insert_with(|| {
            buckets.push(Bucket::new(record));
            buckets.len() - 1
        });
        buckets[slot].violations += 1;
    }

    let mut targets: Vec<PerfTargetSummary> = buckets
        .into_iter()
        .map(|bucket| bucket.finish(budgets))
        .collect();
    targets.sort_by(|left, right| {
        right
            .p95
            .partial_cmp(&left.p95)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.target.cmp(&right.target))
    });

    let missing_routes = PerfRoute::ALL
        .into_iter()
        .filter(|route| {
            let target = route.target();
            !targets
                .iter()
                .any(|summary| summary.target == target && summary.count > 0)
        })
        .map(|route| route.name().to_string())
        .collect();

    PerfSummary {
        schema_version: PERF_SUMMARY_SCHEMA_VERSION,
        since,
        sample_count: records.iter().filter(|record| record.is_sample()).count(),
        violation_count: records
            .iter()
            .filter(|record| record.is_violation())
            .count(),
        missing_routes,
        targets,
    }
}

/// Collect the violations of a period into the `perf.violations` payload.
pub fn violations(records: &[PerfLogRecord], since: Option<DateTime<Utc>>) -> PerfViolationsReport {
    let violations: Vec<PerfLogRecord> = records
        .iter()
        .filter(|record| record.is_violation())
        .cloned()
        .collect();

    PerfViolationsReport {
        schema_version: PERF_SUMMARY_SCHEMA_VERSION,
        since,
        count: violations.len(),
        violations,
    }
}

struct Bucket {
    stream: String,
    target: String,
    unit: String,
    role: Option<String>,
    values: Vec<f64>,
    violations: usize,
}

impl Bucket {
    fn new(record: &PerfLogRecord) -> Self {
        Self {
            stream: record.stream.clone(),
            target: record.target.clone(),
            unit: record.unit.clone(),
            role: record.role.clone(),
            values: Vec::new(),
            violations: 0,
        }
    }

    fn finish(self, budgets: &PerfBudgets) -> PerfTargetSummary {
        let budget = budget_for_target(&self.target, self.role.as_deref(), budgets);
        let p95 = percentile(&self.values, 0.95);
        let worst = self
            .values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);

        PerfTargetSummary {
            stream: self.stream,
            target: self.target,
            unit: self.unit,
            count: self.values.len(),
            p50: percentile(&self.values, 0.5),
            p95,
            worst: if self.values.is_empty() { 0.0 } else { worst },
            over_budget: budget.is_some_and(|budget| p95 > budget) && !self.values.is_empty(),
            budget,
            violations: self.violations,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, hour, minute, 0)
            .single()
            .expect("valid timestamp")
    }

    fn sample_line(timestamp: DateTime<Utc>, stream: &str, target: &str, value: f64) -> String {
        serde_json::json!({
            "schema_version": 1,
            "type": "sample",
            "timestamp": timestamp,
            "stream": stream,
            "target": target,
            "value": value,
            "unit": "ms"
        })
        .to_string()
    }

    fn write_log(dir: &Path, day: &str, lines: &[String]) {
        fs::create_dir_all(dir).expect("create perf dir");
        let mut body = lines.join("\n");
        body.push('\n');
        fs::write(dir.join(format!("perf-{day}.jsonl")), body).expect("write perf log");
    }

    #[test]
    fn reads_every_daily_file_oldest_first_and_skips_malformed_lines() {
        let home = tempfile::tempdir().expect("tempdir");
        let dir = home.path().join("perf");
        write_log(
            &dir,
            "2026-09-07",
            &[sample_line(at(9, 0), "ui", "route:search", 10.0)],
        );
        write_log(
            &dir,
            "2026-09-08",
            &[
                "{ this is not json".to_string(),
                String::new(),
                sample_line(at(10, 0), "ui", "route:search", 20.0),
            ],
        );

        let records =
            read_records_from_dir(&dir, &PerfFilter::default()).expect("read perf records");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].value, 10.0);
        assert_eq!(records[1].value, 20.0);
    }

    #[test]
    fn a_missing_perf_directory_reads_as_an_empty_history() {
        let home = tempfile::tempdir().expect("tempdir");

        let records = read_records_from_dir(&home.path().join("absent"), &PerfFilter::default())
            .expect("missing dir is not an error");

        assert!(records.is_empty());
    }

    #[test]
    fn filters_narrow_by_period_stream_and_target() {
        let home = tempfile::tempdir().expect("tempdir");
        let dir = home.path().join("perf");
        write_log(
            &dir,
            "2026-09-08",
            &[
                sample_line(at(9, 0), "ui", "route:search", 10.0),
                sample_line(at(11, 0), "ui", "route:pane.close", 20.0),
                sample_line(at(11, 0), "op", "gwtd:issue.view", 30.0),
            ],
        );

        let since_only = read_records_from_dir(
            &dir,
            &PerfFilter {
                since: Some(at(10, 0)),
                ..PerfFilter::default()
            },
        )
        .expect("read");
        assert_eq!(since_only.len(), 2);

        let stream_only = read_records_from_dir(
            &dir,
            &PerfFilter {
                stream: Some("op".to_string()),
                ..PerfFilter::default()
            },
        )
        .expect("read");
        assert_eq!(stream_only.len(), 1);
        assert_eq!(stream_only[0].target, "gwtd:issue.view");

        let target_only = read_records_from_dir(
            &dir,
            &PerfFilter {
                target: Some("route:".to_string()),
                ..PerfFilter::default()
            },
        )
        .expect("read");
        assert_eq!(target_only.len(), 2);
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();

        assert_eq!(percentile(&values, 0.5), 50.0);
        assert_eq!(percentile(&values, 0.95), 95.0);
        assert_eq!(percentile(&values, 1.0), 100.0);
        assert_eq!(percentile(&[], 0.95), 0.0);
        assert_eq!(percentile(&[7.0], 0.95), 7.0);
    }

    #[test]
    fn summary_reports_p50_p95_worst_and_budget_state_per_target() {
        let budgets = PerfBudgets::default();
        let mut records: Vec<PerfLogRecord> = (1..=100)
            .map(|step| PerfLogRecord {
                schema_version: 1,
                record_type: "sample".to_string(),
                timestamp: at(10, 0),
                stream: "ui".to_string(),
                target: "route:project.switch".to_string(),
                value: f64::from(step),
                unit: "ms".to_string(),
                role: None,
                budget: None,
                consecutive_count: None,
                duration_seconds: None,
                startup: None,
            })
            .collect();
        records.push(PerfLogRecord {
            schema_version: 1,
            record_type: "violation".to_string(),
            timestamp: at(10, 1),
            stream: "ui".to_string(),
            target: "route:project.switch".to_string(),
            value: 300.0,
            unit: "ms".to_string(),
            role: None,
            budget: Some(100.0),
            consecutive_count: Some(3),
            duration_seconds: Some(1.5),
            startup: None,
        });

        let summary = summarize(&records, &budgets, Some(at(9, 0)));

        assert_eq!(summary.sample_count, 100);
        assert_eq!(summary.violation_count, 1);
        assert_eq!(summary.targets.len(), 1);
        let target = &summary.targets[0];
        assert_eq!(target.target, "route:project.switch");
        assert_eq!(target.count, 100);
        assert_eq!(target.p50, 50.0);
        assert_eq!(target.p95, 95.0);
        assert_eq!(target.worst, 100.0);
        assert_eq!(target.budget, Some(100.0));
        assert!(!target.over_budget);
        assert_eq!(target.violations, 1);
    }

    #[test]
    fn summary_flags_a_p95_over_its_budget() {
        let records: Vec<PerfLogRecord> = (0..10)
            .map(|_| PerfLogRecord {
                schema_version: 1,
                record_type: "sample".to_string(),
                timestamp: at(10, 0),
                stream: "ui".to_string(),
                target: "route:pane.close".to_string(),
                value: 250.0,
                unit: "ms".to_string(),
                role: None,
                budget: None,
                consecutive_count: None,
                duration_seconds: None,
                startup: None,
            })
            .collect();

        let summary = summarize(&records, &PerfBudgets::default(), None);

        assert!(summary.targets[0].over_budget);
    }

    #[test]
    fn summary_names_the_ac_1_routes_that_produced_no_sample() {
        let records = vec![PerfLogRecord {
            schema_version: 1,
            record_type: "sample".to_string(),
            timestamp: at(10, 0),
            stream: "ui".to_string(),
            target: "route:startup".to_string(),
            value: 900.0,
            unit: "ms".to_string(),
            role: None,
            budget: None,
            consecutive_count: None,
            duration_seconds: None,
            startup: None,
        }];

        let summary = summarize(&records, &PerfBudgets::default(), None);

        assert!(!summary.missing_routes.contains(&"startup".to_string()));
        assert!(summary.missing_routes.contains(&"search".to_string()));
        assert_eq!(summary.missing_routes.len(), PerfRoute::ALL.len() - 1);
    }

    #[test]
    fn operation_budgets_are_resolved_from_the_recorded_role() {
        let budgets = PerfBudgets::default();

        assert_eq!(
            budget_for_target("gwtd:issue.view", Some(OPERATION_ROLE_READ), &budgets),
            Some(100.0)
        );
        assert_eq!(
            budget_for_target("gwtd:pr.create", Some(OPERATION_ROLE_MUTATION), &budgets),
            Some(500.0)
        );
        assert_eq!(budget_for_target("gwtd:issue.view", None, &budgets), None);
        assert_eq!(
            budget_for_target("route:search", None, &budgets),
            Some(super::super::route::DEFAULT_SEARCH_BUDGET_MS)
        );
    }

    #[test]
    fn violations_report_keeps_only_violation_records() {
        let records = vec![
            PerfLogRecord {
                schema_version: 1,
                record_type: "sample".to_string(),
                timestamp: at(10, 0),
                stream: "ui".to_string(),
                target: "route:search".to_string(),
                value: 10.0,
                unit: "ms".to_string(),
                role: None,
                budget: None,
                consecutive_count: None,
                duration_seconds: None,
                startup: None,
            },
            PerfLogRecord {
                schema_version: 1,
                record_type: "violation".to_string(),
                timestamp: at(10, 1),
                stream: "ui".to_string(),
                target: "route:search".to_string(),
                value: 5_000.0,
                unit: "ms".to_string(),
                role: None,
                budget: Some(2_000.0),
                consecutive_count: Some(3),
                duration_seconds: Some(4.0),
                startup: None,
            },
        ];

        let report = violations(&records, None);

        assert_eq!(report.count, 1);
        assert_eq!(report.violations[0].value, 5_000.0);
        assert_eq!(report.violations[0].budget, Some(2_000.0));
    }
}

//! `perf.summary` and `perf.violations` JSON operations (SPEC #3700 FR-007).
//!
//! The read side of the always-on performance collector. Both operations are
//! read-only: they aggregate `~/.gwt/logs/perf/perf-<date>.jsonl` and never
//! write, so a PM sweep cannot perturb what it is measuring.

use chrono::{DateTime, Utc};
use gwt_config::Settings;
use gwt_github::{client::ApiError, SpecOpsError};

use crate::{
    cli::{CliEnv, CliParseError},
    perf::{
        budget::PerfBudgets,
        summary::{read_records, summarize, violations, PerfFilter},
    },
};

/// Streams accepted by the `stream` filter.
pub const PERF_STREAMS: &[&str] = &["ui", "op", "resource"];

/// `perf.*` command model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerfCommand {
    /// `perf.startup` — the latest process startup, without mixing runs.
    Startup,
    /// `perf.summary` — p50 / p95 / worst per stream and target.
    Summary {
        /// RFC3339 lower bound of the aggregated period.
        since: Option<String>,
        /// Stream filter (`ui` / `op` / `resource`).
        stream: Option<String>,
        /// Substring the target must contain.
        target: Option<String>,
    },
    /// `perf.violations` — sustained budget violations of the period.
    Violations {
        /// RFC3339 lower bound of the reported period.
        since: Option<String>,
        /// Stream filter (`ui` / `op` / `resource`).
        stream: Option<String>,
        /// Substring the target must contain.
        target: Option<String>,
    },
}

/// Parse and validate the RFC3339 `since` bound shared by both operations.
pub(crate) fn parse_since(raw: &str) -> Result<DateTime<Utc>, CliParseError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| CliParseError::InvalidValue {
            flag: "since",
            reason: "must be RFC3339",
        })
}

/// Validate the `stream` filter against the perf record schema.
pub(crate) fn parse_stream(raw: &str) -> Result<String, CliParseError> {
    if PERF_STREAMS.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(CliParseError::InvalidValue {
            flag: "stream",
            reason: "must be one of ui, op, resource",
        })
    }
}

fn build_filter(
    since: Option<String>,
    stream: Option<String>,
    target: Option<String>,
) -> Result<(PerfFilter, Option<DateTime<Utc>>), SpecOpsError> {
    let since = since
        .as_deref()
        .map(parse_since)
        .transpose()
        .map_err(|error| SpecOpsError::from(ApiError::Network(error.to_string())))?;
    Ok((
        PerfFilter {
            since,
            stream,
            target,
        },
        since,
    ))
}

fn resolved_budgets() -> PerfBudgets {
    PerfBudgets::resolve(&Settings::load().unwrap_or_default().perf.budgets)
}

fn render(payload: &impl serde::Serialize, out: &mut String) -> Result<(), SpecOpsError> {
    let rendered = serde_json::to_string_pretty(payload)
        .map_err(|error| SpecOpsError::from(ApiError::Network(error.to_string())))?;
    out.push_str(&rendered);
    out.push('\n');
    Ok(())
}

/// Run one `perf.*` operation.
pub fn run<E: CliEnv>(
    env: &mut E,
    command: PerfCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let _ = env;
    match command {
        PerfCommand::Startup => {
            let records = read_records(&PerfFilter {
                target: Some("startup:".to_string()),
                ..PerfFilter::default()
            })
            .map_err(|error| SpecOpsError::from(ApiError::Network(error.to_string())))?;
            render(
                &serde_json::json!({
                    "schema_version": 1,
                    "startup": crate::perf::startup::latest_startup(&records),
                }),
                out,
            )?;
            Ok(0)
        }
        PerfCommand::Summary {
            since,
            stream,
            target,
        } => {
            let (filter, since) = build_filter(since, stream, target)?;
            let records = read_records(&filter)
                .map_err(|error| SpecOpsError::from(ApiError::Network(error.to_string())))?;
            render(&summarize(&records, &resolved_budgets(), since), out)?;
            Ok(0)
        }
        PerfCommand::Violations {
            since,
            stream,
            target,
        } => {
            let (filter, since) = build_filter(since, stream, target)?;
            let records = read_records(&filter)
                .map_err(|error| SpecOpsError::from(ApiError::Network(error.to_string())))?;
            render(&violations(&records, since), out)?;
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};

    use super::*;
    use crate::cli::TestEnv;

    fn seed_perf_log(lines: &[serde_json::Value]) {
        let dir = gwt_logs_dir().join("perf");
        fs::create_dir_all(&dir).expect("create perf dir");
        let mut body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        body.push('\n');
        fs::write(dir.join("perf-2026-09-08.jsonl"), body).expect("write perf log");
    }

    fn sample(target: &str, stream: &str, value: f64) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "type": "sample",
            "timestamp": "2026-09-08T10:00:00Z",
            "stream": stream,
            "target": target,
            "value": value,
            "unit": "ms"
        })
    }

    #[test]
    fn summary_reports_percentiles_and_budget_state_per_route() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        seed_perf_log(&[
            sample("route:project.switch", "ui", 10.0),
            sample("route:project.switch", "ui", 20.0),
            sample("route:pane.close", "ui", 400.0),
        ]);

        let mut env = TestEnv::new(home.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            PerfCommand::Summary {
                since: None,
                stream: None,
                target: None,
            },
            &mut out,
        )
        .expect("perf.summary runs");

        assert_eq!(code, 0);
        let payload: serde_json::Value = serde_json::from_str(&out).expect("valid JSON payload");
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["sample_count"], 3);
        let targets = payload["targets"].as_array().expect("targets array");
        assert_eq!(targets.len(), 2);
        let slowest = &targets[0];
        assert_eq!(slowest["target"], "route:pane.close");
        assert_eq!(slowest["p95"], 400.0);
        assert_eq!(slowest["budget"], 100.0);
        assert_eq!(slowest["over_budget"], true);
    }

    #[test]
    fn summary_honours_the_stream_and_target_filters() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        seed_perf_log(&[
            sample("route:search", "ui", 10.0),
            sample("gwtd:issue.view", "op", 20.0),
        ]);

        let mut env = TestEnv::new(home.path().to_path_buf());
        let mut out = String::new();
        run(
            &mut env,
            PerfCommand::Summary {
                since: None,
                stream: Some("op".to_string()),
                target: Some("issue".to_string()),
            },
            &mut out,
        )
        .expect("perf.summary runs");

        let payload: serde_json::Value = serde_json::from_str(&out).expect("valid JSON payload");
        assert_eq!(payload["sample_count"], 1);
        assert_eq!(payload["targets"][0]["target"], "gwtd:issue.view");
    }

    #[test]
    fn violations_returns_only_sustained_violations() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        seed_perf_log(&[
            sample("route:pane.close", "ui", 400.0),
            serde_json::json!({
                "schema_version": 1,
                "type": "violation",
                "timestamp": "2026-09-08T10:00:01Z",
                "stream": "ui",
                "target": "route:pane.close",
                "value": 400.0,
                "unit": "ms",
                "budget": 100.0,
                "consecutive_count": 3,
                "duration_seconds": 1.25
            }),
        ]);

        let mut env = TestEnv::new(home.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            PerfCommand::Violations {
                since: None,
                stream: None,
                target: None,
            },
            &mut out,
        )
        .expect("perf.violations runs");

        assert_eq!(code, 0);
        let payload: serde_json::Value = serde_json::from_str(&out).expect("valid JSON payload");
        assert_eq!(payload["count"], 1);
        assert_eq!(payload["violations"][0]["target"], "route:pane.close");
        assert_eq!(payload["violations"][0]["consecutive_count"], 3);
    }

    #[test]
    fn an_empty_perf_history_is_an_empty_summary_not_an_error() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());

        let mut env = TestEnv::new(home.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            PerfCommand::Summary {
                since: None,
                stream: None,
                target: None,
            },
            &mut out,
        )
        .expect("perf.summary runs against an empty history");

        assert_eq!(code, 0);
        let payload: serde_json::Value = serde_json::from_str(&out).expect("valid JSON payload");
        assert_eq!(payload["sample_count"], 0);
        assert_eq!(
            payload["missing_routes"].as_array().expect("array").len(),
            7
        );
    }

    #[test]
    fn stream_and_since_filters_reject_malformed_values() {
        assert!(parse_stream("ui").is_ok());
        assert!(parse_stream("frontend").is_err());
        assert!(parse_since("2026-09-08T10:00:00Z").is_ok());
        assert!(parse_since("yesterday").is_err());
    }
}

//! `index.repair` response contract (Issue #4435).
//!
//! `index.repair` used to answer with one word per collection — and only
//! after the whole coordinated job had settled, which on a failing `issues`
//! index is the full ten-minute `ISSUE_INDEX_BUILD_TIMEOUT` window. A caller
//! therefore saw a byte-silent process for ten minutes and then a verdict
//! that named neither the collection's health, nor the job, nor the
//! `last_error` / `actual` / `expected` triple that `index.status` reads
//! straight out of `<index-root>/<repo-hash>/issues/repair.json`.
//!
//! This module owns the shape of the answer: every outcome renders a
//! non-empty, greppable block that states what happened to which collection,
//! under which job identifier, and what the index's own repair state says.

use serde_json::Value;

/// What `index.repair` did, as reported back to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepairOutcome {
    /// The collection is already healthy, so no job was started.
    NotRequired,
    /// A repair job was handed to a detached worker and is now running.
    Submitted,
    /// A waiting repair ran to completion.
    Completed,
    /// A waiting repair ran and failed.
    Failed { detail: String },
    /// The repair could not be started at all.
    Refused { detail: String },
}

impl RepairOutcome {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Refused { .. } => "refused",
        }
    }

    fn detail(&self) -> Option<&str> {
        match self {
            Self::Failed { detail } | Self::Refused { detail } => Some(detail.as_str()),
            _ => None,
        }
    }

    /// Exit code the operation answers with. Only an outcome the caller has
    /// to act on is a failure; a healthy collection and a running job are not.
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Failed { .. } | Self::Refused { .. } => 1,
            _ => 0,
        }
    }
}

/// The status runner's entry for one collection, when it has one.
pub(crate) fn scope_status<'a>(payload: &'a Value, collection: &str) -> Option<&'a Value> {
    payload.get("status")?.get(collection)
}

/// Whether a repair should run for the collection this status describes.
///
/// Absence is never health. A status we could not read, or one that does not
/// mention the collection at all, is exactly the situation `index.repair`
/// exists for, so it must not be reported as "nothing to do".
pub(crate) fn repair_required(scope_status: Option<&Value>) -> bool {
    let Some(scope) = scope_status else {
        return true;
    };
    if let Some(required) = scope.get("repair_required").and_then(Value::as_bool) {
        return required;
    }
    !scope
        .get("healthy")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Stable identifier for one repair run, so the submission response and the
/// worker that performs it name the same job.
pub(crate) fn job_id(repo_hash: &str, collection: &str, run: &str) -> String {
    format!("{collection}@{repo_hash}/{run}")
}

/// A run token for a freshly submitted repair.
pub(crate) fn new_run_token() -> String {
    format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        std::process::id()
    )
}

/// Render the answer. Always writes at least the `status` line, whatever the
/// outcome and however little the status probe returned.
pub(crate) fn render_report(
    out: &mut String,
    collection: &str,
    job: &str,
    outcome: &RepairOutcome,
    scope_status: Option<&Value>,
    health_error: Option<&str>,
) {
    out.push_str(&format!(
        "index.repair: collection={collection} status={} job={job}\n",
        outcome.label()
    ));
    if let Some(detail) = outcome.detail() {
        out.push_str(&format!("index.repair: detail={}\n", single_line(detail)));
    }
    match scope_status {
        Some(scope) => {
            out.push_str(&format!(
                "index.repair: health reason={} documents={} repair_required={} mode={}\n",
                scope
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown"),
                scope
                    .get("document_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                repair_required(Some(scope)),
                scope.get("mode").and_then(Value::as_str).unwrap_or("full"),
            ));
            match scope.get("repair") {
                Some(state) => out.push_str(&format!(
                    "index.repair: repair last_error={} failures={} actual={} expected={}\n",
                    state
                        .get("last_error")
                        .and_then(Value::as_str)
                        .unwrap_or("none"),
                    state.get("failures").and_then(Value::as_u64).unwrap_or(0),
                    state
                        .get("actual_document_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    state
                        .get("expected_document_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )),
                None => out.push_str("index.repair: repair last_error=none\n"),
            }
        }
        None => {
            out.push_str(&format!(
                "index.repair: health unavailable error={}\n",
                single_line(health_error.unwrap_or("the status runner reported no collection"))
            ));
        }
    }
    if matches!(outcome, RepairOutcome::Submitted) {
        out.push_str("index.repair: track progress with the index.status JSON operation\n");
    }
}

/// Collapse a multi-line runner failure into one field-safe line.
fn single_line(detail: &str) -> String {
    let collapsed = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "unspecified".to_string()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unhealthy_scope() -> Value {
        json!({
            "healthy": false,
            "repair_required": true,
            "reason": "repair_stopped",
            "document_count": 0,
            "mode": "full",
            "repair": {
                "last_error": "BUILD_INCOMPLETE",
                "failures": 1,
                "actual_document_count": 0,
                "expected_document_count": 1130
            }
        })
    }

    fn all_outcomes() -> Vec<RepairOutcome> {
        vec![
            RepairOutcome::NotRequired,
            RepairOutcome::Submitted,
            RepairOutcome::Completed,
            RepairOutcome::Failed {
                detail: "runner exit=1 detail=BUILD_INCOMPLETE".to_string(),
            },
            RepairOutcome::Refused {
                detail: "index job admission failed".to_string(),
            },
        ]
    }

    /// Issue #4435 AC-1: no outcome may answer with nothing.
    #[test]
    fn every_outcome_renders_a_non_empty_report() {
        for outcome in all_outcomes() {
            for scope in [Some(unhealthy_scope()), None] {
                let mut out = String::new();
                render_report(&mut out, "issues", "job-1", &outcome, scope.as_ref(), None);
                assert!(!out.is_empty(), "{outcome:?} rendered nothing");
                assert!(
                    out.contains("collection=issues"),
                    "{outcome:?} lost the collection: {out}"
                );
                assert!(
                    out.contains(&format!("status={}", outcome.label())),
                    "{outcome:?} lost its status: {out}"
                );
                assert!(out.contains("job=job-1"), "{outcome:?} lost the job: {out}");
            }
        }
    }

    /// Issue #4435 AC-2: a submitted job names itself and says how to follow it.
    #[test]
    fn a_submitted_job_is_traceable_from_the_response() {
        let mut out = String::new();
        render_report(
            &mut out,
            "issues",
            &job_id("deadbeefdeadbeef", "issues", "run-7"),
            &RepairOutcome::Submitted,
            Some(&unhealthy_scope()),
            None,
        );
        assert!(out.contains("job=issues@deadbeefdeadbeef/run-7"), "{out}");
        assert!(out.contains("index.status"), "{out}");
    }

    /// Issue #4435 AC-3: the failure verdict carries the index's own repair
    /// state, so the caller does not have to run `index.status` to learn why.
    #[test]
    fn a_failed_repair_reports_last_error_and_document_counts() {
        let mut out = String::new();
        render_report(
            &mut out,
            "issues",
            "job-1",
            &RepairOutcome::Failed {
                detail: "runner exit=1\ndetail=BUILD_INCOMPLETE".to_string(),
            },
            Some(&unhealthy_scope()),
            None,
        );
        assert!(out.contains("last_error=BUILD_INCOMPLETE"), "{out}");
        assert!(out.contains("actual=0"), "{out}");
        assert!(out.contains("expected=1130"), "{out}");
        assert!(out.contains("failures=1"), "{out}");
        // The runner detail survives, collapsed onto one field-safe line.
        assert!(
            out.contains("detail=runner exit=1 detail=BUILD_INCOMPLETE"),
            "{out}"
        );
    }

    /// Issue #4435 AC-4: a healthy collection says so instead of exiting mute.
    #[test]
    fn a_healthy_collection_is_reported_as_not_required() {
        let healthy = json!({
            "healthy": true,
            "repair_required": false,
            "reason": "ready",
            "document_count": 1130,
            "mode": "incremental"
        });
        assert!(!repair_required(Some(&healthy)));

        let mut out = String::new();
        render_report(
            &mut out,
            "issues",
            "job-1",
            &RepairOutcome::NotRequired,
            Some(&healthy),
            None,
        );
        assert!(out.contains("status=not_required"), "{out}");
        assert!(out.contains("documents=1130"), "{out}");
        assert!(out.contains("repair_required=false"), "{out}");
        assert_eq!(RepairOutcome::NotRequired.exit_code(), 0);
    }

    #[test]
    fn an_unreadable_status_still_asks_for_a_repair_and_says_why() {
        assert!(repair_required(None));
        assert!(repair_required(Some(&json!({}))));

        let mut out = String::new();
        render_report(
            &mut out,
            "issues",
            "job-1",
            &RepairOutcome::Submitted,
            None,
            Some("project index status returned invalid JSON: expected value"),
        );
        assert!(out.contains("health unavailable"), "{out}");
        assert!(out.contains("invalid JSON"), "{out}");
    }

    #[test]
    fn scope_status_reads_the_named_collection() {
        let payload = json!({"status": {"issues": {"healthy": true}}});
        assert!(scope_status(&payload, "issues").is_some());
        assert!(scope_status(&payload, "specs").is_none());
        assert!(scope_status(&json!({}), "issues").is_none());
    }

    #[test]
    fn only_actionable_outcomes_exit_non_zero() {
        assert_eq!(RepairOutcome::Submitted.exit_code(), 0);
        assert_eq!(RepairOutcome::Completed.exit_code(), 0);
        assert_eq!(
            RepairOutcome::Failed {
                detail: "x".to_string()
            }
            .exit_code(),
            1
        );
        assert_eq!(
            RepairOutcome::Refused {
                detail: "x".to_string()
            }
            .exit_code(),
            1
        );
    }

    #[test]
    fn a_run_token_is_unique_per_call() {
        let first = new_run_token();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_ne!(first, new_run_token());
    }
}

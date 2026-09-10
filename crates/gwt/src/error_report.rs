//! Fail-open bridge from gwt/gwtd error sites into the host error ledger.
//!
//! Issue #3778: every recorded row is also published on the daemon `errors`
//! channel when a project root is known. Publish failure never blocks the
//! originating operation.

use chrono::{Duration, Utc};
use gwt_core::error_ledger::{ErrorKind, ErrorRecord, ErrorTarget};

use crate::protocol::BackendEvent;

pub const ERRORS_CHANNEL: &str = "errors";

/// Append one error to the host ledger and fan it out on `errors`.
///
/// JSON operation refusals use [`report_error`] without publish so a rejected
/// `daemon.subscribe` cannot handshake the cwd daemon as a side effect.
pub fn report_error(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) {
    report_error_with_publish(kind, message, target, false);
}

/// Record the error and publish it on the daemon `errors` channel when a
/// project root is known. Used by launch, hook, daemon, and toast paths.
pub fn report_error_and_publish(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) {
    report_error_with_publish(kind, message, target, true);
}

fn report_error_with_publish(
    kind: ErrorKind,
    message: impl Into<String>,
    target: ErrorTarget,
    publish: bool,
) {
    let record = ErrorRecord::new(kind, message, target.clone());
    if recently_recorded(&record) {
        return;
    }
    match gwt_core::error_ledger::record(record) {
        Ok(recorded) if publish => publish_recorded(&recorded, target.project_root.as_deref()),
        Ok(_) => {}
        Err(error) => tracing::warn!(error = %error, "error ledger append failed"),
    }
}

/// Record GUI-visible error events so toast display and the ledger stay aligned.
pub fn record_backend_event(event: &BackendEvent) {
    match event {
        BackendEvent::IssueMonitorToast {
            level,
            message,
            issue_number,
        } if level.eq_ignore_ascii_case("error") => {
            report_error_and_publish(
                ErrorKind::LaunchFailure,
                message,
                ErrorTarget {
                    issue: *issue_number,
                    ..ErrorTarget::default()
                },
            );
        }
        BackendEvent::IssueMonitorLaunchFailed {
            issue_number,
            message,
        } => {
            report_error_and_publish(
                ErrorKind::LaunchFailure,
                message,
                ErrorTarget {
                    issue: Some(*issue_number),
                    ..ErrorTarget::default()
                },
            );
        }
        _ => {}
    }
}

fn recently_recorded(record: &ErrorRecord) -> bool {
    let since = Utc::now() - Duration::seconds(5);
    gwt_core::error_ledger::list_since(Some(since))
        .ok()
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.kind == record.kind && row.message == record.message)
        })
}

fn publish_recorded(record: &ErrorRecord, project_root: Option<&str>) {
    let Some(project_root) = project_root.filter(|root| !root.trim().is_empty()) else {
        return;
    };
    let Ok(payload) = serde_json::to_value(record) else {
        return;
    };
    #[cfg(unix)]
    {
        let _ = crate::daemon_publisher::publish_event(
            std::path::Path::new(project_root),
            ERRORS_CHANNEL,
            payload,
        );
    }
    #[cfg(not(unix))]
    {
        let _ = (project_root, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::ScopedGwtHome;

    fn isolated_home() -> (tempfile::TempDir, ScopedGwtHome) {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = ScopedGwtHome::set(dir.path().join("gwt-home"));
        (dir, home)
    }

    #[test]
    fn error_toast_backend_event_is_written_to_the_ledger() {
        let (_dir, _home) = isolated_home();
        record_backend_event(&BackendEvent::IssueMonitorToast {
            level: "error".into(),
            message: "stale generation launch failed".into(),
            issue_number: Some(3778),
        });

        let listed = gwt_core::error_ledger::list_since(None).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].kind, ErrorKind::LaunchFailure);
        assert_eq!(listed[0].message, "stale generation launch failed");
        assert_eq!(listed[0].target.issue, Some(3778));
    }

    #[test]
    fn info_toasts_are_not_recorded() {
        let (_dir, _home) = isolated_home();
        record_backend_event(&BackendEvent::IssueMonitorToast {
            level: "info".into(),
            message: "scan complete".into(),
            issue_number: None,
        });
        assert!(gwt_core::error_ledger::list_since(None)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn duplicate_error_toasts_within_five_seconds_are_not_repeated() {
        let (_dir, _home) = isolated_home();
        let event = BackendEvent::IssueMonitorToast {
            level: "error".into(),
            message: "same failure".into(),
            issue_number: Some(1),
        };
        record_backend_event(&event);
        record_backend_event(&event);
        assert_eq!(
            gwt_core::error_ledger::list_since(None)
                .expect("list")
                .len(),
            1
        );
    }
}

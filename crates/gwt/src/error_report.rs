//! Fail-open bridge from gwt/gwtd error sites into the host error ledger.
//!
//! Issue #3778: every recorded row is also published on the daemon `errors`
//! channel when a project root is known. Publish failure never blocks the
//! originating operation.

use std::collections::BTreeMap;
use std::sync::{mpsc, OnceLock};

use chrono::Duration;
use gwt_core::error_ledger::{ErrorKind, ErrorRecord, ErrorTarget};
use sha2::{Digest as _, Sha256};

use crate::protocol::BackendEvent;

pub const ERRORS_CHANNEL: &str = "errors";

/// Append one error to the host ledger and fan it out on `errors`.
///
/// JSON operation refusals use [`report_error`] without publish so a rejected
/// `daemon.subscribe` cannot handshake the cwd daemon as a side effect.
pub fn report_error(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) {
    report_error_with_publish(kind, message, target, BTreeMap::new(), false);
}

/// Record the error and publish it on the daemon `errors` channel when a
/// project root is known. Used by launch, hook, daemon, and toast paths.
pub fn report_error_and_publish(kind: ErrorKind, message: impl Into<String>, target: ErrorTarget) {
    report_error_with_publish(kind, message, target, BTreeMap::new(), true);
}

/// [`report_error_and_publish`] with structured detail (Issue #3541). Returns
/// the appended row so the caller can point the user at it, or `None` when the
/// row was suppressed as a recent duplicate or the append failed.
pub fn report_error_and_publish_with_context(
    kind: ErrorKind,
    message: impl Into<String>,
    target: ErrorTarget,
    context: BTreeMap<String, String>,
) -> Option<ErrorRecord> {
    report_error_with_publish(kind, message, target, context, true)
}

fn report_error_with_publish(
    kind: ErrorKind,
    message: impl Into<String>,
    target: ErrorTarget,
    context: BTreeMap<String, String>,
    publish: bool,
) -> Option<ErrorRecord> {
    let record = ErrorRecord::new(kind, message, target).with_context(context);
    append_record(record, publish)
}

fn append_record(mut record: ErrorRecord, publish: bool) -> Option<ErrorRecord> {
    let finalizer_timeout = (record.kind == ErrorKind::LaunchFailure)
        .then(|| {
            crate::project_runtime::workspace::workspace_finalizer_timeout_operation_id(
                &record.message,
            )
        })
        .flatten();
    if let Some(operation_id) = finalizer_timeout {
        // One physical external operation can reach the owner, GUI and cached
        // delivery paths. Keep its first diagnostic across retries and restarts.
        record.id = format!(
            "workspace-finalizer-timeout:{}",
            hex::encode(Sha256::digest(operation_id.as_bytes()))
        );
        record
            .context
            .insert("workspace_operation_id".into(), operation_id.into());
    }
    let once = finalizer_timeout.is_some();
    if recently_recorded(&record) {
        return None;
    }
    let result = if once {
        gwt_core::error_ledger::record_once(record)
    } else {
        gwt_core::error_ledger::record(record)
    };
    match result {
        Ok(recorded) => {
            if publish {
                publish_recorded(&recorded, recorded.target.project_root.as_deref());
            }
            Some(recorded)
        }
        Err(error) => {
            tracing::warn!(error = %error, "error ledger append failed");
            None
        }
    }
}

/// Explicit provenance carried privately from a GUI error source.
#[derive(Debug, Clone)]
pub enum ErrorOrigin {
    Project(String),
    Host,
}

pub fn record_backend_event(event: &BackendEvent) {
    record_backend_event_with_origin(event, None);
}

pub fn report_host_error(kind: ErrorKind, message: impl Into<String>) {
    append_record(ErrorRecord::new_host(kind, message), false);
}

/// Record an error event without inferring ownership from the current process.
pub fn record_backend_event_with_origin(event: &BackendEvent, origin: Option<&ErrorOrigin>) {
    if let Some(record) = backend_event_record(event, origin) {
        append_record(record, true);
    }
}

fn backend_event_record(event: &BackendEvent, origin: Option<&ErrorOrigin>) -> Option<ErrorRecord> {
    let (message, issue) = match event {
        BackendEvent::IssueMonitorToast {
            level,
            message,
            issue_number,
            ..
        } if level.eq_ignore_ascii_case("error") => (message, *issue_number),
        BackendEvent::IssueMonitorLaunchFailed {
            issue_number,
            message,
        } => (message, Some(*issue_number)),
        _ => return None,
    };
    let record = match origin {
        Some(ErrorOrigin::Host) => ErrorRecord::new_host(ErrorKind::LaunchFailure, message),
        _ => ErrorRecord::new(
            ErrorKind::LaunchFailure,
            message,
            ErrorTarget {
                issue,
                project_root: match origin {
                    Some(ErrorOrigin::Project(root)) => Some(root.clone()),
                    _ => None,
                },
                ..ErrorTarget::default()
            },
        ),
    };
    Some(record)
}

enum BackendErrorJob {
    Record {
        record: ErrorRecord,
        #[cfg(test)]
        home: Option<std::path::PathBuf>,
    },
    Publish {
        record: ErrorRecord,
        #[cfg(test)]
        home: Option<std::path::PathBuf>,
    },
    #[cfg(test)]
    Flush(mpsc::SyncSender<()>),
}

fn backend_error_sender() -> Option<&'static mpsc::Sender<BackendErrorJob>> {
    static SENDER: OnceLock<Option<mpsc::Sender<BackendErrorJob>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            match std::thread::Builder::new()
                .name("gwt-error-reporter".into())
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        match job {
                            BackendErrorJob::Record {
                                record,
                                #[cfg(test)]
                                home,
                            } => {
                                #[cfg(test)]
                                let _home = home
                                    .as_ref()
                                    .map(gwt_core::test_support::ScopedGwtHome::set);
                                append_record(record, true);
                            }
                            BackendErrorJob::Publish {
                                record,
                                #[cfg(test)]
                                home,
                            } => {
                                #[cfg(test)]
                                let _home = home
                                    .as_ref()
                                    .map(gwt_core::test_support::ScopedGwtHome::set);
                                publish_recorded(&record, record.target.project_root.as_deref());
                            }
                            #[cfg(test)]
                            BackendErrorJob::Flush(done) => {
                                let _ = done.send(());
                            }
                        }
                    }
                }) {
                Ok(_) => Some(sender),
                Err(error) => {
                    tracing::warn!(%error, "cannot start background error reporter");
                    None
                }
            }
        })
        .as_ref()
}

/// Keep ledger and daemon I/O outside GUI dispatch. A single worker preserves
/// the existing duplicate suppression when several clients receive one error.
pub fn queue_backend_event_with_origin(event: &BackendEvent, origin: Option<&ErrorOrigin>) {
    let Some(record) = backend_event_record(event, origin) else {
        return;
    };
    if let Some(sender) = backend_error_sender() {
        let _ = sender.send(BackendErrorJob::Record {
            record,
            #[cfg(test)]
            home: gwt_core::test_support::gwt_home_override(),
        });
    }
}

#[cfg(test)]
pub(crate) fn flush_backend_errors() {
    let (done, completed) = mpsc::sync_channel(1);
    backend_error_sender()
        .expect("error reporter")
        .send(BackendErrorJob::Flush(done))
        .unwrap();
    completed
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("error reporter completion");
}

fn recently_recorded(record: &ErrorRecord) -> bool {
    // GUI errors can wait in the reporter queue; compare their event times,
    // so processing delay cannot turn one repeated error into several rows.
    let finalizer_timeout = record.kind == ErrorKind::LaunchFailure
        && crate::project_runtime::workspace::workspace_finalizer_timeout_operation_id(
            &record.message,
        )
        .is_some();
    let since = (!finalizer_timeout).then_some(record.recorded_at - Duration::seconds(5));
    gwt_core::error_ledger::list_since(since)
        .ok()
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                if finalizer_timeout {
                    return row.id == record.id;
                }
                row.kind == record.kind
                    && row.message == record.message
                    && row.scope == record.scope
                    && row.target.project_root == record.target.project_root
                    && row.target.window_id == record.target.window_id
            })
        })
}

fn publish_recorded(record: &ErrorRecord, project_root: Option<&str>) {
    let Some(project_root) = project_root.filter(|root| !root.trim().is_empty()) else {
        return;
    };
    // The daemon's capacity tick runs on Tokio. Keep its synchronous ledger
    // append, but publish outside that runtime to preserve the control worker.
    if tokio::runtime::Handle::try_current().is_ok() {
        if let Some(sender) = backend_error_sender() {
            let _ = sender.send(BackendErrorJob::Publish {
                record: record.clone(),
                #[cfg(test)]
                home: gwt_core::test_support::gwt_home_override(),
            });
        }
        return;
    }
    let Ok(payload) = serde_json::to_value(record) else {
        return;
    };
    let _ = crate::daemon_publisher::publish_event(
        std::path::Path::new(project_root),
        ERRORS_CHANNEL,
        payload,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
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
            notification_transition: None,
            notification_transition_id: None,
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
    fn identical_errors_from_different_projects_are_not_suppressed() {
        let (_dir, _home) = isolated_home();
        for root in ["/project-a", "/project-b"] {
            report_error(
                ErrorKind::LaunchFailure,
                "same failure",
                ErrorTarget {
                    project_root: Some(root.into()),
                    ..ErrorTarget::default()
                },
            );
        }
        let rows = gwt_core::error_ledger::list_since(None).expect("list");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn identical_window_errors_keep_window_attribution_and_suppress_same_window() {
        let (_dir, _home) = isolated_home();
        for window in ["issue-a", "issue-b", "issue-a"] {
            report_error(
                ErrorKind::DaemonFault,
                "Issue window load failed: cache unavailable",
                ErrorTarget {
                    project_root: Some("/project".into()),
                    window_id: Some(window.into()),
                    ..ErrorTarget::default()
                },
            );
        }
        let rows = gwt_core::error_ledger::list_since(None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].target.window_id.as_deref(), Some("issue-a"));
        assert_eq!(rows[1].target.window_id.as_deref(), Some("issue-b"));
    }

    #[test]
    fn info_toasts_are_not_recorded() {
        let (_dir, _home) = isolated_home();
        record_backend_event(&BackendEvent::IssueMonitorToast {
            notification_transition: None,
            notification_transition_id: None,
            level: "info".into(),
            message: "scan complete".into(),
            issue_number: None,
        });
        assert!(gwt_core::error_ledger::list_since(None)
            .expect("list")
            .is_empty());
    }

    #[test]
    fn queued_duplicate_errors_keep_their_event_time_deduplication_window() {
        let (_dir, _home) = isolated_home();
        let mut record = ErrorRecord::new_host(ErrorKind::LaunchFailure, "delayed backend error");
        record.recorded_at = Utc::now() - Duration::seconds(60);
        assert!(append_record(record.clone(), false).is_some());
        assert!(append_record(record, false).is_none());
        assert_eq!(gwt_core::error_ledger::list_since(None).unwrap().len(), 1);
    }

    #[test]
    fn duplicate_error_toasts_within_five_seconds_are_not_repeated() {
        let (_dir, _home) = isolated_home();
        let event = BackendEvent::IssueMonitorToast {
            notification_transition: None,
            notification_transition_id: None,
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

    #[test]
    fn issue_6996_finalizer_timeout_records_once_across_launch_and_cached_toast_replays() {
        let (_dir, _home) = isolated_home();
        let message = "workspace finalizer fresh-launch-6996 timed out after 120000ms while publishing launch Work; live external commit did not finish before operation deadline";
        let mut first = ErrorRecord::new(
            ErrorKind::LaunchFailure,
            message,
            ErrorTarget {
                project_root: Some("/project-6996".into()),
                window_id: Some("pane-6996".into()),
                issue: Some(6996),
                ..ErrorTarget::default()
            },
        );
        first.recorded_at = Utc::now() - Duration::minutes(10);
        append_record(first, false).unwrap();
        let launch_failed = BackendEvent::IssueMonitorLaunchFailed {
            issue_number: 6996,
            message: message.into(),
        };
        append_record(backend_event_record(&launch_failed, None).unwrap(), false);
        let toast = BackendEvent::IssueMonitorToast {
            notification_transition: None,
            notification_transition_id: None,
            level: "error".into(),
            message: message.replace("120000ms", "240000ms"),
            issue_number: Some(6996),
        };
        let origin = ErrorOrigin::Project("/project-6996".into());
        append_record(backend_event_record(&toast, Some(&origin)).unwrap(), false);
        let rows = gwt_core::error_ledger::list_since(None).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "one expired operation across all reporting paths"
        );
        assert_eq!(rows[0].target.window_id.as_deref(), Some("pane-6996"));
        assert!(rows[0].message.contains("fresh-launch-6996"));
        assert!(rows[0].message.contains("120000ms"));
    }
}

use base64::{engine::general_purpose, Engine as _};
use gwt_core::{
    paths::{OperationProjectStore, ProjectScopeSource},
    repo_hash::RepoIdentitySource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use crate::WindowProcessStatus;

pub const RUNTIME_OUTPUT_CHANNEL: &str = "runtime_output";
pub const RUNTIME_STATUS_CHANNEL: &str = "runtime_status";
pub const RUNTIME_HOOK_CHANNEL: &str = "runtime_hook";
pub const RUNTIME_APPROVAL_OVERLAY_CHANNEL: &str = "runtime_approval_overlay";
pub const ISSUE_MONITOR_CHANNEL: &str = "issue_monitor";
pub const ISSUE_MONITOR_CONTROL_CHANNEL: &str = "issue_monitor_control";
pub const ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR: &str =
    "issue monitor control rejected: authority recovery is blocked";
pub const ISSUE_MONITOR_CONTROL_CLOSED_ERROR: &str =
    "issue monitor control rejected: worker is closed";
pub const ISSUE_MONITOR_CONTROL_REJECTED_ERROR: &str =
    "issue monitor control rejected before commit";
pub const ISSUE_MONITOR_CONTROL_BUSY_ERROR: &str =
    "issue monitor control rejected: admission is full";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorControlPublishError {
    TransportUnavailable(String),
    OutcomeUnknown(String),
    Busy(String),
    RecoveryBlocked,
    Rejected(String),
}

impl IssueMonitorControlPublishError {
    pub fn allows_local_fallback(&self) -> bool {
        matches!(self, Self::TransportUnavailable(_))
    }
}

impl std::fmt::Display for IssueMonitorControlPublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TransportUnavailable(message)
            | Self::OutcomeUnknown(message)
            | Self::Busy(message)
            | Self::Rejected(message) => formatter.write_str(message),
            Self::RecoveryBlocked => {
                formatter.write_str(ISSUE_MONITOR_CONTROL_RECOVERY_BLOCKED_ERROR)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDaemonEvent {
    Output {
        id: String,
        data: Vec<u8>,
    },
    Status {
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    Hook {
        event: crate::RuntimeHookEvent,
    },
    ApprovalOverlay {
        id: String,
        waiting: bool,
    },
    IssueMonitor {
        event: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeOutputPayload {
    source_pid: u32,
    id: String,
    data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeStatusPayload {
    source_pid: u32,
    id: String,
    status: WindowProcessStatus,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeHookPayload {
    source_pid: u32,
    event: crate::RuntimeHookEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeApprovalOverlayPayload {
    source_pid: u32,
    id: String,
    waiting: bool,
}

/// Issue #3596: identity of the project store that actually supplied an
/// Issue Monitor event. This is the shared wire shape used by both ordinary
/// JSON operation envelopes and daemon broadcasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStoreIdentity {
    project_root: String,
    hash: String,
    source: String,
    identity_resolved: bool,
    store_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repository_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    candidates: Vec<ProjectStoreCandidateIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectStoreCandidateIdentity {
    path: String,
    normalized_origin: String,
    hash: String,
}

impl ProjectStoreIdentity {
    /// Build provenance from the daemon authority that is actually serving
    /// the event, never from the subscriber's process cwd or request text.
    pub fn from_runtime_scope(scope: &gwt_core::daemon::RuntimeScope) -> Self {
        Self::from_project_root(&scope.project_root)
    }

    pub fn from_project_root(project_root: &Path) -> Self {
        let project_root =
            dunce::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
        let scope = gwt_core::paths::resolve_project_scope(&project_root);
        let store = OperationProjectStore {
            project_root,
            store_path: gwt_core::paths::gwt_project_dir(&scope.hash),
            scope,
        };
        Self::from_operation_store(&store)
    }

    pub fn from_operation_store(store: &OperationProjectStore) -> Self {
        let readable = |path: &Path| {
            gwt_core::paths::normalize_windows_child_process_path_text(&path.display().to_string())
        };
        let mut identity = Self {
            project_root: readable(&store.project_root),
            hash: store.scope.hash.as_str().to_string(),
            source: store.scope.source.as_str().to_string(),
            identity_resolved: store.scope.source.identity_resolved(),
            store_path: readable(&store.store_path),
            repository_path: None,
            candidates: Vec::new(),
        };
        match &store.scope.source {
            ProjectScopeSource::Repository(RepoIdentitySource::NestedBareRepository(path)) => {
                identity.repository_path = Some(readable(path));
            }
            ProjectScopeSource::AmbiguousNestedBareRepositories(candidates) => {
                identity.candidates = candidates
                    .iter()
                    .map(|candidate| ProjectStoreCandidateIdentity {
                        path: readable(&candidate.path),
                        normalized_origin: candidate.normalized_origin.clone(),
                        hash: candidate.hash.as_str().to_string(),
                    })
                    .collect();
            }
            ProjectScopeSource::Repository(RepoIdentitySource::Origin)
            | ProjectScopeSource::PathFallback => {}
        }
        identity
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssueMonitorPayload {
    source_pid: u32,
    event: String,
    payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_store: Option<ProjectStoreIdentity>,
}

pub fn runtime_output_payload(id: &str, data: &[u8], source_pid: u32) -> Value {
    serde_json::to_value(RuntimeOutputPayload {
        source_pid,
        id: id.to_string(),
        data_base64: general_purpose::STANDARD.encode(data),
    })
    .expect("runtime output payload serializes")
}

pub fn runtime_status_payload(
    id: &str,
    status: WindowProcessStatus,
    detail: Option<String>,
    source_pid: u32,
) -> Value {
    serde_json::to_value(RuntimeStatusPayload {
        source_pid,
        id: id.to_string(),
        status,
        detail,
    })
    .expect("runtime status payload serializes")
}

pub fn runtime_hook_payload(event: &crate::RuntimeHookEvent, source_pid: u32) -> Value {
    let mut event = event.clone();
    if event.source_event.as_deref() != Some("SessionStart") {
        event.continuation_readiness_nonce = None;
    }
    serde_json::to_value(RuntimeHookPayload { source_pid, event })
        .expect("runtime hook payload serializes")
}

pub fn runtime_approval_overlay_payload(id: &str, waiting: bool, source_pid: u32) -> Value {
    serde_json::to_value(RuntimeApprovalOverlayPayload {
        source_pid,
        id: id.to_string(),
        waiting,
    })
    .expect("runtime approval overlay payload serializes")
}

pub fn issue_monitor_payload(event: &str, payload: Value, source_pid: u32) -> Value {
    serde_json::to_value(IssueMonitorPayload {
        source_pid,
        event: event.to_string(),
        payload,
        project_store: None,
    })
    .expect("issue monitor payload serializes")
}

pub fn issue_monitor_payload_with_project_store(
    event: &str,
    payload: Value,
    source_pid: u32,
    project_store: &ProjectStoreIdentity,
) -> Value {
    serde_json::to_value(IssueMonitorPayload {
        source_pid,
        event: event.to_string(),
        payload,
        project_store: Some(project_store.clone()),
    })
    .expect("issue monitor payload with project store serializes")
}

pub fn decode_runtime_daemon_event(
    channel: &str,
    payload: Value,
    current_pid: u32,
) -> Option<RuntimeDaemonEvent> {
    match channel {
        RUNTIME_OUTPUT_CHANNEL => {
            let payload: RuntimeOutputPayload = serde_json::from_value(payload).ok()?;
            if payload.source_pid == current_pid {
                return None;
            }
            let data = general_purpose::STANDARD
                .decode(payload.data_base64.as_bytes())
                .ok()?;
            Some(RuntimeDaemonEvent::Output {
                id: payload.id,
                data,
            })
        }
        RUNTIME_STATUS_CHANNEL => {
            let payload: RuntimeStatusPayload = serde_json::from_value(payload).ok()?;
            if payload.source_pid == current_pid {
                return None;
            }
            Some(RuntimeDaemonEvent::Status {
                id: payload.id,
                status: payload.status,
                detail: payload.detail,
            })
        }
        RUNTIME_HOOK_CHANNEL => {
            let payload: RuntimeHookPayload = serde_json::from_value(payload).ok()?;
            if payload.source_pid == current_pid {
                return None;
            }
            Some(RuntimeDaemonEvent::Hook {
                event: payload.event,
            })
        }
        RUNTIME_APPROVAL_OVERLAY_CHANNEL => {
            let payload: RuntimeApprovalOverlayPayload = serde_json::from_value(payload).ok()?;
            if payload.source_pid == current_pid {
                return None;
            }
            Some(RuntimeDaemonEvent::ApprovalOverlay {
                id: payload.id,
                waiting: payload.waiting,
            })
        }
        ISSUE_MONITOR_CHANNEL => {
            let payload: IssueMonitorPayload = serde_json::from_value(payload).ok()?;
            if payload.source_pid == current_pid {
                return None;
            }
            let mut event = serde_json::json!({
                "event": payload.event,
                "payload": payload.payload,
            });
            if let Some(project_store) = payload.project_store {
                event["project_store"] = serde_json::to_value(project_store).ok()?;
            }
            Some(RuntimeDaemonEvent::IssueMonitor { event })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_runtime_daemon_event, issue_monitor_payload,
        issue_monitor_payload_with_project_store, runtime_approval_overlay_payload,
        runtime_hook_payload, runtime_output_payload, runtime_status_payload, ProjectStoreIdentity,
        RuntimeDaemonEvent, ISSUE_MONITOR_CHANNEL, RUNTIME_APPROVAL_OVERLAY_CHANNEL,
        RUNTIME_HOOK_CHANNEL, RUNTIME_OUTPUT_CHANNEL, RUNTIME_STATUS_CHANNEL,
    };
    use crate::{RuntimeHookEvent, RuntimeHookEventKind, WindowProcessStatus};

    fn deterministic_path_fallback_store() -> gwt_core::paths::OperationProjectStore {
        let project_root = std::path::PathBuf::from("/synthetic/issue-3596/source");
        gwt_core::paths::OperationProjectStore {
            project_root: project_root.clone(),
            scope: gwt_core::paths::ProjectScope {
                hash: gwt_core::repo_hash::compute_path_hash(&project_root),
                source: gwt_core::paths::ProjectScopeSource::PathFallback,
            },
            store_path: std::path::PathBuf::from("/synthetic/gwt/projects/issue-3596"),
        }
    }

    fn expected_path_fallback_project_store(
        store: &gwt_core::paths::OperationProjectStore,
    ) -> serde_json::Value {
        let readable = |path: &std::path::Path| {
            gwt_core::paths::normalize_windows_child_process_path_text(&path.display().to_string())
        };

        serde_json::json!({
            "project_root": readable(&store.project_root),
            "hash": store.scope.hash.as_str(),
            "source": "path_fallback",
            "identity_resolved": false,
            "store_path": readable(&store.store_path),
        })
    }

    #[test]
    fn runtime_output_payload_round_trips_and_ignores_same_process() {
        let payload = runtime_output_payload("tab-1::shell-1", b"hello", 42);

        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_OUTPUT_CHANNEL, payload.clone(), 99),
            Some(RuntimeDaemonEvent::Output {
                id: "tab-1::shell-1".to_string(),
                data: b"hello".to_vec(),
            })
        );
        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_OUTPUT_CHANNEL, payload, 42),
            None
        );
    }

    #[test]
    fn runtime_status_payload_round_trips_and_ignores_same_process() {
        let payload = runtime_status_payload(
            "tab-1::shell-1",
            WindowProcessStatus::Error,
            Some("boom".to_string()),
            42,
        );

        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_STATUS_CHANNEL, payload.clone(), 99),
            Some(RuntimeDaemonEvent::Status {
                id: "tab-1::shell-1".to_string(),
                status: WindowProcessStatus::Error,
                detail: Some("boom".to_string()),
            })
        );
        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_STATUS_CHANNEL, payload, 42),
            None
        );
    }

    #[test]
    fn runtime_approval_overlay_payload_is_sanitized_and_ignores_same_process() {
        let payload = runtime_approval_overlay_payload("tab-1::agent-1", true, 42);
        let serialized = serde_json::to_string(&payload).expect("serialize overlay payload");

        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_APPROVAL_OVERLAY_CHANNEL, payload.clone(), 99,),
            Some(RuntimeDaemonEvent::ApprovalOverlay {
                id: "tab-1::agent-1".to_string(),
                waiting: true,
            })
        );
        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_APPROVAL_OVERLAY_CHANNEL, payload, 42),
            None
        );
        assert!(!serialized.contains("fingerprint"));
        assert!(!serialized.contains("screen"));
        assert!(!serialized.contains("prompt"));
    }

    #[test]
    fn runtime_hook_payload_round_trips_and_ignores_same_process() {
        let event = RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("Stop".to_string()),
            gwt_session_id: Some("session-1".to_string()),
            continuation_readiness_nonce: None,
            agent_session_id: Some("agent-1".to_string()),
            project_root: Some("/tmp/project".to_string()),
            branch: Some("work/runtime".to_string()),
            status: Some("waiting".to_string()),
            tool_name: None,
            message: None,
            occurred_at: "2026-05-10T00:00:00Z".to_string(),
        };
        let payload = runtime_hook_payload(&event, 42);

        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_HOOK_CHANNEL, payload.clone(), 99),
            Some(RuntimeDaemonEvent::Hook {
                event: event.clone(),
            })
        );
        assert_eq!(
            decode_runtime_daemon_event(RUNTIME_HOOK_CHANNEL, payload, 42),
            None
        );
    }

    #[test]
    fn runtime_hook_payload_preserves_continue_work_readiness_nonce_for_authenticated_fanout() {
        let event = RuntimeHookEvent {
            kind: RuntimeHookEventKind::CoordinationEvent,
            source_event: Some("SessionStart".to_string()),
            gwt_session_id: Some("session-private".to_string()),
            continuation_readiness_nonce: Some("continue-ready-private".to_string()),
            agent_session_id: Some("provider-session".to_string()),
            project_root: Some("/tmp/project".to_string()),
            branch: Some("work/issue-2359".to_string()),
            status: None,
            tool_name: None,
            message: None,
            occurred_at: "2026-07-25T00:00:00Z".to_string(),
        };

        let payload = runtime_hook_payload(&event, 42);
        let encoded = serde_json::to_string(&payload).expect("serialize daemon payload");
        assert!(encoded.contains("continue-ready-private"));

        let RuntimeDaemonEvent::Hook { event: decoded } =
            decode_runtime_daemon_event(RUNTIME_HOOK_CHANNEL, payload, 7)
                .expect("decode authenticated hook fanout")
        else {
            panic!("expected hook event");
        };
        assert_eq!(decoded, event);
    }

    #[test]
    fn runtime_hook_payload_scrubs_readiness_nonce_from_non_session_start_fanout() {
        let mut event = RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("PreToolUse".to_string()),
            gwt_session_id: Some("session-private".to_string()),
            continuation_readiness_nonce: Some("continue-ready-private".to_string()),
            agent_session_id: Some("provider-session".to_string()),
            project_root: Some("/tmp/project".to_string()),
            branch: Some("work/issue-3480".to_string()),
            status: Some("Running".to_string()),
            tool_name: Some("Bash".to_string()),
            message: None,
            occurred_at: "2026-08-15T00:00:00Z".to_string(),
        };

        let payload = runtime_hook_payload(&event, 42);
        let encoded = serde_json::to_string(&payload).expect("serialize daemon payload");
        assert!(!encoded.contains("continue-ready-private"));

        let RuntimeDaemonEvent::Hook { event: decoded } =
            decode_runtime_daemon_event(RUNTIME_HOOK_CHANNEL, payload, 7)
                .expect("decode non-readiness hook fanout")
        else {
            panic!("expected hook event");
        };
        event.continuation_readiness_nonce = None;
        assert_eq!(decoded, event);
    }

    #[test]
    fn issue_monitor_payload_round_trips_and_ignores_same_process() {
        let payload = issue_monitor_payload(
            "status",
            serde_json::json!({"enabled": true, "queue_len": 2}),
            42,
        );
        assert!(payload.get("project_store").is_none());

        assert_eq!(
            decode_runtime_daemon_event(ISSUE_MONITOR_CHANNEL, payload.clone(), 99),
            Some(RuntimeDaemonEvent::IssueMonitor {
                event: serde_json::json!({"event": "status", "payload": {"enabled": true, "queue_len": 2}}),
            })
        );
        assert_eq!(
            decode_runtime_daemon_event(ISSUE_MONITOR_CHANNEL, payload, 42),
            None
        );
    }

    #[test]
    fn issue_monitor_decoder_accepts_literal_legacy_payload_without_project_store() {
        let payload = serde_json::json!({
            "source_pid": 42,
            "event": "status",
            "payload": {"enabled": true, "queue_len": 2},
        });

        assert_eq!(
            decode_runtime_daemon_event(ISSUE_MONITOR_CHANNEL, payload, 99),
            Some(RuntimeDaemonEvent::IssueMonitor {
                event: serde_json::json!({
                    "event": "status",
                    "payload": {"enabled": true, "queue_len": 2},
                }),
            })
        );
    }

    #[test]
    fn project_store_identity_normalizes_windows_paths_from_operation_store() {
        let cases = [
            (
                r"\\?\C:\repo",
                r"\\?\C:\Users\operator\.gwt\projects\store",
                r"C:\repo",
                r"C:\Users\operator\.gwt\projects\store",
            ),
            (
                r"\\?\UNC\server\share\repo",
                r"\\?\UNC\server\share\.gwt\projects\store",
                r"\\server\share\repo",
                r"\\server\share\.gwt\projects\store",
            ),
            (
                r"Microsoft.PowerShell.Core\FileSystem::\\?\C:\repo",
                r"Microsoft.PowerShell.Core\FileSystem::\\?\UNC\server\share\store",
                r"C:\repo",
                r"\\server\share\store",
            ),
        ];

        for (project_root, store_path, expected_project_root, expected_store_path) in cases {
            let store = gwt_core::paths::OperationProjectStore {
                project_root: std::path::PathBuf::from(project_root),
                scope: gwt_core::paths::ProjectScope {
                    hash: gwt_core::repo_hash::compute_repo_hash(
                        "https://example.com/operator/project.git",
                    ),
                    source: gwt_core::paths::ProjectScopeSource::PathFallback,
                },
                store_path: std::path::PathBuf::from(store_path),
            };

            let identity = ProjectStoreIdentity::from_operation_store(&store);
            let reported = serde_json::to_value(identity).expect("serialize project store");

            assert_eq!(reported["project_root"], expected_project_root);
            assert_eq!(reported["store_path"], expected_store_path);
            assert_eq!(reported["hash"], store.scope.hash.as_str());
            assert_eq!(reported["source"], "path_fallback");
            assert_eq!(reported["identity_resolved"], false);
        }
    }

    #[test]
    fn project_store_identity_preserves_nested_repository_diagnostic() {
        let repository_path = std::path::PathBuf::from(r"\\?\C:\layout\project.git");
        let store = gwt_core::paths::OperationProjectStore {
            project_root: std::path::PathBuf::from(r"\\?\C:\layout"),
            scope: gwt_core::paths::ProjectScope {
                hash: gwt_core::repo_hash::compute_repo_hash(
                    "https://example.com/operator/project.git",
                ),
                source: gwt_core::paths::ProjectScopeSource::Repository(
                    gwt_core::repo_hash::RepoIdentitySource::NestedBareRepository(repository_path),
                ),
            },
            store_path: std::path::PathBuf::from(r"\\?\C:\gwt\projects\store"),
        };

        let identity = ProjectStoreIdentity::from_operation_store(&store);
        let reported = serde_json::to_value(identity).expect("serialize project store");

        assert_eq!(reported["source"], "nested_bare_repository");
        assert_eq!(reported["identity_resolved"], true);
        assert_eq!(reported["repository_path"], r"C:\layout\project.git");
        assert!(reported.get("candidates").is_none());
    }

    #[test]
    fn project_store_identity_preserves_ambiguous_candidate_diagnostics() {
        let first_hash =
            gwt_core::repo_hash::compute_repo_hash("https://example.com/operator/first.git");
        let second_hash =
            gwt_core::repo_hash::compute_repo_hash("https://example.com/operator/second.git");
        let store = gwt_core::paths::OperationProjectStore {
            project_root: std::path::PathBuf::from(r"\\?\UNC\server\share\layout"),
            scope: gwt_core::paths::ProjectScope {
                hash: gwt_core::repo_hash::compute_path_hash(std::path::Path::new(
                    "/synthetic/ambiguous-layout",
                )),
                source: gwt_core::paths::ProjectScopeSource::AmbiguousNestedBareRepositories(vec![
                    gwt_core::repo_hash::RepoIdentityCandidate {
                        path: std::path::PathBuf::from(r"\\?\C:\layout\first.git"),
                        normalized_origin: "example.com/operator/first".to_string(),
                        hash: first_hash.clone(),
                    },
                    gwt_core::repo_hash::RepoIdentityCandidate {
                        path: std::path::PathBuf::from(
                            r"Microsoft.PowerShell.Core\FileSystem::\\?\UNC\server\share\second.git",
                        ),
                        normalized_origin: "example.com/operator/second".to_string(),
                        hash: second_hash.clone(),
                    },
                ]),
            },
            store_path: std::path::PathBuf::from(r"\\?\UNC\server\share\gwt\store"),
        };

        let identity = ProjectStoreIdentity::from_operation_store(&store);
        let reported = serde_json::to_value(identity).expect("serialize project store");

        assert_eq!(reported["source"], "ambiguous_nested_bare_repositories");
        assert_eq!(reported["identity_resolved"], false);
        assert_eq!(
            reported["candidates"],
            serde_json::json!([
                {
                    "path": r"C:\layout\first.git",
                    "normalized_origin": "example.com/operator/first",
                    "hash": first_hash.as_str(),
                },
                {
                    "path": r"\\server\share\second.git",
                    "normalized_origin": "example.com/operator/second",
                    "hash": second_hash.as_str(),
                },
            ])
        );
        assert!(reported.get("repository_path").is_none());
    }

    #[test]
    fn project_store_identity_from_project_root_matches_resolved_operation_store() {
        let project = tempfile::tempdir().expect("project tempdir");
        let canonical_root = dunce::canonicalize(project.path()).expect("canonical project root");
        let scope = gwt_core::paths::resolve_project_scope(&canonical_root);
        let store = gwt_core::paths::OperationProjectStore {
            project_root: canonical_root.clone(),
            store_path: gwt_core::paths::gwt_project_dir(&scope.hash),
            scope,
        };

        let from_root = ProjectStoreIdentity::from_project_root(project.path());
        let from_store = ProjectStoreIdentity::from_operation_store(&store);

        assert_eq!(
            serde_json::to_value(from_root).expect("serialize root identity"),
            serde_json::to_value(from_store).expect("serialize operation identity")
        );
    }

    #[test]
    fn project_store_identity_from_runtime_scope_uses_worker_root_not_ambient_cwd() {
        let project = tempfile::tempdir().expect("worker project tempdir");
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            project.path(),
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .expect("worker scope");
        let ambient_cwd = dunce::canonicalize(std::env::current_dir().expect("ambient cwd"))
            .expect("canonical ambient cwd");
        assert_ne!(ambient_cwd, scope.project_root, "fixture roots must differ");

        let reported = serde_json::to_value(ProjectStoreIdentity::from_runtime_scope(&scope))
            .expect("serialize worker identity");
        let expected_root = gwt_core::paths::normalize_windows_child_process_path_text(
            &scope.project_root.display().to_string(),
        );

        assert_eq!(reported["project_root"], expected_root);
        assert_eq!(reported["hash"], scope.repo_hash);
    }

    #[test]
    fn issue_monitor_payload_with_project_store_serializes_actual_source_identity() {
        let store = deterministic_path_fallback_store();
        let project_store = ProjectStoreIdentity::from_operation_store(&store);
        let expected_project_store = expected_path_fallback_project_store(&store);

        let payload = issue_monitor_payload_with_project_store(
            "status",
            serde_json::json!({"enabled": true, "queue_len": 2}),
            42,
            &project_store,
        );

        assert_eq!(
            payload,
            serde_json::json!({
                "source_pid": 42,
                "event": "status",
                "payload": {"enabled": true, "queue_len": 2},
                "project_store": expected_project_store,
            })
        );
    }

    #[test]
    fn issue_monitor_decoder_preserves_project_store_identity() {
        let store = deterministic_path_fallback_store();
        let project_store = ProjectStoreIdentity::from_operation_store(&store);
        let expected_project_store = expected_path_fallback_project_store(&store);
        let payload = issue_monitor_payload_with_project_store(
            "status",
            serde_json::json!({"enabled": true, "queue_len": 2}),
            42,
            &project_store,
        );

        assert_eq!(
            decode_runtime_daemon_event(ISSUE_MONITOR_CHANNEL, payload, 99),
            Some(RuntimeDaemonEvent::IssueMonitor {
                event: serde_json::json!({
                    "event": "status",
                    "payload": {"enabled": true, "queue_len": 2},
                    "project_store": expected_project_store,
                }),
            })
        );
    }
}

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IssueMonitorPayload {
    source_pid: u32,
    event: String,
    payload: Value,
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
    event.continuation_readiness_nonce = None;
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
    })
    .expect("issue monitor payload serializes")
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
            Some(RuntimeDaemonEvent::IssueMonitor {
                event: serde_json::json!({
                    "event": payload.event,
                    "payload": payload.payload,
                }),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_runtime_daemon_event, issue_monitor_payload, runtime_approval_overlay_payload,
        runtime_hook_payload, runtime_output_payload, runtime_status_payload, RuntimeDaemonEvent,
        ISSUE_MONITOR_CHANNEL, RUNTIME_APPROVAL_OVERLAY_CHANNEL, RUNTIME_HOOK_CHANNEL,
        RUNTIME_OUTPUT_CHANNEL, RUNTIME_STATUS_CHANNEL,
    };
    use crate::{RuntimeHookEvent, RuntimeHookEventKind, WindowProcessStatus};

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
    fn runtime_hook_payload_never_publishes_continue_work_readiness_nonce() {
        let mut event = RuntimeHookEvent {
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
        assert!(!encoded.contains("continue-ready-private"));

        let RuntimeDaemonEvent::Hook { event: decoded } =
            decode_runtime_daemon_event(RUNTIME_HOOK_CHANNEL, payload, 7)
                .expect("decode sanitized hook")
        else {
            panic!("expected hook event");
        };
        assert!(decoded.continuation_readiness_nonce.is_none());

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
}

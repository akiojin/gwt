use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::WindowProcessStatus;

pub const RUNTIME_OUTPUT_CHANNEL: &str = "runtime_output";
pub const RUNTIME_STATUS_CHANNEL: &str = "runtime_status";
pub const RUNTIME_HOOK_CHANNEL: &str = "runtime_hook";

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
    serde_json::to_value(RuntimeHookPayload {
        source_pid,
        event: event.clone(),
    })
    .expect("runtime hook payload serializes")
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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_runtime_daemon_event, runtime_hook_payload, runtime_output_payload,
        runtime_status_payload, RuntimeDaemonEvent, RUNTIME_HOOK_CHANNEL, RUNTIME_OUTPUT_CHANNEL,
        RUNTIME_STATUS_CHANNEL,
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
    fn runtime_hook_payload_round_trips_and_ignores_same_process() {
        let event = RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("Stop".to_string()),
            gwt_session_id: Some("session-1".to_string()),
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
}

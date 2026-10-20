//! Coalesce undelivered runtime state across short-lived hook processes.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::{RuntimeHookEvent, RuntimeHookEventKind};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookLiveDeliveryHealth {
    pub consecutive_failures: u64,
    pub last_failure_at: Option<String>,
    pub last_error: Option<String>,
    pub last_success_at: Option<String>,
    pub pending_runtime_state: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeliveryState {
    health: HookLiveDeliveryHealth,
    pending: Option<RuntimeHookEvent>,
}

fn delivery_path(runtime_path: &Path) -> Option<PathBuf> {
    let sessions_dir = gwt_agent::sessions_dir_from_runtime_path(runtime_path)?;
    let host = runtime_path.parent()?.file_name()?;
    Some(
        sessions_dir
            .join("hook-live")
            .join(host)
            .join(runtime_path.file_name()?),
    )
}

pub fn hook_live_delivery_health(runtime_path: &Path) -> Option<HookLiveDeliveryHealth> {
    let bytes = std::fs::read(delivery_path(runtime_path)?).ok()?;
    serde_json::from_slice::<DeliveryState>(&bytes)
        .ok()
        .map(|state| state.health)
}

pub(super) fn deliver(
    runtime_path: &Path,
    event: &RuntimeHookEvent,
    mut send: impl FnMut(&RuntimeHookEvent) -> Result<(), String>,
) -> Result<(), String> {
    // An inherited environment must never journal a different Session's hook.
    if event.gwt_session_id.as_deref() != runtime_path.file_stem().and_then(|id| id.to_str()) {
        return send(event);
    }
    let Some(path) = delivery_path(runtime_path) else {
        return send(event);
    };
    deliver_journaled(&path, event, &mut send)
        .map_err(|error| format!("hook live delivery journal: {error}"))?
}

fn deliver_journaled(
    path: &Path,
    event: &RuntimeHookEvent,
    send: &mut impl FnMut(&RuntimeHookEvent) -> Result<(), String>,
) -> io::Result<Result<(), String>> {
    fs::create_dir_all(path.parent().expect("delivery path has a parent"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    // Serialize this Session only, including its bounded POST, so an older
    // attempt cannot clear or replay state after a newer hook has delivered it.
    fs2::FileExt::lock_exclusive(&lock)?;
    let mut state: DeliveryState = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => DeliveryState::default(),
        Err(error) => return Err(error),
    };
    if event.kind == RuntimeHookEventKind::RuntimeState {
        state.pending = Some(event.clone());
        state.health.pending_runtime_state = true;
        // Publish before attempting the POST; process exit cannot discard it.
        state.save(path)?;
    }
    let mut result = Ok(());
    if let Some(pending) = state.pending.as_ref() {
        result = send(pending);
        state.observe(&result);
        if result.is_ok() {
            state.pending = None;
        }
    }
    if result.is_ok() && event.kind != RuntimeHookEventKind::RuntimeState {
        result = send(event);
        state.observe(&result);
    }
    state.health.pending_runtime_state = state.pending.is_some();
    state.save(path)?;
    Ok(result)
}

impl DeliveryState {
    fn observe(&mut self, result: &Result<(), String>) {
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        match result {
            Ok(()) => {
                self.health.consecutive_failures = 0;
                self.health.last_success_at = Some(now);
            }
            Err(error) => {
                self.health.consecutive_failures =
                    self.health.consecutive_failures.saturating_add(1);
                self.health.last_failure_at = Some(now);
                self.health.last_error =
                    Some(gwt_core::error_ledger::sanitize_error_message(error));
            }
        }
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        gwt_github::cache::write_atomic_with_durability(
            path,
            &bytes,
            gwt_github::cache::Durability::RenameOnly,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_runtime::RuntimeHookEventKind;

    fn event(kind: RuntimeHookEventKind, status: Option<&str>) -> RuntimeHookEvent {
        RuntimeHookEvent {
            kind,
            source_event: Some("PreToolUse".into()),
            gwt_session_id: Some("session-test".into()),
            continuation_readiness_nonce: None,
            agent_session_id: Some("provider-session".into()),
            project_root: Some("/project".into()),
            branch: Some("work/test".into()),
            status: status.map(str::to_string),
            tool_name: None,
            message: None,
            occurred_at: "2026-10-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn issue_4905_later_forward_recovers_failed_runtime_state_and_health() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("runtime/123/session-test.json");
        let runtime = event(RuntimeHookEventKind::RuntimeState, Some("Running"));
        for _ in 0..2 {
            assert!(deliver(&path, &runtime, |_| Err("scripted timeout".into())).is_err());
        }
        let health = hook_live_delivery_health(&path).expect("failure survives hook process exit");
        assert_eq!(health.consecutive_failures, 2);
        assert!(health.pending_runtime_state);
        assert_eq!(health.last_error.as_deref(), Some("scripted timeout"));
        assert!(health.last_failure_at.is_some());

        let mut sent = Vec::new();
        let forward = event(RuntimeHookEventKind::Forward, None);
        deliver(&path, &forward, |event| {
            sent.push(event.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, vec![runtime, forward]);
        let health = hook_live_delivery_health(&path).unwrap();
        assert_eq!(health.consecutive_failures, 0);
        assert!(!health.pending_runtime_state);
        assert!(health.last_success_at.is_some());
    }

    #[test]
    fn issue_4905_new_runtime_state_supersedes_pending_without_replaying_stale_state() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("runtime/123/session-test.json");
        let old = event(RuntimeHookEventKind::RuntimeState, Some("Running"));
        assert!(deliver(&path, &old, |_| Err("scripted timeout".into())).is_err());
        let mut latest = event(RuntimeHookEventKind::RuntimeState, Some("Idle"));
        latest.source_event = Some("Stop".into());
        let mut sent = Vec::new();
        deliver(&path, &latest, |event| {
            sent.push(event.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, vec![latest]);
        assert!(
            !hook_live_delivery_health(&path)
                .unwrap()
                .pending_runtime_state
        );
    }
}

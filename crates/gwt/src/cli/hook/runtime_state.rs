//! `gwtd hook runtime-state <event>` — write a tiny JSON state file that
//! tells the Branches tab whether the agent session is currently running
//! or waiting for user input.
//!
//! Ported from the retired external runtime hook and now used as the
//! managed runtime hook implementation wired from settings.

use std::{
    io,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
#[cfg(test)]
use gwt_agent::persist_agent_session_id;
use gwt_agent::{
    persist_session_hook_metadata_with_wait, AgentStatus, PendingDiscussionResume, Session,
    SessionExecutionIdentity, SessionRuntimeState,
};
use serde::Serialize;

use super::{
    resolve_hook_agent_session_id, GwtSessionId, HookAgentSessionId, HookError, HookSessionId,
    RawHookEvent,
};
use crate::discussion_resume::load_pending_resume;
use crate::window_state::window_state_for_hook_event;

const HOOK_SESSION_METADATA_LEASE_WAIT: Duration = Duration::from_millis(25);

/// A read-only snapshot, never an execution grant. Completion and doctor compare
/// this existing Session/process identity again before publishing evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentHookContext {
    pub identity: SessionExecutionIdentity,
    pub runtime_path: PathBuf,
    pub incarnation: u64,
    host_started_at: u64,
    child_pid: u32,
    child_started_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HookContextError {
    Missing,
    Unreadable,
    Foreign,
}

pub(crate) fn current_hook_context(
    worktree: &Path,
) -> Result<(CurrentHookContext, SessionRuntimeState), HookContextError> {
    use HookContextError::{Foreign, Missing, Unreadable};
    let worktree = gwt_core::paths::resolve_current_worktree_root(worktree);
    let id = GwtSessionId::from_env().ok_or(Missing)?;
    gwt_agent::validate_session_id_path_component(id.as_str()).map_err(|_| Foreign)?;
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let session =
        Session::load(&sessions.join(format!("{}.toml", id.as_str()))).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Missing
            } else {
                Unreadable
            }
        })?;
    if session.id != id.as_str()
        || std::fs::canonicalize(&session.worktree_path).map_err(|_| Unreadable)?
            != std::fs::canonicalize(&worktree).map_err(|_| Unreadable)?
    {
        return Err(Foreign);
    }
    let identity = SessionExecutionIdentity::from_session(&session)
        .map_err(|_| Foreign)?
        .ok_or(Missing)?;
    let path = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(PathBuf::from)
        .ok_or(Missing)?;
    let host_pid = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|p| p.to_str())
        .and_then(|p| p.parse::<u32>().ok())
        .filter(|p| *p > 0)
        .ok_or(Foreign)?;
    let expected = gwt_agent::runtime_state_path_for_pid(&sessions, host_pid, id.as_str());
    if std::fs::canonicalize(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Missing
        } else {
            Unreadable
        }
    })? != std::fs::canonicalize(expected).map_err(|_| Unreadable)?
    {
        return Err(Foreign);
    }
    let runtime = SessionRuntimeState::load(&path).map_err(|_| Unreadable)?;
    let incarnation = runtime
        .runtime_incarnation
        .filter(|n| *n > 0)
        .ok_or(Missing)?;
    let host_started_at = runtime.host_started_at.ok_or(Missing)?;
    let child_pid = runtime.child_pid.filter(|p| *p > 0).ok_or(Missing)?;
    let child_started_at = runtime.child_started_at.ok_or(Missing)?;
    if runtime.execution_identity.as_ref() != Some(&identity)
        || crate::process::host_process_start_time(host_pid) != Some(host_started_at)
        || crate::process::host_process_start_time(child_pid) != Some(child_started_at)
        || (child_pid != std::process::id()
            && !crate::process::is_descendant_of(std::process::id(), child_pid))
    {
        return Err(Foreign);
    }
    Ok((
        CurrentHookContext {
            identity,
            runtime_path: path,
            incarnation,
            host_started_at,
            child_pid,
            child_started_at,
        },
        runtime,
    ))
}

/// The JSON shape the Branches tab polls from `$GWT_SESSION_RUNTIME_PATH`.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RuntimeState {
    pub status: String,
    pub updated_at: String,
    pub last_activity_at: String,
    pub source_event: String,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

/// Map a hook event name to the runtime status it should produce.
///
/// Returns `None` for event names that settings_local.rs should never
/// forward to this handler. Callers translate `None` into a
/// [`HookError::InvalidEvent`].
pub fn status_for_event(event: &str) -> Option<&'static str> {
    match window_state_for_hook_event(event)? {
        crate::persistence::WindowState::Running => Some("Running"),
        crate::persistence::WindowState::Starting => Some("Starting"),
        crate::persistence::WindowState::Idle => Some("Idle"),
        crate::persistence::WindowState::Waiting => Some("Waiting"),
        crate::persistence::WindowState::Stopped => Some("Stopped"),
        crate::persistence::WindowState::Error => Some("Error"),
        crate::persistence::WindowState::Interrupted => Some("Interrupted"),
    }
}

/// Serialize a [`RuntimeState`] for the given event and write it atomically
/// to `path`. On success, no `.tmp-*` siblings remain.
pub fn write_for_event(path: &Path, event: &str) -> Result<(), HookError> {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let session = current_session_from_env(&sessions_dir);
    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });

    write_for_event_with_pending_discussion(path, event, pending_discussion)?;

    Ok(())
}

fn write_for_event_with_pending_discussion(
    path: &Path,
    event: &str,
    pending_discussion: Option<PendingDiscussionResume>,
) -> Result<(), HookError> {
    let status =
        status_for_event(event).ok_or_else(|| HookError::InvalidEvent(event.to_string()))?;
    write_state_with_status(path, event, status, pending_discussion, None)
}

fn write_state_with_status(
    path: &Path,
    event: &str,
    status: &str,
    pending_discussion: Option<PendingDiscussionResume>,
    tool_use_id: Option<&str>,
) -> Result<(), HookError> {
    let _writer = gwt_agent::session::runtime_writer_lease(path)?;
    let _publication = gwt_agent::session::runtime_publication_lease(path)?;
    let previous = SessionRuntimeState::load(path).ok();
    let value = event_runtime_value(previous, event, status, pending_discussion, tool_use_id)?;
    write_hook_runtime_value(path, &value, false)?;
    Ok(())
}

fn event_runtime_value(
    previous: Option<SessionRuntimeState>,
    event: &str,
    status: &str,
    pending_discussion: Option<PendingDiscussionResume>,
    tool_use_id: Option<&str>,
) -> Result<serde_json::Value, HookError> {
    let mut active_tool_calls = previous
        .as_ref()
        .and_then(|state| state.active_tool_calls.clone());
    match (event, tool_use_id) {
        ("SessionStart" | "Stop", _) => active_tool_calls = Some(Default::default()),
        ("PreToolUse", Some(id)) => {
            active_tool_calls
                .get_or_insert_with(Default::default)
                .insert(id.to_string());
        }
        ("PostToolUse", Some(id)) => {
            if let Some(calls) = active_tool_calls.as_mut() {
                calls.remove(id);
            }
        }
        // Providers without call ids cannot establish a complete active set.
        ("PreToolUse" | "PostToolUse", None) => active_tool_calls = None,
        _ => {}
    }
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let state = RuntimeState {
        status: status.to_string(),
        updated_at: now.clone(),
        last_activity_at: now,
        source_event: event.to_string(),
        pending_discussion,
    };

    let mut value = serde_json::to_value(&state)?;
    let object = value
        .as_object_mut()
        .expect("RuntimeState serializes as an object");
    if let Some(calls) = active_tool_calls {
        object.insert(
            "active_tool_calls".to_string(),
            serde_json::to_value(calls)?,
        );
    }
    if let Some(previous) = previous {
        if let Some(identity) = previous.execution_identity {
            object.insert(
                "execution_identity".to_string(),
                serde_json::to_value(identity)?,
            );
        }
        if let Some(incarnation) = previous.runtime_incarnation {
            object.insert(
                "runtime_incarnation".to_string(),
                serde_json::to_value(incarnation)?,
            );
        }
        // Issue #4643: the launch's process identity outlives every hook
        // event. Dropping it left the sweep with nothing but a live Host to
        // judge by, so closed panes kept their worktrees forever.
        for (key, value) in [
            ("host_started_at", previous.host_started_at),
            ("child_pid", previous.child_pid.map(u64::from)),
            ("child_started_at", previous.child_started_at),
        ] {
            if let Some(value) = value {
                object.insert(key.to_string(), serde_json::Value::from(value));
            }
        }
        if let Some(completed_at) = previous.last_completed_hook_event_at {
            object.insert(
                "last_completed_hook_event_at".to_string(),
                serde_json::to_value(completed_at)?,
            );
        }
        if let Some(outcome) = previous.last_completed_hook_outcome {
            object.insert(
                "last_completed_hook_outcome".to_string(),
                serde_json::to_value(outcome)?,
            );
        }
    }
    Ok(value)
}

fn write_hook_runtime_value(
    path: &Path,
    value: &serde_json::Value,
    durable: bool,
) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    // Issue #3777: this file is pure liveness for the Branches tab and every
    // hook event rewrites it, so waiting for the device buys nothing while
    // costing the prompt its budget (522ms measured on a stalled Windows
    // runner). The rename still publishes it whole.
    gwt_github::cache::write_atomic_with_durability(
        path,
        &bytes,
        if durable {
            gwt_github::cache::Durability::FlushToDevice
        } else {
            gwt_github::cache::Durability::RenameOnly
        },
    )?;
    Ok(())
}

/// Captured before dispatch; an old completion cannot stamp a successor runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnboundPhysicalHookContext {
    incarnation: u64,
    host_started_at: u64,
    child_pid: u32,
    child_started_at: u64,
    canonical_runtime_path: PathBuf,
    canonical_worktree: PathBuf,
}

impl UnboundPhysicalHookContext {
    fn capture(runtime: &SessionRuntimeState, path: &Path, worktree: &Path) -> Option<Self> {
        if runtime.execution_identity.is_some() {
            return None;
        }
        let incarnation = runtime.runtime_incarnation.filter(|value| *value > 0)?;
        let host_started_at = runtime.host_started_at.filter(|value| *value > 0)?;
        let child_pid = runtime.child_pid.filter(|value| *value > 0)?;
        let child_started_at = runtime.child_started_at.filter(|value| *value > 0)?;
        let host_pid = path.parent()?.file_name()?.to_str()?.parse::<u32>().ok()?;
        if host_pid == 0
            || crate::process::host_process_start_time(host_pid) != Some(host_started_at)
            || crate::process::host_process_start_time(child_pid) != Some(child_started_at)
            || (child_pid != std::process::id()
                && !crate::process::is_descendant_of(std::process::id(), child_pid))
        {
            return None;
        }
        Some(Self {
            incarnation,
            host_started_at,
            child_pid,
            child_started_at,
            canonical_runtime_path: std::fs::canonicalize(path).ok()?,
            canonical_worktree: std::fs::canonicalize(worktree).ok()?,
        })
    }
}

fn is_legacy_hook_runtime(runtime: &SessionRuntimeState) -> bool {
    runtime.execution_identity.is_none()
        && runtime.runtime_incarnation.is_none()
        && runtime.host_started_at.is_none()
        && runtime.child_pid.is_none()
        && runtime.child_started_at.is_none()
}

#[derive(Debug, Clone)]
pub(crate) struct HookCompletionContext {
    sessions_dir: PathBuf,
    session_id: String,
    runtime_path: PathBuf,
    worktree: PathBuf,
    exact: Option<CurrentHookContext>,
    unbound_physical: Option<UnboundPhysicalHookContext>,
    correlation_id: Option<String>,
    event: Option<String>,
    initially_missing_legacy: bool,
    can_certify_current: bool,
}

pub(crate) fn capture_hook_completion(worktree: &Path) -> Option<HookCompletionContext> {
    let worktree = gwt_core::paths::resolve_current_worktree_root(worktree);
    let id = GwtSessionId::from_env()?;
    gwt_agent::validate_session_id_path_component(id.as_str()).ok()?;
    let path = PathBuf::from(std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)?);
    let sessions = gwt_agent::sessions_dir_from_runtime_path(&path)?;
    let pid = path.parent()?.file_name()?.to_str()?.parse::<u32>().ok()?;
    if pid == 0 || path != gwt_agent::runtime_state_path_for_pid(&sessions, pid, id.as_str()) {
        return None;
    }
    let session = Session::load(&sessions.join(format!("{}.toml", id.as_str()))).ok()?;
    if session.id != id.as_str()
        || std::fs::canonicalize(&session.worktree_path).ok()?
            != std::fs::canonicalize(&worktree).ok()?
    {
        return None;
    }
    let mut initially_missing_legacy = false;
    let mut unbound_physical = None;
    let exact = match SessionExecutionIdentity::from_session(&session).ok()? {
        Some(_) => Some(current_hook_context(&worktree).ok()?.0),
        None => {
            // Ordinary unbound launches retain their physical proof. Only a
            // sidecar with no proof fields is legacy; partial proof is unknown.
            match SessionRuntimeState::load(&path) {
                Ok(runtime) if is_legacy_hook_runtime(&runtime) => {}
                Ok(runtime) => {
                    unbound_physical = Some(UnboundPhysicalHookContext::capture(
                        &runtime, &path, &worktree,
                    )?);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    initially_missing_legacy = true;
                }
                _ => return None,
            }
            None
        }
    };
    Some(HookCompletionContext {
        sessions_dir: sessions,
        session_id: id.as_str().to_string(),
        runtime_path: path,
        worktree,
        exact,
        unbound_physical,
        correlation_id: None,
        event: None,
        initially_missing_legacy,
        can_certify_current: false,
    })
}

/// Observe an invocation without publishing its tool lifetime or Session metadata.
/// Invalid input is left to the original ordered handler for attribution.
pub(crate) fn begin_hook_observation(
    event: &str,
    input: &str,
    worktree: &Path,
) -> Result<Option<HookCompletionContext>, HookError> {
    if !matches!(
        event,
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "Stop"
    ) {
        return Ok(None);
    }
    let Ok(hook_event) = RawHookEvent::read_from_str(input) else {
        return Ok(None);
    };
    let Some(mut context) = capture_hook_completion(worktree) else {
        return Ok(None);
    };
    let Ok(id) = GwtSessionId::required_from_env(event) else {
        return Ok(None);
    };
    context.correlation_id = Some(uuid::Uuid::new_v4().to_string());
    context.event = Some(event.to_string());
    let mut invoked = false;
    let observed = gwt_agent::session::with_session_path_lease_wait(
        &context.sessions_dir,
        &context.session_id,
        HOOK_SESSION_METADATA_LEASE_WAIT,
        |state| {
            invoked = true;
            let gwt_agent::SessionPathState::Present(session) = state else {
                return Ok(false);
            };
            let _writer = gwt_agent::session::runtime_writer_lease(&context.runtime_path)?;
            let _publication =
                gwt_agent::session::runtime_publication_lease(&context.runtime_path)?;
            if !context.matches_session(&session)? {
                return Ok(false);
            }
            if validated_hook_agent_session_id(event, &id, Some(&session), hook_event.as_ref())
                .is_err()
            {
                return Ok(false);
            }
            match context.read_runtime() {
                Ok((runtime, mut value)) => {
                    if context.initially_missing_legacy || !context.matches_source(&runtime) {
                        return Ok(false);
                    }
                    value["current_hook_event"] =
                        serde_json::to_value(context.pending_observation())
                            .map_err(io::Error::other)?;
                    write_hook_runtime_value(&context.runtime_path, &value, false)?;
                    Ok(true)
                }
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && context.initially_missing_legacy =>
                {
                    Ok(true)
                }
                Err(_) => Ok(false),
            }
        },
    );
    match observed {
        Ok(observed) => Ok(observed.then_some(context)),
        Err(error) if !invoked => {
            log_session_metadata_error("observe hook for", &id, &error);
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

impl HookCompletionContext {
    fn pending_observation(&self) -> gwt_agent::session::CurrentHookEventObservation {
        gwt_agent::session::CurrentHookEventObservation {
            correlation_id: self.correlation_id.clone().unwrap_or_default(),
            event: self.event.clone().unwrap_or_default(),
            completed_outcome: None,
        }
    }

    fn matches_session(&self, session: &Session) -> io::Result<bool> {
        let Ok(identity) = SessionExecutionIdentity::from_session(session) else {
            return Ok(false);
        };
        let worktree = std::fs::canonicalize(&session.worktree_path)?;
        Ok(session.id == self.session_id
            && identity.as_ref() == self.exact.as_ref().map(|context| &context.identity)
            && worktree == std::fs::canonicalize(&self.worktree)?
            && self
                .unbound_physical
                .as_ref()
                .is_none_or(|physical| worktree == physical.canonical_worktree))
    }

    fn matches_source(&self, runtime: &SessionRuntimeState) -> bool {
        if let Some(expected) = self.exact.as_ref() {
            current_hook_context(&self.worktree)
                .ok()
                .as_ref()
                .map(|pair| &pair.0)
                == Some(expected)
        } else if let Some(expected) = self.unbound_physical.as_ref() {
            UnboundPhysicalHookContext::capture(runtime, &self.runtime_path, &self.worktree)
                .as_ref()
                == Some(expected)
        } else {
            is_legacy_hook_runtime(runtime)
        }
    }

    fn matches_pending(&self, runtime: &SessionRuntimeState) -> bool {
        let (Some(id), Some(event), Some(current)) = (
            self.correlation_id.as_deref(),
            self.event.as_deref(),
            runtime.current_hook_event.as_ref(),
        ) else {
            return false;
        };
        uuid::Uuid::parse_str(id).is_ok_and(|id_value| {
            id_value.get_version_num() == 4
                && id_value.get_variant() == uuid::Variant::RFC4122
                && id_value.to_string() == id
        }) && current.correlation_id == id
            && current.event == event
            && current.completed_outcome.is_none()
    }

    fn read_runtime(&self) -> io::Result<(SessionRuntimeState, serde_json::Value)> {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&self.runtime_path)?)
            .map_err(io::Error::other)?;
        let runtime = serde_json::from_value(value.clone()).map_err(io::Error::other)?;
        Ok((runtime, value))
    }

    fn owned_runtime(
        &self,
        session: &Session,
    ) -> io::Result<Option<(SessionRuntimeState, serde_json::Value)>> {
        if !self.matches_session(session)? || self.initially_missing_legacy {
            return Ok(None);
        }
        let Ok((runtime, value)) = self.read_runtime() else {
            return Ok(None);
        };
        Ok(
            (self.matches_source(&runtime) && self.matches_pending(&runtime))
                .then_some((runtime, value)),
        )
    }

    /// Keep the existing Session -> writer -> publication lock order. An
    /// unavailable observation leaves the previous pair unchanged.
    pub(crate) fn record(self, outcome: gwt_agent::session::HookCompletionOutcome) {
        let _ = gwt_agent::session::with_session_path_lease_wait(
            &self.sessions_dir,
            &self.session_id,
            HOOK_SESSION_METADATA_LEASE_WAIT,
            |state| {
                let gwt_agent::SessionPathState::Present(session) = state else {
                    return Ok(());
                };
                let _writer = gwt_agent::session::runtime_writer_lease(&self.runtime_path)?;
                let _publication =
                    gwt_agent::session::runtime_publication_lease(&self.runtime_path)?;
                let Some((_, mut value)) = self.owned_runtime(&session)? else {
                    return Ok(());
                };
                // Preserve other sidecar fields, including fields from newer
                // producers, while publishing timestamp/outcome atomically.
                value["last_completed_hook_event_at"] =
                    serde_json::to_value(Utc::now()).map_err(io::Error::other)?;
                value["last_completed_hook_outcome"] =
                    serde_json::to_value(outcome).map_err(io::Error::other)?;
                if self.can_certify_current && self.exact.is_some() {
                    value["current_hook_event"]["completed_outcome"] =
                        serde_json::to_value(outcome).map_err(io::Error::other)?;
                } else {
                    value
                        .as_object_mut()
                        .expect("runtime object")
                        .remove("current_hook_event");
                }
                write_hook_runtime_value(&self.runtime_path, &value, true)
            },
        );
    }
}

#[derive(Default)]
pub(crate) struct HookRuntimePublication {
    pub session: Option<Session>,
    pub published: bool,
}

/// The aggregate path never falls back to the unowned RuntimeState writer.
pub(crate) fn handle_with_input_owned(
    event: &str,
    input: &str,
    completion: Option<&mut HookCompletionContext>,
) -> Result<HookRuntimePublication, HookError> {
    handle_with_input_owned_with(
        event,
        input,
        completion,
        gwt_agent::session::persist_session_hook_metadata_under_lease,
        write_hook_runtime_value,
        std::convert::identity::<OwnedRuntimeResult>,
    )
}

type OwnedRuntimeResult = io::Result<(HookRuntimePublication, bool)>;

// Keep the real lease and ordered operation intact. Private call boundaries
// let tests observe errors after owned begin instead of refusing before it.
fn handle_with_input_owned_with<M, P, F>(
    event: &str,
    input: &str,
    completion: Option<&mut HookCompletionContext>,
    persist_metadata: M,
    publish_runtime: P,
    finish_lease: F,
) -> Result<HookRuntimePublication, HookError>
where
    M: FnOnce(&Path, &str, &str, Option<&str>, Option<&Path>) -> io::Result<Session>,
    P: FnOnce(&Path, &serde_json::Value, bool) -> io::Result<()>,
    F: FnOnce(OwnedRuntimeResult) -> OwnedRuntimeResult,
{
    let Some(path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV) else {
        return Ok(HookRuntimePublication::default());
    };
    // Preserve the original runtime-step parsing and identity errors, even
    // when observation begin could not obtain ownership.
    let hook_event = RawHookEvent::read_from_str(input)?;
    let id = GwtSessionId::required_from_env(event)?;
    let path = PathBuf::from(path);
    let sessions = sessions_dir_for_runtime_path(&path);
    let session = timed_substage(event, "runtime-state/session-load", || {
        // Defer normalization to the guarded metadata helper.
        Session::load(&sessions.join(format!("{}.toml", id.as_str()))).ok()
    });
    let session_start_requires_persistence = event == "SessionStart"
        && matches!(
            resolve_hook_agent_session_id(session.as_ref(), hook_event.as_ref()),
            HookAgentSessionId::Provided(_)
        );
    let status =
        status_for_event(event).ok_or_else(|| HookError::InvalidEvent(event.to_string()))?;
    let Some(completion) = completion else {
        let expected =
            validated_hook_agent_session_id(event, &id, session.as_ref(), hook_event.as_ref())?;
        if event == "SessionStart" {
            if let Some(expected) = expected {
                let durable = Session::load(&sessions.join(format!("{}.toml", id.as_str()))).ok();
                if durable.as_ref().and_then(Session::exact_resume_session_id)
                    != Some(expected.as_str())
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        session_start_persisted_id_missing_message(id.as_str()),
                    )
                    .into());
                }
            }
        }
        // Keep the original runtime-step I/O diagnostics without publishing
        // liveness for a source whose observation could not be authenticated.
        match std::fs::File::open(&path).and_then(|mut file| file.read(&mut [0_u8; 1])) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(HookRuntimePublication::default());
    };
    if completion.runtime_path != path
        || completion.session_id != id.as_str()
        || completion.sessions_dir != sessions
        || completion.event.as_deref() != Some(event)
    {
        completion.can_certify_current = false;
        return Ok(HookRuntimePublication::default());
    }
    let captured = completion.clone();
    completion.can_certify_current = false;
    let mut invoked = false;
    let result = gwt_agent::session::with_session_path_lease_wait(
        &captured.sessions_dir,
        &captured.session_id,
        HOOK_SESSION_METADATA_LEASE_WAIT,
        |state| {
            invoked = true;
            let gwt_agent::SessionPathState::Present(session) = state else {
                return Ok((HookRuntimePublication::default(), false));
            };
            let mut session = *session;
            let _writer = gwt_agent::session::runtime_writer_lease(&captured.runtime_path)?;
            let _publication =
                gwt_agent::session::runtime_publication_lease(&captured.runtime_path)?;
            if !captured.matches_session(&session)? {
                return Ok((HookRuntimePublication::default(), false));
            }
            let previous = if captured.initially_missing_legacy {
                match std::fs::metadata(&captured.runtime_path) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    _ => return Ok((HookRuntimePublication::default(), false)),
                }
            } else {
                let Some((runtime, value)) = captured.owned_runtime(&session)? else {
                    return Ok((HookRuntimePublication::default(), false));
                };
                Some((runtime, value))
            };
            let agent_session_id =
                validated_hook_agent_session_id(event, &id, Some(&session), hook_event.as_ref())
                    .map_err(|error| io::Error::other(error.to_string()))?;
            let legacy_root = session.project_state_root.is_none().then(|| {
                crate::agent_project_state::canonical_project_state_root_for_session(
                    &session,
                    &session.worktree_path,
                )
            });
            let sync_id = agent_session_id.as_ref().filter(|agent_id| {
                event == "SessionStart" || agent_session_id_needs_sync(Some(&session), agent_id)
            });
            let persisted = timed_substage(event, "runtime-state/session-metadata", || {
                persist_metadata(
                    &captured.sessions_dir,
                    &captured.session_id,
                    event,
                    sync_id.map(HookSessionId::as_str),
                    legacy_root.as_deref(),
                )
            });
            let metadata_saved = match persisted {
                Ok(updated) => {
                    session = updated;
                    true
                }
                Err(error) => {
                    log_session_metadata_error("record hook metadata for", &id, &error);
                    if event == "SessionStart" && agent_session_id.is_some() {
                        return Err(error);
                    }
                    false
                }
            };
            let pending = timed_substage(event, "runtime-state/pending-resume", || {
                load_pending_resume(&session.worktree_path).ok().flatten()
            });
            let mut value = event_runtime_value(
                previous.map(|pair| pair.0),
                event,
                status,
                pending,
                hook_event.as_ref().and_then(RawHookEvent::tool_use_id),
            )
            .map_err(|error| io::Error::other(error.to_string()))?;
            // A failed metadata association remains uncertified, but the
            // owned token is needed to publish actual-output history once.
            value["current_hook_event"] =
                serde_json::to_value(captured.pending_observation()).map_err(io::Error::other)?;
            timed_substage(event, "runtime-state/state-write", || {
                publish_runtime(&captured.runtime_path, &value, false)
            })?;
            Ok((
                HookRuntimePublication {
                    session: Some(session),
                    published: true,
                },
                metadata_saved,
            ))
        },
    );
    match finish_lease(result) {
        Ok((publication, metadata_saved)) => {
            if publication.published {
                completion.initially_missing_legacy = false;
            }
            completion.can_certify_current =
                publication.published && metadata_saved && completion.exact.is_some();
            Ok(publication)
        }
        Err(error) if !invoked && !session_start_requires_persistence => {
            log_session_metadata_error("record hook metadata for", &id, &error);
            Ok(HookRuntimePublication::default())
        }
        Err(error) => {
            completion.correlation_id = None;
            Err(error.into())
        }
    }
}

pub(crate) fn record_completed_stop_owned(
    completion: Option<&mut HookCompletionContext>,
) -> Result<(), HookError> {
    let Some(completion) = completion else {
        return Ok(());
    };
    if completion.event.as_deref() != Some("Stop") {
        return Ok(());
    }
    let captured = completion.clone();
    let mut invoked = false;
    let result = gwt_agent::session::with_session_path_lease_wait(
        &captured.sessions_dir,
        &captured.session_id,
        HOOK_SESSION_METADATA_LEASE_WAIT,
        |state| {
            invoked = true;
            let gwt_agent::SessionPathState::Present(session) = state else {
                return Ok(false);
            };
            let mut session = *session;
            let _writer = gwt_agent::session::runtime_writer_lease(&captured.runtime_path)?;
            let _publication =
                gwt_agent::session::runtime_publication_lease(&captured.runtime_path)?;
            if captured.owned_runtime(&session)?.is_none() {
                return Ok(false);
            }
            let expected = session.clone();
            session.migrate_legacy_launch_args();
            session.record_completed_stop();
            session.save_if_unchanged_under_lease(&captured.sessions_dir, &expected)
        },
    );
    match result {
        Ok(saved) => {
            completion.can_certify_current &= saved;
            Ok(())
        }
        Err(error) => {
            completion.can_certify_current = false;
            if invoked {
                completion.correlation_id = None;
                Err(error.into())
            } else {
                Ok(())
            }
        }
    }
}

pub(crate) fn record_blocked_stop_owned(
    completion: Option<&mut HookCompletionContext>,
) -> Result<HookRuntimePublication, HookError> {
    let Some(completion) = completion else {
        return Ok(HookRuntimePublication::default());
    };
    if completion.event.as_deref() != Some("Stop") {
        return Ok(HookRuntimePublication::default());
    }
    let captured = completion.clone();
    let mut invoked = false;
    let result = gwt_agent::session::with_session_path_lease_wait(
        &captured.sessions_dir,
        &captured.session_id,
        HOOK_SESSION_METADATA_LEASE_WAIT,
        |state| {
            invoked = true;
            let gwt_agent::SessionPathState::Present(session) = state else {
                return Ok((HookRuntimePublication::default(), false));
            };
            let mut session = *session;
            let _writer = gwt_agent::session::runtime_writer_lease(&captured.runtime_path)?;
            let _publication =
                gwt_agent::session::runtime_publication_lease(&captured.runtime_path)?;
            let Some((runtime, _)) = captured.owned_runtime(&session)? else {
                return Ok((HookRuntimePublication::default(), false));
            };
            let expected = session.clone();
            session.migrate_legacy_launch_args();
            session.record_hook_status(AgentStatus::Running);
            let saved = session.save_if_unchanged_under_lease(&captured.sessions_dir, &expected)?;
            if !saved {
                return Ok((HookRuntimePublication::default(), false));
            }
            let pending = load_pending_resume(&session.worktree_path).ok().flatten();
            let mut value = event_runtime_value(Some(runtime), "Stop", "Running", pending, None)
                .map_err(|error| io::Error::other(error.to_string()))?;
            value["current_hook_event"] =
                serde_json::to_value(captured.pending_observation()).map_err(io::Error::other)?;
            write_hook_runtime_value(&captured.runtime_path, &value, false)?;
            Ok((
                HookRuntimePublication {
                    session: Some(session),
                    published: true,
                },
                true,
            ))
        },
    );
    match result {
        Ok((publication, saved)) => {
            completion.can_certify_current &= saved;
            Ok(publication)
        }
        Err(error) => {
            completion.can_certify_current = false;
            if invoked {
                completion.correlation_id = None;
                Err(error.into())
            } else {
                Ok(HookRuntimePublication::default())
            }
        }
    }
}

#[cfg(test)]
fn record_hook_event_completed_from_env() {
    let Some(path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).map(PathBuf::from)
    else {
        return;
    };
    let Some(session) = current_session_from_env(&sessions_dir_for_runtime_path(&path)) else {
        return;
    };
    if let Some(context) = begin_hook_observation("PostToolUse", "{}", &session.worktree_path)
        .expect("owned test observation")
    {
        context.record(gwt_agent::session::HookCompletionOutcome::Normal);
    }
}

/// The Issue linked to the current managed session, when its metadata is
/// readable. Used to name the report target of a hook failure (Issue #3541).
pub(crate) fn linked_issue_from_env() -> Option<u64> {
    let gwt_session_id = GwtSessionId::from_env()?;
    let sessions_dir = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(PathBuf::from)
        .map(|path| sessions_dir_for_runtime_path(&path))
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    current_session_for_id(&sessions_dir, &gwt_session_id)?.linked_issue_number
}

fn pending_discussion_for_session(
    sessions_dir: &Path,
    session_id: &str,
) -> io::Result<Option<PendingDiscussionResume>> {
    let session = Session::load_and_migrate(&sessions_dir.join(format!("{session_id}.toml")))?;
    load_pending_resume(&session.worktree_path)
}

fn current_session_from_env(sessions_dir: &Path) -> Option<Session> {
    let session_id = GwtSessionId::from_env()?;
    current_session_for_id(sessions_dir, &session_id)
}

fn current_session_for_id(sessions_dir: &Path, gwt_session_id: &GwtSessionId) -> Option<Session> {
    let path = sessions_dir.join(format!("{}.toml", gwt_session_id.as_str()));
    if !path.exists() {
        return None;
    }
    match Session::load_and_migrate(&path) {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!(
                "gwtd hook runtime-state: failed to load session metadata {}: {error}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
fn sync_agent_session_id(
    sessions_dir: &Path,
    gwt_session_id: &GwtSessionId,
    agent_session_id: &HookSessionId,
) -> io::Result<()> {
    persist_agent_session_id(
        sessions_dir,
        gwt_session_id.as_str(),
        agent_session_id.as_str(),
    )
}

fn agent_session_id_needs_sync(
    session: Option<&Session>,
    agent_session_id: &HookSessionId,
) -> bool {
    session.and_then(Session::exact_resume_session_id) != Some(agent_session_id.as_str())
}

fn validated_hook_agent_session_id(
    event: &str,
    gwt_session_id: &GwtSessionId,
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) -> Result<Option<HookSessionId>, HookError> {
    match resolve_hook_agent_session_id(session, hook_event) {
        HookAgentSessionId::Provided(session_id) => Ok(Some(session_id)),
        HookAgentSessionId::MissingRequiredForCodex => {
            // Fail open, mirroring daemon_runtime::live_event_agent_session_id.
            // Codex omits a usable session_id on tool-use events (PreToolUse /
            // PostToolUse), but the id was already captured at SessionStart and
            // persisted in the session .toml. Returning Ok(None) skips
            // sync_agent_session_id (so the persisted id is preserved and the
            // "agent-session" placeholder is never written) while the caller
            // still writes runtime state and records the hook event. Failing
            // closed here surfaces "hook exited with code 1" to the user on
            // every tool call for no functional gain. Only warn when there is
            // genuinely no persisted id to fall back to.
            if session.and_then(Session::exact_resume_session_id).is_none() {
                log_missing_codex_hook_session_id(event, gwt_session_id, session, hook_event);
            }
            Ok(None)
        }
        HookAgentSessionId::MissingOptional => Ok(None),
    }
}

fn log_missing_codex_hook_session_id(
    event: &str,
    gwt_session_id: &GwtSessionId,
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) {
    let persisted_agent_session_id = session
        .and_then(gwt_agent::Session::exact_resume_session_id)
        .unwrap_or("-");
    let tool_name = hook_event.and_then(RawHookEvent::tool_name).unwrap_or("-");
    eprintln!(
        "gwtd hook runtime-state: missing Codex hook session_id event={event} gwt_session_id={} persisted_agent_session_id={persisted_agent_session_id} tool_name={tool_name}",
        gwt_session_id.as_str()
    );
}

fn log_session_metadata_error(action: &str, gwt_session_id: &GwtSessionId, error: &io::Error) {
    eprintln!(
        "gwtd hook runtime-state: failed to {action} session metadata gwt_session_id={}: {error}",
        gwt_session_id.as_str()
    );
}

#[cfg(test)]
fn sync_coordination_for_session(_session: &Session, _event: &str) {}

/// Production entry point. Reads `$GWT_SESSION_RUNTIME_PATH` and delegates
/// to [`write_for_event`]. An unset env var is a silent no-op so that
/// sessions launched outside of gwt (e.g. a raw `claude` invocation) are
/// not broken by a hook we shipped.
pub fn handle(event: &str) -> Result<(), HookError> {
    if std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV).is_none() {
        return Ok(());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    handle_with_input(event, &input)
}

pub fn handle_with_input(event: &str, input: &str) -> Result<(), HookError> {
    handle_with_input_prepared(event, input).map(|_| ())
}

pub(crate) fn handle_with_input_prepared(
    event: &str,
    input: &str,
) -> Result<Option<Session>, HookError> {
    let Some(runtime_path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV) else {
        return Ok(None);
    };
    let runtime_path = PathBuf::from(runtime_path);
    let hook_event = if input.trim().is_empty() {
        None
    } else {
        RawHookEvent::read_from_str(input)?
    };
    let sessions_dir = sessions_dir_for_runtime_path(&runtime_path);
    let gwt_session_id = GwtSessionId::required_from_env(event)?;
    let mut session = timed_substage(event, "runtime-state/session-load", || {
        current_session_for_id(&sessions_dir, &gwt_session_id)
    });
    let agent_session_id = validated_hook_agent_session_id(
        event,
        &gwt_session_id,
        session.as_ref(),
        hook_event.as_ref(),
    )?;
    if session.is_some() {
        let legacy_project_state_root = session.as_ref().and_then(|session| {
            session.project_state_root.is_none().then(|| {
                crate::agent_project_state::canonical_project_state_root_for_session(
                    session,
                    &session.worktree_path,
                )
            })
        });
        let agent_session_id_to_sync = agent_session_id.as_ref().filter(|agent_session_id| {
            event == "SessionStart"
                || agent_session_id_needs_sync(session.as_ref(), agent_session_id)
        });
        let persisted = timed_substage(event, "runtime-state/session-metadata", || {
            persist_session_hook_metadata_with_wait(
                &sessions_dir,
                gwt_session_id.as_str(),
                event,
                agent_session_id_to_sync.map(HookSessionId::as_str),
                legacy_project_state_root.as_deref(),
                HOOK_SESSION_METADATA_LEASE_WAIT,
            )
        });
        match persisted {
            Ok(updated) => session = Some(updated),
            Err(error) => {
                log_session_metadata_error("record hook metadata for", &gwt_session_id, &error);
                if event == "SessionStart" && agent_session_id.is_some() {
                    return Err(error.into());
                }
            }
        }
    }

    if event == "SessionStart" {
        if let Some(expected) = agent_session_id.as_ref() {
            if session.as_ref().and_then(Session::exact_resume_session_id)
                != Some(expected.as_str())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    session_start_persisted_id_missing_message(gwt_session_id.as_str()),
                )
                .into());
            }
        }
    }

    let pending_discussion = timed_substage(event, "runtime-state/pending-resume", || {
        session
            .as_ref()
            .and_then(|session| load_pending_resume(&session.worktree_path).ok().flatten())
    });
    timed_substage(event, "runtime-state/state-write", || {
        let status =
            status_for_event(event).ok_or_else(|| HookError::InvalidEvent(event.to_string()))?;
        write_state_with_status(
            &runtime_path,
            event,
            status,
            pending_discussion,
            hook_event.as_ref().and_then(RawHookEvent::tool_use_id),
        )
    })
    .map(|_| session)
}

/// Time one `runtime-state` substage into the opt-in hook profile.
///
/// Issue #3777: the aggregate `runtime-state` record hides which of the two
/// durable writes, the Session read, or the pending-resume read consumes the
/// UserPromptSubmit budget, and the slow mode only reproduces on Windows CI.
/// The handler names stay on the content-free allowlist in [`super::diagnostics`].
fn timed_substage<T>(event: &str, handler: &'static str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = work();
    super::diagnostics::record_handler_duration(event, handler, started.elapsed(), "ok");
    value
}

pub(crate) fn session_start_agent_session_diagnostic(input: &str) -> Option<String> {
    let runtime_path = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)?;
    let runtime_path = PathBuf::from(runtime_path);
    let sessions_dir = sessions_dir_for_runtime_path(&runtime_path);
    let gwt_session_id = GwtSessionId::from_env()?;
    let hook_event = match RawHookEvent::read_from_str(input) {
        Ok(event) => event,
        Err(_) => return None,
    };
    let session = current_session_for_id(&sessions_dir, &gwt_session_id);
    match resolve_hook_agent_session_id(session.as_ref(), hook_event.as_ref()) {
        HookAgentSessionId::Provided(expected) => {
            let Some(session) = current_session_for_id(&sessions_dir, &gwt_session_id) else {
                return Some(session_start_persisted_id_missing_message(
                    gwt_session_id.as_str(),
                ));
            };
            if session.exact_resume_session_id() == Some(expected.as_str()) {
                None
            } else {
                Some(session_start_persisted_id_missing_message(
                    gwt_session_id.as_str(),
                ))
            }
        }
        HookAgentSessionId::MissingRequiredForCodex | HookAgentSessionId::MissingOptional => Some(
            session_start_provider_id_missing_message(gwt_session_id.as_str()),
        ),
    }
}

fn session_start_provider_id_missing_message(gwt_session_id: &str) -> String {
    format!(
        "gwt could not associate this agent session because SessionStart did not include a provider session id.\n\
\n\
GWT session: {gwt_session_id}\n\
Expected: Claude Code / Codex SessionStart must provide a concrete provider session id so gwt can persist it as agent_session_id and session_history.\n\
Impact: this Work cannot be treated as resumable until the provider session id is captured."
    )
}

fn session_start_persisted_id_missing_message(gwt_session_id: &str) -> String {
    format!(
        "gwt could not associate this agent session because the provider session id from SessionStart was not persisted into session metadata.\n\
\n\
GWT session: {gwt_session_id}\n\
Expected: SessionStart should write agent_session_id and session_history before Workspace registration.\n\
Impact: this Work cannot be treated as resumable until session metadata is repaired."
    )
}

pub fn record_completed_stop_from_env() -> Result<(), HookError> {
    let Some(gwt_session_id) = GwtSessionId::from_env() else {
        return Ok(());
    };
    let sessions_dir = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .map(PathBuf::from)
        .map(|path| sessions_dir_for_runtime_path(&path))
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    if let Err(error) =
        gwt_agent::persist_session_completed_stop(&sessions_dir, gwt_session_id.as_str())
    {
        log_session_metadata_error("record completed Stop for", &gwt_session_id, &error);
    }
    Ok(())
}

pub fn record_blocked_stop_from_env() -> Result<(), HookError> {
    let Some(gwt_session_id) = GwtSessionId::from_env() else {
        return Ok(());
    };
    let Some(runtime_path) = std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV) else {
        return Ok(());
    };
    let runtime_path = PathBuf::from(runtime_path);
    let sessions_dir = sessions_dir_for_runtime_path(&runtime_path);
    let session = current_session_for_id(&sessions_dir, &gwt_session_id);
    if session.is_some() {
        if let Err(error) = gwt_agent::update_session_if_changed(
            &sessions_dir,
            gwt_session_id.as_str(),
            |session| {
                session.record_hook_status(AgentStatus::Running);
                Ok(())
            },
        ) {
            log_session_metadata_error("restore blocked Stop status for", &gwt_session_id, &error);
        }
    }
    let pending_discussion = session.as_ref().and_then(|session| {
        pending_discussion_for_session(&sessions_dir, &session.id)
            .ok()
            .flatten()
    });
    write_state_with_status(&runtime_path, "Stop", "Running", pending_discussion, None)
}

fn sessions_dir_for_runtime_path(runtime_path: &Path) -> PathBuf {
    gwt_agent::sessions_dir_from_runtime_path(runtime_path)
        .unwrap_or_else(gwt_core::paths::gwt_sessions_dir)
}

#[cfg(test)]
mod tests {
    use fs2::FileExt;
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV};
    use gwt_core::coordination::{coordination_events_segments_dir, load_snapshot};
    use std::ffi::OsString;
    use std::fs::OpenOptions;
    use std::time::Duration;

    use super::*;

    fn env_lock() -> gwt_core::test_support::EnvLockGuard {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn runtime_state_env_lock_shares_crate_wide_env_lock() {
        let global = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).expect("send lock attempt started");
            let _lock = env_lock();
            tx.send(()).expect("send lock acquired");
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("env lock probe thread started");
        assert!(
            rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "runtime_state tests must wait on the crate-wide env lock"
        );
        drop(global);
        handle.join().expect("env lock probe thread");
    }

    struct EnvGuard {
        saved: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self { saved: Vec::new() }
        }

        fn set(&mut self, key: &'static str, value: impl Into<OsString>) {
            if !self.saved.iter().any(|(saved, _)| *saved == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::set_var(key, value.into());
        }

        fn unset(&mut self, key: &'static str) {
            if !self.saved.iter().any(|(saved, _)| *saved == key) {
                self.saved.push((key, std::env::var_os(key)));
            }
            std::env::remove_var(key);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            while let Some((key, value)) = self.saved.pop() {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn assert_no_board_entries_or_events(root: &std::path::Path) {
        let snapshot = load_snapshot(root).unwrap();
        assert!(snapshot.board.entries.is_empty());
        let event_lines = std::fs::read_dir(coordination_events_segments_dir(root))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .map(|entry| {
                std::fs::read_to_string(entry.path())
                    .unwrap()
                    .lines()
                    .count()
            })
            .sum::<usize>();
        assert_eq!(event_lines, 0);
    }

    #[test]
    fn pending_discussion_for_session_reads_active_discussion_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("wt-feature");
        let discussion_path = gwt_core::paths::gwt_repo_local_discussions_path(&worktree);
        std::fs::create_dir_all(discussion_path.parent().unwrap()).unwrap();
        std::fs::write(
            discussion_path,
            r#"## Discussion TODO

### Proposal A - Hook-driven resume [active]
- Summary:
- Open Questions:
- Dependency Checks:
- Deferred Decisions:
- Next Question: Should SessionStart surface the proposal?
- Promotable Changes:
"#,
        )
        .unwrap();

        let session = Session::new(&worktree, "feature", AgentId::Codex);
        session.save(&sessions_dir).unwrap();

        let pending = pending_discussion_for_session(&sessions_dir, &session.id).unwrap();

        assert_eq!(
            pending,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            })
        );
    }

    #[test]
    fn write_for_event_with_pending_discussion_persists_resume_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");

        write_for_event_with_pending_discussion(
            &path,
            "Stop",
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            }),
        )
        .unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Idle");
        assert_eq!(state.source_event, "Stop");
        assert_eq!(
            state.pending_discussion,
            Some(PendingDiscussionResume {
                proposal_label: "Proposal A".to_string(),
                proposal_title: "Hook-driven resume".to_string(),
                next_question: Some("Should SessionStart surface the proposal?".to_string()),
            })
        );
    }

    #[test]
    fn hook_runtime_update_preserves_exact_execution_proof_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");
        let mut session = Session::new(dir.path(), "work/issue-3547", AgentId::Codex);
        session.project_state_root = Some(dir.path().to_path_buf());
        session.repo_hash = Some("repo-hook-proof".to_string());
        session.linked_issue_number = Some(3547);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: "repo-hook-proof".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3547,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-hook-proof".to_string(),
                binding_id: "binding-hook-proof".to_string(),
                ledger_head_hash: "head-hook-proof".to_string(),
            },
            capability_generation: 3,
        };
        session.set_execution_binding(Some(binding)).unwrap();
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let mut runtime =
            gwt_agent::SessionRuntimeState::for_execution(AgentStatus::Running, &identity, 41);
        runtime.host_started_at = Some(1_000);
        runtime.child_pid = Some(4_242);
        runtime.child_started_at = Some(2_000);
        runtime.save(&path).unwrap();

        write_for_event_with_pending_discussion(&path, "Stop", None).unwrap();

        let updated = gwt_agent::SessionRuntimeState::load(&path).unwrap();
        assert_eq!(updated.execution_identity.as_ref(), Some(&identity));
        assert_eq!(updated.runtime_incarnation, Some(41));
        // Issue #4643: the PTY child identity is the only sidecar-local
        // evidence that the launch has ended; a hook event must not erase it.
        assert_eq!(updated.host_started_at, Some(1_000));
        assert_eq!(updated.child_pid, Some(4_242));
        assert_eq!(updated.child_started_at, Some(2_000));
        assert_eq!(updated.source_event.as_deref(), Some("Stop"));
    }

    #[test]
    fn status_for_event_maps_idle_and_running_lifecycle_events() {
        assert_eq!(status_for_event("SessionStart"), Some("Idle"));
        assert_eq!(status_for_event("UserPromptSubmit"), Some("Running"));
        assert_eq!(status_for_event("PreToolUse"), Some("Running"));
        assert_eq!(status_for_event("PostToolUse"), Some("Running"));
        assert_eq!(status_for_event("Stop"), Some("Idle"));
    }

    #[test]
    fn active_tool_calls_follow_identified_provider_lifecycle() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime-state.json");
        env.set(GWT_SESSION_ID_ENV, "tool-lifecycle");
        env.set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, path.as_os_str());

        for (event, id, expected) in [
            ("UserPromptSubmit", None, "null"),
            ("SessionStart", None, "[]"),
            ("PreToolUse", Some("a"), r#"["a"]"#),
            ("PreToolUse", Some("a"), r#"["a"]"#),
            ("PreToolUse", Some("b"), r#"["a","b"]"#),
            ("PostToolUse", Some("other"), r#"["a","b"]"#),
            ("PostToolUse", Some("a"), r#"["b"]"#),
            ("UserPromptSubmit", None, r#"["b"]"#),
            ("PostToolUse", Some("b"), "[]"),
            ("PreToolUse", Some("c"), r#"["c"]"#),
            ("Stop", None, "[]"),
            ("PreToolUse", None, "null"),
            ("PostToolUse", Some("c"), "null"),
            ("PreToolUse", Some("d"), r#"["d"]"#),
            ("PostToolUse", None, "null"),
        ] {
            let input = serde_json::json!({"tool_use_id": id}).to_string();
            handle_with_input(event, &input).unwrap();
            let state: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(
                state["active_tool_calls"].to_string(),
                expected,
                "{event} {input}"
            );
        }
    }

    #[test]
    fn event_state_preserves_the_completion_timestamp_and_outcome_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.json");
        let mut state = SessionRuntimeState::new(AgentStatus::Running);
        state.last_completed_hook_event_at = Some(Utc::now());
        state.last_completed_hook_outcome =
            Some(gwt_agent::session::HookCompletionOutcome::Blocked);
        state.save(&path).unwrap();
        write_state_with_status(&path, "PostToolUse", "Running", None, None).unwrap();
        let updated = SessionRuntimeState::load(&path).unwrap();
        assert_eq!(
            updated.last_completed_hook_event_at,
            state.last_completed_hook_event_at
        );
        assert_eq!(
            updated.last_completed_hook_outcome,
            state.last_completed_hook_outcome
        );
    }

    #[test]
    fn concurrent_tool_hooks_and_completion_share_the_runtime_writer_lease() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path(), "work/issue-6622", AgentId::ClaudeCode);
        session.id = "parallel-tools".to_string();
        session.save(dir.path()).unwrap();
        let path = gwt_agent::runtime_state_path(dir.path(), "parallel-tools");
        env.set(GWT_SESSION_ID_ENV, "parallel-tools");
        env.set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, path.as_os_str());
        write_for_event(&path, "SessionStart").unwrap();
        let ready = std::sync::Barrier::new(4);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let (tools_done_tx, tools_done_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let writer = gwt_agent::session::runtime_writer_lease(&path).unwrap();
            let mut completion_wait = Some(tools_done_rx);
            for id in [Some("a"), Some("b"), None] {
                let (path, ready, done) = (&path, &ready, done_tx.clone());
                let tools_done = tools_done_tx.clone();
                let completion_wait = if id.is_none() {
                    completion_wait.take()
                } else {
                    None
                };
                scope.spawn(move || {
                    ready.wait();
                    if let Some(id) = id {
                        write_state_with_status(path, "PreToolUse", "Running", None, Some(id))
                            .unwrap();
                        tools_done.send(()).unwrap();
                    } else {
                        // Raw publications invalidate earlier observations.
                        // Begin this completion only after both raw writers.
                        let completion_wait = completion_wait.unwrap();
                        for _ in 0..2 {
                            completion_wait.recv().unwrap();
                        }
                        record_hook_event_completed_from_env();
                    }
                    done.send(()).unwrap();
                });
            }
            ready.wait();
            // test-hygiene: allow-wall-clock-deadline This negative receipt probe checks native writer-lock exclusion; successful publication is joined after releasing the lock.
            assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
            drop(writer);
        });
        let state = SessionRuntimeState::load(&path).unwrap();
        assert_eq!(
            state.active_tool_calls,
            Some(["a".into(), "b".into()].into())
        );
        assert!(state.last_completed_hook_event_at.is_some());
    }

    #[test]
    fn handle_with_input_ignores_corrupt_session_metadata() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join("session-corrupt.toml"), "odex\"").unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("PostToolUse", "{}").expect("corrupt metadata must not fail the hook");

        let raw = std::fs::read_to_string(&runtime_path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PostToolUse");
    }

    #[test]
    fn session_start_rejects_corrupt_metadata_with_provider_identity() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(
            sessions_dir.join("session-corrupt.toml"),
            "started_at = \"2026-06-16T04:30:00.\n320310Z\"",
        )
        .unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("SessionStart", r#"{"session_id":"agent-123"}"#)
            .expect_err("SessionStart must not accept an uncommitted provider identity");
        assert!(
            !runtime_path.exists(),
            "failed identity must not publish Idle"
        );
    }

    #[test]
    fn session_start_rejects_failed_identity_commit_under_session_lease() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        env.unset(gwt_agent::GWT_HOOK_FORWARD_URL_ENV);
        env.unset(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV);
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let mut session = Session::new(dir.path(), "work/issue-5102", AgentId::Codex);
        session.agent_session_id = Some("source-provider-5102".to_string());
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        env.set(GWT_SESSION_ID_ENV, &session.id);
        env.set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

        // A held canonical lease rejects a nested writer deterministically;
        // no wall-clock ordering is needed to exercise persistence failure.
        gwt_agent::with_session_path_lease(&sessions_dir, &session.id, |_| {
            crate::daemon_runtime::handle_runtime_state(
                "SessionStart",
                r#"{"session_id":"provider-5102"}"#,
            )
            .expect_err("a failed identity commit must not reach hook-live success");
            Ok(())
        })
        .unwrap();

        let loaded = Session::load(&sessions_dir.join(format!("{}.toml", session.id))).unwrap();
        assert_eq!(loaded.agent_session_id, session.agent_session_id);
        assert!(!runtime_path.exists());
    }

    #[test]
    fn session_start_diagnostic_rejects_a_different_persisted_provider_id() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let mut session = Session::new(dir.path(), "work/issue-5102", AgentId::ClaudeCode);
        session.agent_session_id = Some("source-provider-5102".to_string());
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        env.set(GWT_SESSION_ID_ENV, &session.id);
        env.set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

        assert!(
            session_start_agent_session_diagnostic(r#"{"session_id":"provider-5102"}"#).is_some(),
            "an old resume handle does not prove the supplied identity was committed"
        );
    }

    #[test]
    fn record_completed_stop_fails_open_when_session_metadata_is_corrupt() {
        let _lock = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join("session-corrupt.toml"), "169630Z\"").unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "session-corrupt");
        let mut env = EnvGuard::new();
        env.set(GWT_SESSION_ID_ENV, "session-corrupt");
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        handle_with_input("Stop", "{}").expect("Stop sidecar write must succeed");
        record_completed_stop_from_env()
            .expect("completed-stop metadata update must fail open on corrupt TOML");

        let raw = std::fs::read_to_string(&runtime_path).unwrap();
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Idle");
        assert_eq!(state.source_event, "Stop");
    }

    #[test]
    fn sync_coordination_for_session_running_event_does_not_append_message() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_session_start_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "SessionStart");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_stop_does_not_append_board_status_entry() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "Stop");

        assert_no_board_entries_or_events(dir.path());
    }

    #[test]
    fn sync_coordination_for_session_skips_noop_status_updates() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);

        sync_coordination_for_session(&session, "PreToolUse");
        sync_coordination_for_session(&session, "PostToolUse");

        assert_no_board_entries_or_events(dir.path());
    }

    /// SPEC-3966 AC-2: a Claude Code Session (the resident PM's agent) must
    /// come out of SessionStart with a resume handle written into its durable
    /// record. Everything above this hook — the PM restore path — reads the
    /// TOML, so the field has to survive a reload, not just an in-memory write.
    #[test]
    fn claude_session_start_persists_agent_session_id_into_session_toml() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("pm-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let session = Session::new(&worktree, "work", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        assert!(
            Session::load(&sessions_dir.join(format!("{session_id}.toml")))
                .unwrap()
                .exact_resume_session_id()
                .is_none(),
            "a freshly registered Session starts without a resume handle"
        );
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input(
            "SessionStart",
            r#"{"session_id":"pm-conversation-3966","cwd":"pm-worktree"}"#,
        )
        .expect("SessionStart must persist the provider conversation id");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(
            loaded.exact_resume_session_id(),
            Some("pm-conversation-3966"),
            "the reloaded Session must expose the handle the restore path resumes from"
        );
        assert_eq!(
            loaded
                .session_history
                .iter()
                .map(|entry| entry.agent_session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["pm-conversation-3966"],
        );
    }

    #[test]
    fn sync_agent_session_id_persists_value_into_session_toml() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let session = Session::new(dir.path(), "feature/demo", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        let gwt_session_id = GwtSessionId::from_env().unwrap();

        let agent_session_id = RawHookEvent::read_from_str(r#"{"session_id":"agent-123"}"#)
            .unwrap()
            .unwrap()
            .session_id()
            .unwrap();
        sync_agent_session_id(&sessions_dir, &gwt_session_id, &agent_session_id).unwrap();

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-123"));
    }

    #[test]
    fn sync_agent_session_id_does_not_clear_existing_value_when_hook_session_id_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let mut session = Session::new(dir.path(), "feature/demo", AgentId::Codex);
        session.agent_session_id = Some("agent-existing".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();

        let parsed = RawHookEvent::read_from_str(r#"{"tool_name":"Bash"}"#)
            .unwrap()
            .unwrap()
            .session_id();
        assert!(parsed.is_none());

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));
    }

    #[test]
    fn codex_runtime_state_fails_open_when_hook_session_id_missing() {
        // A Codex tool-use payload without a session_id (the shape Codex
        // actually sends on PreToolUse/PostToolUse) must NOT break the tool
        // call. The agent_session_id was already captured at SessionStart, so
        // the hook fails open: it preserves the persisted id (never writing the
        // "agent-session" placeholder) and still records runtime state.
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let mut session = Session::new(&worktree, "feature/demo", AgentId::Codex);
        session.agent_session_id = Some("agent-existing".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );
        env.unset("CODEX_THREAD_ID");

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#)
            .expect("missing Codex hook session_id must fail open, not break the tool call");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id.as_deref(), Some("agent-existing"));
        assert_eq!(loaded.last_hook_event.as_deref(), Some("PreToolUse"));

        let raw = std::fs::read_to_string(&runtime_path).expect("runtime state written");
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PreToolUse");
    }

    #[test]
    fn user_prompt_submit_session_bookkeeping_fails_open_under_lease_contention() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(sessions_dir.join(format!(".{session_id}.lock")))
            .unwrap();
        lease.lock_exclusive().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            lease.unlock().unwrap();
        });

        handle_with_input("UserPromptSubmit", r#"{"session_id":"agent-new"}"#)
            .expect("contended bookkeeping must fail open");
        release.join().unwrap();

        // Assert the bound itself rather than how long the call happened to
        // take. The wall clock measures the runner as much as the code: under
        // CI's default parallelism this observed 259ms against a 200ms limit
        // even though the wait was clamped (Issue #3777, CI run 34490959331).
        // The hook waits `HOOK_SESSION_METADATA_LEASE_WAIT` for the Session
        // lease, so holding that constant below the 300ms the thread above
        // keeps the lease is what proves the hook cannot have waited the
        // contended lease out — and it stays true on a loaded runner.
        assert!(
            HOOK_SESSION_METADATA_LEASE_WAIT <= Duration::from_millis(25),
            "the Session lease wait must stay clamped well under the contended \
             hold, otherwise UserPromptSubmit blocks on it: \
             {HOOK_SESSION_METADATA_LEASE_WAIT:?}"
        );
        let raw = std::fs::read_to_string(&runtime_path).expect("runtime state written");
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "UserPromptSubmit");
    }

    #[test]
    fn codex_runtime_state_fails_open_when_hook_session_id_blank() {
        // A blank session_id resolves to MissingRequiredForCodex with no
        // persisted id to fall back to. The hook must still fail open: write
        // runtime state and leave agent_session_id unset (never the placeholder
        // or the blank value).
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::Codex);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );
        env.unset("CODEX_THREAD_ID");

        handle_with_input("PostToolUse", r#"{"session_id":"   "}"#)
            .expect("blank Codex session_id must fail open");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.agent_session_id, None);

        let raw = std::fs::read_to_string(&runtime_path).expect("runtime state written");
        let state: RuntimeState = serde_json::from_str(&raw).unwrap();
        assert_eq!(state.status, "Running");
        assert_eq!(state.source_event, "PostToolUse");
    }

    #[test]
    fn managed_runtime_state_rejects_missing_gwt_session_id() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, "missing-gwt-session");
        env.unset(GWT_SESSION_ID_ENV);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        let err = handle_with_input("PreToolUse", r#"{"session_id":"agent-123"}"#)
            .expect_err("managed hooks must include GWT_SESSION_ID");

        match err {
            HookError::InvalidEvent(message) => {
                assert!(message.contains("missing GWT_SESSION_ID"), "{message}");
            }
            other => panic!("expected InvalidEvent, got {other:?}"),
        }
        assert!(
            !runtime_path.exists(),
            "invalid managed identity must not write state"
        );
    }

    #[test]
    fn non_codex_runtime_state_allows_missing_hook_session_id_for_compatibility() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#).unwrap();

        assert!(runtime_path.exists());
    }

    #[test]
    fn runtime_state_persists_hook_lifecycle_to_session_toml() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("PreToolUse", r#"{"tool_name":"Bash"}"#).unwrap();
        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();

        assert_eq!(loaded.last_hook_event.as_deref(), Some("PreToolUse"));
        assert!(loaded.last_hook_event_at.is_some());
        assert!(loaded.last_completed_stop_at.is_none());
        assert_eq!(loaded.status, gwt_agent::AgentStatus::Running);
        assert!(loaded.should_mark_interrupted_from_lifecycle());
    }

    /// Issue #4768: Grok Build sends every hook field under both its
    /// camelCase key and the Claude-compatible snake_case key. Both spellings
    /// in one payload must not fail the hook closed.
    #[test]
    fn runtime_state_records_grok_payload_carrying_both_key_spellings() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "work/issue-277", AgentId::GrokBuild);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input(
            "SessionStart",
            r#"{"hookEventName":"session_start","hook_event_name":"SessionStart","sessionId":"grok-conversation-4768","session_id":"grok-conversation-4768","cwd":"repo"}"#,
        )
        .expect("SessionStart with both session id spellings must succeed");
        handle_with_input(
            "PreToolUse",
            r#"{"hookEventName":"pre_tool_use","hook_event_name":"PreToolUse","sessionId":"grok-conversation-4768","session_id":"grok-conversation-4768","toolName":"run_command","tool_name":"Bash","toolInput":{"command":"ls"},"tool_input":{"command":"ls"}}"#,
        )
        .expect("PreToolUse with both key spellings must succeed");

        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert_eq!(loaded.last_hook_event.as_deref(), Some("PreToolUse"));
        assert_eq!(
            loaded.exact_resume_session_id(),
            Some("grok-conversation-4768")
        );
    }

    #[test]
    fn runtime_state_records_completed_stop_as_clean_boundary() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join(".gwt").join("sessions");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        let session = Session::new(&worktree, "feature/demo", AgentId::ClaudeCode);
        let session_id = session.id.clone();
        session.save(&sessions_dir).unwrap();
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        env.set(GWT_SESSION_ID_ENV, session_id.clone());
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str().to_os_string(),
        );

        handle_with_input("Stop", r#"{}"#).unwrap();
        let stopped_before_completion =
            Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();
        assert!(stopped_before_completion.should_mark_interrupted_from_lifecycle());

        record_completed_stop_from_env().unwrap();
        let loaded = Session::load(&sessions_dir.join(format!("{session_id}.toml"))).unwrap();

        assert_eq!(loaded.last_hook_event.as_deref(), Some("Stop"));
        assert!(loaded.last_completed_stop_at.is_some());
        assert_eq!(serde_json::to_string(&loaded.status).unwrap(), "\"Idle\"");
        assert!(!loaded.should_mark_interrupted_from_lifecycle());
    }

    fn with_legacy_observation(test: impl FnOnce(&Path, &Path, &Path)) {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(dir.path());
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&worktree, "work/observation", AgentId::ClaudeCode);
        session.save(&sessions).unwrap();
        let path = gwt_agent::runtime_state_path(&sessions, &session.id);
        let mut runtime = SessionRuntimeState::new(AgentStatus::Idle);
        runtime.source_event = Some("Stop".into());
        runtime.last_completed_hook_event_at = Some(Utc::now());
        runtime.last_completed_hook_outcome =
            Some(gwt_agent::session::HookCompletionOutcome::Normal);
        runtime.save(&path).unwrap();
        env.set(GWT_SESSION_ID_ENV, &session.id);
        env.set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, path.as_os_str());
        test(
            &worktree,
            &sessions.join(format!("{}.toml", session.id)),
            &path,
        );
    }

    // Reuse the existing observation fixture and the real-process Bound seed
    // used by hook_exit_codes_test, rather than introducing native authority.
    fn with_bound_observation(test: impl FnOnce(&Path, &Path, &Path)) {
        with_legacy_observation(|worktree, session_path, path| {
            let mut session = Session::load(session_path).unwrap();
            session.agent_session_id = Some("provider-failure-3165".into());
            session.project_state_root = Some(worktree.to_path_buf());
            session.repo_hash = Some("repo-stop-3165".into());
            session.linked_issue_number = Some(3165);
            session
                .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                    schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                    session_id: session.id.clone(),
                    repo_hash: "repo-stop-3165".into(),
                    owner_kind: "issue".into(),
                    owner_number: 3165,
                    identity: gwt_agent::ExecutionBindingIdentity {
                        generation_id: "generation-stop-3165".into(),
                        binding_id: "binding-stop-3165".into(),
                        ledger_head_hash: "head-stop-3165".into(),
                    },
                    capability_generation: 3,
                }))
                .unwrap();
            session.save(session_path.parent().unwrap()).unwrap();
            let identity = SessionExecutionIdentity::from_session(&session)
                .unwrap()
                .unwrap();
            let pid = std::process::id();
            let birth = crate::process::host_process_start_time(pid).unwrap();
            SessionRuntimeState::for_execution_process(
                AgentStatus::Running,
                &identity,
                41,
                birth,
                pid,
                birth,
            )
            .save(path)
            .unwrap();
            test(worktree, session_path, path);
        });
    }

    fn begin_bound_failure_observation(worktree: &Path, path: &Path) -> HookCompletionContext {
        let receipt = begin_hook_observation(
            "PostToolUse",
            r#"{"session_id":"provider-failure-3165"}"#,
            worktree,
        )
        .unwrap()
        .expect("actual owned begin must succeed before failure");
        let runtime = SessionRuntimeState::load(path).unwrap();
        assert!(receipt.exact.is_some());
        assert!(receipt.matches_source(&runtime));
        assert!(receipt.matches_pending(&runtime));
        assert!(!receipt.can_certify_current);
        assert!(runtime.last_completed_hook_event_at.is_none());
        assert!(runtime.last_completed_hook_outcome.is_none());
        receipt
    }

    #[test]
    fn owned_metadata_read_failure_keeps_pending_until_history_only_completion() {
        with_bound_observation(|worktree, session_path, path| {
            use gwt_agent::session::HookCompletionOutcome::Normal;
            use std::cell::Cell;
            let mut receipt = begin_bound_failure_observation(worktree, path);
            let correlation = receipt.correlation_id.clone();
            let session_bytes = std::fs::read(session_path).unwrap();
            let metadata_calls = Cell::new(0);
            let publication_calls = Cell::new(0);
            let metadata_error = Cell::new(None);
            let published = handle_with_input_owned_with(
                "PostToolUse",
                r#"{"session_id":"provider-failure-3165"}"#,
                Some(&mut receipt),
                |sessions, id, event, provider, project| {
                    metadata_calls.set(metadata_calls.get() + 1);
                    let backup = session_path.with_extension("metadata-failure-backup");
                    std::fs::rename(session_path, &backup).unwrap();
                    std::fs::create_dir(session_path).unwrap();
                    // Real helper read I/O failure after successful owned source
                    // validation. This is not a simulated atomic-write failure.
                    let result = gwt_agent::session::persist_session_hook_metadata_under_lease(
                        sessions, id, event, provider, project,
                    );
                    metadata_error.set(result.as_ref().err().map(io::Error::kind));
                    std::fs::remove_dir(session_path).unwrap();
                    std::fs::rename(backup, session_path).unwrap();
                    result
                },
                |path, value, durable| {
                    publication_calls.set(publication_calls.get() + 1);
                    write_hook_runtime_value(path, value, durable)
                },
                std::convert::identity::<OwnedRuntimeResult>,
            )
            .unwrap();
            assert!(metadata_error.get().is_some());
            assert!(published.published);
            assert_eq!(metadata_calls.get(), 1);
            assert_eq!(publication_calls.get(), 1);
            assert_eq!(std::fs::read(session_path).unwrap(), session_bytes);
            let pending = SessionRuntimeState::load(path).unwrap();
            assert_eq!(receipt.correlation_id, correlation);
            assert!(receipt.matches_source(&pending));
            assert!(receipt.matches_pending(&pending));
            assert!(!receipt.can_certify_current);
            assert!(pending.last_completed_hook_event_at.is_none());
            // Exercise the actual completion guard as an output-success
            // boundary; protocol delivery itself has separate real-sink tests.
            let replay = receipt.clone();
            receipt.record(Normal);
            let completed = std::fs::read(path).unwrap();
            let state = SessionRuntimeState::load(path).unwrap();
            assert!(state.last_completed_hook_event_at.is_some());
            assert_eq!(state.last_completed_hook_outcome, Some(Normal));
            assert!(state.current_hook_event.is_none());
            replay.record(Normal);
            assert_eq!(std::fs::read(path).unwrap(), completed);
            assert_eq!(std::fs::read(session_path).unwrap(), session_bytes);
        });
    }

    #[test]
    fn owned_runtime_rename_failure_discards_receipt_after_metadata_commit() {
        with_bound_observation(|worktree, session_path, path| {
            use std::cell::Cell;
            let mut receipt = begin_bound_failure_observation(worktree, path);
            let pending_bytes = std::fs::read(path).unwrap();
            let metadata_calls = Cell::new(0);
            let publication_calls = Cell::new(0);
            let publication_error = Cell::new(None);
            let result = handle_with_input_owned_with(
                "PostToolUse",
                r#"{"session_id":"provider-failure-3165"}"#,
                Some(&mut receipt),
                |sessions, id, event, provider, project| {
                    metadata_calls.set(metadata_calls.get() + 1);
                    gwt_agent::session::persist_session_hook_metadata_under_lease(
                        sessions, id, event, provider, project,
                    )
                },
                |path, value, durable| {
                    publication_calls.set(publication_calls.get() + 1);
                    std::fs::remove_file(path).unwrap();
                    std::fs::create_dir(path).unwrap();
                    // Real atomic rename-to-directory error, after the real
                    // metadata commit; no chmod assumption under Linux root.
                    let result = write_hook_runtime_value(path, value, durable);
                    publication_error.set(result.as_ref().err().map(io::Error::kind));
                    result
                },
                std::convert::identity::<OwnedRuntimeResult>,
            );
            assert!(result.is_err());
            assert!(publication_error.get().is_some());
            assert_eq!(metadata_calls.get(), 1);
            assert_eq!(publication_calls.get(), 1);
            assert_eq!(
                Session::load(session_path)
                    .unwrap()
                    .last_hook_event
                    .as_deref(),
                Some("PostToolUse")
            );
            assert!(!receipt.can_certify_current);
            assert!(receipt.correlation_id.is_none());
            std::fs::remove_dir(path).unwrap();
            std::fs::write(path, &pending_bytes).unwrap();
            // Restore a valid, writable pending endpoint so rejection cannot
            // pass merely because the publication destination is unreadable.
            assert!(current_hook_context(worktree).is_ok());
            receipt.record(gwt_agent::session::HookCompletionOutcome::Normal);
            assert_eq!(std::fs::read(path).unwrap(), pending_bytes);
        });
    }

    #[test]
    fn owned_outer_result_failure_discards_successful_callback_without_retry() {
        with_bound_observation(|worktree, session_path, path| {
            use std::cell::Cell;
            let mut receipt = begin_bound_failure_observation(worktree, path);
            let metadata_calls = Cell::new(0);
            let publication_calls = Cell::new(0);
            let result_calls = Cell::new(0);
            let result = handle_with_input_owned_with(
                "PostToolUse",
                r#"{"session_id":"provider-failure-3165"}"#,
                Some(&mut receipt),
                |sessions, id, event, provider, project| {
                    metadata_calls.set(metadata_calls.get() + 1);
                    gwt_agent::session::persist_session_hook_metadata_under_lease(
                        sessions, id, event, provider, project,
                    )
                },
                |path, value, durable| {
                    publication_calls.set(publication_calls.get() + 1);
                    write_hook_runtime_value(path, value, durable)
                },
                |result| {
                    result_calls.set(result_calls.get() + 1);
                    let (publication, metadata_saved) =
                        result.expect("real callback and lease operation must have succeeded");
                    assert!(publication.published);
                    assert!(metadata_saved);
                    // Controlled outer-result uncertainty after the actual
                    // callback. This does not claim a real OS unlock failure.
                    Err(io::Error::other(
                        "controlled post-callback outer result failure",
                    ))
                },
            );
            assert!(result.is_err());
            assert_eq!(metadata_calls.get(), 1);
            assert_eq!(publication_calls.get(), 1);
            assert_eq!(result_calls.get(), 1);
            assert_eq!(
                Session::load(session_path)
                    .unwrap()
                    .last_hook_event
                    .as_deref(),
                Some("PostToolUse")
            );
            let pending = SessionRuntimeState::load(path).unwrap();
            assert!(current_hook_context(worktree).is_ok());
            assert!(pending.current_hook_event.is_some());
            assert!(pending
                .current_hook_event
                .unwrap()
                .completed_outcome
                .is_none());
            assert!(!receipt.can_certify_current);
            assert!(receipt.correlation_id.is_none());
            let runtime_bytes = std::fs::read(path).unwrap();
            let session_bytes = std::fs::read(session_path).unwrap();
            receipt.record(gwt_agent::session::HookCompletionOutcome::Normal);
            assert_eq!(std::fs::read(path).unwrap(), runtime_bytes);
            assert_eq!(std::fs::read(session_path).unwrap(), session_bytes);
        });
    }

    #[test]
    fn owned_unbound_physical_observation_updates_metadata_without_certification() {
        with_legacy_observation(|worktree, session_path, path| {
            let pid = std::process::id();
            let birth = crate::process::host_process_start_time(pid).unwrap();
            let mut runtime = SessionRuntimeState::load(path).unwrap();
            runtime.runtime_incarnation = Some(1);
            runtime.host_started_at = Some(birth);
            runtime.child_pid = Some(pid);
            runtime.child_started_at = Some(birth);
            runtime.save(path).unwrap();
            let input = r#"{"session_id":"provider-regular-unbound"}"#;
            let mut receipt = begin_hook_observation("SessionStart", input, worktree)
                .unwrap()
                .expect("ordinary unbound launches retain owned liveness updates");
            assert!(
                handle_with_input_owned("SessionStart", input, Some(&mut receipt))
                    .unwrap()
                    .published
            );
            assert_eq!(
                Session::load(session_path)
                    .unwrap()
                    .exact_resume_session_id(),
                Some("provider-regular-unbound")
            );
            let mut predecessor = receipt.clone();
            receipt.record(gwt_agent::session::HookCompletionOutcome::Normal);
            let completed = SessionRuntimeState::load(path).unwrap();
            assert!(completed.execution_identity.is_none());
            assert!(completed.current_hook_event.is_none());
            assert_eq!(completed.runtime_incarnation, Some(1));
            assert_eq!(completed.child_pid, Some(pid));
            assert_eq!(
                completed.last_completed_hook_outcome,
                Some(gwt_agent::session::HookCompletionOutcome::Normal)
            );
            let mut replacement = completed;
            replacement.runtime_incarnation = Some(2);
            replacement.save(path).unwrap();
            let mut next = begin_hook_observation("Stop", input, worktree)
                .unwrap()
                .unwrap();
            assert!(
                handle_with_input_owned("Stop", input, Some(&mut next))
                    .unwrap()
                    .published
            );
            let runtime_bytes = std::fs::read(path).unwrap();
            let session_bytes = std::fs::read(session_path).unwrap();
            assert!(
                !handle_with_input_owned(
                    "SessionStart",
                    r#"{"session_id":"stale-unbound-provider"}"#,
                    Some(&mut predecessor),
                )
                .unwrap()
                .published
            );
            predecessor.record(gwt_agent::session::HookCompletionOutcome::Normal);
            assert_eq!(std::fs::read(path).unwrap(), runtime_bytes);
            assert_eq!(std::fs::read(session_path).unwrap(), session_bytes);
        });
    }

    #[test]
    fn early_refusal_observation_preserves_lifetime_and_completes_history_only_once() {
        with_legacy_observation(|worktree, session_path, path| {
            use gwt_agent::session::HookCompletionOutcome::{Blocked, Normal};
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            value["future_field"] = serde_json::json!({"retained": true});
            write_hook_runtime_value(path, &value, false).unwrap();
            let before = std::fs::read(path).unwrap();
            let metadata = std::fs::read(session_path).unwrap();
            assert!(begin_hook_observation("PreToolUse", "{", worktree)
                .unwrap()
                .is_none());
            assert!(
                !handle_with_input_owned("Stop", "{}", None)
                    .unwrap()
                    .published
            );
            capture_hook_completion(worktree).unwrap().record(Normal);
            assert_eq!(std::fs::read(path).unwrap(), before);
            let receipt = begin_hook_observation("PreToolUse", "{}", worktree)
                .unwrap()
                .unwrap();
            let pending: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            let mut unchanged = pending.clone();
            unchanged
                .as_object_mut()
                .unwrap()
                .remove("current_hook_event");
            assert_eq!(unchanged, value);
            assert_eq!(pending["current_hook_event"]["event"], "PreToolUse");
            assert!(pending["current_hook_event"]
                .get("completed_outcome")
                .is_none());
            let replay = receipt.clone();
            receipt.record(Blocked);
            let completed = std::fs::read(path).unwrap();
            let state = SessionRuntimeState::load(path).unwrap();
            assert_eq!(state.status, AgentStatus::Idle);
            assert_eq!(state.last_completed_hook_outcome, Some(Blocked));
            assert!(state.current_hook_event.is_none());
            replay.record(Normal);
            assert_eq!(std::fs::read(path).unwrap(), completed);
            assert_eq!(std::fs::read(session_path).unwrap(), metadata);
        });
    }

    #[test]
    fn stale_same_status_receipt_cannot_mutate_successor_session_or_runtime() {
        with_legacy_observation(|worktree, session_path, path| {
            use gwt_agent::session::HookCompletionOutcome::Normal;
            let mut first = begin_hook_observation("Stop", "{}", worktree)
                .unwrap()
                .unwrap();
            assert!(
                handle_with_input_owned("Stop", r#"{"session_id":"first"}"#, Some(&mut first))
                    .unwrap()
                    .published
            );
            let mut next = begin_hook_observation("Stop", "{}", worktree)
                .unwrap()
                .unwrap();
            assert!(
                handle_with_input_owned("Stop", r#"{"session_id":"next"}"#, Some(&mut next))
                    .unwrap()
                    .published
            );
            let runtime = std::fs::read(path).unwrap();
            let metadata = std::fs::read(session_path).unwrap();
            assert!(
                !handle_with_input_owned("Stop", r#"{"session_id":"stale"}"#, Some(&mut first))
                    .unwrap()
                    .published
            );
            record_completed_stop_owned(Some(&mut first)).unwrap();
            assert!(
                !record_blocked_stop_owned(Some(&mut first))
                    .unwrap()
                    .published
            );
            first.record(Normal);
            assert_eq!(std::fs::read(path).unwrap(), runtime);
            assert_eq!(std::fs::read(session_path).unwrap(), metadata);
            assert_eq!(
                Session::load(session_path)
                    .unwrap()
                    .exact_resume_session_id(),
                Some("next")
            );
            record_completed_stop_owned(Some(&mut next)).unwrap();
            assert!(Session::load(session_path)
                .unwrap()
                .last_completed_stop_at
                .is_some());
            next.record(Normal);
            let completed = SessionRuntimeState::load(path).unwrap();
            assert!(
                completed.current_hook_event.is_none(),
                "legacy never certifies current Normal"
            );
            assert_eq!(completed.last_completed_hook_outcome, Some(Normal));
        });
    }

    #[test]
    fn deferred_legacy_bind_requires_still_missing_and_never_recreates_a_bound_record() {
        with_legacy_observation(|worktree, session_path, path| {
            std::fs::remove_file(path).unwrap();
            let mut first = begin_hook_observation("Stop", "{}", worktree)
                .unwrap()
                .unwrap();
            assert!(!path.exists());
            SessionRuntimeState::new(AgentStatus::Running)
                .save(path)
                .unwrap();
            let runtime = std::fs::read(path).unwrap();
            let metadata = std::fs::read(session_path).unwrap();
            assert!(
                !handle_with_input_owned("Stop", "{}", Some(&mut first))
                    .unwrap()
                    .published
            );
            assert_eq!(std::fs::read(path).unwrap(), runtime);
            assert_eq!(std::fs::read(session_path).unwrap(), metadata);
            std::fs::remove_file(path).unwrap();
            let mut next = begin_hook_observation("Stop", "{}", worktree)
                .unwrap()
                .unwrap();
            assert!(
                handle_with_input_owned("Stop", "{}", Some(&mut next))
                    .unwrap()
                    .published
            );
            assert!(SessionRuntimeState::load(path)
                .unwrap()
                .current_hook_event
                .is_some());
            std::fs::remove_file(path).unwrap();
            let metadata = std::fs::read(session_path).unwrap();
            assert!(
                !handle_with_input_owned("Stop", "{}", Some(&mut next))
                    .unwrap()
                    .published
            );
            record_completed_stop_owned(Some(&mut next)).unwrap();
            next.record(gwt_agent::session::HookCompletionOutcome::Normal);
            assert!(!path.exists());
            assert_eq!(std::fs::read(session_path).unwrap(), metadata);
        });
    }

    #[test]
    fn late_hooks_preserve_explicit_close_until_relaunch() {
        let _lock = env_lock();
        let mut env = EnvGuard::new();
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");
        let mut session = Session::new(dir.path(), "work/closed", AgentId::ClaudeCode);
        session.record_terminal_stop();
        session.save(&sessions_dir).unwrap();
        let path = sessions_dir.join(format!("{}.toml", session.id));
        let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session.id);
        env.set(GWT_SESSION_ID_ENV, &session.id);
        env.set(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV,
            runtime_path.as_os_str(),
        );

        for event in ["PostToolUse", "Stop"] {
            handle_with_input(event, "{}").unwrap();
            let loaded = Session::load(&path).unwrap();
            assert_eq!(loaded.status, AgentStatus::Stopped, "late {event}");
            assert_eq!(loaded.last_hook_event.as_deref(), Some(event));
        }
        record_completed_stop_from_env().unwrap();
        assert_eq!(Session::load(&path).unwrap().status, AgentStatus::Stopped);
        record_blocked_stop_from_env().unwrap();
        let closed = Session::load(&path).unwrap();
        assert_eq!(closed.status, AgentStatus::Stopped);
        assert!(!closed.restore_window_on_startup);

        // Explicit launch enables restore before the new child's hooks run.
        session.restore_window_on_startup = true;
        session.save(&sessions_dir).unwrap();
        record_blocked_stop_from_env().unwrap();
        assert_eq!(Session::load(&path).unwrap().status, AgentStatus::Running);
        handle_with_input("Stop", "{}").unwrap();
        record_completed_stop_from_env().unwrap();
        assert_eq!(Session::load(&path).unwrap().status, AgentStatus::Idle);
    }
}

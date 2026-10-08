//! Free resets: native consent, a bounded provider call, then authoritative hold release.
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use gwt_agent::{LaunchRuntimeTarget, Session};
use gwt_github::SpecOpsError;
use serde_json::json;

use crate::{
    persistence::{PersistedWindowState, WindowState},
    preset::WindowPreset,
    provider_reset::{self, codex::CodexResetClient, native, ResetRequest, CLAUDE_NOTICE},
};

use super::{io_as_api_error, CliEnv};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderResetCommand {
    Proposals { min_reset_wait_secs: u64 },
    Execute { provider: String, window_id: String },
}

pub(super) fn run<E: CliEnv>(
    env: &E,
    command: ProviderResetCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(env.repo_path());
    match command {
        ProviderResetCommand::Proposals {
            min_reset_wait_secs,
        } => {
            let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
            out.push_str(&json!({
                "min_reset_wait_secs": min_reset_wait_secs,
                "proposals": provider_reset::proposals(&prefs.provider_quota_holds, chrono::Utc::now(), min_reset_wait_secs),
                "notice": CLAUDE_NOTICE,
            }).to_string());
            out.push('\n');
            Ok(0)
        }
        ProviderResetCommand::Execute {
            provider,
            window_id,
        } => {
            let request = ResetRequest {
                id: uuid::Uuid::new_v4().to_string(),
                provider,
                window_id,
            };
            let audit_dir = gwt_core::paths::gwt_home().join("provider-resets");
            let result = perform(env, &prefs_path, &audit_dir, &request);
            let code = if result.is_ok() { 0 } else { 1 };
            out.push_str(
                &json!({
                    "request_id": request.id,
                    "provider": request.provider,
                    "window_id": request.window_id,
                    "status": if result.is_ok() { "completed" } else { "failed" },
                    "reason": result.as_ref().err(),
                    "audit_warning": result.as_ref().ok().and_then(|completed| completed.audit_warning.as_ref()),
                    "audit_path": audit_dir.join(format!("{}.jsonl", request.id)),
                })
                .to_string(),
            );
            out.push('\n');
            Ok(code)
        }
    }
}

fn perform<E: CliEnv>(
    env: &E,
    prefs_path: &Path,
    audit_dir: &Path,
    request: &ResetRequest,
) -> Result<provider_reset::ResetCompleted, String> {
    let mut audit = Audit::open(audit_dir, request, env.repo_path())?;
    audit.record(
        "requested",
        "Free reset requested; no approval supplied by caller",
    )?;
    let result = (|| {
        if request.provider != "codex" {
            return Err(format!("unsupported free reset provider. {CLAUDE_NOTICE}"));
        }
        let held_until = held_until(prefs_path, &request.provider)?
            .ok_or("Codex has no provider quota hold to reset")?;
        let target = live_target(env.repo_path(), &request.window_id)?;
        let gwt_home = gwt_core::paths::gwt_home();
        let home = gwt_home
            .parent()
            .ok_or("default Host home is unavailable")?;
        audit.record(
            "authentication",
            &json!({
                "session_id": target.session_id,
                "target_auth_root": target.auth_root,
                "helper_auth_root": target.auth_root.path,
            })
            .to_string(),
        )?;
        let mut client = CodexResetClient::start(
            &target.executable,
            &target.cwd,
            home,
            &target.auth_root.path,
        )?;
        provider_reset::execute(
            request,
            &target.auth_root,
            &mut client,
            native::confirm,
            || {
                if live_target(env.repo_path(), &request.window_id)? != target {
                    return Err("target window/session changed after consent; propose again".into());
                }
                require_same_hold(prefs_path, &request.provider, &held_until)
            },
            || {
                // Never let consent for an old hold erase a newly formed one.
                require_same_hold(prefs_path, &request.provider, &held_until)?;
                let mut detail = String::new();
                let code = super::issue::run_monitor_quota_hold_clear_if_matches(
                    env,
                    Some(env.repo_path()),
                    &request.provider,
                    &format!("verified free Codex reset {}", request.id),
                    &held_until,
                    &mut detail,
                )
                .map_err(|error| error.to_string())?;
                if code == 0 {
                    Ok(())
                } else {
                    Err(detail)
                }
            },
            &mut |phase, detail| audit.record(phase, detail),
        )
    })();
    if let Err(reason) = &result {
        audit.record("operation_failed", reason)?;
    }
    result
}

fn held_until(prefs_path: &Path, provider: &str) -> Result<Option<String>, String> {
    let prefs = crate::load_issue_monitor_prefs(prefs_path).map_err(|error| error.to_string())?;
    Ok(prefs.provider_quota_holds.get(provider).cloned())
}

fn require_same_hold(prefs_path: &Path, provider: &str, expected: &str) -> Result<(), String> {
    match held_until(prefs_path, provider)? {
        Some(current) if current == expected => Ok(()),
        _ => Err("provider quota hold changed after proposal; not releasing it".into()),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Target {
    session_id: String,
    executable: PathBuf,
    cwd: PathBuf,
    auth_root: gwt_agent::CodexAuthRoot,
}

fn live_target(project: &Path, id: &str) -> Result<Target, String> {
    let windows = super::pane::live_windows(project)?;
    let window = windows
        .iter()
        .find(|window| window.id == id)
        .ok_or("target window is not on this project's canvas; use pane.list")?;
    let session_id = window
        .session_id
        .as_deref()
        .ok_or("target window has no Session")?;
    gwt_agent::validate_session_id_path_component(session_id).map_err(|error| error.to_string())?;
    let session =
        Session::load(&gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml")))
            .map_err(|_| "target Session could not be read")?;
    validate_target(window, &session)
}

fn validate_target(window: &PersistedWindowState, session: &Session) -> Result<Target, String> {
    let codex_window = window.preset == WindowPreset::Codex
        || (window.preset == WindowPreset::Agent && window.agent_id.as_deref() == Some("codex"));
    if !codex_window
        || session.agent_id.command() != "codex"
        || window.session_id.as_deref() != Some(session.id.as_str())
        || window.status == WindowState::Starting
    {
        return Err("target is not a stable Codex window/Session".into());
    }
    if session.runtime_target != LaunchRuntimeTarget::Host || session.backend_id.is_some() {
        return Err("free reset requires a Host Codex window using its default provider".into());
    }
    let auth_root = session.codex_auth_root.as_ref().ok_or(
        "target authentication root/source is missing; relaunch this window to record its launch-time authentication proof",
    )?;
    if !auth_root.path.is_absolute()
        || !auth_root.path.is_dir()
        || dunce::canonicalize(&auth_root.path).ok().as_ref() != Some(&auth_root.path)
    {
        return Err(
            "target authentication root no longer matches its launch proof; relaunch this window"
                .into(),
        );
    }
    // Bind the helper to the same installed executable, never resolve a new
    // caller-provided PATH command or execute the session's shell arguments.
    let executable = PathBuf::from(&session.launch_command);
    if !executable.is_absolute()
        || !executable
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("codex")
                    || name.eq_ignore_ascii_case("codex.exe")
                    || (cfg!(windows) && name.eq_ignore_ascii_case("codex.cmd"))
            })
    {
        return Err("free reset requires a directly installed Codex executable; relaunch with the installed version".into());
    }
    Ok(Target {
        session_id: session.id.clone(),
        executable,
        cwd: session.worktree_path.clone(),
        auth_root: auth_root.clone(),
    })
}

struct Audit {
    _lock: File,
    file: File,
    context: serde_json::Value,
}

impl Audit {
    fn open(dir: &Path, request: &ResetRequest, project: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("reset.lock"))
            .map_err(|error| error.to_string())?;
        lock.try_lock_exclusive()
            .map_err(|_| "another free reset is awaiting consent or executing")?;
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(dir.join(format!("{}.jsonl", request.id)))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            _lock: lock,
            file,
            context: json!({"request":request,"project":project}),
        })
    }

    fn record(&mut self, phase: &str, detail: &str) -> Result<(), String> {
        let record = json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "context": self.context,
            "phase": phase,
            "detail": detail,
        });
        writeln!(self.file, "{record}")
            .and_then(|_| self.file.sync_all())
            .map_err(|error| format!("could not persist reset audit: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_fixture() -> (PersistedWindowState, Session) {
        let mut session =
            Session::new(std::env::temp_dir(), "work/test", gwt_agent::AgentId::Codex);
        session.launch_command = std::env::temp_dir()
            .join("codex")
            .to_string_lossy()
            .into_owned();
        let window = serde_json::from_value(json!({
            "id":"tab-1::agent-2", "title":"Codex", "preset":"codex",
            "geometry":{"x":0,"y":0,"width":500,"height":400},
            "z_index":1, "status":"interrupted", "session_id":session.id,
        }))
        .unwrap();
        (window, session)
    }

    #[test]
    fn reset_target_without_launch_authentication_proof_is_refused() {
        let (window, session) = target_fixture();
        assert!(validate_target(&window, &session)
            .unwrap_err()
            .contains("authentication"));
    }

    #[test]
    fn reset_target_allows_a_rate_limited_pane_but_rejects_other_runtimes() {
        let (mut window, mut session) = target_fixture();
        session.codex_auth_root = Some(gwt_agent::CodexAuthRoot {
            path: dunce::canonicalize(std::env::temp_dir()).unwrap(),
            origin: gwt_agent::CodexAuthRootOrigin::Host,
        });
        assert!(validate_target(&window, &session).is_ok());
        session.runtime_target = LaunchRuntimeTarget::Docker;
        assert!(validate_target(&window, &session).is_err());
        session.runtime_target = LaunchRuntimeTarget::Host;
        session.backend_id = Some("custom-backend".into());
        assert!(validate_target(&window, &session).is_err());
        session.backend_id = None;
        window.session_id = Some("different-session".into());
        assert!(validate_target(&window, &session).is_err());
        window.session_id = Some(session.id.clone());
        session.launch_command = "codex".into();
        assert!(validate_target(&window, &session).is_err());
    }

    #[test]
    fn reset_audit_serializes_attempts_and_records_each_phase() {
        let dir = tempfile::tempdir().unwrap();
        let request = ResetRequest {
            id: "attempt-1".into(),
            provider: "codex".into(),
            window_id: "tab-1::agent-2".into(),
        };
        let mut audit = Audit::open(dir.path(), &request, dir.path()).unwrap();
        audit.record("approved", "one native decision").unwrap();
        audit.record("executing", "one free credit").unwrap();
        audit
            .record("failed", "transport failed; hold retained")
            .unwrap();
        assert!(Audit::open(dir.path(), &request, dir.path()).is_err());
        drop(audit);
        // Existing records cannot be overwritten or used as an approval.
        assert!(Audit::open(dir.path(), &request, dir.path()).is_err());
        let rows = std::fs::read_to_string(dir.path().join("attempt-1.jsonl")).unwrap();
        let phases = rows
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["phase"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(phases, ["approved", "executing", "failed"]);
    }
}

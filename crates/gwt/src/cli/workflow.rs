//! `workflow.bypass` JSON operation (Issue #3267, SPEC #1932).
//!
//! Arms or clears the session-scoped owner-guard bypass so sanctioned
//! ownerless flows (the `/release` skill) can run mutating release steps
//! without a linked Issue/SPEC. Self-only by design: the operation targets
//! the calling agent's own session resolved from `GWT_SESSION_ID`, mirroring
//! `pane.send`. The workflow-policy hook honours the arm only while
//! `workflow_bypass_armed_at` is fresh, so a forgotten disarm expires on its
//! own.

use chrono::Utc;
use gwt_agent::{session::GWT_SESSION_ID_ENV, update_session, Session, WorkflowBypass};
use gwt_core::paths::gwt_sessions_dir;
use gwt_github::{ApiError, SpecOpsError};

use super::{CliEnv, WorkflowCommand};

/// Requested bypass state for `workflow.bypass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowBypassMode {
    Release,
    Chore,
    Off,
}

impl WorkflowBypassMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "release" => Some(Self::Release),
            "chore" => Some(Self::Chore),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

pub(super) fn run<E: CliEnv>(
    _env: &mut E,
    command: WorkflowCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        WorkflowCommand::Bypass { mode } => {
            let session_id = current_session_id()?;
            let session = apply_bypass(&gwt_sessions_dir(), &session_id, mode)?;
            out.push_str(&render_result(&session));
            Ok(0)
        }
    }
}

fn current_session_id() -> Result<String, SpecOpsError> {
    std::env::var(GWT_SESSION_ID_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            config_error(format!(
                "{GWT_SESSION_ID_ENV} is not set; workflow.bypass arms only the calling agent's own session"
            ))
        })
}

fn apply_bypass(
    sessions_dir: &std::path::Path,
    session_id: &str,
    mode: WorkflowBypassMode,
) -> Result<Session, SpecOpsError> {
    update_session(sessions_dir, session_id, |session| {
        match mode {
            WorkflowBypassMode::Release => {
                session.workflow_bypass = Some(WorkflowBypass::Release);
                session.workflow_bypass_armed_at = Some(Utc::now());
            }
            WorkflowBypassMode::Chore => {
                session.workflow_bypass = Some(WorkflowBypass::Chore);
                session.workflow_bypass_armed_at = Some(Utc::now());
            }
            WorkflowBypassMode::Off => {
                session.workflow_bypass = None;
                session.workflow_bypass_armed_at = None;
            }
        }
        session.updated_at = Utc::now();
        Ok(())
    })
    .map_err(|err| config_error(format!("failed to update session {session_id}: {err}")))
}

fn render_result(session: &Session) -> String {
    match session.workflow_bypass {
        Some(WorkflowBypass::Release) => format!(
            "workflow bypass armed: release (session {}, auto-expires in 6h)\n",
            session.id
        ),
        Some(WorkflowBypass::Chore) => format!(
            "workflow bypass armed: chore (session {}, auto-expires in 6h)\n",
            session.id
        ),
        None => format!("workflow bypass cleared (session {})\n", session.id),
    }
}

fn config_error(message: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_agent::AgentId;

    fn new_session(dir: &std::path::Path) -> Session {
        let session = Session::new(std::path::Path::new("/tmp/repo"), "develop", AgentId::Codex);
        session.save(dir).expect("save session");
        session
    }

    #[test]
    fn parse_accepts_known_modes_only() {
        assert_eq!(
            WorkflowBypassMode::parse("release"),
            Some(WorkflowBypassMode::Release)
        );
        assert_eq!(
            WorkflowBypassMode::parse("chore"),
            Some(WorkflowBypassMode::Chore)
        );
        assert_eq!(
            WorkflowBypassMode::parse("off"),
            Some(WorkflowBypassMode::Off)
        );
        assert_eq!(WorkflowBypassMode::parse("Release"), None);
        assert_eq!(WorkflowBypassMode::parse(""), None);
    }

    #[test]
    fn apply_bypass_round_trips_arm_and_clear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = new_session(dir.path());

        let armed = apply_bypass(dir.path(), &session.id, WorkflowBypassMode::Release)
            .expect("arm release");
        assert_eq!(armed.workflow_bypass, Some(WorkflowBypass::Release));
        assert!(armed.workflow_bypass_armed_at.is_some());

        let reloaded = Session::load(&dir.path().join(format!("{}.toml", session.id)))
            .expect("reload session");
        assert_eq!(reloaded.workflow_bypass, Some(WorkflowBypass::Release));
        assert!(reloaded.workflow_bypass_armed_at.is_some());

        let cleared =
            apply_bypass(dir.path(), &session.id, WorkflowBypassMode::Off).expect("clear bypass");
        assert_eq!(cleared.workflow_bypass, None);
        assert_eq!(cleared.workflow_bypass_armed_at, None);
    }

    #[test]
    fn apply_bypass_fails_for_missing_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = apply_bypass(dir.path(), "no-such-session", WorkflowBypassMode::Release)
            .expect_err("missing session must fail");
        assert!(err.to_string().contains("no-such-session"));
    }

    #[test]
    fn render_result_names_the_armed_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut session = new_session(dir.path());
        session.workflow_bypass = Some(WorkflowBypass::Release);
        assert!(render_result(&session).contains("armed: release"));
        session.workflow_bypass = Some(WorkflowBypass::Chore);
        assert!(render_result(&session).contains("armed: chore"));
        session.workflow_bypass = None;
        assert!(render_result(&session).contains("cleared"));
    }
}

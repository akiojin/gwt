//! `build.*` JSON lifecycle operations.
//!
//! Exit CLI for the `gwt-build-spec` skill (SPEC-1935 FR-014r). Writes
//! `.gwt/skill-state/build-spec.json` via [`gwt_core::skill_state`].

use gwt_github::SpecOpsError;

use super::skill_state_runtime;
use crate::cli::{CliEnv, SkillStateAction};

pub const SKILL_NAME: &str = "build-spec";
pub const SKILL_DISPLAY: &str = "gwt-build-spec";
pub const VERB: &str = "build";

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if let Err(error) = record_current_work_terminal_before_finalize(env, &action) {
        out.push_str(&format!("{VERB}: Work lifecycle update failed: {error}\n"));
        return Ok(1);
    }
    // SPEC-3248 P8a: a successful build completion also settles the launch's
    // Execution Control Record (best-effort — the build-spec skill flow must
    // not require a second explicit `execution.complete`). Guarded strictly:
    // the settlement fires only when this `build.complete` actually finalized
    // an ACTIVE build state for the same spec — a vacuous "nothing to
    // finalize" exit 0 must not settle the execution — and only when the
    // record names the same owner. Aborting a build never settles.
    let completed_spec = match &action {
        SkillStateAction::Complete { spec } => {
            let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
            let had_active_matching_state = gwt_core::skill_state::load(&worktree, SKILL_NAME)
                .ok()
                .flatten()
                .is_some_and(|state| {
                    state.active && (state.owner_spec.is_none() || state.owner_spec == Some(*spec))
                });
            had_active_matching_state.then_some(*spec)
        }
        _ => None,
    };
    let code = skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)?;
    if code == 0 {
        if let Some(spec) = completed_spec {
            if let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            {
                let worktree = gwt_core::paths::resolve_current_worktree_root(env.repo_path());
                crate::cli::execution_state::settle_completed_best_effort(
                    &worktree,
                    &session_id,
                    spec,
                );
            }
        }
    }
    Ok(code)
}

fn record_current_work_terminal_before_finalize<E: CliEnv>(
    env: &E,
    action: &SkillStateAction,
) -> Result<(), String> {
    let (spec, close_kind) = match action {
        SkillStateAction::Complete { spec } => (*spec, WorkTerminalKind::Done),
        SkillStateAction::Abort { spec, .. } => (*spec, WorkTerminalKind::Discarded),
        SkillStateAction::Start { .. } | SkillStateAction::Phase { .. } => return Ok(()),
    };
    let repo = env.repo_path();
    let state = gwt_core::skill_state::load(repo, SKILL_NAME).map_err(|error| error.to_string())?;
    let Some(state) = state else {
        return Ok(());
    };
    if state.owner_spec.is_some() && state.owner_spec != Some(spec) {
        return Ok(());
    }

    let session_id = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .unwrap_or_default()
        .trim()
        .to_string();
    if session_id.is_empty() {
        return Ok(());
    }
    if !state.active || state.session_id.trim() != session_id {
        return Ok(());
    }
    let (project_state_root, work_event_root) =
        crate::agent_project_state::agent_session_roots_or_fallback(repo, &session_id)
            .map_err(|error| error.to_string())?;
    let legacy_work_id = format!("work-session-{session_id}");

    let now = chrono::Utc::now();
    let outcome = match close_kind {
        WorkTerminalKind::Done => {
            gwt_core::workspace_projection::emit_workspace_done_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
        WorkTerminalKind::Discarded => {
            gwt_core::workspace_projection::emit_workspace_discard_event_for_session_outcome(
                &project_state_root,
                &work_event_root,
                &session_id,
                &legacy_work_id,
                now,
            )
        }
    }
    .map_err(|error| error.to_string())?;
    match outcome {
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::Emitted
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AlreadyMatching
        | gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::NoTarget => Ok(()),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AssignedWorkMissing(
            work_id,
        ) => Err(format!(
            "assigned Work {work_id} is not materialized; retry workspace.ensure before finalizing the build"
        )),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::WrongTerminal => Err(
            format!(
                "assigned Work has the wrong terminal state for {}",
                close_kind.as_str()
            ),
        ),
        gwt_core::workspace_projection::WorkspaceTerminalEventOutcome::AmbiguousTerminal => Err(
            "assigned Work has ambiguous Done and Discarded terminal state".to_string(),
        ),
    }
}

#[derive(Debug, Clone, Copy)]
enum WorkTerminalKind {
    Done,
    Discarded,
}

impl WorkTerminalKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "Done",
            Self::Discarded => "Discarded",
        }
    }
}

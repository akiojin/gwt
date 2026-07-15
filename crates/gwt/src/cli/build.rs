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
    skill_state_runtime::run(env, action, SKILL_NAME, SKILL_DISPLAY, VERB, out)
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
    let work_id = format!("work-session-{session_id}");
    let Some(projection) = gwt_core::workspace_projection::load_workspace_work_items(repo)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let Some(work) = projection.work_items.iter().find(|work| work.id == work_id) else {
        return Ok(());
    };
    if work.is_terminal() {
        return Ok(());
    }

    let now = chrono::Utc::now();
    match close_kind {
        WorkTerminalKind::Done => {
            gwt_core::workspace_projection::emit_workspace_done_event_if_absent(repo, &work_id, now)
        }
        WorkTerminalKind::Discarded => {
            gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
                repo, &work_id, now,
            )
        }
    }
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Copy)]
enum WorkTerminalKind {
    Done,
    Discarded,
}

//! Shared `start|phase|complete|abort` implementation for the
//! `gwtd plan ...` and `gwtd build ...` exit CLIs (SPEC-1935 FR-014q/r).
//!
//! Both CLIs persist a small JSON state file under
//! `<worktree>/.gwt/skill-state/<skill>.json` via
//! [`gwt_core::skill_state`]. The Stop-block handlers
//! (`skill-plan-spec-stop-check` / `skill-build-spec-stop-check`) read
//! that file to decide whether to continue the skill's turn.
//!
//! `start` is idempotent: calling it twice simply refreshes the
//! `started_at` and `session_id` fields. `complete` and `abort` flip
//! `active: false` without deleting the file so later inspection (and
//! Codex smoke tests) can see the skill's final state.

use chrono::Utc;
use gwt_agent::GWT_SESSION_ID_ENV;
use gwt_core::skill_state::{self, SkillState};
use gwt_github::SpecOpsError;

use super::{CliEnv, SkillStateAction};

fn current_session_id() -> String {
    std::env::var(GWT_SESSION_ID_ENV).unwrap_or_default()
}

pub fn run<E: CliEnv>(
    env: &mut E,
    action: SkillStateAction,
    skill_name: &str,
    skill_display: &str,
    verb: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let worktree = env.repo_path().to_path_buf();
    match action {
        SkillStateAction::Start { spec } => {
            let state = SkillState {
                active: true,
                owner_spec: Some(spec),
                started_at: Utc::now(),
                phase: None,
                session_id: current_session_id(),
            };
            match skill_state::save(&worktree, skill_name, &state) {
                Ok(()) => {
                    out.push_str(&format!(
                        "{verb}: started {skill_display} for SPEC-{spec}\n"
                    ));
                    Ok(0)
                }
                Err(err) => {
                    out.push_str(&format!("{verb}: start failed: {err}\n"));
                    Ok(1)
                }
            }
        }
        SkillStateAction::Phase { spec, label } => {
            let current = match skill_state::load(&worktree, skill_name) {
                Ok(Some(state)) => state,
                Ok(None) => {
                    out.push_str(&format!(
                        "{verb}: no active {skill_display} state to update phase\n"
                    ));
                    return Ok(0);
                }
                Err(err) => {
                    out.push_str(&format!("{verb}: load failed: {err}\n"));
                    return Ok(1);
                }
            };
            if current.owner_spec.is_some() && current.owner_spec != Some(spec) {
                out.push_str(&format!(
                    "{verb}: phase refused: state owns SPEC-{owner:?}, got --spec {spec}\n",
                    owner = current.owner_spec
                ));
                return Ok(2);
            }
            let next = SkillState {
                phase: Some(label.clone()),
                ..current
            };
            match skill_state::save(&worktree, skill_name, &next) {
                Ok(()) => {
                    out.push_str(&format!("{verb}: {skill_display} phase -> {label}\n"));
                    Ok(0)
                }
                Err(err) => {
                    out.push_str(&format!("{verb}: phase failed: {err}\n"));
                    Ok(1)
                }
            }
        }
        SkillStateAction::Complete { spec } => {
            finalize(&worktree, skill_name, skill_display, verb, spec, None, out)
        }
        SkillStateAction::Abort { spec, reason } => finalize(
            &worktree,
            skill_name,
            skill_display,
            verb,
            spec,
            reason,
            out,
        ),
    }
}

fn finalize(
    worktree: &std::path::Path,
    skill_name: &str,
    skill_display: &str,
    verb: &str,
    spec: u64,
    reason: Option<String>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let current = match skill_state::load(worktree, skill_name) {
        Ok(Some(state)) => state,
        Ok(None) => {
            out.push_str(&format!(
                "{verb}: no active {skill_display} state; nothing to finalize\n"
            ));
            return Ok(0);
        }
        Err(err) => {
            out.push_str(&format!("{verb}: load failed: {err}\n"));
            return Ok(1);
        }
    };
    if current.owner_spec.is_some() && current.owner_spec != Some(spec) {
        out.push_str(&format!(
            "{verb}: finalize refused: state owns SPEC-{owner:?}, got --spec {spec}\n",
            owner = current.owner_spec
        ));
        return Ok(2);
    }
    let next = SkillState {
        active: false,
        phase: reason
            .as_ref()
            .map(|r| format!("aborted: {r}"))
            .or(current.phase.clone()),
        ..current
    };
    match skill_state::save(worktree, skill_name, &next) {
        Ok(()) => {
            match reason {
                Some(reason) => out.push_str(&format!(
                    "{verb}: aborted {skill_display} for SPEC-{spec}: {reason}\n"
                )),
                None => out.push_str(&format!(
                    "{verb}: completed {skill_display} for SPEC-{spec}\n"
                )),
            }
            Ok(0)
        }
        Err(err) => {
            out.push_str(&format!("{verb}: finalize failed: {err}\n"));
            Ok(1)
        }
    }
}

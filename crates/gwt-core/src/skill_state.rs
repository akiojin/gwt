//! Per-skill Stop-block state file I/O used by SPEC-1935 Phase 10.
//!
//! Multi-turn skills (`gwt-plan-spec`, `gwt-build-spec`, and any future
//! skill that needs autonomous Stop-block continuation) record a small
//! JSON state file under `<worktree>/.gwt/skill-state/<skill>.json`. The
//! Stop hook handler reads this file to decide whether to emit
//! `HookOutput::StopBlock` or stay silent.
//!
//! State transitions:
//!
//! - `save` writes the current struct; use it for `start`, `phase`
//!   updates, and transitions that keep the skill active.
//! - `mark_inactive` flips an existing file to `active: false` without
//!   deleting it — useful for `complete` / `abort` so later inspection
//!   can see the skill's last state.
//! - `load` returns `Ok(None)` when the file is missing; callers treat
//!   that as "skill not active". Malformed JSON or other I/O failures
//!   propagate via `io::Error` so handlers can decide how to fail open.
//!
//! `gwt-discussion` intentionally does **not** use this module; its
//! existing `.gwt/discussion.md` Markdown artifact is the source of
//! truth for the `skill-discussion-stop-check` handler.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Relative directory under the worktree where per-skill state files live.
pub const SKILL_STATE_DIR: &str = ".gwt/skill-state";

/// Persisted per-skill state file.
///
/// `session_id` is the gwt agent session identifier at the moment the
/// skill was started. Handlers use it to skip the Stop-block decision
/// when a different agent session observes the file (see FR-014t).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillState {
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_spec: Option<u64>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub session_id: String,
}

/// Resolve the filesystem path for a given skill's state file.
pub fn state_path(worktree: &Path, skill: &str) -> PathBuf {
    worktree.join(SKILL_STATE_DIR).join(format!("{skill}.json"))
}

/// Load the per-skill state. Returns `Ok(None)` when the file is
/// missing. Propagates I/O or JSON errors so the caller can decide
/// whether to treat them as fail-open.
pub fn load(worktree: &Path, skill: &str) -> io::Result<Option<SkillState>> {
    let path = state_path(worktree, skill);
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let state = serde_json::from_str::<SkillState>(&contents)
                .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
            Ok(Some(state))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Persist the per-skill state, creating the directory if needed.
pub fn save(worktree: &Path, skill: &str, state: &SkillState) -> io::Result<()> {
    let path = state_path(worktree, skill);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_string_pretty(state)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))?;
    fs::write(path, serialized)
}

/// Flip an existing skill-state file to `active: false`. Returns
/// `Ok(false)` when no state file exists (idempotent exit).
pub fn mark_inactive(worktree: &Path, skill: &str) -> io::Result<bool> {
    let Some(mut state) = load(worktree, skill)? else {
        return Ok(false);
    };
    if !state.active {
        return Ok(true);
    }
    state.active = false;
    save(worktree, skill, &state)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_state(session: &str) -> SkillState {
        SkillState {
            active: true,
            owner_spec: Some(1935),
            started_at: Utc.with_ymd_and_hms(2026, 4, 21, 9, 0, 0).unwrap(),
            phase: Some("plan-draft".to_string()),
            session_id: session.to_string(),
        }
    }

    #[test]
    fn load_returns_none_when_file_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path(), "plan-spec").unwrap(), None);
    }

    #[test]
    fn save_then_load_round_trips_state() {
        let dir = tempfile::tempdir().unwrap();
        let state = sample_state("sess-1");
        save(dir.path(), "plan-spec", &state).unwrap();
        let loaded = load(dir.path(), "plan-spec").unwrap();
        assert_eq!(loaded, Some(state));
    }

    #[test]
    fn save_creates_nested_skill_state_dir() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), "build-spec", &sample_state("sess-1")).unwrap();
        assert!(dir.path().join(SKILL_STATE_DIR).exists());
        assert!(dir
            .path()
            .join(SKILL_STATE_DIR)
            .join("build-spec.json")
            .exists());
    }

    #[test]
    fn load_returns_invalid_data_for_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path(), "plan-spec");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not json").unwrap();
        let err = load(dir.path(), "plan-spec").expect_err("expected InvalidData");
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn mark_inactive_flips_active_flag_and_preserves_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        let state = sample_state("sess-1");
        save(dir.path(), "plan-spec", &state).unwrap();

        let changed = mark_inactive(dir.path(), "plan-spec").unwrap();
        assert!(changed);

        let loaded = load(dir.path(), "plan-spec").unwrap().unwrap();
        assert!(!loaded.active);
        assert_eq!(loaded.owner_spec, Some(1935));
        assert_eq!(loaded.phase, Some("plan-draft".to_string()));
        assert_eq!(loaded.session_id, "sess-1");
    }

    #[test]
    fn mark_inactive_is_idempotent_without_state_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!mark_inactive(dir.path(), "plan-spec").unwrap());
    }

    #[test]
    fn mark_inactive_on_already_inactive_state_is_noop_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = sample_state("sess-1");
        state.active = false;
        save(dir.path(), "plan-spec", &state).unwrap();
        assert!(mark_inactive(dir.path(), "plan-spec").unwrap());
        let loaded = load(dir.path(), "plan-spec").unwrap().unwrap();
        assert!(!loaded.active);
    }

    #[test]
    fn state_path_is_scoped_per_skill() {
        let dir = tempfile::tempdir().unwrap();
        let a = state_path(dir.path(), "plan-spec");
        let b = state_path(dir.path(), "build-spec");
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("plan-spec.json"));
        assert!(b.to_string_lossy().contains("build-spec.json"));
    }
}

//! Worktree-root detection shared by `block-cd-command` and
//! `block-file-ops`. Shells out to `git rev-parse --show-toplevel`
//! exactly like the Node helpers did.

use std::path::PathBuf;

/// Return the worktree root as reported by `git rev-parse
/// --show-toplevel`. Falls back to the current process cwd if git is
/// unavailable or the command fails.
pub fn detect_worktree_root() -> PathBuf {
    let cwd = fallback_cwd();
    if let Some(worktree) = crate::pm_registry::pm_worktree_for_runtime_dir(&cwd) {
        return worktree;
    }
    let output = gwt_core::process::hidden_command("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout);
            let trimmed = s.trim();
            if trimmed.is_empty() {
                fallback_cwd()
            } else {
                PathBuf::from(trimmed)
            }
        }
        _ => fallback_cwd(),
    }
}

/// Navigation boundary for `block-cd-command`.
///
/// Matches [`detect_worktree_root`] except in a resident PM session, where
/// `cd` may reach the whole `pm/` directory so the PM can read its runtime
/// skill references (Issue #6989). Mutating policies keep using the worktree.
pub fn detect_cd_boundary() -> PathBuf {
    registered_pm_cd_boundary()
        .or_else(|| crate::pm_registry::pm_session_dir_for_cwd(&fallback_cwd()))
        .unwrap_or_else(detect_worktree_root)
}

/// The durable PM identity keeps navigation stable after a cwd change. A
/// missing, stale, or ordinary Session never widens the navigation boundary.
fn registered_pm_cd_boundary() -> Option<PathBuf> {
    let id = super::GwtSessionId::from_env()?;
    let session = gwt_agent::Session::load(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", id.as_str())),
    )
    .ok()?;
    if session.id != id.as_str()
        || !super::is_resident_pm_session(&session.worktree_path, Some(id.as_str()))
    {
        return None;
    }
    crate::pm_registry::pm_session_dir_for_cwd(&session.worktree_path)
}

fn fallback_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use gwt_core::test_support::{env_lock, ScopedEnvVar, ScopedGwtHome};

    use super::*;

    #[test]
    fn registered_pm_cd_boundary_survives_an_unrelated_cwd() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&fallback_cwd());
        std::fs::create_dir_all(&worktree).unwrap();
        let session = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let prefs = worktree
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("project-state/pm.json");
        crate::pm_registry::try_register_pm(
            &prefs,
            crate::pm_registry::PmRegistration {
                session_id: session.id.clone(),
                agent_id: "codex".into(),
                worktree_path: worktree.to_string_lossy().into_owned(),
                created_at: None,
                consecutive_crashes: 0,
                next_not_before: None,
            },
            |_| false,
        )
        .unwrap();
        let _session_id = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        assert_eq!(detect_cd_boundary(), worktree.parent().unwrap());

        let ordinary = gwt_agent::Session::new(&worktree, "", gwt_agent::AgentId::Codex);
        ordinary.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let _ordinary_id = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &ordinary.id);
        assert_eq!(detect_cd_boundary(), detect_worktree_root());
        let _missing_id = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "missing-session");
        assert_eq!(detect_cd_boundary(), detect_worktree_root());
    }
}

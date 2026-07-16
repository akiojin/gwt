use std::path::{Path, PathBuf};

use chrono::Utc;
use gwt_agent::Session;
use gwt_core::{
    error::Result,
    paths::normalize_windows_child_process_path,
    workspace_projection::{
        load_workspace_projection, mutate_existing_workspace_projection, WorkspaceAgentSummary,
    },
};

pub(crate) fn project_state_root_for_agent_session_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> PathBuf {
    load_session(session_id)
        .map(|session| canonical_project_state_root_for_session(&session, fallback_repo_path))
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

pub(crate) fn work_event_root_for_agent_session_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> PathBuf {
    load_session(session_id)
        .map(|session| normalize_project_state_root(&session.worktree_path))
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

pub(crate) fn agent_session_roots_or_fallback(
    fallback_repo_path: &Path,
    session_id: &str,
) -> std::io::Result<(PathBuf, PathBuf)> {
    let Some(session) = try_load_session(session_id)? else {
        let fallback = normalize_project_state_root(fallback_repo_path);
        return Ok((fallback.clone(), fallback));
    };
    Ok((
        canonical_project_state_root_for_session(&session, fallback_repo_path),
        normalize_project_state_root(&session.worktree_path),
    ))
}

pub(crate) fn canonical_project_state_root_for_session(
    session: &Session,
    fallback_repo_path: &Path,
) -> PathBuf {
    if let Some(root) = session
        .project_state_root
        .as_deref()
        .filter(|root| !root.as_os_str().is_empty())
    {
        return normalize_project_state_root(root);
    }

    derive_legacy_project_state_root(&session.worktree_path)
        .unwrap_or_else(|| normalize_project_state_root(fallback_repo_path))
}

pub(crate) fn repair_split_agent_state_if_needed(
    canonical_root: &Path,
    split_root: &Path,
    session_id: &str,
) -> Result<bool> {
    let canonical_root = normalize_project_state_root(canonical_root);
    let split_root = normalize_project_state_root(split_root);
    if canonical_root == split_root {
        return Ok(false);
    }

    let Some(split_projection) = load_workspace_projection(&split_root)? else {
        return Ok(false);
    };
    let Some(split_agent) = split_projection
        .latest_agent_for_session(session_id)
        .cloned()
    else {
        return Ok(false);
    };

    mutate_existing_workspace_projection(&canonical_root, |canonical_projection| {
        let Some(canonical_agent) = canonical_projection.latest_agent_for_session_mut(session_id)
        else {
            return Ok(false);
        };
        let changed = repair_agent_from_split(canonical_agent, &split_agent);
        if changed {
            let now = Utc::now();
            canonical_agent.updated_at = now;
            canonical_projection.updated_at = now;
        }
        Ok(changed)
    })
    .map(Option::unwrap_or_default)
}

fn load_session(session_id: &str) -> Option<Session> {
    match try_load_session(session_id) {
        Ok(session) => session,
        Err(error) => {
            let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
            tracing::debug!(
                error = %error,
                session_id,
                path = %path.display(),
                "failed to load agent session for Project State root resolution"
            );
            None
        }
    }
}

fn try_load_session(session_id: &str) -> std::io::Result<Option<Session>> {
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
    if !path.try_exists()? {
        return Ok(None);
    }
    Session::load(&path).map(Some)
}

fn derive_legacy_project_state_root(worktree_path: &Path) -> Option<PathBuf> {
    let worktree_path = normalize_project_state_root(worktree_path);
    let main_root = gwt_git::worktree::main_worktree_root(&worktree_path).ok()?;
    let main_root = normalize_project_state_root(&main_root);

    if is_bare_child_common_dir(&main_root) {
        if let Some(parent) = main_root.parent() {
            let parent = normalize_project_state_root(parent);
            if worktree_path.starts_with(&parent) {
                return Some(parent);
            }
        }
    }

    Some(main_root)
}

fn is_bare_child_common_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name != ".git" && name.ends_with(".git"))
}

fn normalize_project_state_root(path: &Path) -> PathBuf {
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_windows_child_process_path(&path)
}

fn repair_agent_from_split(
    canonical: &mut WorkspaceAgentSummary,
    split: &WorkspaceAgentSummary,
) -> bool {
    let mut changed = false;
    let split_is_newer = split.updated_at > canonical.updated_at;
    changed |= fill_option_text_if_missing_or_newer(
        &mut canonical.title_summary,
        split.title_summary.as_deref(),
        split_is_newer,
    );
    changed |= fill_option_text_if_missing_or_newer(
        &mut canonical.current_focus,
        split.current_focus.as_deref(),
        split_is_newer,
    );
    changed |= fill_option_path(&mut canonical.worktree_path, split.worktree_path.as_deref());
    changed |= fill_option_text(&mut canonical.window_id, split.window_id.as_deref());
    changed |= fill_option_text(&mut canonical.branch, split.branch.as_deref());

    if canonical.agent_id.trim().is_empty() && !split.agent_id.trim().is_empty() {
        canonical.agent_id = split.agent_id.clone();
        changed = true;
    }
    if canonical.display_name.trim().is_empty() && !split.display_name.trim().is_empty() {
        canonical.display_name = split.display_name.clone();
        changed = true;
    }
    changed
}

fn fill_option_text_if_missing_or_newer(
    target: &mut Option<String>,
    source: Option<&str>,
    source_is_newer: bool,
) -> bool {
    let Some(source) = source.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let target_has_value = target
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if target_has_value && !source_is_newer {
        return false;
    }
    if target.as_deref().map(str::trim) == Some(source) {
        return false;
    }
    *target = Some(source.to_string());
    true
}

fn fill_option_text(target: &mut Option<String>, source: Option<&str>) -> bool {
    if target
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let Some(source) = source.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    *target = Some(source.to_string());
    true
}

fn fill_option_path(target: &mut Option<PathBuf>, source: Option<&Path>) -> bool {
    if target
        .as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty())
    {
        return false;
    }
    let Some(source) = source.filter(|path| !path.as_os_str().is_empty()) else {
        return false;
    };
    *target = Some(source.to_path_buf());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_summary(
        session_id: &str,
        title_summary: Option<&str>,
        current_focus: Option<&str>,
        updated_at: chrono::DateTime<Utc>,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: Some("project::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: current_focus.map(str::to_string),
            title_summary: title_summary.map(str::to_string),
            worktree_path: Some(PathBuf::from("/tmp/worktree")),
            branch: Some("work/title".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at,
        }
    }

    fn run_git(args: &[&str], cwd: &Path) {
        let output = gwt_core::process::run_git_logged(args, Some(cwd)).expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn legacy_bare_child_worktree_derives_workspace_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace-home");
        let bare_repo = workspace_home.join("gwt.git");
        std::fs::create_dir_all(&workspace_home).expect("workspace home");
        run_git(
            &["init", "--bare", bare_repo.to_str().unwrap()],
            temp.path(),
        );

        let bootstrap = temp.path().join("bootstrap");
        run_git(
            &[
                "clone",
                bare_repo.to_str().unwrap(),
                bootstrap.to_str().unwrap(),
            ],
            temp.path(),
        );
        run_git(&["config", "user.email", "test@example.com"], &bootstrap);
        run_git(&["config", "user.name", "Test User"], &bootstrap);
        run_git(&["checkout", "-b", "develop"], &bootstrap);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &bootstrap);
        run_git(&["push", "origin", "develop"], &bootstrap);

        let worktree = workspace_home.join("work").join("20260601-0934");
        std::fs::create_dir_all(worktree.parent().expect("worktree parent"))
            .expect("worktree parent");
        run_git(
            &["worktree", "add", worktree.to_str().unwrap(), "develop"],
            &bare_repo,
        );

        let session = Session::new(&worktree, "work/20260601-0934", gwt_agent::AgentId::Codex);
        assert_eq!(
            canonical_project_state_root_for_session(&session, &worktree),
            dunce::canonicalize(&workspace_home).expect("canonical workspace home")
        );
    }

    #[test]
    fn repair_agent_from_split_prefers_newer_title_and_focus() {
        let older = Utc::now();
        let newer = older + chrono::Duration::seconds(1);
        let mut canonical = agent_summary(
            "session-1",
            Some("Old canonical title"),
            Some("Old canonical focus"),
            older,
        );
        let split = agent_summary(
            "session-1",
            Some("New split title"),
            Some("New split focus"),
            newer,
        );

        assert!(repair_agent_from_split(&mut canonical, &split));
        assert_eq!(canonical.title_summary.as_deref(), Some("New split title"));
        assert_eq!(canonical.current_focus.as_deref(), Some("New split focus"));
    }

    #[test]
    fn repair_agent_from_split_keeps_newer_canonical_title_and_focus() {
        let older = Utc::now();
        let newer = older + chrono::Duration::seconds(1);
        let mut canonical = agent_summary(
            "session-1",
            Some("New canonical title"),
            Some("New canonical focus"),
            newer,
        );
        let split = agent_summary(
            "session-1",
            Some("Old split title"),
            Some("Old split focus"),
            older,
        );

        assert!(!repair_agent_from_split(&mut canonical, &split));
        assert_eq!(
            canonical.title_summary.as_deref(),
            Some("New canonical title")
        );
        assert_eq!(
            canonical.current_focus.as_deref(),
            Some("New canonical focus")
        );
    }

    #[test]
    fn split_repair_updates_only_latest_duplicate_session_rows() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let canonical_root = temp.path().join("canonical");
        let split_root = temp.path().join("split");
        std::fs::create_dir_all(&canonical_root).expect("create canonical root");
        std::fs::create_dir_all(&split_root).expect("create split root");
        let base = Utc::now();

        let canonical_stale = agent_summary(
            "duplicate-session",
            Some("Stale canonical title"),
            Some("Stale canonical focus"),
            base,
        );
        let canonical_current = agent_summary(
            "duplicate-session",
            Some("Current canonical title"),
            Some("Current canonical focus"),
            base + chrono::Duration::seconds(1),
        );
        let split_stale = agent_summary(
            "duplicate-session",
            Some("Stale split title"),
            Some("Stale split focus"),
            base + chrono::Duration::seconds(2),
        );
        let split_current = agent_summary(
            "duplicate-session",
            Some("Latest split title"),
            Some("Latest split focus"),
            base + chrono::Duration::seconds(3),
        );

        let mut canonical_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical_projection.agents = vec![canonical_stale.clone(), canonical_current];
        gwt_core::workspace_projection::save_workspace_projection(
            &canonical_root,
            &canonical_projection,
        )
        .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split_stale, split_current];
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        let saved_canonical =
            gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
                .expect("load canonical precondition")
                .expect("canonical precondition exists");
        assert_eq!(
            saved_canonical
                .latest_agent_for_session("duplicate-session")
                .and_then(|agent| agent.title_summary.as_deref()),
            Some("Current canonical title")
        );
        let saved_split = gwt_core::workspace_projection::load_workspace_projection(&split_root)
            .expect("load split precondition")
            .expect("split precondition exists");
        assert_eq!(
            saved_split
                .latest_agent_for_session("duplicate-session")
                .and_then(|agent| agent.title_summary.as_deref()),
            Some("Latest split title")
        );

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load canonical projection")
            .expect("canonical projection exists");
        assert_eq!(repaired.agents[0], canonical_stale);
        assert_eq!(
            repaired.agents[1].title_summary.as_deref(),
            Some("Latest split title")
        );
        assert_eq!(
            repaired.agents[1].current_focus.as_deref(),
            Some("Latest split focus")
        );
    }
}

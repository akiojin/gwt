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

/// A detected conflict between the worktree an agent session records and the
/// worktree the current process actually runs in (Issue #3278).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeIdentityConflict {
    /// The `GWT_SESSION_ID` whose recorded worktree disagrees with the cwd.
    pub session_id: String,
    /// The worktree recorded in the session metadata.
    pub expected_worktree: PathBuf,
    /// The worktree the process is actually running in.
    pub actual_worktree: PathBuf,
}

/// Detect when the agent session named by `session_id` records a `worktree_path`
/// that disagrees with the worktree the current process runs in (`cwd`).
///
/// A `workspace.update` writes the git-tracked `events.jsonl` — and derives the
/// event's Work identity — from the session metadata. When `GWT_SESSION_ID`
/// leaks in from another worktree, that write lands in, and is attributed to, a
/// foreign Work. Callers use a returned conflict to refuse the write.
///
/// Returns `None` (no conflict, proceed) when:
/// - the session cannot be loaded — the write path already falls back to `cwd`,
///   so there is nothing to cross-check;
/// - the session runs in a non-Host runtime — a container / devcontainer cwd is
///   legitimately a different spelling than the host `worktree_path`;
/// - the normalized session worktree equals the normalized current worktree.
pub(crate) fn agent_session_worktree_conflict(
    cwd: &Path,
    session_id: &str,
) -> Option<WorktreeIdentityConflict> {
    let session = try_load_session(session_id).ok().flatten()?;
    if session.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return None;
    }
    let expected = normalize_project_state_root(&session.worktree_path);
    let actual = normalize_project_state_root(&gwt_core::paths::resolve_current_worktree_root(cwd));
    if expected == actual {
        return None;
    }
    Some(WorktreeIdentityConflict {
        session_id: session_id.to_string(),
        expected_worktree: expected,
        actual_worktree: actual,
    })
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
        let projection_updated_at = canonical_projection.updated_at;
        let Some(canonical_agent) = canonical_projection.latest_agent_for_session_mut(session_id)
        else {
            return Ok(false);
        };
        let agent_updated_at = canonical_agent.updated_at;
        let changed = repair_agent_from_split(canonical_agent, &split_agent);
        if changed {
            let repaired_floor = Utc::now()
                .max(projection_updated_at)
                .max(agent_updated_at)
                .max(split_agent.updated_at);
            let repaired_at = repaired_floor
                .checked_add_signed(chrono::Duration::nanoseconds(1))
                .ok_or_else(|| {
                    gwt_core::GwtError::Other(
                        "split Agent repair timestamp exceeds the supported range".to_string(),
                    )
                })?;
            canonical_agent.updated_at = repaired_at;
            canonical_projection.updated_at = repaired_at;
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

    #[test]
    fn split_repair_keeps_future_timestamps_monotonic_and_repaired_row_latest() {
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

        let future = Utc::now() + chrono::Duration::days(1);
        let competing_at = future + chrono::Duration::hours(1);
        let canonical_at = future + chrono::Duration::hours(2);
        let split_at = future + chrono::Duration::hours(3);
        let projection_at = future + chrono::Duration::hours(4);
        let competing = agent_summary(
            "duplicate-session",
            Some("Competing canonical title"),
            Some("Competing canonical focus"),
            competing_at,
        );
        let canonical = agent_summary(
            "duplicate-session",
            Some("Canonical title"),
            Some("Canonical focus"),
            canonical_at,
        );
        let split = agent_summary(
            "duplicate-session",
            Some("Repaired split title"),
            Some("Repaired split focus"),
            split_at,
        );

        let mut canonical_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical_projection.agents = vec![competing, canonical];
        canonical_projection.updated_at = projection_at;
        gwt_core::workspace_projection::save_workspace_projection(
            &canonical_root,
            &canonical_projection,
        )
        .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split];
        split_projection.updated_at = split_at;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load canonical projection")
            .expect("canonical projection exists");
        let repaired_agent = repaired
            .agents
            .iter()
            .find(|agent| agent.title_summary.as_deref() == Some("Repaired split title"))
            .expect("repaired agent row");
        assert!(
            repaired_agent.updated_at >= projection_at,
            "repaired Agent timestamp must not regress below Agent/projection inputs"
        );
        assert!(
            repaired.updated_at >= projection_at,
            "projection timestamp must not regress during split repair"
        );
        assert_eq!(
            repaired.latest_agent_for_session("duplicate-session"),
            Some(repaired_agent),
            "the repaired row must remain the latest Session row"
        );
    }

    #[test]
    fn split_repair_makes_repaired_duplicate_strictly_latest_when_timestamps_tie() {
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
        let tied_at = Utc::now() + chrono::Duration::days(1);

        let competing = agent_summary(
            "duplicate-session",
            Some("Competing title"),
            Some("z competing focus"),
            tied_at,
        );
        let repair_target = agent_summary(
            "duplicate-session",
            Some("Repair target title"),
            None,
            tied_at,
        );
        let split = agent_summary(
            "duplicate-session",
            Some("Split title"),
            Some("a repaired focus"),
            tied_at,
        );

        let mut canonical =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical.agents = vec![competing, repair_target];
        canonical.updated_at = tied_at;
        gwt_core::workspace_projection::save_workspace_projection(&canonical_root, &canonical)
            .expect("save canonical projection");

        let mut split_projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split_projection.agents = vec![split];
        split_projection.updated_at = tied_at;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split_projection)
            .expect("save split projection");

        assert!(repair_split_agent_state_if_needed(
            &canonical_root,
            &split_root,
            "duplicate-session"
        )
        .expect("repair split state"));

        let repaired = gwt_core::workspace_projection::load_workspace_projection(&canonical_root)
            .expect("load repaired projection")
            .expect("repaired projection");
        let latest = repaired
            .latest_agent_for_session("duplicate-session")
            .expect("latest repaired Agent");
        assert_eq!(latest.title_summary.as_deref(), Some("Repair target title"));
        assert_eq!(latest.current_focus.as_deref(), Some("a repaired focus"));
        assert!(latest.updated_at > tied_at);
    }

    #[test]
    fn split_repair_timestamp_overflow_does_not_persist_partial_update() {
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
        let max = chrono::DateTime::<Utc>::MAX_UTC;

        let mut canonical =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                &canonical_root,
            );
        canonical.agents = vec![agent_summary(
            "overflow-session",
            Some("Canonical title"),
            None,
            max,
        )];
        canonical.updated_at = max;
        gwt_core::workspace_projection::save_workspace_projection(&canonical_root, &canonical)
            .expect("save canonical projection");

        let mut split =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&split_root);
        split.agents = vec![agent_summary(
            "overflow-session",
            Some("Split title"),
            Some("Split focus"),
            max,
        )];
        split.updated_at = max;
        gwt_core::workspace_projection::save_workspace_projection(&split_root, &split)
            .expect("save split projection");

        let canonical_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&canonical_root);
        let before = std::fs::read(&canonical_path).expect("read canonical before repair");
        let error =
            repair_split_agent_state_if_needed(&canonical_root, &split_root, "overflow-session")
                .expect_err("timestamp overflow must fail closed");

        assert!(error.to_string().contains("timestamp exceeds"));
        assert_eq!(
            std::fs::read(&canonical_path).expect("read canonical after repair"),
            before,
            "failed repair must not persist partially copied Agent fields"
        );
    }

    fn save_session(session_id: &str, worktree: &Path, runtime: gwt_agent::LaunchRuntimeTarget) {
        let mut session = Session::new(worktree, "work/test", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.runtime_target = runtime;
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save session");
    }

    /// Issue #3278: only a Host session whose recorded worktree differs from the
    /// process cwd is a conflict. A missing session, a matching worktree, and a
    /// non-Host (container) runtime all proceed without a false positive.
    #[test]
    fn agent_session_worktree_conflict_flags_host_mismatch_only() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let session_worktree = temp.path().join("work").join("session");
        let actual_worktree = temp.path().join("work").join("actual");
        std::fs::create_dir_all(&session_worktree).expect("session worktree");
        std::fs::create_dir_all(&actual_worktree).expect("actual worktree");

        // Unknown session → no conflict (the write path already falls back to cwd).
        assert!(agent_session_worktree_conflict(&actual_worktree, "missing-session").is_none());

        // Host session whose worktree matches cwd → no conflict.
        save_session(
            "host-match",
            &actual_worktree,
            gwt_agent::LaunchRuntimeTarget::Host,
        );
        assert!(agent_session_worktree_conflict(&actual_worktree, "host-match").is_none());

        // Host session bound to a different worktree → conflict naming both paths.
        save_session(
            "host-mismatch",
            &session_worktree,
            gwt_agent::LaunchRuntimeTarget::Host,
        );
        let conflict = agent_session_worktree_conflict(&actual_worktree, "host-mismatch")
            .expect("host mismatch is a conflict");
        assert_eq!(conflict.session_id, "host-mismatch");
        assert_eq!(
            conflict.expected_worktree,
            normalize_project_state_root(&session_worktree)
        );
        assert_eq!(
            conflict.actual_worktree,
            normalize_project_state_root(&actual_worktree)
        );

        // Docker session cwd is legitimately a container-internal path → skip.
        save_session(
            "docker-mismatch",
            &session_worktree,
            gwt_agent::LaunchRuntimeTarget::Docker,
        );
        assert!(agent_session_worktree_conflict(&actual_worktree, "docker-mismatch").is_none());
    }
}

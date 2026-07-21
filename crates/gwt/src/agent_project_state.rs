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

    fn init_git_repo(root: &Path, name: &str, remote: &str, branch: &str) -> PathBuf {
        let repo = root.join(name);
        std::fs::create_dir_all(&repo).expect("create git fixture");
        run_git(&["init"], &repo);
        run_git(&["config", "user.email", "test@example.com"], &repo);
        run_git(&["config", "user.name", "Test User"], &repo);
        run_git(&["checkout", "-b", branch], &repo);
        run_git(&["remote", "add", "origin", remote], &repo);
        run_git(&["commit", "--allow-empty", "-m", "initial"], &repo);
        dunce::canonicalize(repo).expect("canonical git fixture")
    }

    fn session_fixture(id: &str, repo: &Path, branch: &str) -> Session {
        let mut session = Session::new(repo, branch, gwt_agent::AgentId::Codex);
        session.id = id.to_string();
        session.project_state_root = Some(repo.to_path_buf());
        session
    }

    fn save_session_fixture(session: &Session) {
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save Session ledger fixture");
    }

    #[derive(Debug)]
    enum SessionLedgerFixture {
        Missing { session_id: String },
        Corrupt { session_id: String },
        Persisted(Box<Session>),
    }

    impl SessionLedgerFixture {
        fn session_id(&self) -> &str {
            match self {
                Self::Missing { session_id } | Self::Corrupt { session_id } => session_id,
                Self::Persisted(session) => &session.id,
            }
        }

        fn install(&self) {
            match self {
                Self::Missing { session_id } => {
                    let ledger_path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    assert!(
                        !ledger_path.exists(),
                        "missing-ledger fixture unexpectedly exists: {}",
                        ledger_path.display()
                    );
                }
                Self::Corrupt { session_id } => {
                    let ledger_path =
                        gwt_core::paths::gwt_sessions_dir().join(format!("{session_id}.toml"));
                    std::fs::create_dir_all(ledger_path.parent().expect("Session ledger parent"))
                        .expect("create sessions dir");
                    std::fs::write(&ledger_path, "broken = [")
                        .expect("write corrupt ledger fixture");
                }
                Self::Persisted(session) => save_session_fixture(session),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct WorkMutationSnapshot {
        current: Vec<u8>,
        journal: Vec<u8>,
        works: Vec<u8>,
        tracked_events: Vec<u8>,
    }

    impl WorkMutationSnapshot {
        fn capture(project_state_root: &Path, work_event_root: &Path) -> Self {
            Self {
                current: std::fs::read(
                    gwt_core::paths::gwt_workspace_projection_path_for_repo_path(
                        project_state_root,
                    ),
                )
                .expect("read current projection snapshot"),
                journal: std::fs::read(gwt_core::paths::gwt_workspace_journal_path_for_repo_path(
                    project_state_root,
                ))
                .expect("read journal snapshot"),
                works: std::fs::read(
                    gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(work_event_root),
                )
                .expect("read Work projection snapshot"),
                tracked_events: std::fs::read(gwt_core::paths::gwt_repo_local_work_events_path(
                    work_event_root,
                ))
                .expect("read tracked Work events snapshot"),
            }
        }

        fn changed_surfaces(&self, after: &Self) -> Vec<&'static str> {
            let mut changed = Vec::new();
            if self.current != after.current {
                changed.push("current");
            }
            if self.journal != after.journal {
                changed.push("journal");
            }
            if self.works != after.works {
                changed.push("works");
            }
            if self.tracked_events != after.tracked_events {
                changed.push("tracked events");
            }
            changed
        }
    }

    fn seed_work_mutation_surfaces(project_state_root: &Path, work_event_root: &Path) {
        gwt_core::workspace_projection::update_workspace_projection_with_journal_for_work_event_root(
            project_state_root,
            work_event_root,
            gwt_core::workspace_projection::WorkspaceProjectionUpdate {
                title: Some("Baseline Work".to_string()),
                status_category: Some(
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
                status_text: None,
                owner: Some("baseline-owner".to_string()),
                next_action: None,
                summary: Some("baseline state".to_string()),
                progress_summary: None,
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
        )
        .expect("seed Work mutation surfaces");
    }

    struct RejectedWorkspaceMutationCase {
        label: &'static str,
        expected_error: &'static str,
        ledger: SessionLedgerFixture,
        invocation_cwd: PathBuf,
        project_state_root: PathBuf,
        work_event_root: PathBuf,
    }

    fn init_case_repo(root: &Path, label: &str, branch: &str) -> (PathBuf, String) {
        let remote = format!("https://example.invalid/acme/session-bound-{label}.git");
        let repo = init_git_repo(root, &format!("{label}-repo"), &remote, branch);
        (repo, remote)
    }

    fn json_value_contains(value: &serde_json::Value, needle: &str) -> bool {
        match value {
            serde_json::Value::String(value) => value.contains(needle),
            serde_json::Value::Array(values) => values
                .iter()
                .any(|value| json_value_contains(value, needle)),
            serde_json::Value::Object(values) => values
                .iter()
                .any(|(key, value)| key.contains(needle) || json_value_contains(value, needle)),
            _ => false,
        }
    }

    #[test]
    fn strict_agent_session_roots_reject_missing_ledger_without_fallback() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            "work/strict-session",
        );

        let error = agent_session_roots_or_fallback(&repo, "missing-session-ledger")
            .expect_err("missing Session ledger must fail closed instead of using cwd fallback");
        let message = error.to_string();
        assert!(message.contains("missing-session-ledger"), "{message}");
        assert!(
            message.to_ascii_lowercase().contains("session"),
            "{message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_reject_corrupt_ledger_with_actionable_error() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            "work/strict-session",
        );
        let session_id = "corrupt-session-ledger";
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let ledger_path = sessions_dir.join(format!("{session_id}.toml"));
        std::fs::write(&ledger_path, "broken = [").expect("write corrupt Session ledger");

        let error = agent_session_roots_or_fallback(&repo, session_id)
            .expect_err("corrupt Session ledger must fail closed");
        let message = error.to_string();
        assert!(
            message.contains(session_id),
            "corrupt Session ledger error must identify the Session: {message}"
        );
        assert!(
            message.contains(&ledger_path.display().to_string()),
            "corrupt Session ledger error must identify the full ledger path: {message}"
        );
        let lowercase_message = message.to_ascii_lowercase();
        assert!(
            lowercase_message.contains("session ledger"),
            "corrupt Session ledger error must identify its context: {message}"
        );
        assert!(
            lowercase_message.contains("invalid") || lowercase_message.contains("corrupt"),
            "corrupt Session ledger error must describe the failure: {message}"
        );
    }

    #[test]
    fn strict_agent_session_roots_reject_provenance_mismatch_matrix() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let shared_remote = "https://example.invalid/acme/session-bound.git";
        let repo = init_git_repo(temp.path(), "repo", shared_remote, branch);
        let sibling = init_git_repo(temp.path(), "sibling", shared_remote, branch);
        let foreign = init_git_repo(
            temp.path(),
            "foreign",
            "https://example.invalid/foreign/project.git",
            branch,
        );
        let nested_event_root = repo.join("nested-event-root");
        std::fs::create_dir_all(&nested_event_root).expect("nested event root");

        let mut base = session_fixture("base", &repo, branch);
        base.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());

        let mut repo_hash = base.clone();
        repo_hash.id = "mismatch-repo-hash".to_string();
        repo_hash.repo_hash = Some("foreign-repo-hash".to_string());

        let mut canonical_repository = base.clone();
        canonical_repository.id = "mismatch-canonical-repository".to_string();
        canonical_repository.project_state_root = Some(foreign);

        let mut branch_mismatch = base.clone();
        branch_mismatch.id = "mismatch-branch".to_string();
        branch_mismatch.branch = "work/foreign-branch".to_string();

        let mut worktree = base.clone();
        worktree.id = "mismatch-worktree".to_string();
        worktree.worktree_path = sibling.clone();

        let mut cwd = base.clone();
        cwd.id = "mismatch-cwd".to_string();

        let mut event_root = base;
        event_root.id = "mismatch-event-root".to_string();
        event_root.worktree_path = nested_event_root.clone();

        let cases = [
            ("repo hash", repo_hash, repo.clone()),
            ("canonical repository", canonical_repository, repo.clone()),
            ("branch", branch_mismatch, repo.clone()),
            ("worktree", worktree, repo.clone()),
            ("cwd", cwd, sibling),
            ("event root", event_root, nested_event_root),
        ];
        let mut failures = Vec::new();

        for (expected_mismatch, session, invocation_cwd) in cases {
            save_session_fixture(&session);
            match agent_session_roots_or_fallback(&invocation_cwd, &session.id) {
                Ok((project_state_root, work_event_root)) => failures.push(format!(
                    "{expected_mismatch}: unexpectedly resolved project={} event={}",
                    project_state_root.display(),
                    work_event_root.display()
                )),
                Err(error) => {
                    let message = error.to_string();
                    if !message.to_ascii_lowercase().contains(expected_mismatch) {
                        failures.push(format!(
                            "{expected_mismatch}: error was not actionable: {message}"
                        ));
                    }
                    assert!(
                        !message.contains(RAW_PROVIDER_ACTOR_ID),
                        "provider actor id leaked through {expected_mismatch} diagnostic: {message}"
                    );
                }
            }
        }

        assert!(
            failures.is_empty(),
            "Session provenance mismatches must fail closed:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn workspace_update_dispatch_rejects_invalid_session_provenance_without_mutation() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let _provider_actor =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_THREAD_ID", RAW_PROVIDER_ACTOR_ID);
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let mut cases = Vec::new();

        let (missing_repo, _) = init_case_repo(temp.path(), "missing", branch);
        cases.push(RejectedWorkspaceMutationCase {
            label: "missing ledger",
            expected_error: "session ledger",
            ledger: SessionLedgerFixture::Missing {
                session_id: "dispatch-missing-ledger".to_string(),
            },
            invocation_cwd: missing_repo.clone(),
            project_state_root: missing_repo.clone(),
            work_event_root: missing_repo,
        });

        let (corrupt_repo, _) = init_case_repo(temp.path(), "corrupt", branch);
        cases.push(RejectedWorkspaceMutationCase {
            label: "corrupt ledger",
            expected_error: "session ledger",
            ledger: SessionLedgerFixture::Corrupt {
                session_id: "dispatch-corrupt-ledger".to_string(),
            },
            invocation_cwd: corrupt_repo.clone(),
            project_state_root: corrupt_repo.clone(),
            work_event_root: corrupt_repo,
        });

        let (repo_hash_repo, _) = init_case_repo(temp.path(), "repo-hash", branch);
        let mut repo_hash_session =
            session_fixture("dispatch-mismatch-repo-hash", &repo_hash_repo, branch);
        repo_hash_session.repo_hash = Some("foreign-repo-hash".to_string());
        repo_hash_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "repo hash mismatch",
            expected_error: "repo hash",
            ledger: SessionLedgerFixture::Persisted(Box::new(repo_hash_session)),
            invocation_cwd: repo_hash_repo.clone(),
            project_state_root: repo_hash_repo.clone(),
            work_event_root: repo_hash_repo,
        });

        let (canonical_repo, _) = init_case_repo(temp.path(), "canonical-repository", branch);
        let canonical_foreign = init_git_repo(
            temp.path(),
            "canonical-repository-foreign",
            "https://example.invalid/foreign/canonical-repository.git",
            branch,
        );
        let mut canonical_session = session_fixture(
            "dispatch-mismatch-canonical-repository",
            &canonical_repo,
            branch,
        );
        canonical_session.project_state_root = Some(canonical_foreign.clone());
        canonical_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "canonical repository mismatch",
            expected_error: "canonical repository",
            ledger: SessionLedgerFixture::Persisted(Box::new(canonical_session)),
            invocation_cwd: canonical_repo.clone(),
            project_state_root: canonical_foreign,
            work_event_root: canonical_repo,
        });

        let (branch_repo, _) = init_case_repo(temp.path(), "branch", branch);
        let mut branch_session = session_fixture("dispatch-mismatch-branch", &branch_repo, branch);
        branch_session.branch = "work/foreign-branch".to_string();
        branch_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "branch mismatch",
            expected_error: "branch",
            ledger: SessionLedgerFixture::Persisted(Box::new(branch_session)),
            invocation_cwd: branch_repo.clone(),
            project_state_root: branch_repo.clone(),
            work_event_root: branch_repo,
        });

        let (worktree_repo, worktree_remote) = init_case_repo(temp.path(), "worktree", branch);
        let worktree_sibling =
            init_git_repo(temp.path(), "worktree-sibling", &worktree_remote, branch);
        let mut worktree_session =
            session_fixture("dispatch-mismatch-worktree", &worktree_repo, branch);
        worktree_session.worktree_path = worktree_sibling.clone();
        worktree_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "worktree mismatch",
            expected_error: "worktree",
            ledger: SessionLedgerFixture::Persisted(Box::new(worktree_session)),
            invocation_cwd: worktree_repo.clone(),
            project_state_root: worktree_repo,
            work_event_root: worktree_sibling,
        });

        let (cwd_repo, cwd_remote) = init_case_repo(temp.path(), "cwd", branch);
        let cwd_sibling = init_git_repo(temp.path(), "cwd-sibling", &cwd_remote, branch);
        let mut cwd_session = session_fixture("dispatch-mismatch-cwd", &cwd_repo, branch);
        cwd_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "cwd mismatch",
            expected_error: "cwd",
            ledger: SessionLedgerFixture::Persisted(Box::new(cwd_session)),
            invocation_cwd: cwd_sibling,
            project_state_root: cwd_repo.clone(),
            work_event_root: cwd_repo,
        });

        let (event_repo, _) = init_case_repo(temp.path(), "event-root", branch);
        let nested_event_root = event_repo.join("nested-event-root");
        std::fs::create_dir_all(&nested_event_root).expect("nested event root");
        let mut event_session =
            session_fixture("dispatch-mismatch-event-root", &event_repo, branch);
        event_session.worktree_path = nested_event_root.clone();
        event_session.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        cases.push(RejectedWorkspaceMutationCase {
            label: "event root mismatch",
            expected_error: "event root",
            ledger: SessionLedgerFixture::Persisted(Box::new(event_session)),
            invocation_cwd: nested_event_root.clone(),
            project_state_root: event_repo,
            work_event_root: nested_event_root,
        });

        let mut failures = Vec::new();
        for case in cases {
            seed_work_mutation_surfaces(&case.project_state_root, &case.work_event_root);
            let before =
                WorkMutationSnapshot::capture(&case.project_state_root, &case.work_event_root);
            case.ledger.install();

            let _ambient = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::session::GWT_SESSION_ID_ENV,
                case.ledger.session_id(),
            );
            let mut env = crate::cli::TestEnv::new(case.invocation_cwd);
            env.stdin = serde_json::json!({
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "summary": "must be rejected without mutation",
                },
            })
            .to_string();

            let code = crate::cli::dispatch(&mut env, &["gwtd".to_string()]);
            let stderr = String::from_utf8_lossy(&env.stderr);
            if code == 0 {
                failures.push(format!("{}: unexpectedly accepted", case.label));
            } else if !stderr.to_ascii_lowercase().contains(case.expected_error) {
                failures.push(format!(
                    "{}: rejection was not actionable: {stderr}",
                    case.label
                ));
            }
            if stderr.contains(RAW_PROVIDER_ACTOR_ID) {
                failures.push(format!(
                    "{}: provider actor id leaked in diagnostic: {stderr}",
                    case.label
                ));
            }

            let after =
                WorkMutationSnapshot::capture(&case.project_state_root, &case.work_event_root);
            let changed_surfaces = before.changed_surfaces(&after);
            if !changed_surfaces.is_empty() {
                failures.push(format!(
                    "{}: rejection was not byte-equivalent for {}",
                    case.label,
                    changed_surfaces.join(", ")
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "workspace.update must reject invalid gwt Session provenance before persistence:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn provider_actor_id_is_not_authorization_or_tracked_provenance() {
        const RAW_PROVIDER_ACTOR_ID: &str = "provider-thread-private-sentinel-86";

        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("home");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let branch = "work/strict-session";
        let repo = init_git_repo(
            temp.path(),
            "repo",
            "https://example.invalid/acme/session-bound.git",
            branch,
        );

        let mut provider_present = session_fixture("provider-present", &repo, branch);
        provider_present.agent_session_id = Some(RAW_PROVIDER_ACTOR_ID.to_string());
        let provider_absent = session_fixture("provider-absent", &repo, branch);

        for session in [&provider_present, &provider_absent] {
            save_session_fixture(session);
            let _ambient = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::session::GWT_SESSION_ID_ENV,
                &session.id,
            );
            let mut env = crate::cli::TestEnv::new(repo.clone());
            env.stdin = serde_json::json!({
                "schema_version": 1,
                "operation": "workspace.update",
                "params": {
                    "summary": "provider-neutral mutation",
                },
            })
            .to_string();

            let code = crate::cli::dispatch(&mut env, &["gwtd".to_string()]);
            assert_eq!(
                code,
                0,
                "workspace.update must accept valid gwt Session provenance: {}",
                String::from_utf8_lossy(&env.stderr)
            );
        }

        let tracked_events =
            std::fs::read_to_string(gwt_core::paths::gwt_repo_local_work_events_path(&repo))
                .expect("read tracked Work events");
        let events: Vec<serde_json::Value> = tracked_events
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("parse tracked Work event JSONL"))
            .collect();
        for session in [&provider_present, &provider_absent] {
            let event = events
                .iter()
                .find(|event| event["agent_session_id"].as_str() == Some(session.id.as_str()))
                .unwrap_or_else(|| panic!("tracked Work event missing Session {}", session.id));
            assert_eq!(
                event["agent_session_id"].as_str(),
                Some(session.id.as_str()),
                "tracked provenance must remain the immutable gwt Session id"
            );
        }
        assert!(
            !events
                .iter()
                .any(|event| json_value_contains(event, RAW_PROVIDER_ACTOR_ID)),
            "raw provider actor id must never enter any tracked Work event JSON value: {events:?}"
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
}

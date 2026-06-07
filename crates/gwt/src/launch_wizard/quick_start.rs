use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

use super::QuickStartEntry;

/// Load every persisted session from `sessions_dir` fresh from disk, applying
/// legacy migrations. Resume paths use this instead of the GUI's in-memory
/// session cache so they observe session TOMLs that the managed hook CLI
/// updates out-of-process (e.g. the real `agent_session_id` persisted after an
/// agent starts), which the cache — loaded once at startup and only refreshed
/// per-window at spawn — never picks up (#2995).
pub fn load_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("toml")).then_some(path)
        })
        .filter_map(|path| gwt_agent::Session::load_and_migrate(&path).ok())
        .collect()
}

pub fn load_quick_start_entries(
    repo_path: &Path,
    sessions_dir: &Path,
    branch_name: &str,
) -> Vec<QuickStartEntry> {
    collect_quick_start_entries_from_sessions(repo_path, branch_name, load_sessions(sessions_dir))
}

pub(super) fn collect_quick_start_entries_from_sessions(
    repo_path: &Path,
    branch_name: &str,
    sessions: Vec<gwt_agent::Session>,
) -> Vec<QuickStartEntry> {
    let mut latest_resumable_session = None::<gwt_agent::Session>;
    let mut latest_metadata_only_session = None::<gwt_agent::Session>;
    let repo_scope = WorktreePathScope::new(repo_path);

    for session in sessions {
        if session.branch != branch_name || !repo_scope.matches(&session.worktree_path) {
            continue;
        }

        if agent_session_resume_id(&session).is_some() {
            let replace = latest_resumable_session
                .as_ref()
                .map(|current| session_is_newer(&session, current))
                .unwrap_or(true);
            if replace {
                latest_resumable_session = Some(session);
            }
        } else {
            let replace = latest_metadata_only_session
                .as_ref()
                .map(|current| session_is_newer(&session, current))
                .unwrap_or(true);
            if replace {
                latest_metadata_only_session = Some(session);
            }
        }
    }

    latest_resumable_session
        .or(latest_metadata_only_session)
        .into_iter()
        .map(|session| QuickStartEntry {
            session_id: session.id.clone(),
            agent_id: session.agent_id.command().to_string(),
            tool_label: session.display_name.clone(),
            model: session.model.clone(),
            reasoning: session.reasoning_level.clone(),
            version: session.tool_version.clone().or_else(|| {
                session
                    .agent_id
                    .package_name()
                    .map(|_| "installed".to_string())
            }),
            resume_session_id: agent_session_resume_id(&session),
            live_window_id: None,
            skip_permissions: session.skip_permissions,
            codex_fast_mode: session.fast_mode_enabled(),
            runtime_target: session.runtime_target,
            docker_service: session.docker_service.clone(),
            docker_lifecycle_intent: session.docker_lifecycle_intent,
        })
        .collect()
}

fn session_is_newer(candidate: &gwt_agent::Session, current: &gwt_agent::Session) -> bool {
    compare_session_recency(candidate, current) == Ordering::Greater
}

fn compare_session_recency(left: &gwt_agent::Session, right: &gwt_agent::Session) -> Ordering {
    left.last_activity_at
        .cmp(&right.last_activity_at)
        .then_with(|| left.updated_at.cmp(&right.updated_at))
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn agent_session_resume_id(session: &gwt_agent::Session) -> Option<String> {
    session.exact_resume_session_id().map(str::to_string)
}

struct WorktreePathScope<'a> {
    original: &'a Path,
    canonical: Option<PathBuf>,
}

impl<'a> WorktreePathScope<'a> {
    fn new(original: &'a Path) -> Self {
        Self {
            original,
            canonical: original.canonicalize().ok(),
        }
    }

    fn matches(&self, candidate: &Path) -> bool {
        if candidate == self.original {
            return true;
        }
        match (self.canonical.as_ref(), candidate.canonicalize().ok()) {
            (Some(expected), Some(candidate)) => candidate == *expected,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::*;

    fn sample_session(
        dir: &Path,
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: &str,
    ) {
        sample_session_with_resume(
            dir,
            branch,
            worktree_path,
            agent_id,
            updated_at,
            Some(resume_id),
        );
    }

    fn sample_session_with_resume(
        dir: &Path,
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: Option<&str>,
    ) {
        let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
        session.display_name = session.agent_id.display_name().to_string();
        session.agent_session_id = resume_id.map(str::to_string);
        session.tool_version = Some("installed".to_string());
        session.model = Some("gpt-5.5".to_string());
        session.reasoning_level = Some("high".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("gwt".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.created_at = updated_at;
        session.updated_at = updated_at;
        session.last_activity_at = updated_at;
        session.save(dir).expect("save session");
    }

    fn sample_session_record(
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: Option<&str>,
    ) -> gwt_agent::Session {
        let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
        session.display_name = session.agent_id.display_name().to_string();
        session.agent_session_id = resume_id.map(str::to_string);
        session.tool_version = Some("installed".to_string());
        session.model = Some("gpt-5.5".to_string());
        session.reasoning_level = Some("high".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("gwt".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.created_at = updated_at;
        session.updated_at = updated_at;
        session.last_activity_at = updated_at;
        session
    }

    #[test]
    fn load_quick_start_entries_uses_latest_resumable_session_profile() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "older",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            "newer",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(entries[0].resume_session_id.as_deref(), Some("newer"));
        assert_eq!(entries[0].docker_service.as_deref(), Some("gwt"));
    }

    #[test]
    fn load_quick_start_entries_keeps_only_latest_resumable_session() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "older-resume",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            "newer-resume",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(
            entries.len(),
            1,
            "Launch Agent start methods must expose only the latest resumable session"
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.resume_session_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("newer-resume")]
        );
    }

    #[test]
    fn load_quick_start_entries_uses_latest_resumable_session_when_latest_lacks_resume_id() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "resume-older",
        );
        sample_session_with_resume(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-older")
        );
    }

    #[test]
    fn load_quick_start_entries_does_not_reuse_resume_id_from_other_scope() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        let other_worktree = dir.path().join("other-repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        std::fs::create_dir_all(&other_worktree).expect("other repo dir");
        sample_session(
            dir.path(),
            "feature/other",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "wrong-branch",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &other_worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 30, 0).unwrap(),
            "wrong-worktree",
        );
        sample_session_with_resume(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert!(entries[0].resume_session_id.is_none());
    }

    #[test]
    fn load_quick_start_entries_matches_canonical_worktree_path() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        let same_worktree_with_dot = worktree.join(".");
        sample_session(
            dir.path(),
            "feature/gui",
            &same_worktree_with_dot,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "resume-canonical",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-canonical")
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_quick_start_entries_matches_symlinked_worktree_path() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        let symlink = dir.path().join("repo-link");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        std::os::unix::fs::symlink(&worktree, &symlink).expect("repo symlink");
        sample_session(
            dir.path(),
            "feature/gui",
            &symlink,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "resume-symlink",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-symlink")
        );
    }

    #[test]
    fn collect_quick_start_entries_from_sessions_reuses_resumable_session_profile() {
        let worktree = PathBuf::from("/tmp/repo");
        let mut older = sample_session_record(
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            Some("resume-older"),
        );
        older.tool_version = Some("0.110.0".to_string());
        older.model = Some("gpt-5.5".to_string());
        older.reasoning_level = Some("high".to_string());
        older.skip_permissions = true;
        older.codex_fast_mode = true;
        older.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        older.docker_service = Some("gwt".to_string());

        let mut newer = sample_session_record(
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );
        newer.tool_version = Some("0.111.0".to_string());
        newer.model = Some("gpt-5.4-mini".to_string());
        newer.reasoning_level = Some("low".to_string());
        newer.skip_permissions = false;
        newer.codex_fast_mode = false;
        newer.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        newer.docker_service = None;

        let entries = collect_quick_start_entries_from_sessions(
            &worktree,
            "feature/gui",
            vec![older.clone(), newer],
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id, older.id);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("resume-older")
        );
        assert_eq!(entries[0].model.as_deref(), Some("gpt-5.5"));
        assert_eq!(entries[0].reasoning.as_deref(), Some("high"));
        assert_eq!(entries[0].version.as_deref(), Some("0.110.0"));
        assert_eq!(
            entries[0].runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(entries[0].docker_service.as_deref(), Some("gwt"));
        assert!(entries[0].skip_permissions);
        assert!(entries[0].codex_fast_mode);
    }
}

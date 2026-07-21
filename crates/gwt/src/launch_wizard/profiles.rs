use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use super::{
    quick_start, LaunchWizardPreviousProfile, LaunchWizardPreviousProfiles, QuickStartEntry,
};

pub fn load_previous_launch_profile(
    repo_path: &Path,
    sessions_dir: &Path,
) -> Option<LaunchWizardPreviousProfile> {
    let sessions = load_launch_sessions(sessions_dir);
    previous_launch_profile_from_sessions(repo_path, &sessions)
}

pub fn load_previous_launch_profiles(sessions_dir: &Path) -> LaunchWizardPreviousProfiles {
    let sessions = load_launch_sessions(sessions_dir);
    previous_launch_profiles_from_sessions(&sessions)
}

pub fn previous_launch_profile_from_sessions(
    repo_path: &Path,
    sessions: &[gwt_agent::Session],
) -> Option<LaunchWizardPreviousProfile> {
    let repo_scope = LaunchProfileRepoScope::new(repo_path);
    sessions
        .iter()
        .filter(|session| repo_scope.matches(session))
        .max_by(|left, right| launch_profile_session_cmp(left, right))
        .cloned()
        .map(previous_profile_from_session)
}

pub fn previous_launch_profiles_from_sessions(
    sessions: &[gwt_agent::Session],
) -> LaunchWizardPreviousProfiles {
    let mut latest_by_agent: HashMap<String, gwt_agent::Session> = HashMap::new();
    let mut default_agent_id = None;
    let mut latest_session = None::<gwt_agent::Session>;

    for session in sessions {
        let agent_id = session.agent_id.command().to_string();
        if latest_by_agent
            .get(&agent_id)
            .is_none_or(|existing| launch_profile_session_cmp(session, existing).is_gt())
        {
            latest_by_agent.insert(agent_id.clone(), session.clone());
        }
        if latest_session
            .as_ref()
            .is_none_or(|existing| launch_profile_session_cmp(session, existing).is_gt())
        {
            default_agent_id = Some(agent_id);
            latest_session = Some(session.clone());
        }
    }

    let by_agent = latest_by_agent
        .into_iter()
        .map(|(agent_id, session)| (agent_id, previous_profile_from_session(session)))
        .collect();

    LaunchWizardPreviousProfiles {
        default_agent_id,
        by_agent,
        repo_local: None,
    }
}

/// SPEC-2014 FR-032/FR-035: per-agent global preference に加え、repo-local
/// 最新 successful session profile を `repo_local` 経路として併せ持つ
/// `LaunchWizardPreviousProfiles` を構築する。
pub fn previous_launch_profiles_for_repo_from_sessions(
    repo_path: &Path,
    sessions: &[gwt_agent::Session],
) -> LaunchWizardPreviousProfiles {
    let mut profiles = previous_launch_profiles_from_sessions(sessions);
    profiles.repo_local = previous_launch_profile_from_sessions(repo_path, sessions);
    profiles
}

pub(super) fn load_launch_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
    gwt_agent::load_sessions_with_legacy_import(sessions_dir)
}

fn launch_profile_session_cmp(left: &gwt_agent::Session, right: &gwt_agent::Session) -> Ordering {
    left.updated_at
        .cmp(&right.updated_at)
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

pub fn quick_start_entries_from_sessions(
    repo_path: &Path,
    branch_name: &str,
    sessions: &[gwt_agent::Session],
) -> Vec<QuickStartEntry> {
    let repo_scope = QuickStartRepoScope::new(repo_path);
    let sessions = sessions
        .iter()
        .filter(|session| session.branch == branch_name)
        .filter(|session| repo_scope.matches(session))
        .cloned()
        .map(|mut session| {
            session.worktree_path = repo_path.to_path_buf();
            session
        })
        .collect::<Vec<_>>();
    quick_start::collect_quick_start_entries_from_sessions(repo_path, branch_name, sessions)
}

fn previous_profile_from_session(session: gwt_agent::Session) -> LaunchWizardPreviousProfile {
    let fast_mode = session.fast_mode_enabled();
    LaunchWizardPreviousProfile {
        agent_id: session.agent_id.command().to_string(),
        model: session.model,
        reasoning: session.reasoning_level,
        version: session.tool_version.or_else(|| {
            session
                .agent_id
                .package_name()
                .map(|_| "installed".to_string())
        }),
        session_mode: session.session_mode,
        skip_permissions: session.skip_permissions,
        codex_fast_mode: fast_mode,
        runtime_target: session.runtime_target,
        docker_service: session.docker_service,
        docker_lifecycle_intent: session.docker_lifecycle_intent,
        windows_shell: session.windows_shell,
    }
}

struct LaunchProfileRepoScope<'a> {
    repo_path: &'a Path,
    repo_hash: Option<String>,
    repo_root: OnceLock<Option<PathBuf>>,
    session_root_cache: RefCell<HashMap<PathBuf, Option<PathBuf>>>,
}

impl<'a> LaunchProfileRepoScope<'a> {
    fn new(repo_path: &'a Path) -> Self {
        Self {
            repo_path,
            repo_hash: repo_hash_for_existing_path(repo_path),
            repo_root: OnceLock::new(),
            session_root_cache: RefCell::new(HashMap::new()),
        }
    }

    fn matches(&self, session: &gwt_agent::Session) -> bool {
        if let (Some(current_repo_hash), Some(session_repo_hash)) =
            (self.repo_hash.as_deref(), session.repo_hash.as_deref())
        {
            return current_repo_hash == session_repo_hash;
        }

        let session_worktree_path = &session.worktree_path;
        if same_path_or_exact(self.repo_path, session_worktree_path) {
            return true;
        }

        if !session_worktree_path.exists() {
            return false;
        }

        let Some(repo_root) = self.repo_root() else {
            return false;
        };
        let Some(session_root) = self.session_main_worktree_root(session_worktree_path) else {
            return false;
        };
        same_path_or_exact(repo_root, &session_root)
    }

    fn repo_root(&self) -> Option<&Path> {
        self.repo_root
            .get_or_init(|| gwt_git::worktree::main_worktree_root(self.repo_path).ok())
            .as_deref()
    }

    fn session_main_worktree_root(&self, path: &Path) -> Option<PathBuf> {
        if let Some(cached) = self.session_root_cache.borrow().get(path) {
            return cached.clone();
        }

        let root = gwt_git::worktree::main_worktree_root(path).ok();
        self.session_root_cache
            .borrow_mut()
            .insert(path.to_path_buf(), root.clone());
        root
    }
}

struct QuickStartRepoScope<'a> {
    repo_path: &'a Path,
    canonical: Option<PathBuf>,
    repo_hash: Option<String>,
    repo_root: OnceLock<Option<PathBuf>>,
}

impl<'a> QuickStartRepoScope<'a> {
    fn new(repo_path: &'a Path) -> Self {
        Self {
            repo_path,
            canonical: repo_path.canonicalize().ok(),
            repo_hash: repo_hash_for_existing_path(repo_path),
            repo_root: OnceLock::new(),
        }
    }

    fn matches(&self, session: &gwt_agent::Session) -> bool {
        if let (Some(current_repo_hash), Some(session_repo_hash)) =
            (self.repo_hash.as_deref(), session.repo_hash.as_deref())
        {
            return current_repo_hash == session_repo_hash;
        }
        if session
            .project_state_root
            .as_deref()
            .is_some_and(|root| self.matches_path(root))
        {
            return true;
        }
        self.matches_path(&session.worktree_path)
    }

    fn matches_path(&self, candidate: &Path) -> bool {
        if candidate == self.repo_path {
            return true;
        }
        if let (Some(expected), Ok(candidate)) = (self.canonical.as_ref(), candidate.canonicalize())
        {
            if candidate == *expected {
                return true;
            }
        }
        if !candidate.exists() {
            return false;
        }

        let Some(repo_root) = self.repo_root() else {
            return false;
        };
        let Ok(candidate_root) = gwt_git::worktree::main_worktree_root(candidate) else {
            return false;
        };
        same_path_or_exact(repo_root, &candidate_root)
    }

    fn repo_root(&self) -> Option<&Path> {
        self.repo_root
            .get_or_init(|| gwt_git::worktree::main_worktree_root(self.repo_path).ok())
            .as_deref()
    }
}

fn repo_hash_for_existing_path(path: &Path) -> Option<String> {
    gwt_core::repo_hash::detect_repo_hash(path)
        .or_else(|| {
            gwt_git::worktree::main_worktree_root(path)
                .ok()
                .and_then(|root| gwt_core::repo_hash::detect_repo_hash(&root))
        })
        .map(|hash| hash.as_str().to_string())
}

fn same_path_or_exact(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::tempdir;

    use super::super::test_support::*;
    use super::super::*;
    use super::*;

    #[test]
    fn load_previous_launch_profile_uses_latest_session_for_repo_without_reusing_branch() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        let mut older = sample_session_record(
            "feature/old",
            &worktree,
            gwt_agent::AgentId::ClaudeCode,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            None,
        );
        older.session_mode = gwt_agent::SessionMode::Normal;
        older.save(dir.path()).expect("save older session");

        let mut newer = sample_session_record(
            "feature/previous",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            Some("resume-ignored"),
        );
        newer.tool_version = Some("0.110.0".to_string());
        newer.model = Some("gpt-5.5".to_string());
        newer.reasoning_level = Some("high".to_string());
        newer.session_mode = gwt_agent::SessionMode::Continue;
        newer.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        newer.docker_service = Some("gwt".to_string());
        newer.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        newer.save(dir.path()).expect("save newer session");

        let profile =
            load_previous_launch_profile(&worktree, dir.path()).expect("previous profile");

        assert_eq!(profile.agent_id, "codex");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(profile.reasoning.as_deref(), Some("high"));
        assert_eq!(profile.version.as_deref(), Some("0.110.0"));
        assert_eq!(profile.session_mode, gwt_agent::SessionMode::Continue);
        assert_eq!(
            profile.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(profile.docker_service.as_deref(), Some("gwt"));
    }

    #[test]
    fn previous_launch_profile_tie_breaks_equal_timestamps_by_session_id() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        let timestamp = Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap();
        let mut lower_id = sample_session_record(
            "feature/lower",
            &worktree,
            gwt_agent::AgentId::Codex,
            timestamp,
            None,
        );
        lower_id.id = "session-a".to_string();
        lower_id.model = Some("gpt-5.4".to_string());
        let mut higher_id = sample_session_record(
            "feature/higher",
            &worktree,
            gwt_agent::AgentId::Codex,
            timestamp,
            None,
        );
        higher_id.id = "session-b".to_string();
        higher_id.model = Some("gpt-5.5".to_string());

        let profile = previous_launch_profile_from_sessions(
            &worktree,
            &[higher_id.clone(), lower_id.clone()],
        )
        .expect("profile");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));

        let profile = previous_launch_profile_from_sessions(&worktree, &[lower_id, higher_id])
            .expect("profile");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn agent_preferences_restore_selected_agent_when_latest_session_is_other_agent() {
        let current_repo = PathBuf::from("/tmp/current-repo");
        let codex_repo = PathBuf::from("/tmp/codex-repo");
        let claude_repo = PathBuf::from("/tmp/claude-repo");
        let mut codex = sample_session_record(
            "feature/codex",
            &codex_repo,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
            None,
        );
        codex.model = Some("gpt-5.4".to_string());
        codex.reasoning_level = Some("xhigh".to_string());
        codex.tool_version = Some("0.110.0".to_string());
        codex.session_mode = gwt_agent::SessionMode::Continue;
        codex.skip_permissions = true;
        codex.codex_fast_mode = true;

        let mut claude = sample_session_record(
            "feature/claude",
            &claude_repo,
            gwt_agent::AgentId::ClaudeCode,
            Utc.with_ymd_and_hms(2026, 5, 10, 10, 0, 0).unwrap(),
            None,
        );
        claude.model = Some("sonnet".to_string());
        claude.reasoning_level = Some("low".to_string());
        claude.skip_permissions = false;
        claude.codex_fast_mode = false;

        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.worktree_path = Some(current_repo.clone());
        ctx.quick_start_root = current_repo.clone();
        let profiles = previous_launch_profiles_from_sessions(&[codex, claude]);
        let mut state = LaunchWizardState::open_with_previous_profiles(
            ctx,
            sample_agent_options(),
            Vec::new(),
            profiles,
        );

        assert_eq!(state.view().selected_agent_id, "claude");

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        let view = state.view();

        assert_eq!(view.branch_name, "feature/current");
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.4");
        assert_eq!(view.selected_reasoning, "xhigh");
        assert_eq!(view.selected_version, "0.110.0");
        assert_eq!(view.selected_execution_mode, "continue");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.branch.as_deref(), Some("feature/current"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Continue);
        assert_eq!(config.reasoning_level.as_deref(), Some("xhigh"));
        assert!(config.codex_fast_mode);
        assert!(config.skip_permissions);
        assert_eq!(config.working_dir.as_deref(), Some(current_repo.as_path()));
    }

    #[test]
    fn agent_preferences_do_not_restore_project_runtime_settings() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.quick_start_root = PathBuf::from("/tmp/current-repo");
        ctx.worktree_path = Some(PathBuf::from("/tmp/current-repo"));
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Stopped;

        let mut codex = sample_session_record(
            "feature/codex",
            Path::new("/tmp/other-repo"),
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
            None,
        );
        codex.model = Some("gpt-5.4".to_string());
        codex.reasoning_level = Some("xhigh".to_string());
        codex.tool_version = Some("0.110.0".to_string());
        codex.session_mode = gwt_agent::SessionMode::Continue;
        codex.skip_permissions = true;
        codex.codex_fast_mode = true;
        codex.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        codex.docker_service = Some("worker".to_string());
        codex.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        codex.windows_shell = Some(gwt_agent::WindowsShellKind::PowerShell7);

        let state = LaunchWizardState::open_with_previous_profiles(
            ctx,
            sample_agent_options(),
            Vec::new(),
            previous_launch_profiles_from_sessions(&[codex]),
        );
        let view = state.view();

        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.4");
        assert_eq!(view.selected_reasoning, "xhigh");
        assert_eq!(view.selected_execution_mode, "continue");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("api"));
        assert_eq!(view.selected_docker_lifecycle, "start");
        assert_ne!(view.selected_docker_service.as_deref(), Some("worker"));
        assert_ne!(view.selected_docker_lifecycle, "restart");
    }

    #[test]
    fn load_previous_launch_profile_matches_deleted_worktree_by_persisted_repo_hash() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let origin = "https://github.com/example/project.git";
        init_repo_with_origin(&repo, origin);
        let removed_worktree = dir.path().join("removed-worktree");
        let mut session = sample_session_record(
            "feature/removed",
            &removed_worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            None,
        );
        session.repo_hash = Some(
            gwt_core::repo_hash::compute_repo_hash(origin)
                .as_str()
                .to_string(),
        );
        session
            .save(dir.path())
            .expect("save removed worktree session");

        let profile = load_previous_launch_profile(&repo, dir.path())
            .expect("profile should match persisted repo identity");

        assert_eq!(profile.agent_id, "codex");
    }

    #[test]
    fn quick_start_entries_match_deleted_worktree_by_persisted_repo_hash() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let origin = "https://github.com/example/project.git";
        init_repo_with_origin(&repo, origin);
        let removed_worktree = dir.path().join("removed-worktree");
        let mut session = sample_session_record(
            "feature/gui",
            &removed_worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            Some("native-session"),
        );
        session.repo_hash = Some(
            gwt_core::repo_hash::compute_repo_hash(origin)
                .as_str()
                .to_string(),
        );

        let entries = quick_start_entries_from_sessions(&repo, "feature/gui", &[session]);

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("native-session")
        );
    }

    #[test]
    fn imported_legacy_session_with_deleted_worktree_matches_originless_project_root() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let status = gwt_core::process::hidden_command("git")
            .args(["init"])
            .current_dir(&repo)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        assert_eq!(gwt_core::repo_hash::detect_repo_hash(&repo), None);

        let sessions_dir = temp.path().join(".gwt").join("sessions");
        let legacy_dir = temp.path().join(".config").join("gwt").join("sessions");
        std::fs::create_dir_all(&legacy_dir).expect("legacy dir");
        let removed_worktree = temp.path().join("deleted-worktree");
        std::fs::write(
            legacy_dir.join("originless.json"),
            serde_json::to_vec_pretty(&json!({
                "repositoryRoot": repo,
                "lastWorktreePath": removed_worktree,
                "lastBranch": "main",
                "lastUsedTool": "codex",
                "lastSessionId": "legacy-originless-session",
                "toolLabel": "Codex",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }))
            .expect("serialize fixture"),
        )
        .expect("write fixture");

        let sessions = load_launch_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 1, "legacy JSON should be imported");
        assert_eq!(
            sessions[0].project_state_root.as_deref(),
            Some(repo.as_path())
        );
        assert_eq!(sessions[0].repo_hash, None);

        let entries = quick_start_entries_from_sessions(&repo, "main", &sessions);

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("legacy-originless-session")
        );
    }

    #[test]
    fn previous_launch_profile_treats_mismatched_repo_hash_as_authoritative() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        init_repo_with_origin(&repo, "https://github.com/example/project.git");
        let nested_path = repo.join("nested");
        std::fs::create_dir_all(&nested_path).expect("nested dir");

        let mut session = sample_session_record(
            "feature/wrong-repo",
            &nested_path,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 22, 10, 0, 0).unwrap(),
            None,
        );
        session.repo_hash = Some(
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/other.git")
                .as_str()
                .to_string(),
        );

        assert!(
            previous_launch_profile_from_sessions(&repo, &[session]).is_none(),
            "repo_hash mismatch must not fall back to legacy root matching"
        );
    }

    #[test]
    fn repo_hash_conflict_overrides_exact_path_for_profile_and_quick_start_scopes() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        init_repo_with_origin(&repo, "https://github.com/example/project.git");
        let mut session = sample_session_record(
            "feature/wrong-repo",
            &repo,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 22, 11, 0, 0).unwrap(),
            Some("native-wrong-repo"),
        );
        session.repo_hash = Some(
            gwt_core::repo_hash::compute_repo_hash("https://github.com/example/other.git")
                .as_str()
                .to_string(),
        );

        assert!(
            previous_launch_profile_from_sessions(&repo, &[session.clone()]).is_none(),
            "an exact path must not override conflicting authoritative repo identities"
        );
        assert!(
            quick_start_entries_from_sessions(&repo, "feature/wrong-repo", &[session]).is_empty(),
            "Quick Start profile collection must reject the same repo identity conflict"
        );
    }

    #[test]
    fn quick_start_entries_match_sibling_worktree_without_persisted_repo_hash() {
        let dir = tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let worktree = dir.path().join("repo-work");
        let origin = "https://github.com/example/project.git";
        init_repo_with_origin(&repo, origin);
        std::fs::write(repo.join("README.md"), "# project\n").expect("write readme");
        let status = gwt_core::process::hidden_command("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .status()
            .expect("git add");
        assert!(status.success(), "git add failed");
        let status = gwt_core::process::hidden_command("git")
            .args([
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "initial",
            ])
            .current_dir(&repo)
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit failed");
        let worktree_arg = worktree.to_str().expect("worktree path");
        let status = gwt_core::process::hidden_command("git")
            .args(["worktree", "add", "-b", "feature/gui", worktree_arg])
            .current_dir(&repo)
            .status()
            .expect("git worktree add");
        assert!(status.success(), "git worktree add failed");
        let mut session = sample_session_record(
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            Some("native-session"),
        );
        session.repo_hash = None;

        let entries = quick_start_entries_from_sessions(&repo, "feature/gui", &[session]);

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].resume_session_id.as_deref(),
            Some("native-session")
        );
    }

    #[test]
    fn open_and_quick_start_helpers_cover_real_sessions_and_errors() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 11, 0, 0).unwrap(),
            "resume-1",
        );

        let mut ctx = context(branch("origin/feature/gui"), "feature/gui");
        ctx.quick_start_root = worktree;
        let state = LaunchWizardState::open(ctx, dir.path(), &dir.path().join("versions.json"));

        assert_eq!(state.step, LaunchWizardStep::QuickStart);
        assert_eq!(state.quick_start_entries.len(), 1);
        assert!(state.quick_start_entry_can_reuse(&state.quick_start_entries[0]));
        assert_eq!(
            state.quick_start_reuse_action_label(&state.quick_start_entries[0]),
            Some("Resume")
        );
        assert!(matches!(
            state.quick_start_actions().as_slice(),
            [
                QuickStartAction::ReuseEntry { index: 0 },
                QuickStartAction::StartNewEntry { index: 0 },
                QuickStartAction::ChooseDifferent
            ]
        ));
        assert!(matches!(
            state.selected_quick_start_action(),
            QuickStartAction::ReuseEntry { index: 0 }
        ));
        assert_eq!(
            state
                .selected_quick_start_entry()
                .map(|entry| entry.agent_id.as_str()),
            Some("codex")
        );
        assert_eq!(
            state.view().quick_start_entries[0]
                .reuse_action_label
                .as_deref(),
            Some("Resume")
        );

        let mut loading = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        loading.set_hydration_error("network failed".to_string());
        assert!(!loading.is_hydrating);
        assert_eq!(loading.hydration_error.as_deref(), Some("network failed"));

        let mut resumable = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            state.quick_start_entries,
        );
        resumable.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });
        assert!(matches!(
            resumable.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.resume_session_id.as_deref() == Some("resume-1")
                )
        ));

        let mut missing = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-2",
                "codex",
                None,
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );
        missing.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });
        assert_eq!(
            missing.error.as_deref(),
            Some("No saved session is available")
        );
    }
}

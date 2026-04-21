use super::*;

pub(crate) fn combined_window_id(tab_id: &str, raw_id: &str) -> String {
    format!("{tab_id}::{raw_id}")
}

pub(crate) fn should_auto_close_agent_window(
    active_agent_sessions: &HashMap<String, ActiveAgentSession>,
    window_id: &str,
    status: &WindowProcessStatus,
) -> bool {
    matches!(status, WindowProcessStatus::Exited) && active_agent_sessions.contains_key(window_id)
}

pub(crate) fn close_window_from_workspace(
    tabs: &mut [ProjectTabRuntime],
    window_lookup: &mut HashMap<String, WindowAddress>,
    window_details: &mut HashMap<String, String>,
    id: &str,
) -> bool {
    let Some(address) = window_lookup.get(id).cloned() else {
        return false;
    };
    let Some(tab) = tabs.iter_mut().find(|tab| tab.id == address.tab_id) else {
        return false;
    };
    if !tab.workspace.close_window(&address.raw_id) {
        return false;
    }
    window_lookup.remove(id);
    window_details.remove(id);
    true
}

pub(crate) fn should_auto_start_restored_window(window: &gwt::PersistedWindowState) -> bool {
    window.preset.requires_process()
        && matches!(
            window.status,
            WindowProcessStatus::Starting | WindowProcessStatus::Running
        )
}

pub(crate) fn current_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub(crate) fn workspace_view_for_tab(tab: &ProjectTabRuntime) -> gwt::WorkspaceView {
    gwt::WorkspaceView {
        viewport: tab.workspace.persisted().viewport.clone(),
        windows: tab
            .workspace
            .persisted()
            .windows
            .iter()
            .cloned()
            .map(|mut window| {
                window.id = combined_window_id(&tab.id, &window.id);
                window
            })
            .collect(),
    }
}

pub(crate) fn app_state_view_from_parts(
    tabs: &[ProjectTabRuntime],
    active_tab_id: Option<&str>,
    recent_projects: &[gwt::RecentProjectEntry],
) -> gwt::AppStateView {
    gwt::AppStateView {
        app_version: current_app_version().to_string(),
        tabs: tabs
            .iter()
            .map(|tab| gwt::ProjectTabView {
                id: tab.id.clone(),
                title: tab.title.clone(),
                project_root: tab.project_root.display().to_string(),
                kind: tab.kind,
                workspace: workspace_view_for_tab(tab),
            })
            .collect(),
        active_tab_id: active_tab_id.map(str::to_owned),
        recent_projects: recent_projects
            .iter()
            .map(|project| gwt::RecentProjectView {
                path: project.path.display().to_string(),
                title: project.title.clone(),
                kind: project.kind,
            })
            .collect(),
    }
}

pub(crate) fn normalize_active_tab_id(
    tabs: &[ProjectTabRuntime],
    active_tab_id: Option<String>,
) -> Option<String> {
    let Some(active_tab_id) = active_tab_id else {
        return tabs.first().map(|tab| tab.id.clone());
    };
    if tabs.iter().any(|tab| tab.id == active_tab_id) {
        Some(active_tab_id)
    } else {
        tabs.first().map(|tab| tab.id.clone())
    }
}

pub(crate) fn dedupe_recent_projects(
    entries: Vec<gwt::RecentProjectEntry>,
) -> Vec<gwt::RecentProjectEntry> {
    let mut deduped: Vec<gwt::RecentProjectEntry> = Vec::new();
    for entry in entries {
        if deduped
            .iter()
            .any(|existing| same_worktree_path(&existing.path, &entry.path))
        {
            continue;
        }
        deduped.push(entry);
    }
    deduped
}

pub(crate) fn fallback_project_target(path: PathBuf) -> ProjectOpenTarget {
    ProjectOpenTarget {
        title: gwt::project_title_from_path(&path),
        project_root: path,
        kind: gwt::ProjectKind::NonRepo,
    }
}

pub(crate) fn resolve_project_target(path: &Path) -> Result<ProjectOpenTarget, String> {
    let canonical = dunce::canonicalize(path)
        .map_err(|error| format!("failed to open project {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "selected project is not a directory: {}",
            canonical.display()
        ));
    }
    let title = gwt::project_title_from_path(&canonical);

    let (project_root, kind) = match gwt_git::detect_repo_type(&canonical) {
        gwt_git::RepoType::Normal(root) => (
            dunce::canonicalize(root).unwrap_or_else(|_| canonical.clone()),
            gwt::ProjectKind::Git,
        ),
        gwt_git::RepoType::Bare {
            develop_worktree: Some(worktree),
        } => (
            dunce::canonicalize(worktree).unwrap_or_else(|_| canonical.clone()),
            gwt::ProjectKind::Git,
        ),
        gwt_git::RepoType::Bare {
            develop_worktree: None,
        } => (canonical.clone(), gwt::ProjectKind::Bare),
        gwt_git::RepoType::NonRepo => (canonical.clone(), gwt::ProjectKind::NonRepo),
    };

    Ok(ProjectOpenTarget {
        title,
        project_root,
        kind,
    })
}

pub(crate) fn normalize_branch_name(branch_name: &str) -> String {
    if let Some(name) = branch_name.strip_prefix("refs/remotes/") {
        return name.strip_prefix("origin/").unwrap_or(name).to_string();
    }
    if let Some(name) = branch_name.strip_prefix("origin/") {
        return name.to_string();
    }
    branch_name.to_string()
}

pub(crate) fn synthetic_branch_entry(branch_name: &str) -> BranchListEntry {
    BranchListEntry {
        name: branch_name.to_string(),
        scope: gwt::BranchScope::Local,
        is_head: false,
        upstream: None,
        ahead: 0,
        behind: 0,
        last_commit_date: None,
        cleanup_ready: true,
        cleanup: gwt::BranchCleanupInfo::default(),
    }
}

pub(crate) fn resolve_launch_wizard_hydration(
    project_root: &Path,
    branch_name: &str,
    active_session_branches: &std::collections::HashSet<String>,
    sessions_dir: &Path,
) -> Result<LaunchWizardHydration, String> {
    let agent_options = build_builtin_agent_options(
        gwt_agent::AgentDetector::detect_all(),
        &gwt_agent::VersionCache::load(&default_wizard_version_cache_path()),
    );
    let entries = list_branch_entries_with_active_sessions(project_root, active_session_branches)
        .map_err(|error| error.to_string())?;
    let selected_branch = entries
        .into_iter()
        .find(|entry| entry.name == branch_name)
        .ok_or_else(|| format!("Branch not found: {branch_name}"))?;
    let normalized_branch_name = normalize_branch_name(&selected_branch.name);
    let worktree_path = branch_worktree_path(project_root, &normalized_branch_name);
    let quick_start_root = worktree_path
        .clone()
        .unwrap_or_else(|| project_root.to_path_buf());
    let quick_start_entries = gwt::launch_wizard::load_quick_start_entries(
        &quick_start_root,
        sessions_dir,
        &normalized_branch_name,
    );
    let (docker_context, docker_service_status) =
        detect_wizard_docker_context_and_status(&quick_start_root);

    Ok(LaunchWizardHydration {
        selected_branch: Some(selected_branch),
        normalized_branch_name,
        worktree_path,
        quick_start_root,
        docker_context,
        docker_service_status,
        agent_options,
        quick_start_entries,
    })
}

pub(crate) fn knowledge_kind_for_preset(preset: WindowPreset) -> Option<KnowledgeKind> {
    match preset {
        WindowPreset::Issue => Some(KnowledgeKind::Issue),
        WindowPreset::Spec => Some(KnowledgeKind::Spec),
        WindowPreset::Pr => Some(KnowledgeKind::Pr),
        _ => None,
    }
}

pub(crate) fn branch_worktree_path(repo_path: &Path, branch_name: &str) -> Option<PathBuf> {
    if current_git_branch(repo_path)
        .as_ref()
        .is_ok_and(|current| current == branch_name)
    {
        return Some(repo_path.to_path_buf());
    }

    let main_repo_path = gwt_git::worktree::main_worktree_root(repo_path).ok()?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    manager
        .list()
        .ok()?
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name))
        .map(|worktree| worktree.path)
}

pub(crate) fn first_available_worktree_path(
    preferred_path: &Path,
    worktrees: &[gwt_git::WorktreeInfo],
) -> Option<PathBuf> {
    if !worktree_path_is_occupied(preferred_path, worktrees) && !preferred_path.exists() {
        return Some(preferred_path.to_path_buf());
    }

    for suffix in 2usize.. {
        let candidate = suffixed_worktree_path(preferred_path, suffix)?;
        if !worktree_path_is_occupied(&candidate, worktrees) && !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

pub(crate) fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

pub(crate) fn worktree_path_is_occupied(path: &Path, worktrees: &[gwt_git::WorktreeInfo]) -> bool {
    worktrees
        .iter()
        .any(|worktree| same_worktree_path(&worktree.path, path))
}

pub(crate) fn same_worktree_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

pub(crate) fn origin_remote_ref(branch_name: &str) -> String {
    if let Some(ref_name) = branch_name.strip_prefix("refs/remotes/") {
        ref_name.to_string()
    } else if branch_name.starts_with("origin/") {
        branch_name.to_string()
    } else {
        format!("origin/{branch_name}")
    }
}

pub(crate) fn current_git_branch(repo_path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("git branch --show-current: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git branch --show-current: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        Err("git branch --show-current returned an empty branch name".to_string())
    } else {
        Ok(branch)
    }
}

pub(crate) fn local_branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch_name}"),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("git show-ref --verify refs/heads/{branch_name}: {err}"))?;
    Ok(output.status.success())
}

pub(crate) fn resolve_launch_spec_with_fallback(
    preset: WindowPreset,
    shell: &gwt::ShellProgram,
) -> Result<gwt::LaunchSpec, gwt::PresetResolveError> {
    let _ = shell;
    resolve_launch_spec(preset)
}

pub(crate) fn spawn_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env
}

pub(crate) fn geometry_to_pty_size(geometry: &WindowGeometry) -> (u16, u16) {
    let cols = ((geometry.width.max(420.0) - 26.0) / 8.4).floor() as u16;
    let rows = ((geometry.height.max(260.0) - 58.0) / 18.0).floor() as u16;
    (cols.max(20), rows.max(6))
}

pub(crate) fn run_cli(argv: &[String]) -> io::Result<()> {
    let needs_repo = matches!(
        argv.get(1).map(String::as_str),
        Some("issue" | "pr" | "actions")
    );

    if needs_repo {
        let repo_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (owner, repo) = match resolve_repo_coordinates() {
            Some(coords) => coords,
            None => {
                eprintln!(
                    "gwt {}: could not resolve GitHub owner/repo from the current git remote",
                    argv.get(1).map(String::as_str).unwrap_or("issue")
                );
                std::process::exit(2);
            }
        };
        let mut env = gwt::cli::DefaultCliEnv::new(&owner, &repo, repo_path);
        std::process::exit(gwt::cli::dispatch(&mut env, argv));
    }

    let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
    std::process::exit(gwt::cli::dispatch(&mut env, argv));
}

pub(crate) fn resolve_repo_coordinates() -> Option<(String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_remote_url(&url)
}

pub(crate) fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let trimmed = rest.trim_end_matches(".git");
        let mut parts = trimmed.splitn(2, '/');
        let owner = parts.next()?.to_string();
        let repo = parts.next()?.to_string();
        return Some((owner, repo));
    }

    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "git://github.com/",
    ] {
        if let Some(rest) = url.strip_prefix(prefix) {
            let trimmed = rest.trim_end_matches(".git").trim_end_matches('/');
            let mut parts = trimmed.splitn(2, '/');
            let owner = parts.next()?.to_string();
            let repo = parts.next()?.to_string();
            return Some((owner, repo));
        }
    }

    None
}

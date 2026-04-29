use super::*;
use std::collections::BTreeSet;

pub(crate) fn combined_window_id(tab_id: &str, raw_id: &str) -> String {
    format!("{tab_id}::{raw_id}")
}

pub(crate) fn should_auto_close_agent_window(
    active_agent_sessions: &HashMap<String, ActiveAgentSession>,
    window_id: &str,
    status: &WindowProcessStatus,
) -> bool {
    matches!(status, WindowProcessStatus::Stopped) && active_agent_sessions.contains_key(window_id)
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
    window.preset.requires_process() && window.status == WindowProcessStatus::Running
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
    let agent_options = load_agent_options(&gwt_agent::VersionCache::load(
        &default_wizard_version_cache_path(),
    ));
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
    let previous_profile =
        gwt::launch_wizard::load_previous_launch_profile(&quick_start_root, sessions_dir);
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
        previous_profile,
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
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git show-ref --verify refs/heads/{branch_name} in {} failed with status {}: {}",
            repo_path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

pub(crate) fn resolve_launch_spec_with_fallback(
    preset: WindowPreset,
    shell: &gwt::ShellProgram,
) -> Result<gwt::LaunchSpec, gwt::PresetResolveError> {
    let _ = shell;
    resolve_launch_spec(preset)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffectiveSpawnEnv {
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
}

pub(crate) fn spawn_env() -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    ensure_terminal_env(&mut env);
    env
}

pub(crate) fn active_profile_spawn_env_at<I>(
    config_path: &Path,
    base_env: I,
) -> Result<EffectiveSpawnEnv, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut settings = if config_path.exists() {
        gwt_config::Settings::load_from_path(config_path).map_err(|error| error.to_string())?
    } else {
        gwt_config::Settings::default()
    };
    let active_name = settings.profiles.normalize_active_profile().name;
    let Some(profile) = settings.profiles.get(&active_name) else {
        return Err(format!("active profile not found: {active_name}"));
    };

    let mut env = profile
        .merged_env_pairs(base_env)
        .into_iter()
        .collect::<HashMap<_, _>>();
    ensure_terminal_env(&mut env);

    Ok(EffectiveSpawnEnv {
        env,
        remove_env: normalized_remove_env(&profile.disabled_env),
    })
}

pub(crate) fn active_profile_spawn_env(config_path: &Path) -> Result<EffectiveSpawnEnv, String> {
    active_profile_spawn_env_at(config_path, std::env::vars())
}

pub(crate) fn active_profile_launch_env(
    config_path: &Path,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> Result<EffectiveSpawnEnv, String> {
    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => active_profile_spawn_env(config_path),
        gwt_agent::LaunchRuntimeTarget::Docker => {
            active_profile_spawn_env_at(config_path, std::iter::empty::<(String, String)>())
        }
    }
}

pub(crate) fn apply_effective_spawn_env(
    env_vars: &mut HashMap<String, String>,
    remove_env: &mut Vec<String>,
    effective: EffectiveSpawnEnv,
) {
    let explicit_env = std::mem::take(env_vars);
    *env_vars = effective.env;
    env_vars.extend(explicit_env);
    merge_remove_env(remove_env, effective.remove_env);
}

fn ensure_terminal_env(env: &mut HashMap<String, String>) {
    env.entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
}

fn merge_remove_env(remove_env: &mut Vec<String>, additional: Vec<String>) {
    let keys = remove_env
        .iter()
        .chain(additional.iter())
        .filter_map(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<BTreeSet<_>>();
    *remove_env = keys.into_iter().collect();
}

fn normalized_remove_env(disabled_env: &[String]) -> Vec<String> {
    let keys = disabled_env
        .iter()
        .filter_map(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect::<BTreeSet<_>>();
    keys.into_iter().collect()
}

pub(crate) fn geometry_to_pty_size(geometry: &WindowGeometry) -> (u16, u16) {
    let cols = ((geometry.width.max(420.0) - 26.0) / 8.4).floor() as u16;
    let rows = ((geometry.height.max(260.0) - 58.0) / 18.0).floor() as u16;
    (cols.max(20), rows.max(6))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontDoorRoute {
    Gui,
    RepoBackedCli,
    DetachedCli,
}

pub(crate) fn front_door_route(argv: &[String]) -> FrontDoorRoute {
    match argv.get(1).map(String::as_str) {
        Some("issue" | "pr" | "actions") => FrontDoorRoute::RepoBackedCli,
        Some(top_verb) if gwt::cli::should_dispatch_cli(argv) => {
            debug_assert!(matches!(
                top_verb,
                "board" | "index" | "hook" | "discuss" | "plan" | "build" | "update" | "__internal"
            ));
            FrontDoorRoute::DetachedCli
        }
        _ => FrontDoorRoute::Gui,
    }
}

#[cfg(windows)]
pub(crate) fn attach_parent_console_for_cli() {
    windows_console::attach_parent_console_for_cli();
}

#[cfg(not(windows))]
pub(crate) fn attach_parent_console_for_cli() {}

pub(crate) fn run_cli(argv: &[String]) -> io::Result<()> {
    match front_door_route(argv) {
        FrontDoorRoute::RepoBackedCli => {
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
        FrontDoorRoute::DetachedCli => {
            let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
            std::process::exit(gwt::cli::dispatch(&mut env, argv));
        }
        FrontDoorRoute::Gui => Ok(()),
    }
}

#[cfg(windows)]
mod windows_console {
    use std::{
        ffi::{c_void, OsStr},
        os::windows::ffi::OsStrExt,
        ptr,
    };

    type Handle = *mut c_void;

    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const OPEN_EXISTING: u32 = 3;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn CreateFileW(
            lp_file_name: *const u16,
            dw_desired_access: u32,
            dw_share_mode: u32,
            lp_security_attributes: *mut c_void,
            dw_creation_disposition: u32,
            dw_flags_and_attributes: u32,
            h_template_file: Handle,
        ) -> Handle;
        fn GetStdHandle(n_std_handle: u32) -> Handle;
        fn SetStdHandle(n_std_handle: u32, h_handle: Handle) -> i32;
    }

    pub(super) fn attach_parent_console_for_cli() {
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            restore_standard_handle(STD_INPUT_HANDLE, "CONIN$", GENERIC_READ);
            restore_standard_handle(STD_OUTPUT_HANDLE, "CONOUT$", GENERIC_WRITE);
            restore_standard_handle(STD_ERROR_HANDLE, "CONOUT$", GENERIC_WRITE);
        }
    }

    unsafe fn restore_standard_handle(kind: u32, device: &str, access: u32) {
        if !is_invalid_handle(GetStdHandle(kind)) {
            return;
        }

        let device = wide_null(device);
        let handle = CreateFileW(
            device.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        );
        if !is_invalid_handle(handle) {
            let _ = SetStdHandle(kind, handle);
        }
    }

    fn is_invalid_handle(handle: Handle) -> bool {
        handle.is_null() || handle as isize == -1
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain([0]).collect()
    }
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

#[cfg(test)]
mod tests {
    use super::{front_door_route, FrontDoorRoute};

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn front_door_route_keeps_gui_launch_for_empty_and_repo_path_argv() {
        for args in [
            argv(&["gwt"]),
            argv(&["gwt", "E:/gwt/repo"]),
            argv(&["gwt", "."]),
        ] {
            assert_eq!(front_door_route(&args), FrontDoorRoute::Gui);
            assert!(
                !gwt::cli::should_dispatch_cli(&args),
                "GUI launch argv must not fall through to CLI dispatch: {args:?}"
            );
        }
    }

    #[test]
    fn front_door_route_keeps_repo_backed_issue_pr_and_actions_commands_on_cli_path() {
        for args in [
            argv(&["gwt", "issue", "spec", "1784", "--section", "tasks"]),
            argv(&["gwt", "issue", "view", "1784", "--refresh"]),
            argv(&["gwt", "pr", "current"]),
            argv(&["gwt", "actions", "logs", "--run", "101"]),
        ] {
            assert_eq!(front_door_route(&args), FrontDoorRoute::RepoBackedCli);
            assert!(
                gwt::cli::should_dispatch_cli(&args),
                "repo-backed tooling must stay on the CLI path: {args:?}"
            );
        }
    }

    #[test]
    fn front_door_route_keeps_detached_helper_commands_on_cli_path() {
        for args in [
            argv(&["gwt", "board", "show", "--json"]),
            argv(&["gwt", "hook", "runtime-state", "PreToolUse"]),
            argv(&["gwt", "discuss", "resolve", "--proposal", "Resume"]),
            argv(&["gwt", "plan", "start", "--spec", "1935"]),
            argv(&["gwt", "build", "complete", "--spec", "1935"]),
            argv(&["gwt", "update", "--check"]),
            argv(&["gwt", "__internal", "daemon-hook", "forward"]),
        ] {
            assert_eq!(front_door_route(&args), FrontDoorRoute::DetachedCli);
            assert!(
                gwt::cli::should_dispatch_cli(&args),
                "non-GUI helper tooling must stay on the CLI path: {args:?}"
            );
        }
    }
}

use super::*;

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

/// Issue #1678: drop recent project entries whose paths no longer exist on
/// disk. Called once at load time so subsequent `persist()` writes a clean
/// list back. A separate function (rather than baked into `dedupe_*`) so
/// tests and callers can exercise the path predicate without filesystem.
pub(crate) fn prune_missing_recent_projects(
    entries: Vec<gwt::RecentProjectEntry>,
) -> Vec<gwt::RecentProjectEntry> {
    prune_missing_recent_projects_with(entries, |path| path.exists())
}

pub(crate) fn prune_missing_recent_projects_with(
    entries: Vec<gwt::RecentProjectEntry>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<gwt::RecentProjectEntry> {
    entries
        .into_iter()
        .filter(|entry| exists(&entry.path))
        .collect()
}

pub(crate) fn fallback_project_target(path: PathBuf) -> ProjectOpenTarget {
    ProjectOpenTarget {
        title: gwt::project_title_from_path(&path),
        project_root: path,
        kind: gwt::ProjectKind::NonRepo,
        needs_migration: false,
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

    let (project_root, kind, needs_migration) = match gwt_git::detect_repo_type(&canonical) {
        gwt_git::RepoType::Normal {
            path: root,
            needs_migration,
        } => (
            dunce::canonicalize(root).unwrap_or_else(|_| canonical.clone()),
            gwt::ProjectKind::Git,
            needs_migration,
        ),
        gwt_git::RepoType::Bare {
            develop_worktree: Some(worktree),
        } => (
            dunce::canonicalize(worktree).unwrap_or_else(|_| canonical.clone()),
            gwt::ProjectKind::Git,
            false,
        ),
        gwt_git::RepoType::Bare {
            develop_worktree: None,
        } => (canonical.clone(), gwt::ProjectKind::Bare, false),
        gwt_git::RepoType::NonRepo => (canonical.clone(), gwt::ProjectKind::NonRepo, false),
    };

    Ok(ProjectOpenTarget {
        title,
        project_root,
        kind,
        needs_migration,
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
    let output = gwt_core::process::hidden_command("git")
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
    let output = gwt_core::process::hidden_command("git")
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

#[cfg(test)]
pub(crate) fn spawn_env() -> HashMap<String, String> {
    let (env, _) =
        gwt_agent::LaunchEnvironment::from_base_env(gwt_agent::environment::host_process_env())
            .into_parts();
    env
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
    // Issue #2054: support multi-remote git repos where `origin` points at a
    // local mirror and the GitHub URL lives under a different remote name.
    // Resolution order:
    //   1. `GWT_GITHUB_REPO=owner/name` direct override
    //   2. `GWT_REMOTE=<name>` selects the remote to read
    //   3. `origin` remote URL (legacy default)
    //   4. Scan all remotes and pick the first GitHub URL we find
    select_repo_coordinates(&load_remote_urls(), &repo_env_overrides())
}

/// Pure resolver kept independent of git invocation so it can be unit-tested
/// against synthetic remote fixtures.
pub(crate) fn select_repo_coordinates(
    remotes: &[(String, String)],
    overrides: &RepoEnvOverrides,
) -> Option<(String, String)> {
    if let Some(direct) = overrides
        .github_repo
        .as_deref()
        .and_then(parse_owner_repo_pair)
    {
        return Some(direct);
    }

    if let Some(name) = overrides.remote.as_deref() {
        if let Some((_, url)) = remotes
            .iter()
            .find(|(remote_name, _)| remote_name.as_str() == name)
        {
            if let Some(parsed) = parse_github_remote_url(url) {
                return Some(parsed);
            }
        }
    }

    if let Some((_, url)) = remotes.iter().find(|(name, _)| name.as_str() == "origin") {
        if let Some(parsed) = parse_github_remote_url(url) {
            return Some(parsed);
        }
    }

    remotes
        .iter()
        .find_map(|(_, url)| parse_github_remote_url(url))
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RepoEnvOverrides {
    pub github_repo: Option<String>,
    pub remote: Option<String>,
}

fn repo_env_overrides() -> RepoEnvOverrides {
    RepoEnvOverrides {
        github_repo: std::env::var("GWT_GITHUB_REPO")
            .ok()
            .filter(|v| !v.is_empty()),
        remote: std::env::var("GWT_REMOTE").ok().filter(|v| !v.is_empty()),
    }
}

fn load_remote_urls() -> Vec<(String, String)> {
    let output = gwt_core::process::hidden_command("git")
        .args(["remote", "-v"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_git_remote_v(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_git_remote_v(text: &str) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(url) = parts.next() else { continue };
        // The third token is "(fetch)" / "(push)". `git remote -v` lists each
        // remote twice; dedupe so callers see a single entry per name.
        if !seen.insert(name.to_string()) {
            continue;
        }
        out.push((name.to_string(), url.to_string()));
    }
    out
}

fn parse_owner_repo_pair(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, '/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
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
    use std::path::PathBuf;

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

    #[test]
    fn resolve_project_target_marks_needs_migration_for_normal_repo() {
        // SPEC-1934 US-6 / FR-019: Normal Git layout must propagate the
        // migration flag so the GUI can show the confirmation modal at startup.
        let tmp = tempfile::tempdir().expect("tempdir");
        gwt_core::process::hidden_command("git")
            .args(["init", tmp.path().to_str().unwrap()])
            .output()
            .expect("git init");

        let target = super::resolve_project_target(tmp.path()).expect("normal target");
        assert_eq!(target.kind, gwt::ProjectKind::Git);
        assert!(
            target.needs_migration,
            "Normal Git layout must surface needs_migration=true"
        );
    }

    #[test]
    fn fallback_project_target_does_not_set_needs_migration() {
        let target =
            super::fallback_project_target(std::path::PathBuf::from("/tmp/_does_not_exist"));
        assert!(!target.needs_migration);
    }

    #[test]
    fn resolve_project_target_for_bare_layout_does_not_request_migration() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare = tmp.path().join("repo.git");
        gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .output()
            .expect("git init bare");

        let target = super::resolve_project_target(tmp.path()).expect("bare target");
        assert!(
            !target.needs_migration,
            "Bare layout must not request migration"
        );
    }

    // -------------------------------------------------------------------
    // Issue #2054: gwt pr remote resolution must tolerate non-GitHub
    // `origin` and explicit env overrides.
    // -------------------------------------------------------------------

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn remotes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(name, url)| (s(name), s(url))).collect()
    }

    #[test]
    fn select_repo_coordinates_prefers_origin_when_it_is_github() {
        let coords = super::select_repo_coordinates(
            &remotes(&[
                ("origin", "https://github.com/akiojin/gwt"),
                ("upstream", "https://github.com/anthropics/example"),
            ]),
            &super::RepoEnvOverrides::default(),
        );
        assert_eq!(coords, Some((s("akiojin"), s("gwt"))));
    }

    #[test]
    fn select_repo_coordinates_falls_back_to_other_remote_when_origin_is_local_mirror() {
        // The exact scenario from issue #2054: origin points at a local bare
        // mirror, and the actual GitHub URL is registered under a different
        // remote name (here `github`).
        let coords = super::select_repo_coordinates(
            &remotes(&[
                ("origin", "E:/llmlb/llmlb.git"),
                ("github", "https://github.com/akiojin/llmlb"),
            ]),
            &super::RepoEnvOverrides::default(),
        );
        assert_eq!(coords, Some((s("akiojin"), s("llmlb"))));
    }

    #[test]
    fn select_repo_coordinates_honours_remote_env_override() {
        // GWT_REMOTE=upstream should redirect resolution even if origin is a
        // perfectly valid GitHub URL.
        let coords = super::select_repo_coordinates(
            &remotes(&[
                ("origin", "https://github.com/akiojin/gwt"),
                ("upstream", "git@github.com:anthropics/example.git"),
            ]),
            &super::RepoEnvOverrides {
                github_repo: None,
                remote: Some(s("upstream")),
            },
        );
        assert_eq!(coords, Some((s("anthropics"), s("example"))));
    }

    #[test]
    fn select_repo_coordinates_honours_github_repo_env_override() {
        // GWT_GITHUB_REPO trumps every remote; useful when no GitHub remote
        // is registered locally but the user knows the slug.
        let coords = super::select_repo_coordinates(
            &remotes(&[("origin", "E:/llmlb/llmlb.git")]),
            &super::RepoEnvOverrides {
                github_repo: Some(s("akiojin/llmlb")),
                remote: None,
            },
        );
        assert_eq!(coords, Some((s("akiojin"), s("llmlb"))));
    }

    #[test]
    fn select_repo_coordinates_returns_none_when_no_github_remote_or_override() {
        let coords = super::select_repo_coordinates(
            &remotes(&[
                ("origin", "E:/llmlb/llmlb.git"),
                ("backup", "/srv/git/llmlb.git"),
            ]),
            &super::RepoEnvOverrides::default(),
        );
        assert_eq!(coords, None);
    }

    #[test]
    fn prune_missing_recent_projects_drops_entries_whose_paths_are_gone() {
        // Issue #1678: stale entries must be removed before the next persist
        // round-trip so disk state stops referring to deleted projects.
        let exists_paths: std::collections::HashSet<String> = ["/tmp/exists-a", "/tmp/exists-b"]
            .iter()
            .map(|p| (*p).to_string())
            .collect();
        let entries = vec![
            gwt::RecentProjectEntry {
                path: PathBuf::from("/tmp/exists-a"),
                title: "alive a".to_string(),
                kind: gwt::ProjectKind::Git,
            },
            gwt::RecentProjectEntry {
                path: PathBuf::from("/tmp/missing-x"),
                title: "deleted x".to_string(),
                kind: gwt::ProjectKind::Git,
            },
            gwt::RecentProjectEntry {
                path: PathBuf::from("/tmp/exists-b"),
                title: "alive b".to_string(),
                kind: gwt::ProjectKind::Bare,
            },
        ];

        let pruned = super::prune_missing_recent_projects_with(entries, |path| {
            exists_paths.contains(&path.to_string_lossy().to_string())
        });

        assert_eq!(pruned.len(), 2);
        assert_eq!(pruned[0].title, "alive a");
        assert_eq!(pruned[1].title, "alive b");
    }

    #[test]
    fn prune_missing_recent_projects_returns_input_when_all_paths_exist() {
        let entries = vec![gwt::RecentProjectEntry {
            path: PathBuf::from("/tmp/here"),
            title: "alive".to_string(),
            kind: gwt::ProjectKind::Git,
        }];
        let pruned = super::prune_missing_recent_projects_with(entries.clone(), |_| true);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].path, entries[0].path);
    }

    #[test]
    fn parse_git_remote_v_dedupes_fetch_and_push_lines() {
        let stdout = "\
origin\thttps://github.com/akiojin/gwt (fetch)
origin\thttps://github.com/akiojin/gwt (push)
upstream\tgit@github.com:anthropics/example.git (fetch)
upstream\tgit@github.com:anthropics/example.git (push)
";
        let parsed = super::parse_git_remote_v(stdout);
        assert_eq!(
            parsed,
            vec![
                (s("origin"), s("https://github.com/akiojin/gwt")),
                (s("upstream"), s("git@github.com:anthropics/example.git")),
            ]
        );
    }
}

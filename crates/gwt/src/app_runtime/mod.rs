use super::*;
use gwt::LaunchWizardAction;

#[derive(Clone)]
pub enum AppEventProxy {
    Real(EventLoopProxy<UserEvent>),
    #[cfg(test)]
    Stub(Arc<Mutex<Vec<UserEvent>>>),
}

impl AppEventProxy {
    pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self::Real(proxy)
    }

    pub(crate) fn send(&self, event: UserEvent) {
        match self {
            Self::Real(proxy) => {
                let _ = proxy.send_event(event);
            }
            #[cfg(test)]
            Self::Stub(events) => {
                if let Ok(mut events) = events.lock() {
                    events.push(event);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn stub() -> (Self, Arc<Mutex<Vec<UserEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Self::Stub(events.clone()), events)
    }
}

#[derive(Clone)]
pub enum BlockingTaskSpawner {
    Tokio(tokio::runtime::Handle),
    #[cfg(test)]
    Thread,
}

impl BlockingTaskSpawner {
    pub(crate) fn tokio(handle: tokio::runtime::Handle) -> Self {
        Self::Tokio(handle)
    }

    #[cfg(test)]
    pub(crate) fn thread() -> Self {
        Self::Thread
    }

    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        match self {
            Self::Tokio(handle) => {
                drop(handle.spawn_blocking(task));
            }
            #[cfg(test)]
            Self::Thread => {
                thread::Builder::new()
                    .name("gwt-blocking-task".to_string())
                    .spawn(task)
                    .expect("spawn test blocking task");
            }
        }
    }
}

pub struct KnowledgeSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) selected_number: Option<u64>,
}

pub struct KnowledgeLoadRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) request_id: Option<u64>,
    pub(crate) selected_number: Option<u64>,
    pub(crate) refresh: bool,
}

pub struct ProjectIndexSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) scopes: Vec<gwt::IndexSearchScope>,
    pub(crate) worktree_hash: Option<String>,
    pub(crate) match_mode: gwt::IndexSearchMatchMode,
}

struct KnowledgeRefreshTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    selected_number: Option<u64>,
    force: bool,
}

struct KnowledgeSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    query: String,
    request_id: u64,
    selected_number: Option<u64>,
}

struct ProjectIndexSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    query: String,
    request_id: u64,
    scopes: Vec<gwt::IndexSearchScope>,
    worktree_hash: Option<String>,
    match_mode: gwt::IndexSearchMatchMode,
}

pub struct WindowRuntime {
    pane: Arc<Mutex<Pane>>,
    /// Handle to the background reader thread that forwards PTY output.
    /// Taken and joined during `stop_window_runtime` so the reader releases
    /// its Arc clone of `pane` before the runtime is fully torn down.
    output_thread: Option<JoinHandle<()>>,
    /// Handle to the process status watcher. It is independent from PTY EOF
    /// because some agent exits can leave the terminal reader waiting even
    /// after the direct child has finished.
    status_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
}

pub type AgentLaunchCompletion = (
    ProcessLaunch,
    String,
    String,
    String,
    PathBuf,
    gwt_agent::AgentId,
    Option<u64>,
    Option<String>,
    gwt_agent::LaunchRuntimeTarget,
    String,
);

pub type AgentLaunchResult = Result<AgentLaunchCompletion, String>;

mod board;
mod migration;
pub(crate) mod persist_dispatcher;
mod profile;
mod runtime_events;
mod title_sync;
mod ui_trace;
mod window;
mod wizard;
pub use board::BoardPostRequest;
use profile::ProfileSaveRequest;
use ui_trace::save_ui_trace_to_log_dir;

fn dispatch_agent_launch_success<F>(
    proxy: AppEventProxy,
    window_id: String,
    completion: AgentLaunchCompletion,
    spawn_project_index_bootstrap: F,
) where
    F: FnOnce(AppEventProxy, PathBuf),
{
    let project_index_root = completion.4.clone();
    proxy.send(UserEvent::LaunchComplete {
        window_id,
        result: Ok(completion),
    });
    spawn_project_index_bootstrap(proxy, project_index_root);
}

fn launch_config_from_persisted_session(session: &gwt_agent::Session) -> gwt_agent::LaunchConfig {
    let agent_id = session.agent_id.clone();
    let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id);
    builder = builder.working_dir(session.worktree_path.clone());
    if !session.branch.is_empty() {
        builder = builder.branch(session.branch.clone());
    }
    if let Some(model) = session.model.clone() {
        builder = builder.model(model);
    }
    if let Some(version) = session.tool_version.clone() {
        builder = builder.version(version);
    }
    if let Some(level) = session.reasoning_level.clone() {
        builder = builder.reasoning_level(level);
    }
    if session.skip_permissions {
        builder = builder.skip_permissions(true);
    }
    if session.codex_fast_mode {
        builder = builder.fast_mode(true);
    }
    builder = builder.runtime_target(session.runtime_target);
    if let Some(service) = session.docker_service.clone() {
        builder = builder.docker_service(service);
    }
    builder = builder.docker_lifecycle_intent(session.docker_lifecycle_intent);
    if let Some(shell) = session.windows_shell {
        builder = builder.windows_shell(shell);
    }
    if let Some(linked) = session.linked_issue_number {
        builder = builder.linked_issue_number(linked);
    }

    if let Some(resume_id) = session.exact_resume_session_id() {
        builder = builder
            .session_mode(gwt_agent::SessionMode::Resume)
            .resume_session_id(resume_id.to_string());
    } else {
        builder = builder.session_mode(gwt_agent::SessionMode::Normal);
    }

    let mut config = builder.build();
    if let Some(version) = session.tool_version.clone() {
        config.tool_version = Some(version);
    }
    if !session.display_name.is_empty() {
        config.display_name = session.display_name.clone();
    }
    config
}

const STARTUP_AUTO_RESUME_STALE_AFTER_SECS: i64 = 24 * 60 * 60;
const STARTUP_AUTO_RESUME_STACK_OFFSET_X: f64 = 28.0;
const STARTUP_AUTO_RESUME_STACK_OFFSET_Y: f64 = 24.0;

fn startup_auto_resume_window_geometry(
    index: usize,
    total: usize,
    bounds: gwt::WindowGeometry,
) -> gwt::WindowGeometry {
    let (width, height) = WindowPreset::Agent.default_size();
    let stack_steps = total.saturating_sub(1) as f64;
    let index = index as f64;
    gwt::WindowGeometry {
        x: bounds.x + (bounds.width - width) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_X) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_X,
        y: bounds.y + (bounds.height - height) / 2.0
            - (stack_steps * STARTUP_AUTO_RESUME_STACK_OFFSET_Y) / 2.0
            + index * STARTUP_AUTO_RESUME_STACK_OFFSET_Y,
        width,
        height,
    }
}

fn session_project_scope_hash(session: &gwt_agent::Session) -> Option<String> {
    session
        .repo_hash
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            session
                .worktree_path
                .exists()
                .then(|| gwt_core::paths::project_scope_hash(&session.worktree_path).to_string())
        })
}

fn startup_auto_resume_is_fresh(
    session: &gwt_agent::Session,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    now.signed_duration_since(session.last_activity_at)
        <= chrono::Duration::seconds(STARTUP_AUTO_RESUME_STALE_AFTER_SECS)
}

fn startup_auto_resume_window_was_open(session: &gwt_agent::Session) -> bool {
    if session.restore_window_on_startup {
        return true;
    }
    // Compatibility for sessions saved before the explicit GUI restore flag
    // existed, and for files already migrated once with that flag defaulted.
    session.status != gwt_agent::AgentStatus::Stopped
}

fn mark_auto_resume_source_completed(sessions_dir: &Path, session_id: &str) {
    let path = sessions_dir.join(format!("{session_id}.toml"));
    let Ok(mut session) = gwt_agent::Session::load_and_migrate(&path) else {
        return;
    };
    session.update_status(gwt_agent::AgentStatus::Stopped);
    session.restore_window_on_startup = false;
    let _ = session.save(sessions_dir);
}

#[derive(Default)]
struct FrontendUserActionLog {
    action: &'static str,
    surface: &'static str,
    window_id: String,
    ui_target: String,
    profile_name: String,
    env_keys: String,
    env_var_count: usize,
    disabled_env_count: usize,
    agent_id: String,
    count: usize,
    mode: String,
    forced: bool,
}

impl FrontendUserActionLog {
    fn new(action: &'static str, surface: &'static str) -> Self {
        Self {
            action,
            surface,
            ..Default::default()
        }
    }

    fn window(mut self, id: &str) -> Self {
        self.window_id = sanitize_ui_action_field(id);
        self
    }

    fn target(mut self, value: impl AsRef<str>) -> Self {
        self.ui_target = sanitize_ui_action_field(value.as_ref());
        self
    }

    fn profile(mut self, name: impl AsRef<str>) -> Self {
        self.profile_name = sanitize_ui_action_field(name.as_ref());
        self
    }

    fn agent(mut self, id: impl AsRef<str>) -> Self {
        self.agent_id = sanitize_ui_action_field(id.as_ref());
        self
    }

    fn mode(mut self, value: impl AsRef<str>) -> Self {
        self.mode = sanitize_ui_action_field(value.as_ref());
        self
    }

    fn count(mut self, value: usize) -> Self {
        self.count = value;
        self
    }

    fn force(mut self, value: bool) -> Self {
        self.forced = value;
        self
    }

    fn env_keys<'a>(mut self, values: impl IntoIterator<Item = &'a str>) -> Self {
        let keys: Vec<_> = values.into_iter().collect();
        self.env_var_count = keys.len();
        self.env_keys = summarize_ui_action_values(keys);
        self
    }

    fn disabled_env_count(mut self, value: usize) -> Self {
        self.disabled_env_count = value;
        self
    }
}

fn sanitize_ui_action_field(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(160)
        .collect()
}

fn sanitize_ui_action_url(value: &str) -> String {
    let sanitized = sanitize_ui_action_field(value);
    let Some((scheme, rest)) = sanitized.split_once("://") else {
        return sanitized;
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    if authority.is_empty() {
        sanitized
    } else {
        format!("{scheme}://{authority}")
    }
}

fn summarize_ui_action_values<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    let mut items: Vec<String> = values
        .into_iter()
        .map(sanitize_ui_action_field)
        .filter(|value| !value.is_empty())
        .collect();
    items.sort();
    items.dedup();
    let truncated = items.len() > 12;
    items.truncate(12);
    let mut summary = items.join(",");
    if truncated {
        if !summary.is_empty() {
            summary.push_str(",...");
        } else {
            summary.push_str("...");
        }
    }
    summary
}

fn frontend_user_action_log(event: &FrontendEvent) -> Option<FrontendUserActionLog> {
    let log = match event {
        FrontendEvent::FrontendReady => FrontendUserActionLog::new("frontend_ready", "app"),
        FrontendEvent::OpenProjectDialog => {
            FrontendUserActionLog::new("open_project_dialog", "project")
        }
        FrontendEvent::SelectCloneProjectParent => {
            FrontendUserActionLog::new("select_clone_project_parent", "project")
        }
        FrontendEvent::GithubRepositorySearch { query } => {
            FrontendUserActionLog::new("github_repository_search", "project").count(query.len())
        }
        FrontendEvent::CloneProjectStart { url, parent_path } => {
            FrontendUserActionLog::new("clone_project_start", "project")
                .count(url.len())
                .mode(parent_path)
        }
        FrontendEvent::ReopenRecentProject { path } => {
            FrontendUserActionLog::new("reopen_recent_project", "project").target(path)
        }
        FrontendEvent::SelectProjectTab { tab_id } => {
            FrontendUserActionLog::new("select_project_tab", "project").target(tab_id)
        }
        FrontendEvent::CloseProjectTab { tab_id } => {
            FrontendUserActionLog::new("close_project_tab", "project").target(tab_id)
        }
        FrontendEvent::CreateWindow { preset, .. } => {
            FrontendUserActionLog::new("create_window", "window").target(format!("{preset:?}"))
        }
        FrontendEvent::LoadProcessConsole { id } => {
            FrontendUserActionLog::new("load_process_console", "console").window(id)
        }
        FrontendEvent::FocusWindow { id, .. } => {
            FrontendUserActionLog::new("focus_window", "window").window(id)
        }
        FrontendEvent::CycleFocus { direction, .. } => {
            FrontendUserActionLog::new("cycle_focus", "window").mode(format!("{direction:?}"))
        }
        FrontendEvent::ArrangeWindows { mode, .. } => {
            FrontendUserActionLog::new("arrange_windows", "window").mode(format!("{mode:?}"))
        }
        FrontendEvent::MaximizeWindow { id, .. } => {
            FrontendUserActionLog::new("maximize_window", "window").window(id)
        }
        FrontendEvent::MinimizeWindow { id } => {
            FrontendUserActionLog::new("minimize_window", "window").window(id)
        }
        FrontendEvent::RestoreWindow { id } => {
            FrontendUserActionLog::new("restore_window", "window").window(id)
        }
        FrontendEvent::DockWindowTab { id, target_id } => {
            FrontendUserActionLog::new("dock_window_tab", "window")
                .window(id)
                .target(target_id)
        }
        FrontendEvent::ActivateWindowTab { id } => {
            FrontendUserActionLog::new("activate_window_tab", "window").window(id)
        }
        FrontendEvent::DetachWindowTab { id, .. } => {
            FrontendUserActionLog::new("detach_window_tab", "window").window(id)
        }
        FrontendEvent::ListWindows => FrontendUserActionLog::new("list_windows", "window"),
        FrontendEvent::CloseWindow { id } => {
            FrontendUserActionLog::new("close_window", "window").window(id)
        }
        FrontendEvent::LoadFileTree { id, path } => {
            FrontendUserActionLog::new("load_file_tree", "file")
                .window(id)
                .target(path.as_deref().unwrap_or_default())
        }
        FrontendEvent::ListFileTreeWorktrees { id } => {
            FrontendUserActionLog::new("list_file_tree_worktrees", "file").window(id)
        }
        FrontendEvent::SelectFileTreeWorktree { id, worktree_id } => {
            FrontendUserActionLog::new("select_file_tree_worktree", "file")
                .window(id)
                .target(worktree_id)
        }
        FrontendEvent::LoadFileContent { id, path, mode, .. } => {
            FrontendUserActionLog::new("load_file_content", "file")
                .window(id)
                .target(path)
                .mode(format!("{mode:?}"))
        }
        FrontendEvent::SaveFileContent { id, path, mode, .. } => {
            FrontendUserActionLog::new("save_file_content", "file")
                .window(id)
                .target(path)
                .mode(format!("{mode:?}"))
        }
        FrontendEvent::LoadBranches { id } => {
            FrontendUserActionLog::new("load_branches", "branches").window(id)
        }
        FrontendEvent::RunBranchCleanup {
            id,
            branches,
            delete_remote,
            force_filesystem_delete,
        } => FrontendUserActionLog::new("run_branch_cleanup", "branches")
            .window(id)
            .target(summarize_ui_action_values(
                branches.iter().map(String::as_str),
            ))
            .count(branches.len())
            .mode(if *delete_remote {
                "delete_remote"
            } else {
                "local_only"
            })
            .force(*force_filesystem_delete),
        FrontendEvent::RunWorkspaceCleanup {
            branch,
            delete_remote,
            force_filesystem_delete,
        } => FrontendUserActionLog::new("run_workspace_cleanup", "workspace")
            .target(branch)
            .count(1)
            .mode(if *delete_remote {
                "delete_remote"
            } else {
                "local_only"
            })
            .force(*force_filesystem_delete),
        FrontendEvent::LoadBoard { id, all } => FrontendUserActionLog::new("load_board", "board")
            .window(id)
            .mode(if *all { "all" } else { "workspace" }),
        FrontendEvent::LoadBoardHistory { id, all, limit, .. } => {
            FrontendUserActionLog::new("load_board_history", "board")
                .window(id)
                .count(*limit)
                .mode(if *all { "all" } else { "workspace" })
        }
        FrontendEvent::PostBoardEntry {
            id,
            entry_kind,
            body,
            ..
        } => FrontendUserActionLog::new("post_board_entry", "board")
            .window(id)
            .mode(format!("{entry_kind:?}"))
            .count(body.len()),
        FrontendEvent::OpenBoardOriginAgent {
            id,
            origin_session_id,
            ..
        } => FrontendUserActionLog::new("open_board_origin_agent", "board")
            .window(id)
            .target(origin_session_id),
        FrontendEvent::LoadProfile { id } => {
            FrontendUserActionLog::new("load_profile", "profile").window(id)
        }
        FrontendEvent::SelectProfile { id, profile_name } => {
            FrontendUserActionLog::new("select_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::CreateProfile { id, name } => {
            FrontendUserActionLog::new("create_profile", "profile")
                .window(id)
                .profile(name)
        }
        FrontendEvent::SetActiveProfile { id, profile_name } => {
            FrontendUserActionLog::new("set_active_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::SaveProfile {
            id,
            name,
            env_vars,
            disabled_env,
            ..
        } => FrontendUserActionLog::new("save_profile", "profile")
            .window(id)
            .profile(name)
            .env_keys(env_vars.iter().map(|entry| entry.key.as_str()))
            .disabled_env_count(disabled_env.len()),
        FrontendEvent::DeleteProfile { id, profile_name } => {
            FrontendUserActionLog::new("delete_profile", "profile")
                .window(id)
                .profile(profile_name)
        }
        FrontendEvent::LoadLogs { id } => {
            FrontendUserActionLog::new("load_logs", "logs").window(id)
        }
        FrontendEvent::LoadKnowledgeBridge {
            id,
            knowledge_kind,
            refresh,
            ..
        } => FrontendUserActionLog::new("load_knowledge_bridge", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .force(*refresh),
        FrontendEvent::SearchKnowledgeBridge {
            id,
            knowledge_kind,
            query,
            ..
        } => FrontendUserActionLog::new("search_knowledge_bridge", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .count(query.len()),
        FrontendEvent::SearchProjectIndex {
            id,
            query,
            scopes,
            worktree_hash,
            ..
        } => FrontendUserActionLog::new("search_project_index", "index")
            .window(id)
            .mode(summarize_ui_action_values(
                scopes.iter().map(|scope| scope.as_str()),
            ))
            .agent(worktree_hash.as_deref().unwrap_or_default())
            .count(query.len()),
        FrontendEvent::SelectKnowledgeBridgeEntry {
            id,
            knowledge_kind,
            number,
            ..
        } => FrontendUserActionLog::new("select_knowledge_bridge_entry", "knowledge")
            .window(id)
            .mode(format!("{knowledge_kind:?}"))
            .target(number.to_string()),
        FrontendEvent::UpdateKnowledgeBridgePhase {
            id,
            issue_number,
            target_phase,
            ..
        } => FrontendUserActionLog::new("update_knowledge_bridge_phase", "knowledge")
            .window(id)
            .target(issue_number.to_string())
            .mode(target_phase.as_deref().unwrap_or("backlog")),
        FrontendEvent::RebuildIndexCell {
            project_root,
            scope,
            worktree_hash,
        } => FrontendUserActionLog::new("rebuild_index_cell", "index")
            .target(project_root)
            .mode(format!("{scope:?}"))
            .agent(worktree_hash.as_deref().unwrap_or_default()),
        FrontendEvent::RefreshIndexStatus { project_root } => {
            FrontendUserActionLog::new("refresh_index_status", "index").target(project_root)
        }
        FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
            FrontendUserActionLog::new("open_issue_launch_wizard", "launch")
                .window(id)
                .target(issue_number.to_string())
        }
        FrontendEvent::OpenStartWork => FrontendUserActionLog::new("open_start_work", "launch"),
        FrontendEvent::ResumeWorkspace { source, .. } => {
            FrontendUserActionLog::new("resume_workspace", "workspace").mode(format!("{source:?}"))
        }
        FrontendEvent::ListResumableAgents { workspace_id } => {
            FrontendUserActionLog::new("list_resumable_agents", "workspace")
                .target(workspace_id.as_deref().unwrap_or_default())
        }
        FrontendEvent::ResumeWorkspaceAgent { session_id, .. } => {
            FrontendUserActionLog::new("resume_workspace_agent", "workspace").target(session_id)
        }
        FrontendEvent::ResumeBranchLatestAgent {
            id, branch_name, ..
        } => FrontendUserActionLog::new("resume_branch_latest_agent", "launch")
            .window(id)
            .target(branch_name),
        FrontendEvent::OpenLaunchWizard {
            id,
            branch_name,
            linked_issue_number,
        } => FrontendUserActionLog::new("open_launch_wizard", "launch")
            .window(id)
            .target(branch_name)
            .count(linked_issue_number.unwrap_or_default() as usize),
        FrontendEvent::OpenActiveWorkLaunchWizard {
            branch_name,
            linked_issue_number,
        } => FrontendUserActionLog::new("open_active_work_launch_wizard", "launch")
            .target(branch_name)
            .count(linked_issue_number.unwrap_or_default() as usize),
        FrontendEvent::LaunchWizardAction { action, .. } => {
            let mut log = FrontendUserActionLog::new("launch_wizard_action", "launch")
                .mode(AppRuntime::launch_wizard_action_label(action));
            match action {
                LaunchWizardAction::SetAgent { agent_id } => {
                    log = log.agent(agent_id);
                }
                LaunchWizardAction::SetBranchName { value }
                | LaunchWizardAction::SetBranchType { prefix: value }
                | LaunchWizardAction::SetModel { model: value }
                | LaunchWizardAction::SetReasoning { reasoning: value }
                | LaunchWizardAction::SetVersion { version: value }
                | LaunchWizardAction::SetExecutionMode { mode: value }
                | LaunchWizardAction::SetDockerService { service: value } => {
                    log = log.target(value);
                }
                LaunchWizardAction::SubmitText { value } => {
                    log = log.count(value.len());
                }
                LaunchWizardAction::SetSkipPermissions { enabled }
                | LaunchWizardAction::SetCodexFastMode { enabled } => {
                    log = log.force(*enabled);
                }
                LaunchWizardAction::SetLinkedIssue { issue_number } => {
                    log = log.target(issue_number.to_string());
                }
                _ => {}
            }
            log
        }
        FrontendEvent::ApplyUpdate => FrontendUserActionLog::new("apply_update", "update"),
        FrontendEvent::ApplyUpdateStart => {
            FrontendUserActionLog::new("apply_update_start", "update")
        }
        FrontendEvent::CancelUpdateDownload => {
            FrontendUserActionLog::new("cancel_update_download", "update")
        }
        FrontendEvent::ApplyUpdateLater => {
            FrontendUserActionLog::new("apply_update_later", "update")
        }
        FrontendEvent::ApplyUpdateRestartNow => {
            FrontendUserActionLog::new("apply_update_restart_now", "update")
        }
        FrontendEvent::OpenUpdateLog { log_path } => {
            FrontendUserActionLog::new("open_update_log", "update")
                .target(log_path.as_deref().unwrap_or_default())
        }
        FrontendEvent::OpenServerUrl { .. } => {
            FrontendUserActionLog::new("open_server_url", "status")
        }
        FrontendEvent::ListCustomAgents => {
            FrontendUserActionLog::new("list_custom_agents", "custom_agents")
        }
        FrontendEvent::ListCustomAgentPresets => {
            FrontendUserActionLog::new("list_custom_agent_presets", "custom_agents")
        }
        FrontendEvent::AddCustomAgentFromPreset { input } => {
            FrontendUserActionLog::new("add_custom_agent_from_preset", "custom_agents")
                .agent(&input.id)
                .profile(&input.display_name)
        }
        FrontendEvent::UpdateCustomAgent { agent } => {
            FrontendUserActionLog::new("update_custom_agent", "custom_agents")
                .agent(&agent.id)
                .profile(&agent.display_name)
                .env_keys(agent.env.keys().map(String::as_str))
        }
        FrontendEvent::DeleteCustomAgent { agent_id } => {
            FrontendUserActionLog::new("delete_custom_agent", "custom_agents").agent(agent_id)
        }
        FrontendEvent::TestBackendConnection { base_url, .. } => {
            FrontendUserActionLog::new("test_backend_connection", "custom_agents")
                .target(sanitize_ui_action_url(base_url))
        }
        FrontendEvent::ListAgentBackends { agent } => {
            FrontendUserActionLog::new("list_agent_backends", "agent_backends")
                .agent(agent.as_str())
        }
        FrontendEvent::AddAgentBackend { agent, profile } => {
            FrontendUserActionLog::new("add_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(&profile.id)
        }
        FrontendEvent::UpdateAgentBackend { agent, id, .. } => {
            FrontendUserActionLog::new("update_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(id)
        }
        FrontendEvent::DeleteAgentBackend { agent, id } => {
            FrontendUserActionLog::new("delete_agent_backend", "agent_backends")
                .agent(agent.as_str())
                .profile(id)
        }
        FrontendEvent::TestAgentBackendConnection {
            agent, base_url, ..
        } => FrontendUserActionLog::new("test_agent_backend_connection", "agent_backends")
            .agent(agent.as_str())
            .target(sanitize_ui_action_url(base_url)),
        FrontendEvent::StartMigration { tab_id } => {
            FrontendUserActionLog::new("start_migration", "migration").target(tab_id)
        }
        FrontendEvent::SkipMigration { tab_id } => {
            FrontendUserActionLog::new("skip_migration", "migration").target(tab_id)
        }
        FrontendEvent::QuitMigration { tab_id } => {
            FrontendUserActionLog::new("quit_migration", "migration").target(tab_id)
        }
        FrontendEvent::GetSystemSettings => {
            FrontendUserActionLog::new("get_system_settings", "settings")
        }
        FrontendEvent::UpdateSystemSettings {
            language,
            codex_trust_managed_hooks,
        } => FrontendUserActionLog::new("update_system_settings", "settings")
            .target(language)
            .force(codex_trust_managed_hooks.unwrap_or(false)),
        FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
            FrontendUserActionLog::new("workspace_projection_prune", "workspace")
                .mode(if *dry_run { "dry_run" } else { "apply" })
                .count(ids.len())
        }
        FrontendEvent::SaveUiTrace { trace } => {
            let entries = trace.entries().map(|entries| entries.len()).unwrap_or(0);
            FrontendUserActionLog::new("save_ui_trace", "diagnostics")
                .target(trace.session_id().unwrap_or_default())
                .count(entries)
        }
        FrontendEvent::OpenReleaseNotes { focus_version, .. } => {
            FrontendUserActionLog::new("open_release_notes", "release_notes")
                .target(focus_version.as_deref().unwrap_or_default())
        }
        // These events can contain high-volume, high-frequency, or sensitive
        // payloads. They are handled by more specific logs or diagnostics.
        FrontendEvent::StartupAutoResumeReady { .. }
        | FrontendEvent::UpdateViewport { .. }
        | FrontendEvent::UpdateWindowGeometry { .. }
        | FrontendEvent::TerminalInput { .. }
        | FrontendEvent::PasteImage { .. } => return None,
    };
    Some(log)
}

fn log_frontend_user_action(client_id: &str, event: &FrontendEvent) {
    let Some(log) = frontend_user_action_log(event) else {
        return;
    };
    tracing::info!(
        target: "gwt_ui_action",
        client_id = %client_id,
        action = %log.action,
        surface = %log.surface,
        window_id = %log.window_id,
        ui_target = %log.ui_target,
        profile_name = %log.profile_name,
        env_keys = %log.env_keys,
        env_var_count = log.env_var_count as u64,
        disabled_env_count = log.disabled_env_count as u64,
        agent_id = %log.agent_id,
        count = log.count as u64,
        mode = %log.mode,
        forced = log.forced,
        "frontend user action"
    );
}

#[derive(Debug, Clone)]
pub struct ActiveAgentSession {
    pub(crate) window_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) branch_name: String,
    pub(crate) display_name: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) agent_project_root: String,
    pub(crate) runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub(crate) tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceResumeContext {
    pub(crate) title: Option<String>,
    pub(crate) owner: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) next_action: Option<String>,
}

impl WorkspaceResumeContext {
    fn purpose_title(&self) -> Option<String> {
        self.title
            .as_deref()
            .or(self.summary.as_deref())
            .or(self.owner.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingStartupAutoResumeSession {
    pub(crate) tab_id: String,
    pub(crate) session: gwt_agent::Session,
    pub(crate) workspace_resume_context: Option<WorkspaceResumeContext>,
}

#[derive(Debug, Clone)]
enum AgentWindowPlacement {
    Centered(WindowGeometry),
    Exact(WindowGeometry),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagePasteFile {
    pub(crate) bytes: Vec<u8>,
    pub(crate) storage_path: PathBuf,
    pub(crate) agent_path: String,
    pub(crate) prompt_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImagePasteError {
    UnsupportedMimeType(String),
    EmptyPayload,
    InvalidBase64(String),
    WriteFailed(String),
}

impl std::fmt::Display for ImagePasteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMimeType(mime_type) => {
                write!(formatter, "unsupported image MIME type: {mime_type}")
            }
            Self::EmptyPayload => formatter.write_str("image paste payload is empty"),
            Self::InvalidBase64(error) => write!(formatter, "invalid image paste payload: {error}"),
            Self::WriteFailed(error) => write!(formatter, "failed to save pasted image: {error}"),
        }
    }
}

const IMAGE_PASTE_RELATIVE_DIR: &str = ".gwt/paste-images";
const IMAGE_PASTE_PROMPT_PREFIX: &str = "Image file: ";
static IMAGE_PASTE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn image_extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn sanitize_image_paste_stem(filename: Option<&str>) -> String {
    let raw_stem = filename
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for character in raw_stem.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character);
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "image".to_string()
    } else {
        sanitized.to_string()
    }
}

fn join_agent_visible_path(agent_project_root: &str, relative_path: &str) -> String {
    let root = agent_project_root.trim();
    if root.is_empty() {
        return relative_path.to_string();
    }
    if root.contains('\\') && !root.contains('/') {
        format!(
            "{}\\{}",
            root.trim_end_matches('\\'),
            relative_path.replace('/', "\\")
        )
    } else {
        format!("{}/{}", root.trim_end_matches('/'), relative_path)
    }
}

pub(crate) fn prepare_image_paste_file(
    worktree_path: &Path,
    agent_project_root: &str,
    data_base64: &str,
    mime_type: &str,
    filename: Option<&str>,
    unique_token: &str,
) -> Result<ImagePasteFile, ImagePasteError> {
    let extension = image_extension_for_mime(mime_type)
        .ok_or_else(|| ImagePasteError::UnsupportedMimeType(mime_type.to_string()))?;
    if data_base64.trim().is_empty() {
        return Err(ImagePasteError::EmptyPayload);
    }
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        data_base64.trim(),
    )
    .map_err(|error| ImagePasteError::InvalidBase64(error.to_string()))?;
    if bytes.is_empty() {
        return Err(ImagePasteError::EmptyPayload);
    }

    let stem = sanitize_image_paste_stem(filename);
    let file_name = format!("{unique_token}-{stem}.{extension}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("paste-images")
        .join(&file_name);
    let relative_path = format!("{IMAGE_PASTE_RELATIVE_DIR}/{file_name}");
    let agent_path = join_agent_visible_path(agent_project_root, &relative_path);
    let prompt_text = format!("{IMAGE_PASTE_PROMPT_PREFIX}{agent_path}");

    Ok(ImagePasteFile {
        bytes,
        storage_path,
        agent_path,
        prompt_text,
    })
}

fn image_paste_unique_token() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let sequence = IMAGE_PASTE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{millis}-{sequence}")
}

fn save_image_paste_file(image: &ImagePasteFile) -> Result<(), ImagePasteError> {
    let Some(parent) = image.storage_path.parent() else {
        return Err(ImagePasteError::WriteFailed(
            "pasted image path has no parent directory".to_string(),
        ));
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| ImagePasteError::WriteFailed(error.to_string()))?;
    std::fs::write(&image.storage_path, &image.bytes)
        .map_err(|error| ImagePasteError::WriteFailed(error.to_string()))
}

#[derive(Debug, Clone)]
pub struct LaunchWizardMemoryCache {
    sessions: Vec<gwt_agent::Session>,
    agent_options: Vec<gwt::AgentOption>,
}

impl LaunchWizardMemoryCache {
    pub(crate) fn load(sessions_dir: &Path) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options: Self::load_agent_options(),
        }
    }

    #[cfg(test)]
    pub(crate) fn load_with_agent_options(
        sessions_dir: &Path,
        agent_options: Vec<gwt::AgentOption>,
    ) -> Self {
        Self {
            sessions: Self::load_sessions(sessions_dir),
            agent_options,
        }
    }

    fn load_sessions(sessions_dir: &Path) -> Vec<gwt_agent::Session> {
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

    fn load_agent_options() -> Vec<gwt::AgentOption> {
        gwt::load_agent_options(&gwt_agent::VersionCache::load(
            &gwt::default_wizard_version_cache_path(),
        ))
    }

    fn refresh_agent_options(&mut self) {
        self.agent_options = Self::load_agent_options();
    }

    fn agent_options(&self) -> Vec<gwt::AgentOption> {
        self.agent_options.clone()
    }

    fn quick_start_entries(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Vec<gwt::QuickStartEntry> {
        gwt::launch_wizard::quick_start_entries_from_sessions(
            repo_path,
            branch_name,
            &self.sessions,
        )
    }

    fn latest_resumable_branch_session(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        let entry = self
            .quick_start_entries(repo_path, branch_name)
            .into_iter()
            .find(|entry| entry.resume_session_id.is_some())?;
        self.sessions
            .iter()
            .find(|session| session.id == entry.session_id)
            .cloned()
    }

    fn session_by_id(&self, session_id: &str) -> Option<&gwt_agent::Session> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
    }

    fn agent_preferences(&self) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_from_sessions(&self.sessions)
    }

    fn previous_profiles(&self, repo_path: &Path) -> gwt::LaunchWizardPreviousProfiles {
        gwt::launch_wizard::previous_launch_profiles_for_repo_from_sessions(
            repo_path,
            &self.sessions,
        )
    }

    fn record_session(&mut self, session: gwt_agent::Session) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|existing| existing.id == session.id)
        {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
    }

    fn mark_stopped(&mut self, session_id: &str) {
        if let Some(session) = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.update_status(gwt_agent::AgentStatus::Stopped);
        }
    }
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct IssueBranchLinkStore {
    #[serde(default)]
    branches: HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub enum DispatchTarget {
    Broadcast,
    Client(ClientId),
}

#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub(crate) target: DispatchTarget,
    pub(crate) event: BackendEvent,
}

impl OutboundEvent {
    pub(crate) fn broadcast(event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Broadcast,
            event,
        }
    }

    pub(crate) fn reply(client_id: impl Into<ClientId>, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
        }
    }
}

// SPEC-2809 — per-spawn correlation id for Launch Wizard stages so the
// Console window's `agent` tab can group multiple stage events (binary
// resolve / env prep / worktree create / PTY handoff) under one
// invocation header. Atomic so parallel wizard sessions do not collide.
static AGENT_LAUNCH_STAGE_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_agent_launch_stage_id() -> u64 {
    AGENT_LAUNCH_STAGE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Emit a `gwt.process.summary` event for one Launch Wizard stage so the
/// Console window's `agent` tab surfaces the pipeline that ends in the
/// PTY spawn. Stage semantics (`start`, `done`, `error`) follow the same
/// vocabulary as the `spawn_logged` summary contract.
pub(crate) fn emit_agent_launch_stage(spawn_id: u64, stage: &str, detail: &str) {
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = spawn_id,
        stage = stage,
        detail = detail,
        "agent launch stage",
    );
    // Also push a synthetic line into the hub so the agent tab shows the
    // stage banner in real time (the summary event alone lives in
    // canonical log + Logs window only).
    let hub = gwt_core::process_console::global();
    let label = format!("[{stage}] {detail}");
    hub.push(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::AgentBootstrap,
        spawn_id,
        gwt_core::process_console::ProcessStream::Stdout,
        label,
    ));
}

fn knowledge_error_event(
    id: impl Into<String>,
    kind: KnowledgeKind,
    message: impl Into<String>,
    request_id: Option<u64>,
    query: Option<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeError {
        id: id.into(),
        knowledge_kind: kind,
        request_id,
        query,
        message: message.into(),
    }
}

fn knowledge_phase_update_error_event(
    id: impl Into<String>,
    request_id: u64,
    issue_number: u64,
    message: impl Into<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeBridgePhaseUpdated {
        id: id.into(),
        request_id,
        issue_number,
        result: gwt::protocol::KnowledgePhaseUpdateResult::Error {
            message: message.into(),
        },
    }
}

fn knowledge_view_events(
    client_id: String,
    id: String,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    view: gwt::KnowledgeBridgeView,
) -> Vec<OutboundEvent> {
    vec![
        OutboundEvent::reply(
            client_id.clone(),
            BackendEvent::KnowledgeEntries {
                id: id.clone(),
                knowledge_kind: kind,
                request_id,
                entries: view.entries,
                selected_number: view.selected_number,
                empty_message: view.empty_message,
                refresh_enabled: view.refresh_enabled,
            },
        ),
        OutboundEvent::reply(
            client_id,
            BackendEvent::KnowledgeDetail {
                id,
                knowledge_kind: kind,
                request_id,
                detail: view.detail,
            },
        ),
    ]
}

pub fn build_frontend_sync_events(
    client_id: &str,
    workspace: gwt::AppStateView,
    terminal_statuses: Vec<(String, WindowProcessStatus, String)>,
    terminal_snapshots: Vec<(String, Vec<u8>)>,
    launch_wizard: Option<gwt::LaunchWizardView>,
    pending_update: Option<gwt_core::update::UpdateState>,
) -> Vec<OutboundEvent> {
    let mut events = vec![OutboundEvent::reply(
        client_id,
        BackendEvent::WorkspaceState { workspace },
    )];

    for (id, status, detail) in terminal_statuses {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::TerminalStatus {
                id,
                status,
                detail: Some(detail),
            },
        ));
    }

    for (id, snapshot) in terminal_snapshots {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::TerminalSnapshot {
                id,
                data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
            },
        ));
    }

    if let Some(wizard) = launch_wizard {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::LaunchWizardState {
                wizard: Some(Box::new(wizard)),
            },
        ));
    }

    if let Some(state) = pending_update {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::UpdateState(state),
        ));
    }

    events
}

fn workspace_status_category_wire(
    category: gwt_core::workspace_projection::WorkspaceStatusCategory,
) -> &'static str {
    use gwt_core::workspace_projection::WorkspaceStatusCategory;

    match category {
        WorkspaceStatusCategory::Active => "active",
        WorkspaceStatusCategory::Idle => "idle",
        WorkspaceStatusCategory::Blocked => "blocked",
        WorkspaceStatusCategory::Done => "done",
        WorkspaceStatusCategory::Unknown => "unknown",
    }
}

const WORKSPACE_OVERVIEW_JOURNAL_LIMIT: usize = 8;
const WORKSPACE_CLEANUP_EVENT_ID: &str = "__workspace_cleanup__";

#[cfg(test)]
fn active_work_projection_from_saved(
    projection: gwt_core::workspace_projection::WorkspaceProjection,
) -> gwt::ActiveWorkProjectionView {
    let cleanup_candidate = projection
        .cleanup_candidate(false)
        .map(active_work_cleanup_candidate_view_from_candidate);
    active_work_projection_from_saved_with_journal(
        projection,
        Vec::new(),
        Vec::new(),
        cleanup_candidate,
    )
}

fn active_work_projection_from_saved_with_journal(
    projection: gwt_core::workspace_projection::WorkspaceProjection,
    journal_entries: Vec<gwt::WorkspaceJournalEntryView>,
    workspaces: Vec<gwt::WorkspaceHistoryView>,
    cleanup_candidate: Option<gwt::ActiveWorkCleanupCandidateView>,
) -> gwt::ActiveWorkProjectionView {
    use gwt_core::workspace_projection::WorkspaceStatusCategory;

    let active_agents = projection
        .assigned_agents()
        .filter(|agent| agent.status_category == WorkspaceStatusCategory::Active)
        .count();
    let blocked_agents = projection
        .assigned_agents()
        .filter(|agent| agent.status_category == WorkspaceStatusCategory::Blocked)
        .count();
    let agent_branch = projection
        .assigned_agents()
        .find_map(|agent| agent.branch.clone());
    let agent_worktree = projection.assigned_agents().find_map(|agent| {
        agent
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string())
    });
    let status_category =
        workspace_status_category_wire(projection.effective_status_category()).to_string();
    let (branch, worktree_path, pr_number, pr_url, pr_state, pr_created_at) =
        match projection.git_details.as_ref() {
            Some(details) => (
                details.branch.clone().or(agent_branch),
                details
                    .worktree_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .or(agent_worktree),
                details.pr_number,
                details.pr_url.clone(),
                details.pr_state.clone(),
                details
                    .pr_created_at
                    .map(|created_at| created_at.to_rfc3339()),
            ),
            None => (agent_branch, agent_worktree, None, None, None, None),
        };
    let mut agents = projection
        .assigned_agents()
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        active_work_agent_priority_rank(left)
            .cmp(&active_work_agent_priority_rank(right))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let mut unassigned_agents = projection
        .unassigned_agents()
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    unassigned_agents.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });

    gwt::ActiveWorkProjectionView {
        id: projection.id,
        title: projection.title,
        status_category,
        status_text: projection.status_text,
        summary: projection.summary,
        owner: projection.owner,
        next_action: projection.next_action,
        active_agents,
        blocked_agents,
        branch,
        worktree_path,
        pr_number,
        pr_url,
        pr_state,
        pr_created_at,
        board_refs: projection.board_refs,
        journal_entries,
        workspaces,
        cleanup_candidate,
        agents,
        unassigned_agents,
    }
}

fn empty_active_work_projection_view(
    tab_id: &str,
    tab: &ProjectTabRuntime,
) -> gwt::ActiveWorkProjectionView {
    gwt::ActiveWorkProjectionView {
        id: tab_id.to_string(),
        title: format!("{} workspace", tab.title),
        status_category: "idle".to_string(),
        status_text: String::new(),
        summary: None,
        owner: None,
        next_action: None,
        active_agents: 0,
        blocked_agents: 0,
        branch: None,
        worktree_path: None,
        pr_number: None,
        pr_url: None,
        pr_state: None,
        pr_created_at: None,
        board_refs: Vec::new(),
        journal_entries: Vec::new(),
        workspaces: Vec::new(),
        cleanup_candidate: None,
        agents: Vec::new(),
        unassigned_agents: Vec::new(),
    }
}

fn active_work_cleanup_candidate_view_from_candidate(
    candidate: gwt_core::workspace_projection::WorkspaceCleanupCandidate,
) -> gwt::ActiveWorkCleanupCandidateView {
    gwt::ActiveWorkCleanupCandidateView {
        branch: candidate.branch,
        worktree_path: candidate
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        reason: candidate.reason.as_str().to_string(),
        default_delete_remote: candidate.default_delete_remote,
        remote_delete_available: candidate.remote_delete_available,
    }
}

fn workspace_journal_entry_view_from_entry(
    entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> gwt::WorkspaceJournalEntryView {
    gwt::WorkspaceJournalEntryView {
        id: entry.id.clone(),
        updated_at: entry
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        title: entry.title.clone(),
        status_category: entry
            .status_category
            .map(workspace_status_category_wire)
            .map(str::to_string),
        status_text: entry.status_text.clone(),
        summary: entry.summary.clone(),
        owner: entry.owner.clone(),
        next_action: entry.next_action.clone(),
        agent_session_id: entry.agent_session_id.clone(),
        agent_current_focus: entry.agent_current_focus.clone(),
        agent_title_summary: entry.agent_title_summary.clone(),
    }
}

pub(crate) fn workspace_work_item_view_from_item(
    item: &gwt_core::workspace_projection::WorkspaceWorkItem,
) -> gwt::WorkspaceHistoryView {
    gwt::WorkspaceHistoryView {
        id: item.id.clone(),
        title: item.title.clone(),
        intent: item.intent.clone(),
        summary: item.summary.clone(),
        status_category: workspace_status_category_wire(item.status_category).to_string(),
        owner: item.owner.clone(),
        created_at: item
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        updated_at: item
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        completed_at: item
            .completed_at
            .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        agents: item
            .agents
            .iter()
            .map(workspace_work_agent_view_from_ref)
            .collect(),
        execution_containers: item
            .execution_containers
            .iter()
            .map(workspace_execution_container_view_from_ref)
            .collect(),
        board_refs: item.board_refs.clone(),
        related_workspace_ids: item.related_work_item_ids.clone(),
        events: item
            .events
            .iter()
            .map(workspace_work_event_view_from_event)
            .collect(),
    }
}

fn workspace_work_agent_view_from_ref(
    agent: &gwt_core::workspace_projection::WorkspaceWorkAgentRef,
) -> gwt::WorkspaceHistoryAgentView {
    gwt::WorkspaceHistoryAgentView {
        session_id: agent.session_id.clone(),
        agent_id: agent.agent_id.clone(),
        display_name: agent.display_name.clone(),
        updated_at: agent
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

fn workspace_execution_container_view_from_ref(
    container: &gwt_core::workspace_projection::WorkspaceExecutionContainerRef,
) -> gwt::WorkspaceExecutionContainerView {
    gwt::WorkspaceExecutionContainerView {
        branch: container.branch.clone(),
        worktree_path: container
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        pr_number: container.pr_number,
        pr_url: container.pr_url.clone(),
        pr_state: container.pr_state.clone(),
    }
}

fn workspace_work_event_view_from_event(
    event: &gwt_core::workspace_projection::WorkspaceWorkEvent,
) -> gwt::WorkspaceHistoryEventView {
    gwt::WorkspaceHistoryEventView {
        id: event.id.clone(),
        workspace_id: event.work_item_id.clone(),
        kind: workspace_work_event_kind_wire(event.kind).to_string(),
        title: event.title.clone(),
        intent: event.intent.clone(),
        summary: event.summary.clone(),
        status_category: event
            .status_category
            .map(workspace_status_category_wire)
            .map(str::to_string),
        owner: event.owner.clone(),
        next_action: event.next_action.clone(),
        agent_session_id: event.agent_session_id.clone(),
        board_entry_id: event.board_entry_id.clone(),
        related_workspace_id: event.related_work_item_id.clone(),
        updated_at: event
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

fn workspace_work_event_kind_wire(
    kind: gwt_core::workspace_projection::WorkspaceWorkEventKind,
) -> &'static str {
    use gwt_core::workspace_projection::WorkspaceWorkEventKind;

    match kind {
        WorkspaceWorkEventKind::Start => "start",
        WorkspaceWorkEventKind::Claim => "claim",
        WorkspaceWorkEventKind::Update => "update",
        WorkspaceWorkEventKind::Blocked => "blocked",
        WorkspaceWorkEventKind::Handoff => "handoff",
        WorkspaceWorkEventKind::Resume => "resume",
        WorkspaceWorkEventKind::Split => "split",
        WorkspaceWorkEventKind::Merge => "merge",
        WorkspaceWorkEventKind::Pr => "pr",
        WorkspaceWorkEventKind::Done => "done",
    }
}

fn non_empty_workspace_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn workspace_resume_context_from_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
) -> WorkspaceResumeContext {
    WorkspaceResumeContext {
        title: non_empty_workspace_text(Some(&projection.title)),
        owner: non_empty_workspace_text(projection.owner.as_deref()),
        summary: non_empty_workspace_text(projection.summary.as_deref()),
        next_action: non_empty_workspace_text(projection.next_action.as_deref()),
    }
}

fn workspace_resume_context_from_journal(
    entry: &gwt_core::workspace_projection::WorkspaceJournalEntry,
) -> WorkspaceResumeContext {
    WorkspaceResumeContext {
        title: non_empty_workspace_text(entry.title.as_deref())
            .or_else(|| non_empty_workspace_text(entry.agent_title_summary.as_deref())),
        owner: non_empty_workspace_text(entry.owner.as_deref()),
        summary: non_empty_workspace_text(entry.summary.as_deref())
            .or_else(|| non_empty_workspace_text(entry.agent_current_focus.as_deref()))
            .or_else(|| non_empty_workspace_text(entry.status_text.as_deref())),
        next_action: non_empty_workspace_text(entry.next_action.as_deref()),
    }
}

fn workspace_resume_owner_issue_number(owner: Option<&str>) -> Option<u64> {
    let owner = owner?.trim();
    if owner.is_empty() {
        return None;
    }
    let lower = owner.to_ascii_lowercase();
    if !(owner.starts_with('#') || lower.contains("issue") || lower.contains("spec")) {
        return None;
    }

    let mut digits = String::new();
    let mut started = false;
    for character in owner.chars() {
        if character.is_ascii_digit() {
            started = true;
            digits.push(character);
        } else if started {
            break;
        }
    }
    digits.parse::<u64>().ok()
}

fn workspace_resume_branch_from_journal_project_root(
    project_root: &Path,
    active_project_root: &Path,
) -> Option<String> {
    if let Ok(branch) = current_git_branch(project_root) {
        let branch = normalize_branch_name(branch.trim());
        if !branch.is_empty() {
            return Some(branch);
        }
    }

    let main_repo_path = gwt_git::worktree::main_worktree_root(active_project_root).ok()?;
    let layout_root = main_repo_path.parent()?;
    let normalized_project_root = normalize_existing_path_prefix(project_root);
    let normalized_layout_root = normalize_existing_path_prefix(layout_root);
    let relative_path = normalized_project_root
        .strip_prefix(&normalized_layout_root)
        .ok()?;
    let branch = relative_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if branch.is_empty() {
        return None;
    }
    Some(branch)
}

fn normalize_existing_path_prefix(path: &Path) -> PathBuf {
    if path.exists() {
        return std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    }

    let mut missing_components = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            return path.to_path_buf();
        };
        missing_components.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            return path.to_path_buf();
        };
        current = parent;
    }

    let mut normalized = std::fs::canonicalize(current).unwrap_or_else(|_| current.to_path_buf());
    for component in missing_components.iter().rev() {
        normalized.push(component);
    }
    normalized
}

fn workspace_resume_branch_exists(project_root: &Path, branch_name: &str) -> bool {
    let branch_name = normalize_branch_name(branch_name.trim());
    if branch_name.is_empty() {
        return false;
    }
    let Ok(main_repo_path) = gwt_git::worktree::main_worktree_root(project_root) else {
        return false;
    };
    if local_branch_exists(&main_repo_path, &branch_name).unwrap_or(false) {
        return true;
    }
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    manager
        .remote_branch_exists(&origin_remote_ref(&branch_name))
        .unwrap_or(false)
}

fn active_work_agent_priority_rank(agent: &gwt::ActiveWorkAgentView) -> u8 {
    match agent.status_category.as_str() {
        "blocked" => 0,
        "active" => match agent.last_board_entry_kind.as_deref() {
            Some("handoff") => 1,
            Some("next") => 2,
            Some("claim") => 3,
            Some("decision") => 4,
            Some("status") => 5,
            _ => 6,
        },
        "idle" => 7,
        "done" => 8,
        _ => 9,
    }
}

fn active_work_agent_view_from_summary(
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> gwt::ActiveWorkAgentView {
    let affiliation_status = match agent.affiliation_status {
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned => "unassigned",
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned => "assigned",
    };
    gwt::ActiveWorkAgentView {
        session_id: agent.session_id.clone(),
        window_id: agent.window_id.clone(),
        agent_id: agent.agent_id.clone(),
        display_name: agent.display_name.clone(),
        affiliation_status: affiliation_status.to_string(),
        workspace_id: agent.workspace_id.clone(),
        status_category: workspace_status_category_wire(agent.status_category).to_string(),
        current_focus: agent.current_focus.clone(),
        title_summary: agent.title_summary.clone(),
        branch: agent.branch.clone(),
        worktree_path: agent
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string()),
        last_board_entry_id: agent.last_board_entry_id.clone(),
        last_board_entry_kind: agent
            .last_board_entry_kind
            .as_ref()
            .map(|kind| kind.as_str().to_string()),
        coordination_scope: agent.coordination_scope.clone(),
        updated_at: agent.updated_at.to_rfc3339(),
    }
}

fn active_agent_summary_from_session(
    session: &ActiveAgentSession,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
    gwt_core::workspace_projection::WorkspaceAgentSummary {
        session_id: session.session_id.clone(),
        window_id: Some(session.window_id.clone()),
        agent_id: session.agent_id.clone(),
        display_name: session.display_name.clone(),
        status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
        current_focus: None,
        title_summary: None,
        worktree_path: Some(session.worktree_path.clone()),
        branch: Some(session.branch_name.clone()),
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        affiliation_status:
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
        workspace_id: None,
        updated_at,
    }
}

fn unassigned_agent_summary_from_session(
    session: &ActiveAgentSession,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
    let mut summary = active_agent_summary_from_session(session, updated_at);
    summary.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
    summary.workspace_id = None;
    summary
}

fn agent_launch_purpose_title(
    project_root: &Path,
    linked_issue_number: Option<u64>,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> Option<String> {
    linked_issue_number
        .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        .or_else(|| {
            linked_issue_number_for_branch(project_root, branch_name, issue_link_cache_dir)
                .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        })
        .or_else(|| workspace_projection_owner_title(project_root, branch_name))
}

fn issue_title_from_cache(project_root: &Path, issue_number: u64) -> Option<String> {
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(project_root)?;
    let cache_root = gwt_core::paths::gwt_cache_dir()
        .join("issues")
        .join(repo_hash.as_str());
    let entry =
        gwt_github::Cache::new(cache_root).load_entry(gwt_github::IssueNumber(issue_number))?;
    let title = entry.snapshot.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn linked_issue_number_for_branch(
    project_root: &Path,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> Option<u64> {
    let branch_name = branch_name?.trim();
    if branch_name.is_empty() {
        return None;
    }
    let repo_hash = gwt::index_worker::detect_repo_hash(project_root)?;
    let path = issue_link_cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let bytes = std::fs::read(path).ok()?;
    let store = serde_json::from_slice::<IssueBranchLinkStore>(&bytes).ok()?;
    store.branches.get(branch_name).copied()
}

fn workspace_projection_owner_title(
    project_root: &Path,
    branch_name: Option<&str>,
) -> Option<String> {
    let branch_name = branch_name?.trim();
    if branch_name.is_empty() {
        return None;
    }
    let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
        .ok()
        .flatten()?;
    let projection_branch = projection.git_details.as_ref()?.branch.as_deref()?.trim();
    if projection_branch != branch_name {
        return None;
    }
    let owner = projection.owner?.trim().to_string();
    (!owner.is_empty()).then_some(owner)
}

fn upsert_workspace_agent(
    agents: &mut Vec<gwt_core::workspace_projection::WorkspaceAgentSummary>,
    summary: gwt_core::workspace_projection::WorkspaceAgentSummary,
) {
    use gwt_core::workspace_projection::WorkspaceStatusCategory;

    if let Some(existing) = agents
        .iter_mut()
        .find(|agent| agent.session_id == summary.session_id)
    {
        existing.agent_id = summary.agent_id;
        existing.window_id = summary.window_id;
        existing.display_name = summary.display_name;
        existing.worktree_path = summary.worktree_path;
        existing.branch = summary.branch;
        if existing.status_category != WorkspaceStatusCategory::Blocked {
            existing.status_category = summary.status_category;
        }
        if summary.current_focus.is_some() {
            existing.current_focus = summary.current_focus;
        }
        if summary.last_board_entry_id.is_some() {
            existing.last_board_entry_id = summary.last_board_entry_id;
        }
        if summary.last_board_entry_kind.is_some() {
            existing.last_board_entry_kind = summary.last_board_entry_kind;
        }
        if summary.coordination_scope.is_some() {
            existing.coordination_scope = summary.coordination_scope;
        }
        if summary.title_summary.is_some() {
            existing.title_summary = summary.title_summary;
        }
        existing.affiliation_status = summary.affiliation_status;
        existing.workspace_id = summary.workspace_id;
        if summary.updated_at > existing.updated_at {
            existing.updated_at = summary.updated_at;
        }
    } else {
        agents.push(summary);
    }
}

fn merge_active_sessions_into_projection<'a>(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: impl IntoIterator<Item = &'a ActiveAgentSession>,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    for session in sessions {
        let existing = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session.session_id)
            .or_else(|| {
                projection
                    .agents
                    .iter()
                    .find(|agent| agent.window_id.as_deref() == Some(session.window_id.as_str()))
            });
        let mut summary = active_agent_summary_from_session(session, updated_at);
        if let Some(existing) = existing {
            summary.affiliation_status = existing.affiliation_status;
            summary.workspace_id = existing.workspace_id.clone();
            summary.title_summary = existing.title_summary.clone();
            summary.current_focus = existing.current_focus.clone();
            summary.last_board_entry_id = existing.last_board_entry_id.clone();
            summary.last_board_entry_kind = existing.last_board_entry_kind.clone();
            summary.coordination_scope = existing.coordination_scope.clone();
        } else {
            summary.affiliation_status =
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
            summary.workspace_id = None;
        }
        upsert_workspace_agent(&mut projection.agents, summary);
    }
}

fn retain_live_workspace_agents(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    let live_session_ids = sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    projection
        .agents
        .retain(|agent| live_session_ids.contains(agent.session_id.as_str()));
    if !projection.agents.iter().any(|agent| {
        agent.is_assigned()
            && matches!(
                agent.status_category,
                gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                    | gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked
            )
    }) {
        projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
        projection.status_text = "No active work".to_string();
        projection.next_action = None;
        projection.updated_at = updated_at;
    }
}

fn workspace_projection_has_current_agents(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
) -> bool {
    projection.agents.iter().any(|agent| {
        agent.is_assigned()
            && matches!(
                agent.status_category,
                gwt_core::workspace_projection::WorkspaceStatusCategory::Active
                    | gwt_core::workspace_projection::WorkspaceStatusCategory::Blocked
            )
    })
}

fn reset_idle_workspace_current_identity(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    tab_title: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    let title = tab_title.trim();
    projection.title = if title.is_empty() {
        "Project workspace".to_string()
    } else {
        format!("{title} workspace")
    };
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
    projection.status_text = "No active work".to_string();
    projection.summary = None;
    projection.owner = None;
    projection.next_action = None;
    projection.git_details = None;
    projection.board_refs.clear();
    projection.updated_at = updated_at;
}

fn workspace_projection_for_current_resume(
    mut projection: gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    tab_title: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceProjection {
    merge_active_sessions_into_projection(&mut projection, sessions.iter().copied(), updated_at);
    retain_live_workspace_agents(&mut projection, sessions, updated_at);
    if !workspace_projection_has_current_agents(&projection) {
        reset_idle_workspace_current_identity(&mut projection, tab_title, updated_at);
    }
    projection
}

fn workspace_cleanup_candidate_for_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ActiveWorkCleanupCandidateView> {
    let branch = projection.git_details.as_ref()?.branch.as_deref()?;
    let branch_has_live_agent = sessions.iter().any(|session| session.branch_name == branch);
    let candidate = projection.cleanup_candidate(branch_has_live_agent)?;
    Some(active_work_cleanup_candidate_view_from_candidate(candidate))
}

fn save_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
    created_by_start_work: bool,
) -> Result<(), String> {
    use gwt_core::workspace_projection::{GitDetails, WorkspaceStatusCategory};

    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::load_or_default_workspace_projection(project_root)
            .map_err(|error| error.to_string())?;
    projection.project_root = project_root.to_path_buf();
    projection.title = workspace_resume_context
        .and_then(|context| non_empty_workspace_text(context.title.as_deref()))
        .unwrap_or_else(|| "Start Work".to_string());
    projection.status_category = WorkspaceStatusCategory::Active;
    projection.next_action = workspace_resume_context
        .and_then(|context| non_empty_workspace_text(context.next_action.as_deref()))
        .or_else(|| Some("Check Board for latest updates".to_string()));
    if let Some(summary) = workspace_resume_context
        .and_then(|context| non_empty_workspace_text(context.summary.as_deref()))
    {
        projection.summary = Some(summary);
    }
    if let Some(owner) = workspace_resume_context
        .and_then(|context| non_empty_workspace_text(context.owner.as_deref()))
    {
        projection.owner = Some(owner);
    } else if let Some(issue_number) = linked_issue_number {
        projection.owner = Some(format!("Issue #{issue_number}"));
    }
    let mut agent = active_agent_summary_from_session(session, now);
    agent.affiliation_status =
        gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned;
    agent.workspace_id = Some(projection.id.clone());
    upsert_workspace_agent(&mut projection.agents, agent);
    let active_agents = projection
        .assigned_agents()
        .filter(|agent| agent.status_category == WorkspaceStatusCategory::Active)
        .count();
    projection.status_text = if active_agents == 1 {
        format!("{} is running", session.display_name)
    } else {
        format!("{active_agents} active agents")
    };
    let previous_base_branch = projection
        .git_details
        .as_ref()
        .and_then(|details| details.base_branch.clone());
    projection.git_details = Some(GitDetails {
        branch: Some(session.branch_name.clone()),
        worktree_path: Some(session.worktree_path.clone()),
        base_branch: base_branch.map(str::to_string).or(previous_base_branch),
        pr_number: None,
        pr_state: None,
        pr_url: None,
        pr_created_at: None,
        created_by_start_work,
        created_at: now,
    });
    projection.updated_at = now;

    gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
        .map_err(|error| error.to_string())?;
    let work_event_kind = if workspace_resume_context.is_some() {
        gwt_core::workspace_projection::WorkspaceWorkEventKind::Resume
    } else {
        gwt_core::workspace_projection::WorkspaceWorkEventKind::Start
    };
    let work_event =
        workspace_work_event_from_launch_projection(&projection, session, work_event_kind, now);
    gwt_core::workspace_projection::record_workspace_work_event(project_root, work_event)
        .map_err(|error| error.to_string())
}

fn workspace_work_event_from_launch_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    session: &ActiveAgentSession,
    kind: gwt_core::workspace_projection::WorkspaceWorkEventKind,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceWorkEvent {
    let mut event = gwt_core::workspace_projection::WorkspaceWorkEvent::new(
        kind,
        projection.id.clone(),
        updated_at,
    );
    event.title = Some(projection.title.clone());
    event.intent = projection
        .summary
        .clone()
        .or_else(|| projection.next_action.clone());
    event.summary = Some(projection.status_text.clone());
    event.status_category = Some(projection.status_category);
    event.owner = projection.owner.clone();
    event.next_action = projection.next_action.clone();
    event.agent_session_id = Some(session.session_id.clone());
    event.agent_id = Some(session.agent_id.to_string());
    event.display_name = Some(session.display_name.clone());
    event.execution_container = projection.git_details.as_ref().map(|details| {
        gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
            branch: details.branch.clone(),
            worktree_path: details.worktree_path.clone(),
            pr_number: details.pr_number,
            pr_url: details.pr_url.clone(),
            pr_state: details.pr_state.clone(),
        }
    });
    event
}

fn save_unassigned_workspace_launch_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let mut projection =
        gwt_core::workspace_projection::load_or_default_workspace_projection(project_root)
            .map_err(|error| error.to_string())?;
    projection.project_root = project_root.to_path_buf();
    projection.register_unassigned_agent(unassigned_agent_summary_from_session(session, now));
    projection.updated_at = now;
    gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
        .map_err(|error| error.to_string())
}

fn save_start_work_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    _base_branch: &str,
    linked_issue_number: Option<u64>,
    workspace_resume_context: Option<&WorkspaceResumeContext>,
) -> Result<(), String> {
    if workspace_resume_context.is_none() {
        return save_unassigned_workspace_launch_projection(project_root, session);
    }
    save_workspace_launch_projection(
        project_root,
        session,
        Some(_base_branch),
        linked_issue_number,
        workspace_resume_context,
        true,
    )
}

fn save_resumed_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    workspace_resume_context: &WorkspaceResumeContext,
) -> Result<(), String> {
    save_workspace_launch_projection(
        project_root,
        session,
        base_branch,
        linked_issue_number,
        Some(workspace_resume_context),
        session.branch_name.starts_with("work/"),
    )
}

#[derive(Debug, Clone)]
pub struct ProjectTabRuntime {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) project_root: PathBuf,
    pub(crate) kind: gwt::ProjectKind,
    pub(crate) workspace: WorkspaceState,
    /// SPEC-1934 US-6: in-memory flag set when the tab was opened on a Normal
    /// Git layout that we want to migrate. The frontend sees a
    /// [`BackendEvent::MigrationDetected`] until the user picks Migrate /
    /// Skip / Quit. Not persisted: re-detected on every launch.
    pub(crate) migration_pending: bool,
    /// SPEC-2014 FR-PERF-003: cached `git rev-parse --git-common-dir`
    /// resolution for this tab. `gwt_git::worktree::main_worktree_root`
    /// spawns `git.exe`; on Windows every spawn costs several hundred
    /// milliseconds (`CreateProcess` + Defender real-time scan). The Launch
    /// Wizard / Start Work / Add Agent / Resume Workspace paths used to call
    /// it on every open, accounting for the bulk of the cold-open delay.
    /// We resolve the value on first access and reuse it for the lifetime
    /// of the tab; the [`Arc`] wrapper keeps `ProjectTabRuntime: Clone`.
    pub(crate) main_worktree_root_cache: std::sync::Arc<std::sync::OnceLock<PathBuf>>,
}

impl ProjectTabRuntime {
    /// Return the cached primary repository root for this tab, lazily
    /// resolving it on first access (FR-PERF-003). Falls back to
    /// `project_root` when `git rev-parse --git-common-dir` fails so the
    /// caller never has to deal with `Result`.
    pub(crate) fn main_worktree_root(&self) -> PathBuf {
        self.main_worktree_root_cache
            .get_or_init(|| {
                gwt_git::worktree::main_worktree_root(&self.project_root)
                    .unwrap_or_else(|_| self.project_root.clone())
            })
            .clone()
    }
}

fn recovery_state_label(recovery: gwt_core::migration::RecoveryState) -> &'static str {
    use gwt_core::migration::RecoveryState;
    match recovery {
        RecoveryState::Untouched => "untouched",
        RecoveryState::RolledBack => "rolled_back",
        RecoveryState::Partial => "partial",
    }
}

/// Best-effort `git symbolic-ref --short HEAD` for the migration modal
/// preview. Returns `None` for detached HEAD or unreadable repositories so
/// the frontend can fall back to a neutral label.
fn read_head_branch(project_root: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

/// `true` when `git status --porcelain` reports any entry. Failures are
/// treated as "not dirty" since the backend can fall through to the regular
/// validator pass.
/// Build a Phase 14 message-only [`BackendEvent::UpdateApplyError`].
/// New callers should prefer [`update_apply_error_failed`] which also fills
/// the structured Phase 19 fields.
fn update_apply_error_message(message: &str) -> BackendEvent {
    BackendEvent::UpdateApplyError {
        message: Some(message.to_string()),
        stage: None,
        reason: None,
        log_path: None,
    }
}

/// SPEC-2041 Phase 19 (FR-063): structured update failure event with stage
/// and reason. The legacy `message` field is populated with `reason` for
/// frontends that still read it.
fn update_apply_error_failed(stage: &str, reason: &str) -> BackendEvent {
    BackendEvent::UpdateApplyError {
        message: Some(reason.to_string()),
        stage: Some(stage.to_string()),
        reason: Some(reason.to_string()),
        log_path: None,
    }
}

/// SPEC-2041 Phase 19 (FR-065, CodeRabbit review on PR #2630): pure
/// validator for renderer-supplied update log paths. Returns the canonical
/// path when (1) the input is non-empty and contains no URL scheme,
/// (2) it canonicalizes successfully, (3) it is a file, and (4) it
/// resides within the canonicalized `logs_root`. Returns `None` otherwise so
/// callers can silently drop the request.
fn validate_update_log_path(raw: &str, logs_root: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains("://") {
        return None;
    }
    let canonical_root = std::fs::canonicalize(logs_root).ok()?;
    let candidate = std::fs::canonicalize(trimmed).ok()?;
    if !candidate.starts_with(&canonical_root) || !candidate.is_file() {
        return None;
    }
    Some(candidate)
}

#[derive(Debug, serde::Deserialize)]
struct GhRepositorySearchRecord {
    #[serde(rename = "fullName")]
    full_name: Option<String>,
    description: Option<String>,
    url: Option<String>,
    #[serde(rename = "defaultBranch")]
    default_branch: Option<String>,
    visibility: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
}

pub(crate) fn parse_github_repository_search_results(
    raw: &str,
) -> Result<Vec<gwt::GitHubRepositorySearchResultView>, String> {
    let records: Vec<GhRepositorySearchRecord> =
        serde_json::from_str(raw).map_err(|error| format!("parse gh search JSON: {error}"))?;
    let mut repositories = Vec::new();
    for record in records {
        let Some(full_name) = record.full_name.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let Some(url) = record.url.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        repositories.push(gwt::GitHubRepositorySearchResultView {
            full_name,
            description: record.description.filter(|value| !value.trim().is_empty()),
            url,
            default_branch: record
                .default_branch
                .filter(|value| !value.trim().is_empty()),
            visibility: record.visibility.filter(|value| !value.trim().is_empty()),
            updated_at: record.updated_at.filter(|value| !value.trim().is_empty()),
        });
    }
    Ok(repositories)
}

fn search_github_repositories(
    query: &str,
    limit: usize,
) -> Result<Vec<gwt::GitHubRepositorySearchResultView>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("repository search query is required".to_string());
    }
    let hub = gwt_core::process_console::global();
    let limit_str = limit.to_string();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "search",
            "repos",
            trimmed,
            "--json",
            "fullName,description,url,defaultBranch,visibility,updatedAt",
            "--limit",
            limit_str.as_str(),
        ],
        gwt_core::process_console::SpawnOptions::new("gh search repos"),
    )
    .map_err(|error| format!("gh search repos: {error}"))?;
    if !output.success() {
        let stderr = output.stderr.trim().to_string();
        return Err(if stderr.is_empty() {
            "gh search repos failed".to_string()
        } else {
            stderr
        });
    }
    parse_github_repository_search_results(&output.stdout)
}

/// SPEC-2785 FR-E: exact same-origin match between the embedded server's
/// bound URL and a frontend-supplied URL. Used as the pre-spawn gate by
/// [`AppRuntime::open_server_url_events`] so a renderer compromise cannot
/// smuggle an arbitrary URL into the OS opener.
///
/// Comparison is byte-exact. Trailing-slash and case differences are NOT
/// normalized — the frontend derives its URL from `window.location.origin`
/// so the two strings are always produced by the same source and any drift
/// is a bug worth surfacing rather than papering over.
fn validate_server_url(allowed: Option<&str>, requested: &str) -> bool {
    matches!(allowed, Some(value) if value == requested)
}

/// SPEC-2785 FR-C / FR-E: launch the platform default browser for a URL
/// argument (analogous to [`open_path_with_os_default`] but reserved for URLs
/// that have already cleared [`validate_server_url`]). The opener receives
/// the URL via argv directly, never through a shell, so URL contents cannot
/// trigger shell metacharacter expansion.
fn open_url_with_os_default(url: &str) -> Result<(), std::io::Error> {
    use std::process::Command;
    let child = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(url);
        cmd.spawn()?
    } else if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        // The empty "" before the URL is required by `start` so that a URL
        // beginning with quoted text is not interpreted as a window title.
        cmd.args(["/C", "start", "", url]);
        cmd.spawn()?
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd.spawn()?
    };
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// SPEC-2041 Phase 19 (FR-065): launch the platform default opener
/// (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows). Errors are
/// silently dropped so the modal does not surface noise; the path is logged
/// at the trace level.
fn open_path_with_os_default(path: &str) -> Result<(), std::io::Error> {
    use std::process::Command;
    // Reap the spawned opener on a detached thread so repeated invocations
    // do not accumulate zombie processes on Unix. `std::process::Child` has
    // no Drop-time wait, so without this the PID stays in the process table
    // until parent exit (CodeRabbit review on PR #2630).
    let child = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        cmd.spawn()?
    } else if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", path]);
        cmd.spawn()?
    } else {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(path);
        cmd.spawn()?
    };
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

fn detect_dirty(project_root: &Path) -> bool {
    gwt_core::process::hidden_command("git")
        .args(["status", "--porcelain"])
        .current_dir(project_root)
        .output()
        .map(|out| out.status.success() && !out.stdout.is_empty())
        .unwrap_or(false)
}

/// `true` when any worktree under `project_root` is locked. Mirrors the more
/// thorough check inside `gwt_core::migration::validator::check_locked_worktrees`.
fn detect_locked_worktrees(project_root: &Path) -> bool {
    let Ok(output) = gwt_core::process::hidden_command("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(project_root)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.starts_with("locked"))
}

#[derive(Debug, Clone)]
pub struct WindowAddress {
    pub(crate) tab_id: String,
    pub(crate) raw_id: String,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardSession {
    pub(crate) tab_id: String,
    pub(crate) wizard_id: String,
    pub(crate) wizard: LaunchWizardState,
    pub(crate) workspace_resume_context: Option<WorkspaceResumeContext>,
}

#[derive(Debug, Clone)]
pub struct LaunchFeedbackContext {
    pub(crate) client_id: ClientId,
    pub(crate) title: String,
}

#[derive(Debug, Clone)]
pub struct IssueLaunchWizardPrepared {
    pub(crate) client_id: ClientId,
    pub(crate) id: String,
    pub(crate) knowledge_kind: KnowledgeKind,
    pub(crate) tab_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) issue_number: u64,
    pub(crate) result: Result<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProjectOpenTarget {
    pub(crate) project_root: PathBuf,
    pub(crate) title: String,
    pub(crate) kind: gwt::ProjectKind,
    /// `true` when the resolved layout is a Normal Git checkout that gwt would
    /// like to migrate to its Nested Bare+Worktree convention (SPEC-1934 US-6).
    pub(crate) needs_migration: bool,
}

pub struct AppRuntime {
    pub(crate) tabs: Vec<ProjectTabRuntime>,
    pub(crate) active_tab_id: Option<String>,
    pub(crate) recent_projects: Vec<gwt::RecentProjectEntry>,
    pub(crate) profile_selections: HashMap<String, String>,
    pub(crate) profile_config_path: Option<PathBuf>,
    pub(crate) runtimes: HashMap<String, WindowRuntime>,
    pub(crate) window_details: HashMap<String, String>,
    pub(crate) window_lookup: HashMap<String, WindowAddress>,
    pub(crate) board_all_view_windows: HashSet<String>,
    pub(crate) session_state_path: PathBuf,
    pub(crate) log_dir: PathBuf,
    pub(crate) proxy: AppEventProxy,
    pub(crate) blocking_tasks: BlockingTaskSpawner,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) launch_wizard_cache: LaunchWizardMemoryCache,
    pub(crate) launch_wizard: Option<LaunchWizardSession>,
    pub(crate) pending_workspace_resume_contexts: HashMap<String, WorkspaceResumeContext>,
    pub(crate) pending_launch_feedback_contexts: HashMap<String, LaunchFeedbackContext>,
    pub(crate) pending_auto_resume_sources: HashMap<String, String>,
    pub(crate) pending_startup_auto_resume_sessions: Vec<PendingStartupAutoResumeSession>,
    pub(crate) active_agent_sessions: HashMap<String, ActiveAgentSession>,
    pub(crate) window_pty_statuses: HashMap<String, WindowProcessStatus>,
    pub(crate) window_hook_states: HashMap<String, WindowProcessStatus>,
    pub(crate) hook_forward_target: Option<HookForwardTarget>,
    pub(crate) issue_link_cache_dir: PathBuf,
    /// Cached update state so late-connecting WebView clients get the toast.
    pub(crate) pending_update: Option<gwt_core::update::UpdateState>,
    /// Shared PTY writer registry published to the WebSocket fast-path.
    pub(crate) pty_writers: PtyWriterRegistry,
    /// Async writer that flushes session/workspace snapshots off the event
    /// loop thread (Issue #2694 Phase B).
    pub(crate) persist_dispatcher: persist_dispatcher::PersistDispatcher,
    /// SPEC-2009 amendment: per-window selected worktree root for File Tree
    /// windows. Reset every time the user reopens the picker, so this is a
    /// transient in-memory map and is not persisted with the session state.
    pub(crate) file_tree_worktree_roots: HashMap<String, PathBuf>,
    /// SPEC-2785 FR-E: embedded server URL captured after the axum bind so
    /// `open_server_url_events` can reject requests whose origin differs from
    /// the bound URL. `None` before the server is started (e.g. during early
    /// AppRuntime construction or unit tests that never call
    /// `set_server_url`).
    pub(crate) server_url: Option<String>,
}

impl ProjectTabRuntime {
    pub(crate) fn from_persisted(
        tab: gwt::PersistedSessionTabState,
        workspace: gwt::PersistedWorkspaceState,
    ) -> Self {
        Self {
            id: tab.id,
            title: tab.title,
            project_root: tab.project_root,
            kind: tab.kind,
            workspace: WorkspaceState::from_persisted(workspace),
            // Re-detected at startup via resolve_project_target; persistence
            // does not carry the flag.
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }
}

impl AppRuntime {
    pub(crate) fn new(
        proxy: EventLoopProxy<UserEvent>,
        pty_writers: PtyWriterRegistry,
        blocking_tasks: BlockingTaskSpawner,
    ) -> std::io::Result<Self> {
        let session_state_path = gwt_core::paths::gwt_session_state_path();
        let launch_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let log_dir = gwt_core::paths::gwt_project_logs_dir_for_project_path(&launch_dir);
        let legacy_target = resolve_project_target(&launch_dir)
            .unwrap_or_else(|_| fallback_project_target(launch_dir.clone()));
        migrate_legacy_workspace_state(
            &gwt::legacy_workspace_state_path(),
            &session_state_path,
            &legacy_target.project_root,
            legacy_target.kind,
        )?;
        let persisted = load_session_state(&session_state_path)?;
        let tabs = persisted
            .tabs
            .into_iter()
            .map(|tab| {
                let workspace = load_restored_workspace_state(&tab.project_root)?;
                Ok(ProjectTabRuntime::from_persisted(tab, workspace))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let active_tab_id = normalize_active_tab_id(&tabs, persisted.active_tab_id);
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let _ = gwt_agent::reset_runtime_state_dir(&sessions_dir);
        let launch_wizard_cache = LaunchWizardMemoryCache::load(&sessions_dir);

        let persist_dispatcher = persist_dispatcher::PersistDispatcher::new(&blocking_tasks);
        let mut app = Self {
            tabs,
            active_tab_id,
            recent_projects: prune_missing_recent_projects(dedupe_recent_projects(
                normalize_recent_projects(persisted.recent_projects),
            )),
            profile_selections: HashMap::new(),
            profile_config_path: None,
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            board_all_view_windows: HashSet::new(),
            session_state_path,
            log_dir,
            proxy: AppEventProxy::new(proxy),
            blocking_tasks,
            sessions_dir,
            launch_wizard_cache,
            launch_wizard: None,
            pending_workspace_resume_contexts: HashMap::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::new(),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            pending_update: None,
            pty_writers,
            persist_dispatcher,
            file_tree_worktree_roots: HashMap::new(),
            server_url: None,
        };
        app.rebuild_window_lookup();
        app.seed_window_pty_statuses();
        app.seed_restored_window_details();
        Ok(app)
    }

    pub(crate) fn bootstrap(&mut self) {
        // SPEC-2359 US-37 / FR-119 / FR-123: One-shot retroactive migration to
        // mark historical merged `work/*` Start Work Workspaces as Done so the
        // Workspace Overview Completed column reflects past completions on the
        // first startup after auto-done emission lands. The scan is idempotent
        // per `work_item_id` and skips silently when journal / work_events
        // files are missing or unreadable.
        let now = chrono::Utc::now();
        for tab in &self.tabs {
            let _ =
                gwt_core::workspace_projection::retroactive_auto_done_scan(&tab.project_root, now);
            // SPEC-2359 US-39 / FR-142..145: backfill Phase U-6 schema
            // additions (`summary`, `created_at`, `creator`,
            // `lifecycle_stage`) on legacy `workspace.json` files. Runs
            // alongside the auto-done scan above with independent helpers
            // and an independent `workspace.migration.json` marker, so the
            // two migrations are exactly-once each and never duplicate work.
            // Errors are silently dropped (`let _ = ...`) so a corrupt or
            // unreadable Workspace cannot block daemon startup.
            let _ = gwt_core::workspace_projection_migration::migrate_workspace_projection_for_repo(
                &tab.project_root,
            );
            // SPEC-2359 US-37: One-shot rebuild of work_items.json from the
            // event log. Recovers legacy installations whose work_items.json
            // shows status=active/idle for items that already have a Done
            // event in work_events.jsonl (caused by the old apply_event
            // semantics that regressed Done on subsequent update events).
            // Idempotent via `work_items.migration.json` marker.
            let _ = gwt_core::workspace_projection::rebuild_work_items_from_events_for_repo(
                &tab.project_root,
            );
        }

        self.queue_startup_auto_resume_sessions();

        let windows = self
            .tabs
            .iter()
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .clone()
                    .into_iter()
                    .map(|window| (tab.id.clone(), window))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (tab_id, window) in windows {
            if !should_auto_start_restored_window(&window) {
                continue;
            }
            let _ = self.start_window(&tab_id, &window.id, window.preset, window.geometry.clone());
        }
        let _ = self.persist();
    }

    fn queue_startup_auto_resume_sessions(&mut self) {
        self.pending_startup_auto_resume_sessions.clear();
        let mut sessions = self.load_recovery_sessions();
        sessions.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| left.id.cmp(&right.id))
        });

        let now = chrono::Utc::now();
        let mut resumed_native_sessions = std::collections::HashSet::new();
        for session in sessions {
            if !startup_auto_resume_window_was_open(&session) {
                continue;
            }
            if !session.exact_auto_resume_candidate() {
                continue;
            }
            if !startup_auto_resume_is_fresh(&session, now) {
                continue;
            }
            let Some(native_session_id) = session.exact_resume_session_id() else {
                continue;
            };
            if !resumed_native_sessions.insert(native_session_id.to_string()) {
                continue;
            }
            if self
                .active_agent_sessions
                .values()
                .any(|active| active.session_id == session.id)
            {
                continue;
            }
            let Some(tab_id) = self.auto_resume_tab_id_for_session(&session) else {
                continue;
            };
            let Some(tab) = self.tab(&tab_id) else {
                continue;
            };
            if tab.kind != gwt::ProjectKind::Git || tab.migration_pending {
                continue;
            }
            let config = launch_config_from_persisted_session(&session);
            if config.session_mode != gwt_agent::SessionMode::Resume {
                continue;
            }
            let workspace_resume_context =
                gwt_core::workspace_projection::load_workspace_projection(&session.worktree_path)
                    .ok()
                    .flatten()
                    .map(|projection| workspace_resume_context_from_projection(&projection));
            self.pending_startup_auto_resume_sessions
                .push(PendingStartupAutoResumeSession {
                    tab_id,
                    session,
                    workspace_resume_context,
                });
        }
    }

    fn startup_auto_resume_ready_events(&mut self, bounds: WindowGeometry) -> Vec<OutboundEvent> {
        if self.pending_startup_auto_resume_sessions.is_empty() {
            return Vec::new();
        }

        let pending = std::mem::take(&mut self.pending_startup_auto_resume_sessions);
        let total = pending.len();
        let mut events = Vec::new();
        for (index, pending_session) in pending.into_iter().enumerate() {
            let config = launch_config_from_persisted_session(&pending_session.session);
            let existing_windows = self
                .window_lookup
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let geometry = startup_auto_resume_window_geometry(index, total, bounds.clone());
            match self.spawn_agent_window_at_geometry(
                &pending_session.tab_id,
                config,
                geometry,
                pending_session.workspace_resume_context,
            ) {
                Ok(mut spawned_events) => events.append(&mut spawned_events),
                Err(error) => {
                    tracing::warn!(
                        session_id = %pending_session.session.id,
                        error = %error,
                        "failed to spawn startup auto-resume agent window"
                    );
                    continue;
                }
            }
            if let Some(window_id) = self
                .window_lookup
                .keys()
                .find(|window_id| !existing_windows.contains(*window_id))
                .cloned()
            {
                self.pending_auto_resume_sources
                    .insert(window_id, pending_session.session.id);
            }
        }
        events
    }

    fn auto_resume_tab_id_for_session(&self, session: &gwt_agent::Session) -> Option<String> {
        if let Some(tab) = self.tabs.iter().find(|tab| {
            tab.kind == gwt::ProjectKind::Git
                && !tab.migration_pending
                && same_worktree_path(&tab.project_root, &session.worktree_path)
        }) {
            return Some(tab.id.clone());
        }

        let session_scope = session_project_scope_hash(session)?;
        self.tabs
            .iter()
            .find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && gwt_core::paths::project_scope_hash(&tab.project_root).to_string()
                        == session_scope
            })
            .map(|tab| tab.id.clone())
    }

    fn load_recovery_sessions(&self) -> Vec<gwt_agent::Session> {
        let Ok(entries) = std::fs::read_dir(&self.sessions_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"))
            .filter_map(|path| {
                let mut session = gwt_agent::Session::load_and_migrate(&path).ok()?;
                if session.worktree_path.exists()
                    && session.should_mark_interrupted_from_lifecycle()
                {
                    session.update_status(gwt_agent::AgentStatus::Interrupted);
                }
                let _ = session.save(&self.sessions_dir);
                Some(session)
            })
            .collect()
    }

    pub(crate) fn set_hook_forward_target(&mut self, target: HookForwardTarget) {
        self.hook_forward_target = Some(target);
    }

    /// SPEC-2785 FR-E: capture the embedded server URL after the axum bind
    /// completes so `open_server_url_events` can reject mismatched origin
    /// requests before invoking the OS opener.
    pub(crate) fn set_server_url(&mut self, url: String) {
        self.server_url = Some(url);
    }

    pub(crate) fn handle_frontend_event(
        &mut self,
        client_id: ClientId,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        log_frontend_user_action(&client_id, &event);
        match event {
            FrontendEvent::FrontendReady => self.frontend_sync_events(&client_id),
            FrontendEvent::StartupAutoResumeReady { bounds } => {
                self.startup_auto_resume_ready_events(bounds)
            }
            FrontendEvent::OpenProjectDialog => self.open_project_dialog_events(),
            FrontendEvent::SelectCloneProjectParent => {
                self.select_clone_project_parent_events(&client_id)
            }
            FrontendEvent::GithubRepositorySearch { query } => {
                self.github_repository_search_events(&client_id, &query)
            }
            FrontendEvent::CloneProjectStart { url, parent_path } => {
                self.clone_project_start_events(&client_id, &url, &parent_path)
            }
            FrontendEvent::ReopenRecentProject { path } => {
                self.open_project_path_events(PathBuf::from(path))
            }
            FrontendEvent::SelectProjectTab { tab_id } => self.select_project_tab_events(&tab_id),
            FrontendEvent::CloseProjectTab { tab_id } => self.close_project_tab_events(&tab_id),
            FrontendEvent::CreateWindow { preset, bounds } => {
                self.create_window_events(preset, bounds)
            }
            FrontendEvent::LoadProcessConsole { id } => {
                // SPEC-2809 Phase F2 — Console window mount asks for the
                // current ring buffer. Use the global hub installed by
                // `gwt_core::logging::init`. Reply to the requesting
                // client only so other Consoles do not see duplicates.
                let hub = gwt_core::process_console::global();
                vec![OutboundEvent::reply(
                    client_id.clone(),
                    BackendEvent::ProcessConsoleSnapshot {
                        id,
                        lines: hub.snapshot_all(),
                    },
                )]
            }
            FrontendEvent::FocusWindow { id, bounds } => self.focus_window_events(&id, bounds),
            FrontendEvent::CycleFocus { direction, bounds } => {
                self.cycle_focus_events(direction, bounds)
            }
            FrontendEvent::UpdateViewport { viewport } => self.update_viewport_events(viewport),
            FrontendEvent::ArrangeWindows { mode, bounds } => {
                self.arrange_windows_events(mode, bounds)
            }
            FrontendEvent::MaximizeWindow { id, bounds } => {
                self.maximize_window_events(&id, bounds)
            }
            FrontendEvent::MinimizeWindow { id } => self.minimize_window_events(&id),
            FrontendEvent::RestoreWindow { id } => self.restore_window_events(&id),
            FrontendEvent::DockWindowTab { id, target_id } => {
                self.dock_window_tab_events(&id, &target_id)
            }
            FrontendEvent::ActivateWindowTab { id } => self.activate_window_tab_events(&id),
            FrontendEvent::DetachWindowTab { id, geometry } => {
                self.detach_window_tab_events(&id, geometry)
            }
            FrontendEvent::ListWindows => {
                vec![OutboundEvent::reply(client_id, self.list_windows_event())]
            }
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry,
                cols,
                rows,
                base_geometry_revision,
            } => self.update_window_geometry_events(
                &id,
                geometry,
                cols,
                rows,
                base_geometry_revision,
            ),
            FrontendEvent::CloseWindow { id } => self.close_window_events(&id),
            FrontendEvent::TerminalInput { id, data } => self.terminal_input_events(&id, &data),
            FrontendEvent::PasteImage {
                id,
                data_base64,
                mime_type,
                filename,
            } => self.paste_image_events(&id, &data_base64, &mime_type, filename.as_deref()),
            FrontendEvent::LoadFileTree { id, path } => {
                let path = path.unwrap_or_default();
                vec![OutboundEvent::reply(
                    client_id,
                    self.load_file_tree_event(&id, &path),
                )]
            }
            FrontendEvent::ListFileTreeWorktrees { id } => vec![OutboundEvent::reply(
                client_id,
                self.list_file_tree_worktrees_event(&id),
            )],
            FrontendEvent::SelectFileTreeWorktree { id, worktree_id } => {
                vec![OutboundEvent::reply(
                    client_id,
                    self.select_file_tree_worktree_event(&id, &worktree_id),
                )]
            }
            FrontendEvent::LoadFileContent {
                id,
                path,
                mode,
                hex_offset,
                hex_length,
            } => vec![OutboundEvent::reply(
                client_id,
                self.load_file_content_event(&id, &path, mode, hex_offset, hex_length),
            )],
            FrontendEvent::SaveFileContent {
                id,
                path,
                mode,
                expected_mtime,
                expected_size,
                text,
                encoding,
                newline,
                has_bom,
                hex_offset,
                hex_byte,
            } => vec![OutboundEvent::reply(
                client_id,
                self.save_file_content_event(
                    &id,
                    &path,
                    mode,
                    expected_mtime,
                    expected_size,
                    text,
                    encoding,
                    newline,
                    has_bom,
                    hex_offset,
                    hex_byte,
                ),
            )],
            FrontendEvent::LoadBranches { id } => self.load_branches_events(&client_id, &id),
            FrontendEvent::LoadBoard { id, all } => self.load_board_events(&client_id, &id, all),
            FrontendEvent::LoadBoardHistory {
                id,
                before_entry_id,
                limit,
                all,
            } => self.load_board_history_events(
                &client_id,
                &id,
                before_entry_id.as_deref(),
                limit,
                all,
            ),
            FrontendEvent::LoadProfile { id } => self.load_profile_events(&client_id, &id),
            FrontendEvent::LoadLogs { id } => self.load_logs_events(&client_id, &id),
            FrontendEvent::LoadKnowledgeBridge {
                id,
                knowledge_kind,
                request_id,
                selected_number,
                refresh,
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number,
                    refresh,
                },
            ),
            FrontendEvent::SearchKnowledgeBridge {
                id,
                knowledge_kind,
                query,
                request_id,
                selected_number,
            } => self.search_knowledge_bridge_events(
                &client_id,
                KnowledgeSearchRequest {
                    id: &id,
                    kind: knowledge_kind,
                    query: &query,
                    request_id,
                    selected_number,
                },
            ),
            FrontendEvent::SearchProjectIndex {
                id,
                query,
                request_id,
                scopes,
                worktree_hash,
                match_mode,
            } => self.search_project_index_events(
                &client_id,
                ProjectIndexSearchRequest {
                    id: &id,
                    query: &query,
                    request_id,
                    scopes,
                    worktree_hash,
                    match_mode,
                },
            ),
            FrontendEvent::SelectKnowledgeBridgeEntry {
                id,
                knowledge_kind,
                request_id,
                number,
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number: Some(number),
                    refresh: false,
                },
            ),
            FrontendEvent::UpdateKnowledgeBridgePhase {
                id,
                request_id,
                issue_number,
                target_phase,
            } => self.update_knowledge_bridge_phase_events(
                &client_id,
                &id,
                request_id,
                issue_number,
                target_phase.as_deref(),
            ),
            FrontendEvent::RunBranchCleanup {
                id,
                branches,
                delete_remote,
                force_filesystem_delete,
            } => self.run_branch_cleanup_events(
                &client_id,
                &id,
                &branches,
                delete_remote,
                force_filesystem_delete,
            ),
            FrontendEvent::RunWorkspaceCleanup {
                branch,
                delete_remote,
                force_filesystem_delete,
            } => self.run_workspace_cleanup_events(
                &client_id,
                &branch,
                delete_remote,
                force_filesystem_delete,
            ),
            FrontendEvent::RebuildIndexCell {
                project_root,
                scope,
                worktree_hash,
            } => self.rebuild_index_cell_events(project_root, scope, worktree_hash),
            FrontendEvent::RefreshIndexStatus { project_root } => {
                self.refresh_index_status_events(project_root)
            }
            FrontendEvent::PostBoardEntry {
                id,
                entry_kind,
                body,
                parent_id,
                topics,
                owners,
                targets,
                mentions,
            } => self.post_board_entry_events(
                &client_id,
                BoardPostRequest {
                    id,
                    entry_kind,
                    body,
                    parent_id,
                    topics,
                    owners,
                    targets,
                    mentions,
                },
            ),
            FrontendEvent::OpenBoardOriginAgent {
                id,
                origin_session_id,
                bounds,
            } => self.open_board_origin_agent_events(&client_id, &id, &origin_session_id, bounds),
            FrontendEvent::SelectProfile { id, profile_name } => {
                self.select_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::CreateProfile { id, name } => {
                self.create_profile_events(&client_id, &id, &name)
            }
            FrontendEvent::SetActiveProfile { id, profile_name } => {
                self.set_active_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::SaveProfile {
                id,
                current_name,
                name,
                description,
                env_vars,
                disabled_env,
            } => self.save_profile_events(
                &client_id,
                &id,
                ProfileSaveRequest {
                    current_name,
                    name,
                    description,
                    env_vars,
                    disabled_env,
                },
            ),
            FrontendEvent::DeleteProfile { id, profile_name } => {
                self.delete_profile_events(&client_id, &id, &profile_name)
            }
            FrontendEvent::OpenIssueLaunchWizard { id, issue_number } => {
                self.open_issue_launch_wizard_events(&client_id, &id, issue_number)
            }
            FrontendEvent::OpenStartWork => self.open_start_work(&client_id),
            FrontendEvent::ResumeWorkspace { source, journal_id } => {
                self.resume_workspace_events(&client_id, source, journal_id)
            }
            FrontendEvent::ListResumableAgents { workspace_id } => {
                self.list_resumable_agents_events(&client_id, workspace_id)
            }
            FrontendEvent::ResumeWorkspaceAgent { session_id, bounds } => {
                self.resume_workspace_agent_events(&client_id, session_id, bounds)
            }
            FrontendEvent::ResumeBranchLatestAgent {
                id,
                branch_name,
                bounds,
            } => self.resume_branch_latest_agent_events(&client_id, &id, &branch_name, bounds),
            FrontendEvent::OpenLaunchWizard {
                id,
                branch_name,
                linked_issue_number,
            } => self.open_launch_wizard(&client_id, &id, &branch_name, linked_issue_number),
            FrontendEvent::OpenActiveWorkLaunchWizard {
                branch_name,
                linked_issue_number,
            } => self.open_active_work_launch_wizard(&client_id, &branch_name, linked_issue_number),
            FrontendEvent::LaunchWizardAction { action, bounds } => {
                self.handle_launch_wizard_action_for_client(Some(&client_id), action, bounds)
            }
            FrontendEvent::ApplyUpdate => self.apply_pending_update_events(&client_id),
            FrontendEvent::ApplyUpdateStart => self.apply_update_start_events(&client_id),
            FrontendEvent::CancelUpdateDownload => self.cancel_update_download_events(&client_id),
            FrontendEvent::ApplyUpdateLater => self.apply_update_later_events(&client_id),
            FrontendEvent::ApplyUpdateRestartNow => {
                self.apply_update_restart_now_events(&client_id)
            }
            FrontendEvent::OpenUpdateLog { log_path } => {
                self.open_update_log_events(&client_id, log_path)
            }
            FrontendEvent::OpenServerUrl { url } => self.open_server_url_events(&client_id, url),
            FrontendEvent::ListCustomAgents => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::list_event(),
            )],
            FrontendEvent::ListCustomAgentPresets => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::list_presets_event(),
            )],
            FrontendEvent::AddCustomAgentFromPreset { input } => {
                let event = gwt::custom_agents_dispatch::add_from_preset_event(
                    gwt::PresetId::ClaudeCodeOpenaiCompat,
                    serde_json::to_value(input)
                        .expect("custom agent preset payload should serialize"),
                );
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::UpdateCustomAgent { agent } => {
                let event = gwt::custom_agents_dispatch::update_event(*agent);
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::DeleteCustomAgent { agent_id } => {
                let event = gwt::custom_agents_dispatch::delete_event(agent_id);
                self.custom_agent_reply_with_cache_refresh(client_id, event)
            }
            FrontendEvent::TestBackendConnection { base_url, api_key } => {
                self.spawn_backend_connection_probe(client_id, base_url, api_key);
                Vec::new()
            }
            FrontendEvent::ListAgentBackends { agent } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::list_event(agent),
            )],
            FrontendEvent::AddAgentBackend { agent, profile } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::add_event(agent, *profile),
            )],
            FrontendEvent::UpdateAgentBackend { agent, id, profile } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::update_event(agent, id, *profile),
            )],
            FrontendEvent::DeleteAgentBackend { agent, id } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::delete_event(agent, id),
            )],
            FrontendEvent::TestAgentBackendConnection {
                agent,
                base_url,
                api_key,
            } => vec![OutboundEvent::reply(
                client_id,
                gwt::agent_backend_dispatch::test_connection_event(agent, &base_url, &api_key),
            )],
            FrontendEvent::StartMigration { tab_id } => self.start_migration_events(&tab_id),
            FrontendEvent::SkipMigration { tab_id } => self.skip_migration_events(&tab_id),
            FrontendEvent::QuitMigration { tab_id } => self.quit_migration_events(&tab_id),
            FrontendEvent::GetSystemSettings => self.system_settings_get_events(client_id),
            FrontendEvent::UpdateSystemSettings {
                language,
                codex_trust_managed_hooks,
            } => self.system_settings_update_events(client_id, language, codex_trust_managed_hooks),
            FrontendEvent::WorkspaceProjectionPrune { dry_run, ids } => {
                self.workspace_projection_prune_events(client_id, dry_run, ids)
            }
            FrontendEvent::SaveUiTrace { trace } => self.save_ui_trace_events(client_id, trace),
            FrontendEvent::OpenReleaseNotes { id, focus_version } => {
                self.release_notes_events(client_id, id, focus_version)
            }
        }
    }

    /// SPEC #2780: serve the bundled `CHANGELOG.md` to the Release Notes
    /// window. The parse runs once per process (cached) so this handler is
    /// effectively a copy from a static slice.
    fn release_notes_events(
        &self,
        client_id: ClientId,
        id: String,
        focus_version: Option<String>,
    ) -> Vec<OutboundEvent> {
        let entries = gwt_core::release_notes::bundled_releases();
        let event = if entries.is_empty() {
            BackendEvent::ReleaseNotesError {
                id,
                message: "Release notes could not be loaded.".to_string(),
            }
        } else {
            BackendEvent::ReleaseNotesPayload {
                id,
                entries: entries.to_vec(),
                focus_version,
            }
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    fn save_ui_trace_events(
        &self,
        client_id: ClientId,
        trace: UiTracePayload,
    ) -> Vec<OutboundEvent> {
        let event = match save_ui_trace_to_log_dir(&self.log_dir, trace) {
            Ok(result) => BackendEvent::UiTraceSaved {
                path: result.path.display().to_string(),
                entries: result.entries,
            },
            Err(message) => BackendEvent::UiTraceError { message },
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// SPEC-2359 US-41 (FR-153, FR-154, FR-155): handle
    /// [`FrontendEvent::WorkspaceProjectionPrune`] by classifying every
    /// projection under `~/.gwt/projects/`, applying or previewing the plan,
    /// and replying with a count summary or an error.
    ///
    /// Note: `is_active_session` is `|_| false` here as a first-pass; a
    /// follow-up commit will bridge the live-window registry so currently
    /// running Agents block their owning Workspace from prune.
    fn workspace_projection_prune_events(
        &self,
        client_id: ClientId,
        dry_run: bool,
        ids: Vec<String>,
    ) -> Vec<OutboundEvent> {
        use gwt_core::paths::gwt_projects_dir;
        use gwt_core::workspace_projection::{
            apply_prune_plan, classify_workspace_projections, WorkspaceRetentionConfig,
        };

        let scan_root = gwt_projects_dir();
        let now = chrono::Utc::now();
        let config = WorkspaceRetentionConfig::default();
        let live_session_ids: std::collections::HashSet<String> =
            self.active_agent_sessions.keys().cloned().collect();
        let is_active_session =
            |projection: &gwt_core::workspace_projection::WorkspaceProjection| {
                projection
                    .agents
                    .iter()
                    .any(|agent| live_session_ids.contains(&agent.session_id))
            };
        let plan = classify_workspace_projections(&scan_root, &config, now, is_active_session);
        let filtered: Vec<_> = if ids.is_empty() {
            plan
        } else {
            plan.into_iter()
                .filter(|item| ids.iter().any(|id| id == &item.workspace_id))
                .collect()
        };

        match apply_prune_plan(&filtered, dry_run) {
            Ok(summary) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::WorkspaceProjectionPruneResult {
                    mode: if dry_run {
                        "dry_run".to_string()
                    } else {
                        "applied".to_string()
                    },
                    archived: summary.archived,
                    deleted: summary.deleted,
                    skipped: summary.skipped,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::WorkspaceProjectionPruneError {
                    message: error.to_string(),
                },
            )],
        }
    }

    fn system_settings_get_events(&self, client_id: ClientId) -> Vec<OutboundEvent> {
        let path = match gwt_config::Settings::global_config_path() {
            Some(p) => p,
            None => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::SystemSettingsError {
                        message: "unable to resolve home directory (`~/.gwt/config.toml`)"
                            .to_string(),
                    },
                )];
            }
        };
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::get_event(&path),
        )]
    }

    fn system_settings_update_events(
        &self,
        client_id: ClientId,
        language: String,
        codex_trust_managed_hooks: Option<bool>,
    ) -> Vec<OutboundEvent> {
        let path = match gwt_config::Settings::global_config_path() {
            Some(p) => p,
            None => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::SystemSettingsError {
                        message: "unable to resolve home directory (`~/.gwt/config.toml`)"
                            .to_string(),
                    },
                )];
            }
        };
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::update_event(&path, language, codex_trust_managed_hooks),
        )]
    }

    fn custom_agent_reply_with_cache_refresh(
        &mut self,
        client_id: ClientId,
        event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        if matches!(
            &event,
            BackendEvent::CustomAgentSaved { .. } | BackendEvent::CustomAgentDeleted { .. }
        ) {
            self.launch_wizard_cache.refresh_agent_options();
            let had_open_wizard = self.launch_wizard.is_some();
            self.refresh_open_launch_wizard_from_cache();
            let mut events = vec![OutboundEvent::reply(client_id, event)];
            if had_open_wizard {
                events.push(self.launch_wizard_state_outbound());
            }
            return events;
        }
        vec![OutboundEvent::reply(client_id, event)]
    }

    pub(crate) fn spawn_backend_connection_probe(
        &self,
        client_id: ClientId,
        base_url: String,
        api_key: String,
    ) {
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = gwt::custom_agents_dispatch::test_connection_event(&base_url, &api_key);
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    fn apply_pending_update_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdate {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            Some(gwt_core::update::UpdateState::Available { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message(
                    "No applicable update asset is available for this platform.",
                ),
            )],
            Some(gwt_core::update::UpdateState::UpToDate { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message("No pending update is available."),
            )],
            Some(gwt_core::update::UpdateState::Failed { message, .. }) => {
                vec![OutboundEvent::reply(
                    client_id,
                    update_apply_error_message(&format!("Update check failed: {message}")),
                )]
            }
            None => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message("No pending update is available."),
            )],
        }
    }

    /// SPEC-2041 Phase 19 (FR-052): user clicked the update CTA and the modal
    /// is opening in the `downloading` state. Backend kicks off
    /// `prepare_update` on a worker thread and emits
    /// [`BackendEvent::UpdateReady`] (or [`BackendEvent::UpdateApplyError`])
    /// without exiting the parent process.
    fn apply_update_start_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdateStart {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            Some(gwt_core::update::UpdateState::Available { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed(
                    "Download asset",
                    "No applicable update asset is available for this platform.",
                ),
            )],
            Some(gwt_core::update::UpdateState::UpToDate { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Download asset", "No pending update is available."),
            )],
            Some(gwt_core::update::UpdateState::Failed { message, .. }) => {
                vec![OutboundEvent::reply(
                    client_id,
                    update_apply_error_failed(
                        "Update check",
                        &format!("Update check failed: {message}"),
                    ),
                )]
            }
            None => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Download asset", "No pending update is available."),
            )],
        }
    }

    /// SPEC-2041 Phase 19 (FR-055): user pressed `Cancel` mid-download.
    /// `prepare_update` runs synchronously on a worker thread, so a true
    /// mid-download abort is best-effort. We still defensively clear any
    /// `~/.gwt/pending-update/manifest.json` that the worker may have
    /// persisted between the user's click and the modal close — without this
    /// guard, a race would leave the bootstrap path applying an update the
    /// user explicitly cancelled (CodeRabbit P1 review on PR #2630).
    fn cancel_update_download_events(&self, _client_id: &str) -> Vec<OutboundEvent> {
        let _ = gwt_core::update::clear_pending_update_manifest();
        vec![]
    }

    /// SPEC-2041 Phase 19 (FR-059..061, FR-064): user pressed `Later`.
    /// Verifies the manifest persisted by `ApplyUpdateStart`'s worker thread
    /// is still on disk via [`crate::update_front_door::commit_update_later_pending`],
    /// then emits [`BackendEvent::UpdateApplyPendingPersisted`] so the CTA
    /// morphs to ready state. If persistence somehow vanished (external
    /// cleanup, disk-full race), surface a structured error instead of
    /// silently lying about pending state.
    fn apply_update_later_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let version = match self.pending_update.as_ref() {
            Some(gwt_core::update::UpdateState::Available { latest, .. }) => latest.clone(),
            _ => return vec![],
        };
        match crate::update_front_door::commit_update_later_pending() {
            Ok(()) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::UpdateApplyPendingPersisted { version },
            )],
            Err(message) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Persist pending", &message),
            )],
        }
    }

    /// SPEC-2041 Phase 19 (FR-058): user pressed `Restart now`. Backend
    /// commits the prepared payload via the helper subprocess and exits the
    /// parent. Falls back to the legacy `apply_update_state_and_exit` path
    /// when no prepared payload exists yet (e.g. user manually re-clicked CTA
    /// before download completed).
    fn apply_update_restart_now_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdateRestartNow {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            _ => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed(
                    "Restart now",
                    "No prepared update available for restart.",
                ),
            )],
        }
    }

    /// SPEC-2785 US-1 / FR-C / FR-E: user clicked the server URL cell in the
    /// status strip. The renderer-supplied `url` is treated as untrusted and
    /// is only forwarded to [`open_url_with_os_default`] when it matches the
    /// embedded server's bound URL captured by [`Self::set_server_url`].
    /// Mismatched origins (or an unset server URL) are dropped with a trace
    /// log so a compromised renderer cannot redirect the OS opener to an
    /// arbitrary URL. The handler returns no outbound events; the click is a
    /// side-effect only.
    fn open_server_url_events(&self, _client_id: &str, url: String) -> Vec<OutboundEvent> {
        if validate_server_url(self.server_url.as_deref(), &url) {
            if let Err(error) = open_url_with_os_default(&url) {
                tracing::trace!(
                    target: "gwt::open_server_url",
                    %error,
                    "failed to spawn OS browser opener"
                );
            }
        } else {
            tracing::trace!(
                target: "gwt::open_server_url",
                requested = %url,
                allowed = ?self.server_url,
                "rejected open_server_url request: origin mismatch"
            );
        }
        Vec::new()
    }

    /// SPEC-2041 Phase 19 (FR-065): user pressed `Open log` on the failed
    /// modal. Backend opens the log file with the OS default application.
    /// The renderer-supplied `log_path` is treated as untrusted: the path
    /// must canonicalize to a child of the gwt logs directory, must exist as
    /// a file, and must not contain a URL scheme (CodeRabbit review on PR
    /// #2630). Validation failures are silently dropped — the modal already
    /// surfaces the in-memory `Reason` so a missing log file is not blocking.
    fn open_update_log_events(
        &self,
        _client_id: &str,
        log_path: Option<String>,
    ) -> Vec<OutboundEvent> {
        if let Some(raw) = log_path {
            // Derive the allowed logs root from the canonical update log
            // resolver itself. AppRuntime is not allowed to call the legacy
            // `gwt_logs_dir()` directly (project-scoped resolver test in
            // main.rs), so we ride on `update_log_path()`'s parent.
            if let Some(logs_root) = gwt_core::update::update_log_path()
                .parent()
                .map(|p| p.to_path_buf())
            {
                if let Some(safe) = validate_update_log_path(&raw, &logs_root) {
                    let _ = open_path_with_os_default(&safe.to_string_lossy());
                }
            }
        }
        vec![]
    }

    pub(crate) fn frontend_sync_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let terminal_statuses = self
            .window_details
            .iter()
            .filter_map(|(id, detail)| {
                self.window_status(id)
                    .map(|status| (id.clone(), status, detail.clone()))
            })
            .collect();
        let terminal_snapshots = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                // SPEC-1919 FR-001a: snapshot replay must reproduce SGR
                // attributes (color, bold, italic, underline, inverse) so
                // tab switch / focus cycle / WebSocket reconnect do not
                // collapse colored history into default-color text. Use
                // vt100 `Screen::contents_formatted()` which emits a CSI
                // escape stream xterm.js can replay verbatim, instead of
                // `Screen::contents()` which strips formatting.
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.screen().contents_formatted())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then_some((id.clone(), snapshot))
            })
            .collect();

        let mut events = build_frontend_sync_events(
            client_id,
            self.app_state_view(),
            terminal_statuses,
            terminal_snapshots,
            self.launch_wizard
                .as_ref()
                .map(|wizard| wizard.wizard.view()),
            self.pending_update.clone(),
        );
        if let Some(event) = self.active_work_projection_reply(client_id) {
            events.insert(1, event);
        }
        // SPEC-1934 US-6.1: surface pending migrations to a newly-connected
        // frontend during state hydration so the modal opens without waiting
        // for another roundtrip.
        events.extend(self.migration_detected_replies(client_id));
        events.extend(self.migration_recovery_replies(client_id));
        events
    }

    fn active_work_projection_reply(&self, client_id: &str) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self.active_work_projection_for_tab(tab_id, tab)?;
        Some(OutboundEvent::reply(
            client_id,
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    pub(crate) fn active_work_projection_broadcast_for_active_tab(&self) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self.active_work_projection_for_tab(tab_id, tab)?;
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    /// Like `active_work_projection_broadcast_for_active_tab`, but always emits an event
    /// when an active tab exists — falling back to an empty projection so that frontends
    /// clear stale per-project data when the tab focus moves to a project without
    /// any saved projection or live agent sessions.
    fn active_work_projection_broadcast_on_tab_change(&self) -> Option<OutboundEvent> {
        let tab_id = self.active_tab_id.as_ref()?;
        let tab = self.tab(tab_id)?;
        let projection = self
            .active_work_projection_for_tab(tab_id, tab)
            .unwrap_or_else(|| empty_active_work_projection_view(tab_id, tab));
        Some(OutboundEvent::broadcast(
            BackendEvent::ActiveWorkProjection {
                projection: Box::new(projection),
            },
        ))
    }

    fn active_work_projection_for_tab(
        &self,
        tab_id: &str,
        tab: &ProjectTabRuntime,
    ) -> Option<gwt::ActiveWorkProjectionView> {
        let sessions = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .collect::<Vec<_>>();
        if let Ok(Some(projection)) =
            gwt_core::workspace_projection::load_workspace_projection(&tab.project_root)
        {
            let mut projection = projection;
            let had_saved_agents = !projection.agents.is_empty();
            let cleanup_candidate =
                workspace_cleanup_candidate_for_projection(&projection, &sessions);
            merge_active_sessions_into_projection(
                &mut projection,
                sessions.iter().copied(),
                chrono::Utc::now(),
            );
            let updated_at = chrono::Utc::now();
            retain_live_workspace_agents(&mut projection, &sessions, updated_at);
            if had_saved_agents && !workspace_projection_has_current_agents(&projection) {
                reset_idle_workspace_current_identity(&mut projection, &tab.title, updated_at);
            }
            let journal_entries =
                gwt_core::workspace_projection::load_recent_workspace_journal_entries(
                    &tab.project_root,
                    WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
                )
                .unwrap_or_default()
                .iter()
                .map(workspace_journal_entry_view_from_entry)
                .collect::<Vec<_>>();
            let workspaces =
                gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(
                    &tab.project_root,
                )
                .unwrap_or_else(
                    |_| gwt_core::workspace_projection::WorkspaceWorkItemsProjection {
                        updated_at,
                        work_items: Vec::new(),
                    },
                )
                .work_items
                .iter()
                .map(workspace_work_item_view_from_item)
                .collect::<Vec<_>>();
            return Some(active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                cleanup_candidate,
            ));
        }

        let first = sessions.first()?;
        let active_agents = sessions.len();
        let now = chrono::Utc::now();
        let mut agents = sessions
            .iter()
            .map(|session| {
                let summary = active_agent_summary_from_session(session, now);
                active_work_agent_view_from_summary(&summary)
            })
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Some(gwt::ActiveWorkProjectionView {
            id: tab_id.to_string(),
            title: format!("{} workspace", tab.title),
            status_category: "active".to_string(),
            status_text: if active_agents == 1 {
                "1 active agent".to_string()
            } else {
                format!("{active_agents} active agents")
            },
            summary: None,
            owner: None,
            next_action: Some("Check Board for latest updates".to_string()),
            active_agents,
            blocked_agents: 0,
            branch: Some(first.branch_name.clone()),
            worktree_path: Some(first.worktree_path.display().to_string()),
            pr_number: None,
            pr_url: None,
            pr_state: None,
            pr_created_at: None,
            board_refs: Vec::new(),
            journal_entries: Vec::new(),
            workspaces: Vec::new(),
            cleanup_candidate: None,
            agents,
            unassigned_agents: Vec::new(),
        })
    }

    pub(crate) fn open_project_dialog_events(&mut self) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        self.open_project_path_events(path)
    }

    pub(crate) fn select_clone_project_parent_events(
        &mut self,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::CloneProjectParentSelected {
                path: path.display().to_string(),
            },
        )]
    }

    pub(crate) fn github_repository_search_events(
        &mut self,
        client_id: &str,
        query: &str,
    ) -> Vec<OutboundEvent> {
        match search_github_repositories(query, 20) {
            Ok(repositories) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::GithubRepositorySearchResults {
                    query: query.to_string(),
                    repositories,
                },
            )],
            Err(message) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::GithubRepositorySearchError {
                    query: query.to_string(),
                    message,
                },
            )],
        }
    }

    pub(crate) fn clone_project_start_events(
        &mut self,
        client_id: &str,
        url: &str,
        parent_path: &str,
    ) -> Vec<OutboundEvent> {
        let trimmed_url = url.trim();
        if trimmed_url.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::CloneProjectError {
                    message: "repository URL is required".to_string(),
                },
            )];
        }
        let trimmed_parent = parent_path.trim();
        if trimmed_parent.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::CloneProjectError {
                    message: "destination parent folder is required".to_string(),
                },
            )];
        }

        let proxy = self.proxy.clone();
        let url = trimmed_url.to_string();
        let parent = PathBuf::from(trimmed_parent);
        self.blocking_tasks.spawn(move || {
            proxy.send(UserEvent::CloneProjectProgress {
                message: "Cloning repository...".to_string(),
            });
            match gwt_git::clone_project_as_nested_bare(&url, &parent) {
                Ok(outcome) => proxy.send(UserEvent::CloneProjectDone {
                    workspace_home: outcome.workspace_home,
                }),
                Err(error) => proxy.send(UserEvent::CloneProjectError {
                    message: error.to_string(),
                }),
            }
        });

        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::CloneProjectProgress {
                message: "Cloning repository...".to_string(),
            },
        )]
    }

    pub(crate) fn open_project_path_events(&mut self, path: PathBuf) -> Vec<OutboundEvent> {
        match self.open_project_path(path) {
            Ok(wizard_closed) => {
                let mut events = vec![self.workspace_state_broadcast()];
                if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
                    events.push(event);
                }
                if wizard_closed {
                    events.push(self.launch_wizard_state_broadcast(None));
                }
                // SPEC-1934 US-6.1: when a tab was opened on a Normal Git
                // layout, surface the confirmation modal alongside the regular
                // workspace broadcast.
                events.extend(self.migration_detected_broadcasts());
                events.extend(self.migration_recovery_broadcasts());
                events
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                message: error,
            })],
        }
    }

    pub(crate) fn handle_clone_project_done(
        &mut self,
        workspace_home: &Path,
    ) -> Vec<OutboundEvent> {
        match self.open_project_path(workspace_home.to_path_buf()) {
            Ok(wizard_closed) => {
                self.remember_recent_clone_workspace_home(workspace_home);
                let _ = self.persist();
                let mut events = vec![
                    self.workspace_state_broadcast(),
                    OutboundEvent::broadcast(BackendEvent::CloneProjectDone {
                        workspace_home: workspace_home.display().to_string(),
                    }),
                ];
                if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
                    events.push(event);
                }
                if wizard_closed {
                    events.push(self.launch_wizard_state_broadcast(None));
                }
                events
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::CloneProjectError {
                message: error,
            })],
        }
    }

    fn remember_recent_clone_workspace_home(&mut self, workspace_home: &Path) {
        let canonical_home =
            dunce::canonicalize(workspace_home).unwrap_or_else(|_| workspace_home.to_path_buf());
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &canonical_home));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: canonical_home.clone(),
                title: gwt::project_title_from_path(&canonical_home),
                kind: gwt::ProjectKind::Git,
            },
        );
        if self.recent_projects.len() > 12 {
            self.recent_projects.truncate(12);
        }
    }

    pub(crate) fn open_project_path(&mut self, path: PathBuf) -> Result<bool, String> {
        let target = resolve_project_target(&path)?;
        if let Some(existing) = self
            .tabs
            .iter()
            .find(|tab| same_worktree_path(&tab.project_root, &target.project_root))
            .map(|tab| tab.id.clone())
        {
            let wizard_closed = self.set_active_tab(existing);
            self.remember_recent_project(&target);
            self.persist().map_err(|error| error.to_string())?;
            return Ok(wizard_closed);
        }

        let tab_id = format!("project-{}", Uuid::new_v4().simple());
        self.tabs.push(ProjectTabRuntime {
            id: tab_id.clone(),
            title: target.title.clone(),
            project_root: target.project_root.clone(),
            kind: target.kind,
            workspace: WorkspaceState::from_persisted({
                load_restored_workspace_state(&target.project_root)
                    .map_err(|error| error.to_string())?
            }),
            migration_pending: target.needs_migration,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        });
        self.active_tab_id = Some(tab_id);
        self.remember_recent_project(&target);
        let wizard_closed = self.clear_launch_wizard().is_some();
        self.persist().map_err(|error| error.to_string())?;
        Ok(wizard_closed)
    }

    fn migration_detected_event_for(&self, tab: &ProjectTabRuntime) -> BackendEvent {
        BackendEvent::MigrationDetected {
            tab_id: tab.id.clone(),
            project_root: tab.project_root.display().to_string(),
            branch: read_head_branch(&tab.project_root),
            has_dirty: detect_dirty(&tab.project_root),
            has_locked: detect_locked_worktrees(&tab.project_root),
            has_submodules: tab.project_root.join(".gitmodules").is_file(),
        }
    }

    fn has_migration_backup(tab: &ProjectTabRuntime) -> bool {
        tab.project_root
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME)
            .is_dir()
    }

    fn migration_backup_error_event_for(&self, tab: &ProjectTabRuntime) -> BackendEvent {
        let backup_path = tab
            .project_root
            .join(gwt_core::migration::backup::BACKUP_DIR_NAME);
        BackendEvent::MigrationError {
            tab_id: tab.id.clone(),
            phase: gwt_core::migration::MigrationPhase::Backup
                .as_str()
                .to_string(),
            message: format!(
                "Previous migration backup found at {}. A migration may have been interrupted before cleanup; inspect or restore the backup before starting another migration.",
                backup_path.display()
            ),
            recovery: recovery_state_label(gwt_core::migration::RecoveryState::Partial)
                .to_string(),
        }
    }

    /// SPEC-1934 US-6.1 broadcast variant: used by `open_project_path_events`
    /// to inform every connected frontend that a tab needs migration.
    pub(crate) fn migration_detected_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::broadcast(self.migration_detected_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 US-6.6/T-085: if a previous migration was interrupted after
    /// Backup, surface the leftover snapshot on launch so the user does not
    /// start another destructive migration over an unresolved backup.
    pub(crate) fn migration_recovery_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending && Self::has_migration_backup(tab))
            .map(|tab| OutboundEvent::broadcast(self.migration_backup_error_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 US-6.1 reply variant: used by `frontend_sync_events` so a
    /// freshly-connected frontend learns about pending migrations during
    /// state hydration without resending to other clients.
    pub(crate) fn migration_detected_replies(&self, client_id: &str) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::reply(client_id, self.migration_detected_event_for(tab)))
            .collect()
    }

    pub(crate) fn migration_recovery_replies(&self, client_id: &str) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending && Self::has_migration_backup(tab))
            .map(|tab| OutboundEvent::reply(client_id, self.migration_backup_error_event_for(tab)))
            .collect()
    }

    /// SPEC-1934 FR-019: user accepted the migration confirmation modal.
    ///
    /// Issue #2867: Recent Projects は同一プロジェクトの worktree で埋め尽く
    /// されないよう、`target.project_root` を workspace home に正規化してから
    /// 登録する。タブ open 時の direct-pick semantics は `target` 側で保持。
    pub(crate) fn remember_recent_project(&mut self, target: &ProjectOpenTarget) {
        let recent_path = normalize_recent_project_path(&target.project_root);
        let recent_title = if recent_path == target.project_root {
            target.title.clone()
        } else {
            gwt::project_title_from_path(&recent_path)
        };
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &recent_path));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: recent_path,
                title: recent_title,
                kind: target.kind,
            },
        );
        if self.recent_projects.len() > 12 {
            self.recent_projects.truncate(12);
        }
    }

    pub(crate) fn select_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return Vec::new();
        }
        let wizard_closed = self.set_active_tab(tab_id.to_string());
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        events
    }

    pub(crate) fn close_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return Vec::new();
        };

        let window_ids = self
            .tabs
            .get(index)
            .map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| combined_window_id(&tab.id, &window.id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for window_id in window_ids {
            self.clear_agent_window_startup_restore(&window_id);
            self.stop_window_runtime(&window_id);
            self.remove_window_state_tracking(&window_id);
            self.window_lookup.remove(&window_id);
            self.profile_selections.remove(&window_id);
        }

        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active_tab_id = None;
        } else if self.active_tab_id.as_deref() == Some(tab_id) {
            let next_index = index.saturating_sub(1).min(self.tabs.len() - 1);
            self.active_tab_id = self.tabs.get(next_index).map(|tab| tab.id.clone());
        }

        let wizard_closed = self
            .launch_wizard
            .as_ref()
            .is_some_and(|wizard| wizard.tab_id == tab_id);
        if wizard_closed {
            self.launch_wizard = None;
        }
        let _ = self.persist();

        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = self.active_work_projection_broadcast_on_tab_change() {
            events.push(event);
        }
        if wizard_closed {
            events.push(self.launch_wizard_state_broadcast(None));
        }
        events
    }

    pub(crate) fn terminal_input_events(&mut self, id: &str, data: &str) -> Vec<OutboundEvent> {
        let data_len = data.len();
        let write_result = {
            let Some(runtime) = self.runtimes.get(id) else {
                tracing::debug!(
                    target: "gwt_input_trace",
                    stage = "event_loop_runtime_missing",
                    window_id = %id,
                    data_len,
                    "terminal_input dropped: no runtime for window"
                );
                return Vec::new();
            };

            let lock_started = Instant::now();
            let lock_result = runtime.pane.lock().map_err(|error| error.to_string());
            let lock_wait_us = lock_started.elapsed().as_micros() as u64;

            match lock_result {
                Ok(pane) => {
                    let write_started = Instant::now();
                    let result = pane
                        .write_input(data.as_bytes())
                        .map_err(|error| error.to_string());
                    tracing::debug!(
                        target: "gwt_input_trace",
                        stage = "pty_write",
                        window_id = %id,
                        data_len,
                        lock_wait_us,
                        write_us = write_started.elapsed().as_micros() as u64,
                        ok = result.is_ok(),
                        "terminal_input forwarded to PTY writer"
                    );
                    result
                }
                Err(error) => {
                    tracing::debug!(
                        target: "gwt_input_trace",
                        stage = "pane_lock_failed",
                        window_id = %id,
                        data_len,
                        lock_wait_us,
                        error = %error,
                        "terminal_input dropped: pane mutex poisoned"
                    );
                    Err(error)
                }
            }
        };

        match write_result {
            Ok(()) => Vec::new(),
            Err(error) => {
                self.handle_runtime_status(id.to_string(), WindowProcessStatus::Error, Some(error))
            }
        }
    }

    pub(crate) fn paste_image_events(
        &mut self,
        id: &str,
        data_base64: &str,
        mime_type: &str,
        filename: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            tracing::debug!(window_id = %id, "image paste dropped: window not found");
            return Vec::new();
        };
        if self.tab(&address.tab_id).is_none() {
            tracing::debug!(window_id = %id, "image paste dropped: project tab not found");
            return Vec::new();
        }
        let Some(session) = self.active_agent_sessions.get(id) else {
            tracing::debug!(window_id = %id, "image paste dropped: active agent session not found");
            return Vec::new();
        };
        let worktree_path = session.worktree_path.clone();
        let agent_project_root = session.agent_project_root.clone();
        let runtime_target = session.runtime_target;

        let image = match prepare_image_paste_file(
            &worktree_path,
            &agent_project_root,
            data_base64,
            mime_type,
            filename,
            &image_paste_unique_token(),
        ) {
            Ok(image) => image,
            Err(error) => {
                tracing::debug!(
                    window_id = %id,
                    mime_type,
                    error = %error,
                    "image paste dropped"
                );
                return Vec::new();
            }
        };

        if let Err(error) = save_image_paste_file(&image) {
            return self.handle_runtime_status(
                id.to_string(),
                WindowProcessStatus::Error,
                Some(error.to_string()),
            );
        }

        tracing::debug!(
            window_id = %id,
            runtime_target = ?runtime_target,
            path = %image.storage_path.display(),
            agent_path = %image.agent_path,
            "saved pasted image"
        );
        self.terminal_input_events(id, &image.prompt_text)
    }

    pub(crate) fn load_file_tree_event(&self, id: &str, path: &str) -> BackendEvent {
        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                return BackendEvent::FileTreeError {
                    id: id.to_string(),
                    path: path.to_string(),
                    message,
                };
            }
        };

        let relative_path = if path.is_empty() {
            None
        } else {
            Some(Path::new(path))
        };

        match list_directory_entries(&root, relative_path) {
            Ok(entries) => BackendEvent::FileTreeEntries {
                id: id.to_string(),
                path: path.to_string(),
                entries,
            },
            Err(error) => BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: error.to_string(),
            },
        }
    }

    /// Resolve the worktree root for a File Tree window. Prefers the user's
    /// picker selection (`file_tree_worktree_roots`); falls back to
    /// `tab.project_root` for backward compatibility with existing callers
    /// that pre-date the picker. Returns a human-readable error message on
    /// invalid window id / wrong preset.
    fn resolve_file_tree_root(&self, id: &str) -> Result<std::path::PathBuf, String> {
        let address = self
            .window_lookup
            .get(id)
            .ok_or_else(|| "Window not found".to_string())?;
        let tab = self
            .tab(&address.tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let window = tab
            .workspace
            .window(&address.raw_id)
            .ok_or_else(|| "Window not found".to_string())?;
        if window.preset != WindowPreset::FileTree {
            return Err("Window is not a file tree".to_string());
        }
        Ok(self
            .file_tree_worktree_roots
            .get(id)
            .cloned()
            .unwrap_or_else(|| tab.project_root.clone()))
    }

    pub(crate) fn list_file_tree_worktrees_event(&self, id: &str) -> BackendEvent {
        let address = match self.window_lookup.get(id) {
            Some(addr) => addr,
            None => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                };
            }
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };
        if window.preset != WindowPreset::FileTree {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window is not a file tree".to_string(),
            };
        }
        match gwt::worktree_inventory::enumerate_worktrees(
            &tab.project_root,
            Some(&tab.project_root),
        ) {
            Ok(entries) => BackendEvent::FileTreeWorktrees {
                id: id.to_string(),
                entries,
            },
            Err(err) => BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: err.to_string(),
            },
        }
    }

    pub(crate) fn select_file_tree_worktree_event(
        &mut self,
        id: &str,
        worktree_id: &str,
    ) -> BackendEvent {
        let address = match self.window_lookup.get(id) {
            Some(addr) => addr,
            None => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                };
            }
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };
        if window.preset != WindowPreset::FileTree {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Window is not a file tree".to_string(),
            };
        }
        let entries = match gwt::worktree_inventory::enumerate_worktrees(
            &tab.project_root,
            Some(&tab.project_root),
        ) {
            Ok(entries) => entries,
            Err(err) => {
                return BackendEvent::FileTreeWorktreeError {
                    id: id.to_string(),
                    message: err.to_string(),
                };
            }
        };
        let Some(selected) = entries.into_iter().find(|entry| entry.id == worktree_id) else {
            return BackendEvent::FileTreeWorktreeError {
                id: id.to_string(),
                message: "Unknown worktree id".to_string(),
            };
        };
        self.file_tree_worktree_roots
            .insert(id.to_string(), selected.path);
        BackendEvent::FileTreeWorktreeSelected {
            id: id.to_string(),
            worktree_id: worktree_id.to_string(),
        }
    }

    pub(crate) fn load_file_content_event(
        &self,
        id: &str,
        path: &str,
        mode: FileContentMode,
        hex_offset: Option<u64>,
        hex_length: Option<u64>,
    ) -> BackendEvent {
        let make_error =
            |kind: FileContentErrorKind, message: String, size: Option<u64>, limit: Option<u64>| {
                BackendEvent::FileContentError {
                    id: id.to_string(),
                    path: path.to_string(),
                    error_kind: kind,
                    message,
                    size,
                    limit,
                }
            };

        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                let kind = if message == "Window is not a file tree" {
                    FileContentErrorKind::WindowMismatch
                } else {
                    FileContentErrorKind::WindowNotFound
                };
                return make_error(kind, message, None, None);
            }
        };

        let relative_path = Path::new(path);
        let limits = ContentLimits::default();

        match mode {
            FileContentMode::Text => match read_text_file(&root, relative_path, &limits) {
                Ok(result) => BackendEvent::FileContentText {
                    id: id.to_string(),
                    path: path.to_string(),
                    encoding: result.encoding,
                    text: result.text,
                    total_size: result.total_size,
                    mtime: result.mtime,
                    has_bom: result.has_bom,
                    newline: result.newline,
                    read_only: result.read_only,
                },
                Err(err) => file_content_error_to_event(id, path, err),
            },
            FileContentMode::Hex => {
                let offset = hex_offset.unwrap_or(0);
                let length = hex_length.unwrap_or(64 * 16);
                match read_binary_chunk(&root, relative_path, offset, length, &limits) {
                    Ok(chunk) => BackendEvent::FileContentHex {
                        id: id.to_string(),
                        path: path.to_string(),
                        offset: chunk.offset,
                        bytes_b64: base64::engine::general_purpose::STANDARD.encode(chunk.bytes),
                        total_size: chunk.total_size,
                        mtime: chunk.mtime,
                        read_only: chunk.read_only,
                    },
                    Err(err) => file_content_error_to_event(id, path, err),
                }
            }
        }
    }

    /// SPEC-2006 Phase 2 amendment: write the modified text or single hex
    /// byte back to disk, mapping every domain error to the structured
    /// `FileContentSaveErrorKind` variant the GUI listens for.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_file_content_event(
        &self,
        id: &str,
        path: &str,
        mode: FileContentMode,
        expected_mtime: u64,
        expected_size: u64,
        text: Option<String>,
        encoding: Option<gwt::Encoding>,
        newline: Option<gwt::Newline>,
        has_bom: Option<bool>,
        hex_offset: Option<u64>,
        hex_byte: Option<u8>,
    ) -> BackendEvent {
        let root = match self.resolve_file_tree_root(id) {
            Ok(root) => root,
            Err(message) => {
                let kind = if message == "Window is not a file tree" {
                    gwt::FileContentSaveErrorKind::WindowMismatch
                } else {
                    gwt::FileContentSaveErrorKind::WindowNotFound
                };
                return BackendEvent::FileContentSaveError {
                    id: id.to_string(),
                    path: path.to_string(),
                    mode,
                    error_kind: kind,
                    message,
                    current_mtime: None,
                    current_size: None,
                };
            }
        };

        let relative_path = Path::new(path);
        let limits = ContentLimits::default();
        let expected = gwt::ExpectedMetadata {
            mtime: expected_mtime,
            size: expected_size,
        };

        match mode {
            FileContentMode::Text => {
                let Some(text) = text else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(text) missing text payload".to_string(),
                        None,
                        None,
                    );
                };
                let encoding = encoding.unwrap_or(gwt::Encoding::Utf8);
                let newline = newline.unwrap_or(gwt::Newline::Lf);
                let has_bom = has_bom.unwrap_or(false);
                match gwt::write_text_file(
                    &root,
                    relative_path,
                    &text,
                    encoding,
                    newline,
                    has_bom,
                    expected,
                    &limits,
                ) {
                    Ok(outcome) => BackendEvent::FileContentSaved {
                        id: id.to_string(),
                        path: path.to_string(),
                        mode,
                        new_mtime: outcome.new_mtime,
                        new_size: outcome.new_size,
                        encoding_fallback: outcome.encoding_fallback,
                    },
                    Err(err) => write_error_to_event(id, path, mode, err),
                }
            }
            FileContentMode::Hex => {
                let Some(offset) = hex_offset else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(hex) missing hex_offset".to_string(),
                        None,
                        None,
                    );
                };
                let Some(byte) = hex_byte else {
                    return file_content_save_error(
                        id,
                        path,
                        mode,
                        gwt::FileContentSaveErrorKind::IoError,
                        "save_file_content(hex) missing hex_byte".to_string(),
                        None,
                        None,
                    );
                };
                match gwt::write_binary_byte(&root, relative_path, offset, byte, expected) {
                    Ok(outcome) => BackendEvent::FileContentSaved {
                        id: id.to_string(),
                        path: path.to_string(),
                        mode,
                        new_mtime: outcome.new_mtime,
                        new_size: outcome.new_size,
                        encoding_fallback: outcome.encoding_fallback,
                    },
                    Err(err) => write_error_to_event(id, path, mode, err),
                }
            }
        }
    }

    pub(crate) fn load_branches_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };

        if window.preset != WindowPreset::Branches {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window is not a branches list".to_string(),
                },
            )];
        }

        spawn_branch_load_async(
            self.proxy.clone(),
            id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(&address.tab_id),
            self.launch_wizard_cache.sessions.clone(),
        );
        Vec::new()
    }

    pub(crate) fn load_board_events(
        &mut self,
        client_id: &str,
        id: &str,
        all: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }
        let project_root = tab.project_root.clone();
        if all {
            self.board_all_view_windows.insert(id.to_string());
        } else {
            self.board_all_view_windows.remove(id);
        }

        let scope = if all {
            gwt_core::coordination::BoardAudienceScope::All
        } else {
            match board::gui_default_board_scope_for_project(&project_root) {
                Ok(scope) => scope,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::BoardError {
                            id: id.to_string(),
                            message: error.to_string(),
                        },
                    )];
                }
            }
        };
        let snapshot_result = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
            gwt_core::coordination::load_snapshot(&project_root)
        } else {
            gwt_core::coordination::load_snapshot_for_scope(&project_root, &scope)
        };
        match snapshot_result {
            Ok(snapshot) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardEntries {
                    id: id.to_string(),
                    entries: snapshot.board.entries,
                    has_more_before: snapshot.board.has_more_before,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn load_board_history_events(
        &mut self,
        client_id: &str,
        id: &str,
        before_entry_id: Option<&str>,
        limit: usize,
        all: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }
        let project_root = tab.project_root.clone();
        if all {
            self.board_all_view_windows.insert(id.to_string());
        } else {
            self.board_all_view_windows.remove(id);
        }

        let scope = if all {
            gwt_core::coordination::BoardAudienceScope::All
        } else {
            match board::gui_default_board_scope_for_project(&project_root) {
                Ok(scope) => scope,
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::BoardError {
                            id: id.to_string(),
                            message: error.to_string(),
                        },
                    )];
                }
            }
        };
        let page_result = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
            gwt_core::coordination::load_entries_before(&project_root, before_entry_id, limit)
        } else {
            gwt_core::coordination::load_entries_before_for_scope(
                &project_root,
                before_entry_id,
                limit,
                &scope,
            )
        };
        match page_result {
            Ok(page) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardHistoryPage {
                    id: id.to_string(),
                    entries: page.entries,
                    has_more_before: page.has_more_before,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn load_logs_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Logs {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: "Window is not a Logs surface".to_string(),
                },
            )];
        }

        match load_log_entries_from_dir(&self.log_dir) {
            Ok(outcome) => {
                let mut events = vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::LogEntries {
                        id: id.to_string(),
                        entries: outcome.entries,
                    },
                )];
                if outcome.diagnostics.skipped > 0 {
                    events.push(OutboundEvent::reply(
                        client_id,
                        BackendEvent::LogEntryAppended {
                            entry: skipped_lines_warning(&outcome.diagnostics),
                        },
                    ));
                }
                events
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogError {
                    id: id.to_string(),
                    message: error,
                },
            )],
        }
    }

    pub(crate) fn handle_board_projection_changed_events(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Ok(snapshot) = gwt_core::coordination::load_snapshot(project_root) else {
            return Vec::new();
        };

        let mut events = Vec::new();
        let latest_entry = snapshot.board.entries.last().cloned();
        for tab in &self.tabs {
            if !same_worktree_path(&tab.project_root, project_root) {
                continue;
            }
            for window in &tab.workspace.persisted().windows {
                if window.preset != WindowPreset::Board {
                    continue;
                }
                let window_id = combined_window_id(&tab.id, &window.id);
                let scope = if self.board_all_view_windows.contains(&window_id) {
                    gwt_core::coordination::BoardAudienceScope::All
                } else {
                    board::gui_default_board_scope_for_project(&tab.project_root)
                        .unwrap_or(gwt_core::coordination::BoardAudienceScope::All)
                };
                let board = if matches!(scope, gwt_core::coordination::BoardAudienceScope::All) {
                    snapshot.board.clone()
                } else {
                    gwt_core::coordination::load_snapshot_for_scope(&tab.project_root, &scope)
                        .map(|snapshot| snapshot.board)
                        .unwrap_or_else(|_| snapshot.board.clone())
                };
                events.push(OutboundEvent::broadcast(BackendEvent::BoardEntries {
                    id: window_id,
                    entries: board.entries,
                    has_more_before: board.has_more_before,
                }));
            }
        }
        if let Some(entry) = latest_entry.as_ref() {
            if let Some((tab_id, project_root)) = self
                .tabs
                .iter()
                .find(|tab| {
                    same_worktree_path(&tab.project_root, project_root)
                        && self.active_tab_id.as_deref() == Some(tab.id.as_str())
                })
                .map(|tab| (tab.id.clone(), tab.project_root.clone()))
            {
                events.extend(self.record_workspace_board_milestone_event(
                    &tab_id,
                    &project_root,
                    entry,
                ));
            }
        }
        events
    }

    pub(crate) fn handle_workspace_projection_changed_events(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Ok(Some(projection)) =
            gwt_core::workspace_projection::load_workspace_projection(project_root)
        else {
            return Vec::new();
        };
        self.apply_workspace_projection_title_sync(project_root, &projection)
    }

    /// Sync `projection.agents[<i>].title_summary` / `current_focus` into the
    /// matching `tab.workspace.windows[<id>].dynamic_title` /
    /// `dynamic_title_detail`. Returns `true` if at least one window was
    /// touched.
    ///
    /// Callers should generally go through
    /// [`AppRuntime::apply_workspace_projection_title_sync`] (Phase U-1+)
    /// rather than calling this directly, so that the consequent broadcasts
    /// are emitted in the same batch.
    pub(crate) fn sync_agent_window_titles_from_workspace_projection(
        &mut self,
        project_root: &Path,
        projection: &gwt_core::workspace_projection::WorkspaceProjection,
    ) -> bool {
        let updates = projection
            .agents
            .iter()
            .filter_map(|agent| {
                let title = agent
                    .title_summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?;
                let window_id = self.resolve_title_sync_window_id(agent, project_root)?;
                Some((window_id, title.to_string(), agent.current_focus.clone()))
            })
            .collect::<Vec<_>>();

        let mut changed = false;
        for (window_id, title, detail) in updates {
            let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                continue;
            };
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                continue;
            };
            if tab
                .workspace
                .set_dynamic_title_with_detail(&address.raw_id, Some(title), detail)
            {
                changed = true;
            }
        }
        changed
    }

    /// Resolve the window_id that title sync should target for a given
    /// projection agent.
    ///
    /// Fast path: `active_agent_sessions` (gwt's live launch tracking).
    ///
    /// Phase U-3 fallback (SPEC-2359 US-26): for sessions that gwt's
    /// launch flow has not (yet) registered — e.g. GUI restarted after a
    /// session started, a session that was launched outside the tracked
    /// `gwtd` path but still publishes its `GWT_SESSION_ID` — use the
    /// `window_id` / `worktree_path` carried by the projection itself. The
    /// fallback intentionally does **not** mutate `active_agent_sessions`
    /// (that lifecycle stays in the launch flow, see US-24). It only
    /// resolves the lookup needed for title sync.
    ///
    /// Phase U-4 fallback: when the projection record only carries
    /// `worktree_path` (e.g. SessionStart hook registered the agent
    /// before any GUI launch picked it up so `window_id` is `None`),
    /// try to match against `active_agent_sessions` by worktree alone.
    /// Only resolves when there is exactly one matching session in the
    /// worktree with the same `agent_id`, to avoid mis-targeting when
    /// the worktree has multiple panes.
    ///
    /// Phase U-7 (SPEC-2359): the fast path used to require
    /// `same_worktree_path(session.worktree_path, project_root)` so that
    /// only the watcher firing for the *agent's own* tab would resolve
    /// the window. In practice this filter prevented title updates
    /// whenever the watcher event came from a different tab (e.g. the
    /// startup tab's watcher firing for a change in another tab's
    /// agent, since both tabs share `current.json`). `session_id` is
    /// globally unique to one launched window — finding it in
    /// `active_agent_sessions` is sufficient to identify the target.
    fn resolve_title_sync_window_id(
        &self,
        agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
        project_root: &Path,
    ) -> Option<String> {
        if let Some((window_id, _session)) = self
            .active_agent_sessions
            .iter()
            .find(|(_, session)| session.session_id == agent.session_id)
        {
            return Some(window_id.clone());
        }
        if let Some(worktree) = agent.worktree_path.as_deref() {
            if same_worktree_path(worktree, project_root) {
                if let Some(projected_window_id) = agent.window_id.as_deref() {
                    if self.window_lookup.contains_key(projected_window_id) {
                        return Some(projected_window_id.to_string());
                    }
                }
                let mut matches = self.active_agent_sessions.iter().filter(|(_, session)| {
                    same_worktree_path(&session.worktree_path, worktree)
                        && session.agent_id == agent.agent_id
                });
                if let Some((window_id, _)) = matches.next() {
                    if matches.next().is_none() {
                        return Some(window_id.clone());
                    }
                }
            }
        }
        None
    }

    pub(crate) fn load_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeLoadRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Project tab not found", request.request_id, None),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    request.request_id,
                    None,
                ),
            )];
        }

        if request.refresh {
            self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                client_id: client_id.to_string(),
                id: id.to_string(),
                project_root: tab.project_root.clone(),
                kind,
                request_id: request.request_id,
                selected_number: request.selected_number,
                force: true,
            });
            return Vec::new();
        }

        match load_knowledge_bridge(&tab.project_root, kind, request.selected_number, false) {
            Ok(view) => {
                if request.request_id.is_some() && view.refresh_enabled {
                    self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                        client_id: client_id.to_string(),
                        id: id.to_string(),
                        project_root: tab.project_root.clone(),
                        kind,
                        request_id: request.request_id,
                        selected_number: request.selected_number,
                        force: false,
                    });
                }
                knowledge_view_events(
                    client_id.to_string(),
                    id.to_string(),
                    kind,
                    request.request_id,
                    view,
                )
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, error, request.request_id, None),
            )],
        }
    }

    fn spawn_knowledge_bridge_refresh(&self, task: KnowledgeRefreshTask) {
        let KnowledgeRefreshTask {
            client_id,
            id,
            project_root,
            kind,
            request_id,
            selected_number,
            force,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let refreshed = match gwt::refresh_knowledge_bridge_cache(&project_root, force) {
                Ok(refreshed) => refreshed,
                Err(error) => {
                    if force {
                        proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                            client_id,
                            knowledge_error_event(id, kind, error, request_id, None),
                        )]));
                    }
                    return;
                }
            };
            if !force && !refreshed {
                return;
            }
            let event =
                match gwt::load_knowledge_bridge(&project_root, kind, selected_number, false) {
                    Ok(view) => knowledge_view_events(client_id, id, kind, request_id, view),
                    Err(error) => vec![OutboundEvent::reply(
                        client_id,
                        knowledge_error_event(id, kind, error, request_id, None),
                    )],
                };
            proxy.send(UserEvent::Dispatch(event));
        });
    }

    pub(crate) fn search_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Project tab not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        }

        self.spawn_knowledge_bridge_search(KnowledgeSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            kind,
            query: request.query.to_string(),
            request_id: request.request_id,
            selected_number: request.selected_number,
        });
        Vec::new()
    }

    fn spawn_knowledge_bridge_search(&self, task: KnowledgeSearchTask) {
        let KnowledgeSearchTask {
            client_id,
            id,
            project_root,
            kind,
            query,
            request_id,
            selected_number,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event =
                match gwt::search_knowledge_bridge(&project_root, kind, &query, selected_number) {
                    Ok(view) => BackendEvent::KnowledgeSearchResults {
                        id: id.clone(),
                        knowledge_kind: kind,
                        query: query.clone(),
                        request_id,
                        entries: view.entries,
                        selected_number: view.selected_number,
                        empty_message: view.empty_message,
                        refresh_enabled: view.refresh_enabled,
                    },
                    Err(error) => {
                        knowledge_error_event(id, kind, error, Some(request_id), Some(query))
                    }
                };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    pub(crate) fn search_project_index_events(
        &self,
        client_id: &str,
        request: ProjectIndexSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Index {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window is not an Index surface".to_string(),
                },
            )];
        }

        self.spawn_project_index_search(ProjectIndexSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            query: request.query.to_string(),
            request_id: request.request_id,
            scopes: request.scopes,
            worktree_hash: request.worktree_hash,
            match_mode: request.match_mode,
        });
        Vec::new()
    }

    fn spawn_project_index_search(&self, task: ProjectIndexSearchTask) {
        let ProjectIndexSearchTask {
            client_id,
            id,
            project_root,
            query,
            request_id,
            scopes,
            worktree_hash,
            match_mode,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = match gwt::search_project_index(
                &project_root,
                &query,
                &scopes,
                worktree_hash.as_deref(),
                match_mode,
            ) {
                Ok(outcome) => BackendEvent::ProjectIndexSearchResults {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    results: outcome.results,
                    suggestions: outcome.suggestions,
                },
                Err(error) => BackendEvent::ProjectIndexSearchError {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    message: error,
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    /// SPEC-2017 US-8 — Apply a Kanban phase change to the owning
    /// GitHub Issue. Validates that the target window is a knowledge
    /// bridge surface and dispatches a blocking task that calls
    /// `gwt::update_knowledge_phase`. The result is delivered as
    /// [`BackendEvent::KnowledgeBridgePhaseUpdated`] so the optimistic
    /// frontend UI can either confirm or rollback.
    pub(crate) fn update_knowledge_bridge_phase_events(
        &self,
        client_id: &str,
        id: &str,
        request_id: u64,
        issue_number: u64,
        target_phase: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Project tab not found",
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset).is_none() {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window is not a knowledge bridge",
                ),
            )];
        }

        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id_owned = id.to_string();
        let project_root = tab.project_root.clone();
        let target_phase = target_phase.map(str::to_string);
        self.blocking_tasks.spawn(move || {
            let event = match gwt::update_knowledge_phase(
                &project_root,
                issue_number,
                target_phase.as_deref(),
            ) {
                Ok(fresh_entry) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Ok { fresh_entry },
                },
                Err(error) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Error { message: error },
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &client_id, event,
            )]));
        });
        Vec::new()
    }

    pub(crate) fn run_branch_cleanup_events(
        &self,
        client_id: &str,
        id: &str,
        branches: &[String],
        delete_remote: bool,
        force_filesystem_delete: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };

        if window.preset != WindowPreset::Branches {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window is not a branches list".to_string(),
                },
            )];
        }

        spawn_branch_cleanup_async(
            self.proxy.clone(),
            client_id.to_string(),
            id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(&address.tab_id),
            branches.to_vec(),
            BranchCleanupOptions {
                delete_remote,
                force_filesystem_delete,
            },
        );
        Vec::new()
    }

    pub(crate) fn run_workspace_cleanup_events(
        &self,
        client_id: &str,
        branch: &str,
        delete_remote: bool,
        force_filesystem_delete: bool,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.as_deref() else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };

        spawn_workspace_cleanup_async(
            self.proxy.clone(),
            client_id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(tab_id),
            branch.to_string(),
            BranchCleanupOptions {
                delete_remote,
                force_filesystem_delete,
            },
        );
        Vec::new()
    }

    /// SPEC-1939 US-5 / T-IDX-102: handle a per-cell rebuild request from the
    /// frontend. Spawns the rebuild via the global bootstrap service so the
    /// in-flight set is shared with the orchestrator and CLI.
    pub(crate) fn rebuild_index_cell_events(
        &self,
        project_root: String,
        scope: gwt::IndexRebuildScope,
        worktree_hash: Option<String>,
    ) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = crate::project_index_bootstrap::spawn_per_cell_rebuild(
            service,
            self.proxy.clone(),
            project_root,
            scope,
            worktree_hash,
        );
        Vec::new()
    }

    /// Settings.Index requests the full all-worktree health table on demand.
    /// The startup path stays current-worktree only to avoid UI-visible CPU
    /// spikes on repositories with many active worktrees.
    pub(crate) fn refresh_index_status_events(&self, project_root: String) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = service.spawn_full_status_refresh(self.proxy.clone(), project_root);
        Vec::new()
    }
}

/// Read the active canonical log file via the SPEC-1924 FR-035 reader.
///
/// Returns the decoded snapshot together with `ReadDiagnostics` so the caller
/// can surface a non-blocking warning when malformed lines were skipped
/// (FR-036 / SC-010). IO errors other than `NotFound` are forwarded as a
/// human-readable message so the Logs window can switch to an error state
/// without crashing the agent.
fn load_log_entries_from_dir(log_dir: &Path) -> Result<gwt_core::logging::ReadOutcome, String> {
    let path = gwt_core::logging::current_log_file(log_dir);
    gwt_core::logging::read_log_file(&path)
        .map_err(|error| format!("Failed to read log file {}: {error}", path.display()))
}

/// Build the synthetic warning event surfaced when `read_log_file` skipped
/// malformed lines. Keeps the message phrasing consistent with the Logs
/// window expectation of a single notice per load (FR-036 / SC-010).
fn skipped_lines_warning(
    diagnostics: &gwt_core::logging::ReadDiagnostics,
) -> gwt_core::logging::LogEvent {
    let count = diagnostics.skipped;
    let plural = if count == 1 { "line" } else { "lines" };
    gwt_core::logging::LogEvent::new(
        gwt_core::logging::LogLevel::Warn,
        "gwt_core::logging::reader",
        format!(
            "Skipped {count} malformed {plural} while reading {}",
            diagnostics.path.display()
        ),
    )
}

fn spawn_branch_cleanup_async(
    proxy: AppEventProxy,
    client_id: ClientId,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branches: Vec<String>,
    options: BranchCleanupOptions,
) {
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_client_id = client_id.clone();
                    let progress_window_id = window_id.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        &branches,
                        options,
                        move |progress| {
                            progress_proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                                progress_client_id.clone(),
                                BackendEvent::BranchCleanupProgress {
                                    id: progress_window_id.clone(),
                                    branch: progress.branch,
                                    execution_branch: progress.execution_branch,
                                    index: progress.index,
                                    total: progress.total,
                                    phase: progress.phase,
                                    message: progress.message,
                                },
                            )]));
                        },
                    );
                    let mut events = vec![OutboundEvent::reply(
                        client_id.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: window_id.clone(),
                            results,
                        },
                    )];
                    match list_branch_entries_with_active_sessions(
                        &project_root,
                        &active_session_branches,
                    ) {
                        Ok(entries) => events.push(OutboundEvent::reply(
                            client_id.clone(),
                            BackendEvent::BranchEntries {
                                id: window_id.clone(),
                                phase: BranchEntriesPhase::Hydrated,
                                entries,
                            },
                        )),
                        Err(error) => events.push(OutboundEvent::reply(
                            client_id.clone(),
                            BackendEvent::BranchError {
                                id: window_id.clone(),
                                message: error.to_string(),
                            },
                        )),
                    }
                    events
                }
                Err(error) => vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BranchError {
                        id: window_id,
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::Dispatch(events));
    });
}

fn spawn_workspace_cleanup_async(
    proxy: AppEventProxy,
    client_id: ClientId,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branch: String,
    options: BranchCleanupOptions,
) {
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_client_id = client_id.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        std::slice::from_ref(&branch),
                        options,
                        move |progress| {
                            progress_proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                                progress_client_id.clone(),
                                BackendEvent::BranchCleanupProgress {
                                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                                    branch: progress.branch,
                                    execution_branch: progress.execution_branch,
                                    index: progress.index,
                                    total: progress.total,
                                    phase: progress.phase,
                                    message: progress.message,
                                },
                            )]));
                        },
                    );
                    let mut events = vec![OutboundEvent::reply(
                        client_id.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                            results: results.clone(),
                        },
                    )];
                    if results.iter().any(|result| {
                        result.branch == branch
                            && matches!(
                                result.status,
                                gwt::BranchCleanupResultStatus::Success
                                    | gwt::BranchCleanupResultStatus::Partial
                            )
                    }) {
                        // SPEC-2359 US-37 / FR-118: emit Done only after the
                        // matching workspace cleanup actually succeeded.
                        let _ =
                            gwt_core::workspace_projection::emit_workspace_done_event_for_branch(
                                &project_root,
                                &branch,
                                chrono::Utc::now(),
                            );
                        if let Some(event) =
                            clear_workspace_cleanup_git_details_event(&project_root)
                        {
                            events.push(event);
                        }
                    }
                    events
                }
                Err(error) => vec![OutboundEvent::reply(
                    client_id.clone(),
                    BackendEvent::BranchError {
                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::Dispatch(events));
    });
}

fn clear_workspace_cleanup_git_details_event(project_root: &Path) -> Option<OutboundEvent> {
    let mut projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
        .ok()
        .flatten()?;
    projection.git_details = None;
    projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
    projection.status_text = "No active work".to_string();
    projection.next_action = None;
    projection.updated_at = chrono::Utc::now();
    if let Err(error) =
        gwt_core::workspace_projection::save_workspace_projection(project_root, &projection)
    {
        tracing::warn!(
            project_root = %project_root.display(),
            error = %error,
            "workspace projection cleanup state update skipped"
        );
        return None;
    }
    let journal_entries = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
        project_root,
        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
    )
    .unwrap_or_default()
    .iter()
    .map(workspace_journal_entry_view_from_entry)
    .collect::<Vec<_>>();
    let workspaces =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(project_root)
            .unwrap_or_else(
                |_| gwt_core::workspace_projection::WorkspaceWorkItemsProjection {
                    updated_at: projection.updated_at,
                    work_items: Vec::new(),
                },
            )
            .work_items
            .iter()
            .map(workspace_work_item_view_from_item)
            .collect::<Vec<_>>();
    Some(OutboundEvent::broadcast(
        BackendEvent::ActiveWorkProjection {
            projection: Box::new(active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                None,
            )),
        },
    ))
}

impl AppRuntime {
    fn latest_resumable_branch_session(
        &self,
        project_root: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        let normalized_branch_name = normalize_branch_name(branch_name);
        self.launch_wizard_cache
            .latest_resumable_branch_session(project_root, &normalized_branch_name)
    }

    pub(crate) fn live_sessions_for_branch(
        &self,
        tab_id: &str,
        branch_name: &str,
    ) -> Vec<LiveSessionEntry> {
        let mut entries = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id && session.branch_name == branch_name)
            .map(|session| LiveSessionEntry {
                session_id: session.session_id.clone(),
                window_id: session.window_id.clone(),
                agent_id: session.agent_id.clone(),
                kind: "agent".to_string(),
                name: session.display_name.clone(),
                detail: Some(session.worktree_path.display().to_string()),
                active: true,
                runtime_status: self
                    .window_status(&session.window_id)
                    .unwrap_or(WindowProcessStatus::Running),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            match (
                self.launch_wizard_cache.session_by_id(&left.session_id),
                self.launch_wizard_cache.session_by_id(&right.session_id),
            ) {
                (Some(left_session), Some(right_session)) => right_session
                    .last_activity_at
                    .cmp(&left_session.last_activity_at)
                    .then_with(|| right_session.updated_at.cmp(&left_session.updated_at))
                    .then_with(|| right_session.created_at.cmp(&left_session.created_at))
                    .then_with(|| right_session.id.cmp(&left_session.id)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.name.cmp(&right.name),
            }
        });
        entries
    }

    pub(crate) fn active_session_branches_for_tab(
        &self,
        tab_id: &str,
    ) -> std::collections::HashSet<String> {
        self.active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.branch_name.clone())
            .collect()
    }

    pub(crate) fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: AgentLaunchResult,
    ) -> Vec<OutboundEvent> {
        let workspace_resume_context = self.pending_workspace_resume_contexts.remove(&window_id);
        let launch_feedback_context = self.pending_launch_feedback_contexts.remove(&window_id);
        let auto_resume_source_session_id = self.pending_auto_resume_sources.remove(&window_id);
        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                agent_project_root,
            )) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Project tab not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        launch_feedback_context.clone(),
                    );
                };
                let tab_id = address.tab_id.clone();
                let project_root = tab.project_root.clone();
                let geometry = window.geometry.clone();
                let session_id_for_restore = session_id.clone();

                self.active_agent_sessions.insert(
                    window_id.clone(),
                    ActiveAgentSession {
                        window_id: window_id.clone(),
                        session_id,
                        agent_id: agent_id.to_string(),
                        branch_name,
                        display_name,
                        worktree_path: worktree_path.clone(),
                        agent_project_root,
                        runtime_target,
                        tab_id: tab_id.clone(),
                    },
                );
                let _ = gwt_agent::persist_session_restore_window_on_startup(
                    &self.sessions_dir,
                    &session_id_for_restore,
                    true,
                );
                if let Some(source_session_id) = auto_resume_source_session_id {
                    mark_auto_resume_source_completed(&self.sessions_dir, &source_session_id);
                }
                self.refresh_launch_wizard_session_cache(&window_id);

                // SPEC-2809 — Launch Wizard always spawns an AI agent
                // launch sequence (binary resolve / env prep / PTY
                // spawn) so the Console window's `agent` tab shows the
                // wizard pipeline up to the moment xterm.js takes over.
                let stage_id = next_agent_launch_stage_id();
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &format!("worktree={}", worktree_path.display()),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "spawn_pty",
                    &format!("argv=[{}]", process_launch.args.join(" ")),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        let linkage_result = match linked_issue_number {
                            Some(issue_number) => record_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &self.active_agent_sessions[&window_id].branch_name,
                                issue_number,
                                &self.issue_link_cache_dir,
                            ),
                            None => clear_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &self.active_agent_sessions[&window_id].branch_name,
                                &self.issue_link_cache_dir,
                            ),
                        };
                        if let Err(error) = linkage_result {
                            tracing::warn!(
                                worktree = %worktree_path.display(),
                                branch = %self.active_agent_sessions[&window_id].branch_name,
                                ?linked_issue_number,
                                error = %error,
                                "issue branch linkage update skipped after agent launch"
                            );
                        }
                        let mut workspace_projection_updated = false;
                        let active_session = &self.active_agent_sessions[&window_id];
                        if let Some(context) = workspace_resume_context.as_ref() {
                            match save_resumed_workspace_projection(
                                &project_root,
                                active_session,
                                base_branch.as_deref(),
                                linked_issue_number,
                                context,
                            ) {
                                Ok(()) => {
                                    workspace_projection_updated = true;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        project_root = %project_root.display(),
                                        branch = %active_session.branch_name,
                                        error = %error,
                                        "workspace projection update skipped after Workspace Resume launch"
                                    );
                                }
                            }
                        } else if let Some(base_branch) = base_branch.as_deref() {
                            match save_start_work_workspace_projection(
                                &project_root,
                                active_session,
                                base_branch,
                                linked_issue_number,
                                None,
                            ) {
                                Ok(()) => {
                                    workspace_projection_updated = true;
                                }
                                Err(error) => {
                                    tracing::warn!(
                                        project_root = %project_root.display(),
                                        branch = %active_session.branch_name,
                                        error = %error,
                                        "workspace projection update skipped after Start Work launch"
                                    );
                                }
                            }
                        }
                        let _ = self.persist();
                        let mut events = vec![self.workspace_state_broadcast()];
                        if workspace_projection_updated
                            && self.active_tab_id.as_deref() == Some(tab_id.as_str())
                        {
                            if let Some(tab) = self.tab(&tab_id) {
                                if let Some(projection) =
                                    self.active_work_projection_for_tab(&tab_id, tab)
                                {
                                    events.push(OutboundEvent::broadcast(
                                        BackendEvent::ActiveWorkProjection {
                                            projection: Box::new(projection),
                                        },
                                    ));
                                }
                            }
                        }
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(window_id, composed_status, None));
                        events
                    }
                    Err(error) => {
                        self.launch_error_events(window_id, error, launch_feedback_context)
                    }
                }
            }
            Err(error) => self.launch_error_events(window_id, error, launch_feedback_context),
        }
    }

    pub(crate) fn handle_shell_launch_complete(
        &mut self,
        window_id: String,
        result: Result<ProcessLaunch, String>,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok(process_launch) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Project tab not found".to_string(),
                        None,
                    );
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(
                        window_id,
                        "Window not found".to_string(),
                        None,
                    );
                };
                let geometry = window.geometry.clone();

                // SPEC-2809 (revised) — second Launch Wizard exit path
                // emits the same launch banner sequence as the primary
                // handler so the Console window's `agent` tab is
                // consistent regardless of which wizard outcome the user
                // came in through.
                let stage_id = next_agent_launch_stage_id();
                emit_agent_launch_stage(
                    stage_id,
                    "resolve_binary",
                    &format!("wizard launch {}", process_launch.command),
                );
                emit_agent_launch_stage(
                    stage_id,
                    "prepare_env",
                    &format!("argv=[{}]", process_launch.args.join(" ")),
                );
                match self.spawn_process_window_with_console_kind(
                    &window_id,
                    geometry,
                    process_launch,
                    Some(gwt_core::process_console::ProcessKind::AgentBootstrap),
                ) {
                    Ok(()) => {
                        emit_agent_launch_stage(stage_id, "ready", "PTY handoff complete");
                        let mut events = vec![self.workspace_state_broadcast()];
                        let composed_status = self
                            .window_status(&window_id)
                            .unwrap_or(WindowProcessStatus::Running);
                        events.extend(Self::status_events(window_id, composed_status, None));
                        events
                    }
                    Err(error) => {
                        emit_agent_launch_stage(stage_id, "error", &error);
                        self.launch_error_events(window_id, error, None)
                    }
                }
            }
            Err(error) => self.launch_error_events(window_id, error, None),
        }
    }

    pub(crate) fn start_window(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        self.register_window(tab_id, raw_id);
        let window_id = combined_window_id(tab_id, raw_id);
        if !preset.requires_process() {
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Running);
            return Self::status_events(window_id, WindowProcessStatus::Running, None);
        }

        let project_root = self
            .tab(tab_id)
            .map(|tab| tab.project_root.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let launch = match resolve_launch_spec_with_fallback(preset, &shell) {
            Ok(launch) => launch,
            Err(error) => {
                let detail = error.to_string();
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        };

        let effective_env = match self.active_profile_spawn_env() {
            Ok(env) => env,
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                return Self::status_events(window_id, WindowProcessStatus::Error, Some(error));
            }
        }
        .with_project_root(&project_root);
        let (env, remove_env) = effective_env.into_parts();

        // SPEC-2809 (revised) — Surface the launch pipeline for AI
        // agent presets (Codex / Claude / Gemini / Agent) so the Console
        // window's `agent` tab shows what gwt is doing leading up to the
        // PTY spawn. Plain `Shell` panes do not emit launch banners
        // because nothing distinguishes them from arbitrary terminals.
        let is_agent_preset = matches!(
            preset,
            WindowPreset::Claude | WindowPreset::Codex | WindowPreset::Agent
        );
        let console_kind =
            is_agent_preset.then_some(gwt_core::process_console::ProcessKind::AgentBootstrap);
        let stage_id = is_agent_preset.then(next_agent_launch_stage_id);
        if let Some(id) = stage_id {
            emit_agent_launch_stage(
                id,
                "resolve_binary",
                &format!("{} ({})", preset.title(), launch.command),
            );
            emit_agent_launch_stage(
                id,
                "prepare_env",
                &format!("project_root={}", project_root.display()),
            );
            emit_agent_launch_stage(
                id,
                "spawn_pty",
                &format!("argv=[{}]", launch.args.join(" ")),
            );
        }
        match self.spawn_process_window_with_console_kind(
            &window_id,
            geometry,
            ProcessLaunch {
                command: launch.command,
                args: launch.args,
                env,
                remove_env,
                cwd: Some(project_root),
            },
            console_kind,
        ) {
            Ok(()) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "ready", "PTY handoff complete");
                }
                let composed_status = self
                    .window_status(&window_id)
                    .unwrap_or(WindowProcessStatus::Running);
                Self::status_events(window_id, composed_status, None)
            }
            Err(error) => {
                if let Some(id) = stage_id {
                    emit_agent_launch_stage(id, "error", &error);
                }
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                Self::status_events(window_id, WindowProcessStatus::Error, Some(error))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn spawn_process_window(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
    ) -> Result<(), String> {
        self.spawn_process_window_with_console_kind(id, geometry, launch, None)
    }

    pub(crate) fn spawn_process_window_with_console_kind(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
        console_kind: Option<gwt_core::process_console::ProcessKind>,
    ) -> Result<(), String> {
        let (cols, rows) = geometry_to_pty_size(&geometry);
        let pane = Pane::new_with_spawn_config(
            id.to_string(),
            gwt_terminal::pty::SpawnConfig {
                command: launch.command,
                args: launch.args,
                cols,
                rows,
                env: launch.env,
                remove_env: launch.remove_env,
                cwd: launch.cwd,
            },
        )
        .map_err(|error| error.to_string())?;
        let pane = Arc::new(Mutex::new(pane));

        let output_thread = self.spawn_output_thread(id.to_string(), pane.clone(), console_kind);
        let status_thread = self.spawn_status_thread(id.to_string(), pane.clone());
        if let Some(address) = self.window_lookup.get(id).cloned() {
            self.window_pty_statuses
                .insert(id.to_string(), WindowProcessStatus::Running);
            self.window_hook_states.remove(id);
            self.set_window_status(
                &address.tab_id,
                &address.raw_id,
                WindowProcessStatus::Running,
            );
        }
        self.window_details.remove(id);
        // Publish the PTY handle to the WebSocket fast-path registry BEFORE
        // inserting the runtime so that the first `terminal_input` from the
        // frontend (which can arrive immediately after `TerminalStatus`) has a
        // target to write to. Registry holds a cloned `Arc<PtyHandle>`; the
        // real owner remains the `Mutex<Pane>` in `WindowRuntime`.
        self.register_pty_writer(id, &pane);
        self.runtimes.insert(
            id.to_string(),
            WindowRuntime {
                pane,
                output_thread: Some(output_thread),
                status_thread: Some(status_thread),
            },
        );
        Ok(())
    }

    pub(crate) fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Centered(bounds),
            workspace_resume_context,
            None,
        )
    }

    pub(crate) fn spawn_agent_window_with_feedback(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Centered(bounds),
            workspace_resume_context,
            Some(launch_feedback_context),
        )
    }

    pub(crate) fn spawn_agent_window_at_geometry(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        geometry: WindowGeometry,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_placement(
            tab_id,
            config,
            AgentWindowPlacement::Exact(geometry),
            workspace_resume_context,
            None,
        )
    }

    fn spawn_agent_window_with_placement(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        placement: AgentWindowPlacement,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        let issue_link_cache_dir = self.issue_link_cache_dir.clone();
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root_path = tab.project_root.clone();
        let project_root = project_root_path.display().to_string();
        let title = config.display_name.clone();
        let purpose_title = workspace_resume_context
            .as_ref()
            .and_then(WorkspaceResumeContext::purpose_title)
            .or_else(|| {
                agent_launch_purpose_title(
                    &project_root_path,
                    config.linked_issue_number,
                    config.branch.as_deref(),
                    &issue_link_cache_dir,
                )
            });
        let window = match placement {
            AgentWindowPlacement::Centered(bounds) => {
                tab.workspace
                    .add_window_with_title(WindowPreset::Agent, title, false, bounds)
            }
            AgentWindowPlacement::Exact(geometry) => tab
                .workspace
                .add_window_at_geometry_with_title(WindowPreset::Agent, title, false, geometry),
        };
        if let Some(purpose_title) = purpose_title {
            let _ = tab
                .workspace
                .set_purpose_title(&window.id, Some(purpose_title));
        }
        let _ = tab
            .workspace
            .set_agent_id(&window.id, config.agent_id.command().to_string());
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        let mut events = vec![self.workspace_state_broadcast()];
        let composed_status = self
            .window_status(&window_id)
            .unwrap_or(WindowProcessStatus::Running);
        events.extend(Self::status_events(
            window_id.clone(),
            composed_status,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let hook_forward_target = self.hook_forward_target.clone();
        let profile_config_path = self.profile_config_path()?;
        if let Some(context) = workspace_resume_context {
            self.pending_workspace_resume_contexts
                .insert(window_id.clone(), context);
        }
        if let Some(context) = launch_feedback_context {
            self.pending_launch_feedback_contexts
                .insert(window_id.clone(), context);
        }

        thread::spawn(move || {
            Self::spawn_agent_window_async(
                proxy,
                sessions_dir,
                project_root,
                window_id,
                config,
                profile_config_path,
                hook_forward_target,
            );
        });

        Ok(events)
    }

    pub(crate) fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        mut config: gwt_agent::LaunchConfig,
        profile_config_path: PathBuf,
        hook_forward_target: Option<HookForwardTarget>,
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_launch_worktree(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Starting Docker service...".to_string(),
            });
            apply_docker_runtime_to_launch_config(Path::new(&project_root), &mut config)?;

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Configuring workspace...".to_string(),
            });
            let worktree_path = config
                .working_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(&project_root));
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
            refresh_managed_gwt_assets_for_agent(&worktree_path, &config.agent_id)
                .map_err(|error| error.to_string())?;
            if let Some(report) = maybe_register_codex_managed_hook_trust_for_launch(
                &profile_config_path,
                &worktree_path,
                &config.agent_id,
                config.runtime_target,
                config.docker_service.as_deref(),
            )? {
                if !report.trusted_entries.is_empty() {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message: format!(
                            "Trusted {} gwt-managed Codex hooks.",
                            report.trusted_entries.len()
                        ),
                    });
                }
            }

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host
                && apply_host_package_runner_fallback(&mut config)
            {
                proxy.send(UserEvent::LaunchProgress {
                    window_id: window_id.clone(),
                    message: "bunx unavailable, switching to npx...".to_string(),
                });
            }
            install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
            apply_windows_host_shell_wrapper(&mut config)?;

            let branch_name = config
                .branch
                .clone()
                .unwrap_or_else(|| "workspace".to_string());

            let agent_id = config.agent_id.clone();
            let mut session =
                gwt_agent::Session::new(&worktree_path, branch_name.clone(), agent_id.clone());
            session.display_name = config.display_name.clone();
            session.tool_version = config.tool_version.clone();
            session.model = config.model.clone();
            session.reasoning_level = config.reasoning_level.clone();
            session.session_mode = config.session_mode;
            session.skip_permissions = config.skip_permissions;
            session.codex_fast_mode = config.codex_fast_mode;
            session.runtime_target = config.runtime_target;
            session.docker_service = config.docker_service.clone();
            session.docker_lifecycle_intent = config.docker_lifecycle_intent;
            session.linked_issue_number = config.linked_issue_number;
            session.launch_command = config.command.clone();
            session.launch_args = config.args.clone();
            session.windows_shell = config.windows_shell;
            if session.session_mode == gwt_agent::SessionMode::Resume {
                session.agent_session_id = config.resume_session_id.clone();
            }
            session.update_status(gwt_agent::AgentStatus::Running);

            let session_id = session.id.clone();
            let runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                session_id.clone(),
            );
            config.env_vars.insert(
                gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                runtime_path.display().to_string(),
            );
            if let Some(target) = hook_forward_target {
                config
                    .env_vars
                    .insert(gwt_agent::GWT_HOOK_FORWARD_URL_ENV.to_string(), target.url);
                config.env_vars.insert(
                    gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV.to_string(),
                    target.token,
                );
            }
            config
                .env_vars
                .entry("COLORTERM".to_string())
                .or_insert_with(|| "truecolor".to_string());
            finalize_docker_agent_launch_config(Path::new(&project_root), &mut config)?;
            let runtime_target = config.runtime_target;
            let agent_project_root = if runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                resolve_docker_launch_plan(&worktree_path, config.docker_service.as_deref())?
                    .container_cwd
            } else {
                config
                    .env_vars
                    .get("GWT_PROJECT_ROOT")
                    .cloned()
                    .unwrap_or_else(|| worktree_path.display().to_string())
            };

            session
                .save(&sessions_dir)
                .map_err(|error| error.to_string())?;
            gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
                .save(&runtime_path)
                .map_err(|error| error.to_string())?;

            let process_launch = ProcessLaunch {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env_vars.clone(),
                remove_env: config.remove_env.clone(),
                cwd: config.working_dir.clone(),
            };

            Ok((
                process_launch,
                session_id,
                branch_name,
                config.display_name,
                worktree_path,
                agent_id,
                config.linked_issue_number,
                config.base_branch.clone(),
                runtime_target,
                agent_project_root,
            ))
        })();

        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
                base_branch,
                runtime_target,
                agent_project_root,
            )) => {
                dispatch_agent_launch_success(
                    proxy,
                    window_id,
                    (
                        process_launch,
                        session_id,
                        branch_name,
                        display_name,
                        worktree_path,
                        agent_id,
                        linked_issue_number,
                        base_branch,
                        runtime_target,
                        agent_project_root,
                    ),
                    |proxy, project_index_root| {
                        crate::project_index_bootstrap::ProjectIndexBootstrapService::global()
                            .spawn(proxy, project_index_root);
                    },
                );
            }
            Err(error) => {
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    result: Err(error),
                });
            }
        }
    }

    pub(crate) fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            return;
        };
        if let Some(project_root) = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.clone())
        {
            if let Err(error) = gwt_core::workspace_projection::mark_workspace_agent_stopped(
                &project_root,
                &session.session_id,
                Some(&session.window_id),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %session.session_id,
                    window_id = %session.window_id,
                    "failed to clean stopped Agent from Workspace projection"
                );
            }
        }
        let _ = gwt_agent::persist_session_status(
            &self.sessions_dir,
            &session.session_id,
            gwt_agent::AgentStatus::Stopped,
        );
        self.launch_wizard_cache.mark_stopped(&session.session_id);
    }

    pub(crate) fn clear_agent_window_startup_restore(&self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let _ = gwt_agent::persist_session_restore_window_on_startup(
            &self.sessions_dir,
            &session.session_id,
            false,
        );
    }

    fn refresh_launch_wizard_session_cache(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.get(window_id) else {
            return;
        };
        let path = self
            .sessions_dir
            .join(format!("{}.toml", session.session_id));
        match gwt_agent::Session::load_and_migrate(&path) {
            Ok(session) => self.launch_wizard_cache.record_session(session),
            Err(error) => tracing::warn!(
                path = %path.display(),
                error = %error,
                "failed to refresh Launch Wizard session cache"
            ),
        }
    }

    pub(crate) fn register_pty_writer(&self, id: &str, pane: &Arc<Mutex<Pane>>) {
        let Ok(pane_guard) = pane.lock() else {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "registry_lock_poisoned",
                window_id = %id,
                "failed to register PTY writer: pane mutex poisoned"
            );
            return;
        };
        let pty = pane_guard.shared_pty();
        drop(pane_guard);
        match self.pty_writers.write() {
            Ok(mut guard) => {
                guard.insert(id.to_string(), pty);
            }
            Err(error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "registry_write_poisoned",
                    window_id = %id,
                    error = %error,
                    "failed to register PTY writer: registry poisoned"
                );
            }
        }
    }

    pub(crate) fn deregister_pty_writer(&self, id: &str) {
        match self.pty_writers.write() {
            Ok(mut guard) => {
                guard.remove(id);
            }
            Err(error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "registry_deregister_poisoned",
                    window_id = %id,
                    error = %error,
                    "failed to deregister PTY writer: registry poisoned"
                );
            }
        }
    }

    pub(crate) fn stop_window_runtime(&mut self, window_id: &str) {
        self.stop_window_runtime_inner(window_id, true);
    }

    fn stop_window_runtime_inner(&mut self, window_id: &str, mark_session_stopped: bool) {
        if mark_session_stopped {
            self.mark_agent_session_stopped(window_id);
        }
        self.remove_window_state_tracking(window_id);
        self.deregister_pty_writer(window_id);
        if let Some(mut runtime) = self.runtimes.remove(window_id) {
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
            }
            if let Some(handle) = runtime.output_thread.take() {
                // PTY and its process group were already terminated by
                // `pane.kill()`, so the reader should see EOF quickly. Cap
                // the wait anyway so shutdown never stalls the event loop
                // if a stuck syscall keeps the reader in `read`. If the
                // timeout elapses the reader thread is detached; its Arc
                // clone of the Pane will still be released when the thread
                // does finally observe EOF.
                let (tx, rx) = std_mpsc::channel();
                thread::spawn(move || {
                    let _ = handle.join();
                    let _ = tx.send(());
                });
                let _ = rx.recv_timeout(Duration::from_millis(500));
            }
            if let Some(handle) = runtime.status_thread.take() {
                let (tx, rx) = std_mpsc::channel();
                thread::spawn(move || {
                    let _ = handle.join();
                    let _ = tx.send(());
                });
                let _ = rx.recv_timeout(Duration::from_millis(500));
            }
        }
        self.window_details.remove(window_id);
    }

    /// Stop every active window runtime. Called from the application shutdown
    /// paths so no PTY / agent process outlives the GUI.
    pub(crate) fn stop_all_runtimes(&mut self) {
        let ids: Vec<String> = self.runtimes.keys().cloned().collect();
        for id in ids {
            self.stop_window_runtime_inner(&id, false);
        }
    }

    pub(crate) fn spawn_output_thread(
        &self,
        id: String,
        pane: Arc<Mutex<Pane>>,
        _console_kind: Option<gwt_core::process_console::ProcessKind>,
    ) -> JoinHandle<()> {
        // SPEC-2809 (revised) — the Console window is the gwt-side
        // equivalent of VS Code's Output panel. It surfaces what gwt
        // itself spawns in the background (gh / git / docker / agent
        // bootstrap stages / Python index runner) per kind. The agent
        // tab is for the **Launch Wizard pipeline** that culminates in
        // the PTY spawn — not the agent's own runtime stdout. That
        // runtime stdout already lives in the workspace terminal pane
        // (xterm.js) and would only duplicate noise here. `_console_kind`
        // is retained on the API for forward compatibility with future
        // kind-aware hooks (e.g. recording the PTY exit code as a
        // summary at thread end).
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let reader = match pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| pane.reader().map_err(|error| error.to_string()))
            {
                Ok(reader) => reader,
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    });
                    return;
                }
            };

            let mut reader = reader;
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let chunk = buffer[..read].to_vec();
                        let lock_started = Instant::now();
                        if let Ok(mut pane) = pane.lock() {
                            let lock_wait_us = lock_started.elapsed().as_micros() as u64;
                            let parse_started = Instant::now();
                            pane.process_bytes(&chunk);
                            let parse_us = parse_started.elapsed().as_micros() as u64;
                            // Log only when the contention window is large enough
                            // to plausibly starve a concurrent `write_input`. The
                            // threshold keeps the log volume bounded during
                            // normal output bursts while still surfacing the
                            // lock-hold windows that matter for drop triage.
                            if lock_wait_us > 500 || parse_us > 500 {
                                tracing::debug!(
                                    target: "gwt_input_trace",
                                    stage = "reader_pane_lock",
                                    window_id = %id,
                                    chunk_len = read,
                                    lock_wait_us,
                                    parse_us,
                                    "reader thread held pane mutex (output parsing)"
                                );
                            }
                        }
                        proxy.send(UserEvent::RuntimeOutput {
                            id: id.clone(),
                            data: chunk,
                        });
                    }
                    Err(error) => {
                        proxy.send(UserEvent::RuntimeStatus {
                            id: id.clone(),
                            status: WindowProcessStatus::Error,
                            detail: Some(error.to_string()),
                        });
                        return;
                    }
                }
            }

            let status = pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|mut pane| {
                    pane.check_status()
                        .cloned()
                        .map_err(|error| error.to_string())
                });

            match status {
                Ok(status) => {
                    let (status, detail) = Self::runtime_status_from_pane_status(&status);
                    proxy.send(UserEvent::RuntimeStatus { id, status, detail });
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    });
                }
            }
        })
    }

    pub(crate) fn spawn_status_thread(&self, id: String, pane: Arc<Mutex<Pane>>) -> JoinHandle<()> {
        let proxy = self.proxy.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(100));
            let status = pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|mut pane| {
                    pane.check_status()
                        .cloned()
                        .map_err(|error| error.to_string())
                });

            match status {
                Ok(PaneStatus::Running) => continue,
                Ok(status) => {
                    if matches!(status, PaneStatus::Completed(_)) {
                        if let Ok(pane) = pane.lock() {
                            let _ = pane.kill();
                        }
                    }
                    let (status, detail) = Self::runtime_status_from_pane_status(&status);
                    proxy.send(UserEvent::RuntimeStatus { id, status, detail });
                    break;
                }
                Err(error) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    });
                    break;
                }
            }
        })
    }

    fn runtime_status_from_pane_status(
        status: &PaneStatus,
    ) -> (WindowProcessStatus, Option<String>) {
        match status {
            PaneStatus::Running => (WindowProcessStatus::Running, None),
            PaneStatus::Completed(0) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some("Process exited".to_string()),
            ),
            PaneStatus::Completed(code) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some(format!("Process exited with status {code}")),
            ),
            PaneStatus::Error(message) => (
                gwt::window_state::window_state_from_pane_status(status),
                Some(message.clone()),
            ),
        }
    }

    pub(crate) fn app_state_view(&self) -> gwt::AppStateView {
        gwt::AppStateView {
            app_version: crate::runtime_support::current_app_version().to_string(),
            tabs: self
                .tabs
                .iter()
                .map(|tab| {
                    let workspace = self.workspace_view_for_tab(tab);
                    let running_agents =
                        crate::runtime_support::collect_running_agents(&workspace.windows);
                    gwt::ProjectTabView {
                        id: tab.id.clone(),
                        title: tab.title.clone(),
                        project_root: tab.project_root.display().to_string(),
                        kind: tab.kind,
                        workspace,
                        running_agent_count: running_agents.len() as u32,
                        running_agents,
                    }
                })
                .collect(),
            active_tab_id: self.active_tab_id.clone(),
            recent_projects: self
                .recent_projects
                .iter()
                .map(|project| gwt::RecentProjectView {
                    path: project.path.display().to_string(),
                    title: project.title.clone(),
                    kind: project.kind,
                })
                .collect(),
        }
    }

    fn workspace_view_for_tab(&self, tab: &ProjectTabRuntime) -> gwt::WorkspaceView {
        gwt::WorkspaceView {
            viewport: tab.workspace.persisted().viewport.clone(),
            windows: tab
                .workspace
                .persisted()
                .windows
                .iter()
                .cloned()
                .map(|mut window| {
                    let raw_id = window.id.clone();
                    window.id = combined_window_id(&tab.id, &raw_id);
                    if let Some(status) = self.window_status(&window.id) {
                        window.status = status;
                    }
                    window
                })
                .collect(),
            work_items: Vec::new(),
        }
    }

    pub(crate) fn workspace_state_broadcast(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::WorkspaceState {
            workspace: self.app_state_view(),
        })
    }

    pub(crate) fn push_workspace_and_active_work_projection_broadcasts(
        &self,
        events: &mut Vec<OutboundEvent>,
    ) {
        events.push(self.workspace_state_broadcast());
        if let Some(event) = self.active_work_projection_broadcast_for_active_tab() {
            events.push(event);
        }
    }

    pub(crate) fn window_status(&self, window_id: &str) -> Option<WindowProcessStatus> {
        let pty_state = self
            .window_pty_statuses
            .get(window_id)
            .copied()
            .or_else(|| {
                let address = self.window_lookup.get(window_id)?;
                let tab = self.tab(&address.tab_id)?;
                let window = tab.workspace.window(&address.raw_id)?;
                Some(window.status)
            })?;
        let hook_state = self.window_hook_states.get(window_id).copied();
        let preset = self.window_preset(window_id)?;
        Some(gwt::window_state::compose_window_state_with_active_session(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
        ))
    }

    pub(crate) fn register_window(&mut self, tab_id: &str, raw_id: &str) {
        self.window_lookup.insert(
            combined_window_id(tab_id, raw_id),
            WindowAddress {
                tab_id: tab_id.to_string(),
                raw_id: raw_id.to_string(),
            },
        );
    }

    pub(crate) fn set_window_status(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        status: WindowProcessStatus,
    ) {
        if let Some(tab) = self.tab_mut(tab_id) {
            let _ = tab.workspace.set_status(raw_id, status);
            if let Some(window) = tab.workspace.window(raw_id) {
                let window_id = combined_window_id(tab_id, raw_id);
                if window.preset.requires_process() {
                    self.window_pty_statuses.insert(window_id, status);
                } else {
                    self.window_pty_statuses.remove(&window_id);
                }
            }
        }
    }

    pub(crate) fn resize_runtime_to_window(&self, window_id: &str) {
        let Some(address) = self.window_lookup.get(window_id) else {
            return;
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return;
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return;
        };
        if !window.preset.requires_process() {
            return;
        }
        if let Some(runtime) = self.runtimes.get(window_id) {
            if let Ok(mut pane) = runtime.pane.lock() {
                let (cols, rows) = geometry_to_pty_size(&window.geometry);
                let _ = pane.resize(cols.max(20), rows.max(6));
            }
        }
    }

    pub(crate) fn tab(&self, tab_id: &str) -> Option<&ProjectTabRuntime> {
        self.tabs.iter().find(|tab| tab.id == tab_id)
    }

    pub(crate) fn active_project_root(&self) -> Option<&Path> {
        let active_tab_id = self.active_tab_id.as_ref()?;
        self.tab(active_tab_id)
            .map(|tab| tab.project_root.as_path())
    }

    pub(crate) fn tab_mut(&mut self, tab_id: &str) -> Option<&mut ProjectTabRuntime> {
        self.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }

    pub(crate) fn active_tab_mut(&mut self) -> Option<&mut ProjectTabRuntime> {
        let active_tab_id = self.active_tab_id.clone()?;
        self.tab_mut(&active_tab_id)
    }

    pub(crate) fn set_active_tab(&mut self, tab_id: String) -> bool {
        let wizard_closed = self
            .launch_wizard
            .as_ref()
            .is_some_and(|wizard| wizard.tab_id != tab_id);
        self.active_tab_id = Some(tab_id);
        if wizard_closed {
            self.launch_wizard = None;
        }
        wizard_closed
    }

    pub(crate) fn rebuild_window_lookup(&mut self) {
        self.window_lookup.clear();
        let pairs = self
            .tabs
            .iter()
            .flat_map(|tab| {
                tab.workspace
                    .persisted()
                    .windows
                    .iter()
                    .map(|window| (tab.id.clone(), window.id.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (tab_id, raw_id) in pairs {
            self.register_window(&tab_id, &raw_id);
        }
    }

    fn window_preset(&self, window_id: &str) -> Option<WindowPreset> {
        let address = self.window_lookup.get(window_id)?;
        let tab = self.tab(&address.tab_id)?;
        let window = tab.workspace.window(&address.raw_id)?;
        Some(window.preset)
    }

    pub(crate) fn seed_window_pty_statuses(&mut self) {
        self.window_pty_statuses.clear();
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if window.preset.requires_process() {
                    self.window_pty_statuses
                        .insert(combined_window_id(&tab.id, &window.id), window.status);
                }
            }
        }
        self.window_hook_states.clear();
    }

    fn active_window_for_runtime_event(&self, event: &gwt::RuntimeHookEvent) -> Option<String> {
        [
            event.gwt_session_id.as_deref(),
            event.agent_session_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(|session_id| {
            self.active_agent_sessions
                .iter()
                .find(|(_, session)| session.session_id == session_id)
                .map(|(window_id, _)| window_id.clone())
        })
    }

    fn recompute_window_state(&mut self, window_id: &str) -> Option<WindowProcessStatus> {
        let pty_state = self
            .window_pty_statuses
            .get(window_id)
            .copied()
            .or_else(|| self.window_status(window_id))?;
        let hook_state = self.window_hook_states.get(window_id).copied();
        let preset = self.window_preset(window_id)?;
        let composed = gwt::window_state::compose_window_state_with_active_session(
            pty_state,
            preset,
            hook_state,
            self.active_agent_sessions.contains_key(window_id),
        );
        let address = self.window_lookup.get(window_id)?.clone();
        if let Some(tab) = self.tab_mut(&address.tab_id) {
            let _ = tab.workspace.set_status(&address.raw_id, composed);
        }
        Some(composed)
    }

    fn remove_window_state_tracking(&mut self, window_id: &str) {
        self.window_pty_statuses.remove(window_id);
        self.window_hook_states.remove(window_id);
        self.board_all_view_windows.remove(window_id);
    }

    fn tracked_window_exists(&self, window_id: &str) -> bool {
        let Some(address) = self.window_lookup.get(window_id) else {
            return false;
        };
        self.tab(&address.tab_id)
            .and_then(|tab| tab.workspace.window(&address.raw_id))
            .is_some()
    }

    fn launch_wizard_action_error_stage(action: &gwt::LaunchWizardAction) -> &'static str {
        match action {
            gwt::LaunchWizardAction::Submit => "wizard_submit",
            gwt::LaunchWizardAction::ApplyQuickStart { .. } => "quick_start",
            gwt::LaunchWizardAction::SetLaunchPath { .. }
            | gwt::LaunchWizardAction::SelectQuickStart { .. }
            | gwt::LaunchWizardAction::SelectLiveSession { .. }
            | gwt::LaunchWizardAction::UseStartMethod { .. } => "launch_path_select",
            gwt::LaunchWizardAction::FocusExistingSession { .. } => "focus_existing_session",
            gwt::LaunchWizardAction::SetAgent { .. } => "agent_select",
            gwt::LaunchWizardAction::SetLaunchTarget { .. } => "launch_target_select",
            gwt::LaunchWizardAction::Select { .. } => "wizard_select",
            _ => "wizard_action",
        }
    }

    fn launch_wizard_action_label(action: &gwt::LaunchWizardAction) -> &'static str {
        match action {
            gwt::LaunchWizardAction::Select { .. } => "select",
            gwt::LaunchWizardAction::Back => "back",
            gwt::LaunchWizardAction::Cancel => "cancel",
            gwt::LaunchWizardAction::SubmitText { .. } => "submit_text",
            gwt::LaunchWizardAction::ApplyQuickStart { .. } => "apply_quick_start",
            gwt::LaunchWizardAction::UseStartMethod { .. } => "use_start_method",
            gwt::LaunchWizardAction::SetLaunchPath { .. } => "set_launch_path",
            gwt::LaunchWizardAction::SelectQuickStart { .. } => "select_quick_start",
            gwt::LaunchWizardAction::SelectLiveSession { .. } => "select_live_session",
            gwt::LaunchWizardAction::FocusExistingSession { .. } => "focus_existing_session",
            gwt::LaunchWizardAction::SetBranchMode { .. } => "set_branch_mode",
            gwt::LaunchWizardAction::SetBranchType { .. } => "set_branch_type",
            gwt::LaunchWizardAction::SetBranchName { .. } => "set_branch_name",
            gwt::LaunchWizardAction::SetLaunchTarget { .. } => "set_launch_target",
            gwt::LaunchWizardAction::SetAgent { .. } => "set_agent",
            gwt::LaunchWizardAction::SetModel { .. } => "set_model",
            gwt::LaunchWizardAction::SetReasoning { .. } => "set_reasoning",
            gwt::LaunchWizardAction::SetRuntimeTarget { .. } => "set_runtime_target",
            gwt::LaunchWizardAction::SetWindowsShell { .. } => "set_windows_shell",
            gwt::LaunchWizardAction::SetDockerService { .. } => "set_docker_service",
            gwt::LaunchWizardAction::SetDockerLifecycle { .. } => "set_docker_lifecycle",
            gwt::LaunchWizardAction::SetVersion { .. } => "set_version",
            gwt::LaunchWizardAction::SetExecutionMode { .. } => "set_execution_mode",
            gwt::LaunchWizardAction::SetLinkedIssue { .. } => "set_linked_issue",
            gwt::LaunchWizardAction::ClearLinkedIssue => "clear_linked_issue",
            gwt::LaunchWizardAction::SetSkipPermissions { .. } => "set_skip_permissions",
            gwt::LaunchWizardAction::SetCodexFastMode { .. } => "set_codex_fast_mode",
            gwt::LaunchWizardAction::Submit => "submit",
        }
    }

    fn log_launch_wizard_error(
        session: &LaunchWizardSession,
        stage: &'static str,
        action: &'static str,
        requested_agent_id: Option<&str>,
        error: &str,
    ) {
        let view = session.wizard.view();
        let sanitized_error = Self::sanitize_launch_log_error(error);
        let linked_issue_number = view
            .linked_issue_number
            .map(|issue_number| issue_number.to_string())
            .unwrap_or_else(|| "none".to_string());
        let requested_agent_id = requested_agent_id.unwrap_or("none");
        let selected_docker_service = view.selected_docker_service.as_deref().unwrap_or("none");
        tracing::error!(
            target: "gwt::agent_launch",
            stage = %stage,
            action = %action,
            wizard_id = %session.wizard_id,
            tab_id = %session.tab_id,
            requested_agent_id = %requested_agent_id,
            selected_agent_id = %view.selected_agent_id,
            selected_launch_target = %view.selected_launch_target,
            selected_runtime_target = %view.selected_runtime_target,
            selected_tool_version = %view.selected_version,
            selected_docker_service = %selected_docker_service,
            linked_issue_number = %linked_issue_number,
            error = %sanitized_error,
            "launch wizard action failed"
        );
    }

    fn log_window_launch_error(&self, stage: &'static str, window_id: &str, error: &str) {
        let (tab_id, raw_window_id) = self
            .window_lookup
            .get(window_id)
            .map(|address| (address.tab_id.as_str(), address.raw_id.as_str()))
            .unwrap_or(("unknown", "unknown"));
        let session = self.active_agent_sessions.get(window_id);
        let session_id = session
            .map(|session| session.session_id.as_str())
            .unwrap_or("unknown");
        let agent_id = session
            .map(|session| session.agent_id.as_str())
            .unwrap_or("unknown");
        let branch_name = session
            .map(|session| session.branch_name.as_str())
            .unwrap_or("unknown");
        let sanitized_error = Self::sanitize_launch_log_error(error);
        tracing::error!(
            target: "gwt::agent_launch",
            stage = %stage,
            window_id = %window_id,
            tab_id = %tab_id,
            raw_window_id = %raw_window_id,
            session_id = %session_id,
            agent_id = %agent_id,
            branch = %branch_name,
            error = %sanitized_error,
            "window launch failed"
        );
    }

    fn sanitize_launch_log_error(error: &str) -> String {
        let sensitive_env_keys = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GITHUB_TOKEN",
            "GH_TOKEN",
            "GWT_HOOK_TOKEN",
            "HOOK_TOKEN",
        ];
        let sensitive_flags = [
            "--api-key",
            "--apikey",
            "--token",
            "--auth-token",
            "--hook-token",
        ];

        let mut tokens = Vec::new();
        let mut redact_next = false;
        for token in error.split_whitespace() {
            if redact_next {
                tokens.push("[REDACTED]".to_string());
                redact_next = false;
                continue;
            }

            let normalized = token
                .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';'))
                .to_ascii_lowercase();
            if sensitive_flags.iter().any(|flag| normalized == *flag) {
                tokens.push(token.to_string());
                redact_next = true;
                continue;
            }
            if let Some(flag) = sensitive_flags
                .iter()
                .find(|flag| normalized.starts_with(&format!("{flag}=")))
            {
                tokens.push(format!("{flag}=[REDACTED]"));
                continue;
            }
            if let Some((key, _value)) = token.split_once('=') {
                let normalized_key = key.trim_matches(|ch: char| matches!(ch, '"' | '\''));
                if sensitive_env_keys
                    .iter()
                    .any(|candidate| normalized_key.eq_ignore_ascii_case(candidate))
                {
                    tokens.push(format!("{normalized_key}=[REDACTED]"));
                    continue;
                }
            }

            tokens.push(token.to_string());
        }
        tokens.join(" ")
    }

    fn launch_error_events(
        &mut self,
        window_id: String,
        detail: String,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Vec<OutboundEvent> {
        self.log_window_launch_error("launch_complete", &window_id, &detail);
        if self.tracked_window_exists(&window_id) {
            return self.handle_runtime_status(window_id, WindowProcessStatus::Error, Some(detail));
        }
        let mut events =
            Self::status_events(window_id, WindowProcessStatus::Error, Some(detail.clone()));
        if let Some(context) = launch_feedback_context {
            events.push(OutboundEvent::reply(
                context.client_id,
                BackendEvent::LaunchWizardOpenError {
                    title: context.title,
                    message: detail,
                },
            ));
        }
        events
    }

    fn status_events(
        window_id: impl Into<String>,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        let window_id = window_id.into();
        vec![
            OutboundEvent::broadcast(BackendEvent::WindowState {
                window_id: window_id.clone(),
                state: status,
            }),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id,
                status,
                detail,
            }),
        ]
    }

    pub(crate) fn seed_restored_window_details(&mut self) {
        self.window_details.clear();
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if window.preset.requires_process() && window.status == WindowProcessStatus::Stopped
                {
                    self.window_details.insert(
                        combined_window_id(&tab.id, &window.id),
                        "Restored window is paused. Launch a new terminal when you want to start it."
                            .to_string(),
                    );
                }
            }
        }
    }

    /// Capture the current session + workspace state and hand it off to the
    /// persist dispatcher. The dispatcher writes the snapshot atomically on a
    /// worker thread, so this call returns without blocking on disk I/O.
    /// Bursts of `persist()` calls collapse to a single disk write because the
    /// dispatcher keeps only the latest snapshot.
    ///
    /// Issue #2694 Phase B: prior to this change the call wrote
    /// `session-state.json` and every active workspace file synchronously on
    /// the tao event-loop thread, which Windows Defender / EDR scans amplified
    /// into multi-hundred-millisecond freezes during routine UI interactions.
    pub(crate) fn persist(&self) -> std::io::Result<()> {
        let snapshot = persist_dispatcher::PersistSnapshot {
            session_path: self.session_state_path.clone(),
            session: gwt::PersistedSessionState {
                tabs: self
                    .tabs
                    .iter()
                    .map(|tab| gwt::PersistedSessionTabState {
                        id: tab.id.clone(),
                        title: tab.title.clone(),
                        project_root: tab.project_root.clone(),
                        kind: tab.kind,
                    })
                    .collect(),
                active_tab_id: normalize_active_tab_id(&self.tabs, self.active_tab_id.clone()),
                recent_projects: self.recent_projects.clone(),
            },
            workspaces: self
                .tabs
                .iter()
                .map(|tab| {
                    (
                        workspace_state_path(&tab.project_root),
                        tab.workspace.persistable_state(),
                    )
                })
                .collect(),
        };
        self.persist_dispatcher.enqueue(snapshot);
        Ok(())
    }
}

fn record_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: u64,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, Some(issue_number), cache_dir)
}

fn clear_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, None, cache_dir)
}

fn update_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: Option<u64>,
    cache_dir: &Path,
) -> Result<(), String> {
    let branch_name = branch_name.trim();
    if branch_name.is_empty() {
        return Ok(());
    }
    let Some(repo_hash) = gwt::index_worker::detect_repo_hash(repo_path) else {
        return Err("repository hash is unavailable for issue linkage".to_string());
    };
    let path = cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));

    let mut store = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<IssueBranchLinkStore>(&bytes)
            .map_err(|error| format!("failed to parse issue linkage store: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            IssueBranchLinkStore::default()
        }
        Err(error) => return Err(format!("failed to read issue linkage store: {error}")),
    };

    match issue_number {
        Some(issue_number) => {
            store.branches.insert(branch_name.to_string(), issue_number);
        }
        None => {
            if store.branches.remove(branch_name).is_none() {
                return Ok(());
            }
        }
    }

    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|error| format!("failed to serialize issue linkage store: {error}"))?;
    gwt_github::cache::write_atomic(&path, &bytes)
        .map_err(|error| format!("failed to write issue linkage store: {error}"))
}

fn maybe_register_codex_managed_hook_trust_for_launch(
    profile_config_path: &Path,
    worktree_path: &Path,
    agent_id: &gwt_agent::AgentId,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_service: Option<&str>,
) -> Result<Option<gwt_skills::CodexHookTrustReport>, String> {
    if agent_id != &gwt_agent::AgentId::Codex {
        return Ok(None);
    }

    let settings = if profile_config_path.exists() {
        match gwt_config::Settings::load_from_path(profile_config_path) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    error = %error,
                    "failed to read gwt config while preparing Codex hook trust; continuing launch"
                );
                gwt_config::Settings::default()
            }
        }
    } else {
        gwt_config::Settings::default()
    };
    if settings.agent.codex_trust_managed_hooks == Some(false) {
        return Ok(None);
    }

    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let Some(codex_config_path) = codex_config_path_for_profile_config(profile_config_path)
            else {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    "cannot derive Codex config path while preparing Codex hook trust; continuing launch"
                );
                return Ok(None);
            };
            match gwt_skills::register_codex_managed_hook_trust(worktree_path, &codex_config_path) {
                Ok(report) => Ok(Some(report)),
                Err(error) => {
                    tracing::warn!(
                        worktree = %worktree_path.display(),
                        codex_config = %codex_config_path.display(),
                        error = %error,
                        "failed to register gwt-managed Codex hook trust; continuing launch"
                    );
                    Ok(None)
                }
            }
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            if let Err(error) = gwt_agent::register_codex_managed_hook_trust_in_docker(
                worktree_path,
                docker_service,
            ) {
                tracing::warn!(
                    worktree = %worktree_path.display(),
                    error = %error,
                    "failed to register gwt-managed Codex hook trust in Docker; continuing launch"
                );
            }
            Ok(None)
        }
    }
}

fn codex_config_path_for_profile_config(profile_config_path: &Path) -> Option<PathBuf> {
    let gwt_config_dir = profile_config_path.parent()?;
    if gwt_config_dir.file_name().and_then(|name| name.to_str()) != Some(".gwt") {
        return None;
    }
    Some(gwt_config_dir.parent()?.join(".codex").join("config.toml"))
}

fn file_content_save_error(
    id: &str,
    path: &str,
    mode: FileContentMode,
    kind: gwt::FileContentSaveErrorKind,
    message: String,
    current_mtime: Option<u64>,
    current_size: Option<u64>,
) -> BackendEvent {
    BackendEvent::FileContentSaveError {
        id: id.to_string(),
        path: path.to_string(),
        mode,
        error_kind: kind,
        message,
        current_mtime,
        current_size,
    }
}

fn write_error_to_event(
    id: &str,
    path: &str,
    mode: FileContentMode,
    err: FileContentError,
) -> BackendEvent {
    use gwt::FileContentSaveErrorKind as Kind;
    let (kind, message, current_mtime, current_size) = match err {
        FileContentError::Denied => (Kind::Denied, "Access denied".to_string(), None, None),
        FileContentError::TooLarge { size, limit } => (
            Kind::TooLarge,
            format!("File too large ({} bytes, limit {})", size, limit),
            None,
            None,
        ),
        FileContentError::IoError(message) => (Kind::IoError, message, None, None),
        FileContentError::NotAFile => (Kind::NotAFile, "Not a file".to_string(), None, None),
        FileContentError::BinaryNotText => (
            Kind::IoError,
            "Cannot decode as text".to_string(),
            None,
            None,
        ),
        FileContentError::Conflict {
            current_mtime,
            current_size,
        } => (
            Kind::Conflict,
            format!("File changed externally (current mtime={current_mtime}, size={current_size})"),
            Some(current_mtime),
            Some(current_size),
        ),
        FileContentError::ReadOnly => (Kind::ReadOnly, "File is read-only".to_string(), None, None),
        FileContentError::OutOfRange { offset, size } => (
            Kind::OutOfRange,
            format!("Offset {offset} is outside file (size {size})"),
            None,
            Some(size),
        ),
    };
    file_content_save_error(id, path, mode, kind, message, current_mtime, current_size)
}

fn file_content_error_to_event(id: &str, path: &str, err: FileContentError) -> BackendEvent {
    let (kind, message, size, limit) = match err {
        FileContentError::Denied => (
            FileContentErrorKind::Denied,
            "Access denied".to_string(),
            None,
            None,
        ),
        FileContentError::TooLarge { size, limit } => (
            FileContentErrorKind::TooLarge,
            format!("File too large ({} bytes, limit {})", size, limit),
            Some(size),
            Some(limit),
        ),
        FileContentError::IoError(message) => (FileContentErrorKind::IoError, message, None, None),
        FileContentError::NotAFile => (
            FileContentErrorKind::NotAFile,
            "Not a file".to_string(),
            None,
            None,
        ),
        FileContentError::BinaryNotText => (
            FileContentErrorKind::BinaryNotText,
            "Cannot decode as text".to_string(),
            None,
            None,
        ),
        // SPEC-2006 Phase 2 variants are write-only and should never reach
        // the read-path mapping. Map defensively to IoError so the read
        // surface keeps working if a future caller funnels them here by
        // mistake; the write surface owns the structured Save error variant.
        FileContentError::Conflict {
            current_mtime,
            current_size,
        } => (
            FileContentErrorKind::IoError,
            format!("Unexpected Conflict in read path (mtime={current_mtime} size={current_size})"),
            Some(current_size),
            None,
        ),
        FileContentError::ReadOnly => (
            FileContentErrorKind::IoError,
            "Unexpected ReadOnly in read path".to_string(),
            None,
            None,
        ),
        FileContentError::OutOfRange { offset, size } => (
            FileContentErrorKind::IoError,
            format!("Unexpected OutOfRange in read path (offset={offset} size={size})"),
            Some(size),
            None,
        ),
    };
    BackendEvent::FileContentError {
        id: id.to_string(),
        path: path.to_string(),
        error_kind: kind,
        message,
        size,
        limit,
    }
}

#[cfg(test)]
mod agent_launch_stage_tests {
    //! SPEC-2809 (revised) — Tests for the Launch Wizard -> Console
    //! `agent` tab stage emission. Confirms that `emit_agent_launch_stage`
    //! pushes a banner line to the ProcessConsoleHub under the
    //! `AgentBootstrap` kind so the Console window surfaces the launch
    //! pipeline before the PTY pane takes over.
    use super::{emit_agent_launch_stage, next_agent_launch_stage_id};
    use gwt_core::process_console::{ProcessConsoleHub, ProcessKind, ProcessStream};

    fn drain_lines(hub: &ProcessConsoleHub) -> Vec<String> {
        hub.snapshot_kind(ProcessKind::AgentBootstrap)
            .into_iter()
            .map(|line| line.message)
            .collect()
    }

    #[test]
    fn launch_stage_ids_are_unique_per_caller() {
        let a = next_agent_launch_stage_id();
        let b = next_agent_launch_stage_id();
        assert!(b > a, "stage ids must strictly increase: {a} -> {b}");
    }

    #[test]
    fn emit_agent_launch_stage_pushes_a_banner_line_to_global_hub() {
        // The global hub is installed lazily by `gwt_core::logging::init`
        // in production, but tests run without that bootstrap. Install
        // a hub before exercising the emit helper so the snapshot read
        // observes the same instance the helper writes to. `set_global`
        // succeeds at most once per process; ignore the result so this
        // test cooperates with peers that also install the hub.
        let _ = gwt_core::process_console::set_global(ProcessConsoleHub::new());
        let spawn_id = next_agent_launch_stage_id();
        emit_agent_launch_stage(spawn_id, "resolve_binary", "claude");
        let hub = gwt_core::process_console::global();
        let recent = hub.snapshot_kind(ProcessKind::AgentBootstrap);
        assert!(
            recent.iter().any(|line| line.spawn_id == spawn_id
                && line.message == "[resolve_binary] claude"
                && line.stream == ProcessStream::Stdout),
            "expected a banner for the resolve_binary stage, got: {recent:?}",
        );
    }

    #[test]
    fn launch_stage_banner_includes_stage_label_in_message() {
        let hub = ProcessConsoleHub::new();
        for stage in ["prepare_env", "spawn_pty", "ready"] {
            hub.push(gwt_core::process_console::ProcessLine::new(
                ProcessKind::AgentBootstrap,
                42,
                ProcessStream::Stdout,
                format!("[{stage}] codex"),
            ));
        }
        let lines = drain_lines(&hub);
        assert_eq!(
            lines,
            vec![
                "[prepare_env] codex".to_string(),
                "[spawn_pty] codex".to_string(),
                "[ready] codex".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        ffi::OsString,
        fs,
        io::Write,
        path::{Path, PathBuf},
        sync::{mpsc, Arc, Mutex, RwLock},
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use base64::Engine;
    use chrono::{TimeZone, Utc};
    use gwt::{
        empty_workspace_state, load_restored_workspace_state, load_session_state, BackendEvent,
        BranchCleanupInfo, BranchListEntry, BranchScope, FrontendEvent, LaunchWizardAction,
        LaunchWizardContext, LaunchWizardState, ProfileEnvEntryView, ProjectKind, UiTracePayload,
        WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState,
    };
    use gwt_config::{Profile, Settings};
    use gwt_core::{
        coordination::{
            coordination_events_path, load_snapshot, post_entry, AuthorKind, BoardAudienceScope,
            BoardEntry, BoardEntryKind, BoardMention, BoardMentionTargetKind, CoordinationEvent,
        },
        logging::{current_log_file, LogLevel},
        paths::gwt_cache_dir,
        repo_hash::detect_repo_hash,
    };
    use gwt_github::{
        Cache, CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
    };
    use gwt_terminal::Pane;
    use tracing::{field::Visit, Event, Level, Subscriber};
    use tracing_subscriber::{layer::Context, prelude::*, Layer};

    use super::{
        active_work_projection_from_saved, dispatch_agent_launch_success,
        save_start_work_workspace_projection, save_workspace_launch_projection, ActiveAgentSession,
        AgentLaunchCompletion, AppEventProxy, AppRuntime, BlockingTaskSpawner, DispatchTarget,
        KnowledgeLoadRequest, KnowledgeRefreshTask, KnowledgeSearchRequest,
        LaunchWizardMemoryCache, LaunchWizardSession, OutboundEvent, ProcessLaunch,
        ProjectTabRuntime, UserEvent, WindowRuntime, WorkspaceResumeContext,
    };
    use crate::{combined_window_id, geometry_to_pty_size, same_worktree_path, PtyWriterRegistry};

    #[derive(Debug, Clone)]
    struct CapturedTracingEvent {
        level: Level,
        target: String,
        fields: HashMap<String, String>,
    }

    #[derive(Clone)]
    struct CaptureTracingLayer {
        events: Arc<Mutex<Vec<CapturedTracingEvent>>>,
    }

    impl<S> Layer<S> for CaptureTracingLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = CaptureTracingVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("captured tracing events")
                .push(CapturedTracingEvent {
                    level: *event.metadata().level(),
                    target: event.metadata().target().to_string(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct CaptureTracingVisitor {
        fields: HashMap<String, String>,
    }

    impl CaptureTracingVisitor {
        fn insert(&mut self, field: &tracing::field::Field, value: impl ToString) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    impl Visit for CaptureTracingVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            let raw = format!("{value:?}");
            self.insert(field, raw.trim_matches('"'));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.insert(field, value);
        }

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.insert(field, value);
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.insert(field, value);
        }

        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.insert(field, value);
        }
    }

    fn capture_tracing_events(run: impl FnOnce()) -> Vec<CapturedTracingEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(CaptureTracingLayer {
            events: Arc::clone(&events),
        });
        tracing::subscriber::with_default(subscriber, run);
        let captured_events = events.lock().expect("captured tracing events").clone();
        captured_events
    }

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_test_lock() -> &'static Mutex<()> {
        crate::env_test_lock()
    }

    fn fake_gh_test_lock() -> &'static Mutex<()> {
        static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_profile_config(path: &Path, settings: &Settings) {
        settings.save(path).expect("write profile config");
    }

    fn sample_issue_snapshot(
        number: u64,
        title: &str,
        labels: &[&str],
        body: &str,
        updated_at: &str,
    ) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new(updated_at),
            comments: vec![CommentSnapshot {
                id: CommentId(number * 10),
                body: format!("Comment for #{number}"),
                updated_at: UpdatedAt::new(updated_at),
            }],
        }
    }

    fn init_repo(repo_path: &Path) {
        let remote = format!(
            "https://github.com/example/repo-{:x}.git",
            remote_suffix(repo_path)
        );
        for args in [
            ["init", "-q"].as_slice(),
            ["remote", "add", "origin", remote.as_str()].as_slice(),
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(repo_path)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn remote_suffix(repo_path: &Path) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        repo_path.display().to_string().hash(&mut hasher);
        hasher.finish()
    }

    fn issue_cache_root(repo_path: &Path) -> PathBuf {
        let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
        gwt_cache_dir().join("issues").join(repo_hash.as_str())
    }

    fn write_issue_link_store(repo_path: &Path, branches: HashMap<String, u64>) {
        let repo_hash = detect_repo_hash(repo_path).expect("repo hash");
        let path = gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()));
        fs::create_dir_all(path.parent().expect("parent")).expect("create link dir");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({ "branches": branches }))
                .expect("serialize store"),
        )
        .expect("write link store");
    }

    #[cfg(unix)]
    fn write_fake_project_index_runtime(home: &Path) {
        let legacy_python = home
            .join(".gwt")
            .join("runtime")
            .join("chroma-venv")
            .join("bin")
            .join("python3");
        let script = r#"#!/bin/sh
for arg in "$@"; do
  if [ "$arg" = "-c" ]; then
    exit 0
  fi
done
case "$*" in
  *"-m pip"*)
    exit 0
    ;;
  *"--action probe"*)
    exit 0
    ;;
  *"--action search-issues"*)
    printf '%s\n' '{"ok":true,"issueResults":[{"number":42,"distance":0.25}]}'
    exit 0
    ;;
  *"--action search-specs"*)
    printf '%s\n' '{"ok":true,"specResults":[{"spec_id":1930,"distance":0.4}]}'
    exit 0
    ;;
esac
printf '%s\n' '{"ok":false,"error":"unexpected fake python invocation"}'
exit 1
"#;
        for python in [
            legacy_python,
            gwt_core::runtime::project_index_python_path(),
        ] {
            fs::create_dir_all(python.parent().expect("fake python parent"))
                .expect("create fake python dir");
            fs::write(&python, script).expect("write fake python");
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&python, fs::Permissions::from_mode(0o755))
                .expect("chmod fake python");
        }
    }

    fn write_fake_gh_issue_list(temp_root: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            let fake_gh = temp_root.join("gh.cmd");
            fs::write(
                &fake_gh,
                "@echo off\r\n\
if not \"%GWT_FAKE_GH_MARKER%\"==\"\" echo called>>\"%GWT_FAKE_GH_MARKER%\"\r\n\
if /I \"%GWT_FAKE_GH_MODE%\"==\"fail\" (\r\n\
  >&2 echo gh refresh failed\r\n\
  exit /b 1\r\n\
)\r\n\
echo [{\"number\":43,\"title\":\"Refreshed issue\",\"body\":\"Fresh body\",\"labels\":[{\"name\":\"bug\"}],\"state\":\"OPEN\",\"url\":\"https://example.test/issues/43\",\"updatedAt\":\"2026-04-20T00:00:00Z\"}]\r\n\
exit /b 0\r\n",
            )
            .expect("write fake gh");
            fake_gh
        }
        #[cfg(not(windows))]
        {
            let fake_gh = temp_root.join("gh");
            fs::write(
                &fake_gh,
                r#"#!/bin/sh
if [ -n "$GWT_FAKE_GH_MARKER" ]; then
  touch "$GWT_FAKE_GH_MARKER"
fi
if [ "$GWT_FAKE_GH_MODE" = "fail" ]; then
  printf '%s\n' 'gh refresh failed' >&2
  exit 1
fi
printf '%s\n' '[{"number":43,"title":"Refreshed issue","body":"Fresh body","labels":[{"name":"bug"}],"state":"OPEN","url":"https://example.test/issues/43","updatedAt":"2026-04-20T00:00:00Z"}]'
exit 0
"#,
            )
            .expect("write fake gh");
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755))
                .expect("chmod fake gh");
            fake_gh
        }
    }

    fn write_fake_git_recorder(temp_root: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            let fake_git = temp_root.join("git.cmd");
            fs::write(
                &fake_git,
                "@echo off\r\n\
if not \"%GWT_FAKE_GIT_LOG%\"==\"\" echo %*>>\"%GWT_FAKE_GIT_LOG%\"\r\n\
exit /b 1\r\n",
            )
            .expect("write fake git");
            fake_git
        }
        #[cfg(not(windows))]
        {
            let fake_git = temp_root.join("git");
            fs::write(
                &fake_git,
                r#"#!/bin/sh
if [ -n "$GWT_FAKE_GIT_LOG" ]; then
  printf '%s\n' "$*" >> "$GWT_FAKE_GIT_LOG"
fi
exit 1
"#,
            )
            .expect("write fake git");
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755))
                .expect("chmod fake git");
            fake_git
        }
    }

    fn prepend_tool_parent_to_path(tool: &Path) -> ScopedEnvVar {
        let parent = tool.parent().expect("tool parent");
        let mut paths = vec![parent.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        let joined = std::env::join_paths(paths).expect("join PATH");
        ScopedEnvVar::set("PATH", joined)
    }

    fn prepend_fake_gh_to_path(fake_gh: &Path) -> ScopedEnvVar {
        prepend_tool_parent_to_path(fake_gh)
    }

    fn canvas_bounds() -> WindowGeometry {
        WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1400.0,
            height: 900.0,
        }
    }

    fn sample_window(
        raw_id: &str,
        preset: WindowPreset,
        status: WindowProcessStatus,
    ) -> gwt::PersistedWindowState {
        gwt::PersistedWindowState {
            id: raw_id.to_string(),
            title: "Sample".to_string(),
            preset,
            geometry: WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 420.0,
            },
            geometry_revision: 0,
            z_index: 1,
            status,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist: true,
            purpose_title: None,
            dynamic_title: None,
            dynamic_title_detail: None,
            agent_id: None,
            agent_color: None,
            tab_group_id: None,
            tab_group_active: false,
        }
    }

    fn sample_project_tab_with_window(
        tab_id: &str,
        raw_window_id: &str,
        preset: WindowPreset,
        status: WindowProcessStatus,
    ) -> ProjectTabRuntime {
        sample_project_tab_with_window_at(
            tab_id,
            raw_window_id,
            PathBuf::from("E:/gwt/test-repo"),
            preset,
            status,
        )
    }

    fn sample_project_tab_with_window_at(
        tab_id: &str,
        raw_window_id: &str,
        project_root: PathBuf,
        preset: WindowPreset,
        status: WindowProcessStatus,
    ) -> ProjectTabRuntime {
        let mut persisted = empty_workspace_state();
        persisted
            .windows
            .push(sample_window(raw_window_id, preset, status));
        persisted.next_z_index = 2;
        ProjectTabRuntime {
            id: tab_id.to_string(),
            title: "Repo".to_string(),
            project_root,
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    fn sample_project_tab(
        tab_id: &str,
        title: &str,
        project_root: PathBuf,
        kind: ProjectKind,
        presets: &[WindowPreset],
    ) -> ProjectTabRuntime {
        let mut workspace = WorkspaceState::from_persisted(empty_workspace_state());
        for preset in presets {
            let _ = workspace.add_window(*preset, canvas_bounds());
        }
        ProjectTabRuntime {
            id: tab_id.to_string(),
            title: title.to_string(),
            project_root,
            kind,
            workspace,
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    #[test]
    fn app_runtime_rejects_removed_legacy_memo_window_creation() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.create_window_events(WindowPreset::Memo, canvas_bounds());

        assert!(events.is_empty());
        assert!(runtime.window_lookup.is_empty());
        assert!(runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .persisted()
            .windows
            .is_empty());
    }

    fn sample_active_agent_session(tab_id: &str, window_id: &str) -> ActiveAgentSession {
        ActiveAgentSession {
            window_id: window_id.to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/test".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            agent_project_root: "E:/gwt/test-repo".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: tab_id.to_string(),
        }
    }

    fn save_assigned_workspace_projection_for_test(
        repo: &Path,
        session: &ActiveAgentSession,
    ) -> Result<(), String> {
        let context = WorkspaceResumeContext {
            title: Some("Start Work".to_string()),
            owner: Some("SPEC-2359".to_string()),
            summary: Some("Assigned Workspace".to_string()),
            next_action: Some("Check Board for latest updates".to_string()),
        };
        save_workspace_launch_projection(repo, session, Some("develop"), None, Some(&context), true)
    }

    fn workspace_agent_summary_for_test(
        session_id: &str,
        workspace_id: Option<&str>,
    ) -> gwt_core::workspace_projection::WorkspaceAgentSummary {
        gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: Some("Board audience follow-up".to_string()),
            title_summary: Some("Board audience follow-up".to_string()),
            worktree_path: None,
            branch: Some("work/test".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: workspace_id.map(str::to_string),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn image_paste_prepare_uses_host_absolute_path_reference() {
        let temp = tempdir().expect("tempdir");
        let payload = base64::engine::general_purpose::STANDARD.encode(b"image-bytes");
        let agent_root = temp.path().display().to_string();

        let prepared = super::prepare_image_paste_file(
            temp.path(),
            &agent_root,
            &payload,
            "image/png",
            Some("../Screen Shot.png"),
            "20260507-160000",
        )
        .expect("prepare image paste");
        let expected_path = temp
            .path()
            .join(".gwt")
            .join("paste-images")
            .join("20260507-160000-screen-shot.png");

        assert_eq!(prepared.bytes, b"image-bytes");
        assert_eq!(prepared.storage_path, expected_path);
        assert_eq!(prepared.agent_path, expected_path.display().to_string());
        assert_eq!(
            prepared.prompt_text,
            format!("Image file: {}", expected_path.display())
        );
    }

    #[test]
    fn image_paste_prepare_uses_docker_project_root_reference() {
        let temp = tempdir().expect("tempdir");
        let payload = base64::engine::general_purpose::STANDARD.encode(b"jpeg-bytes");

        let prepared = super::prepare_image_paste_file(
            temp.path(),
            "/workspace/project",
            &payload,
            "image/jpeg",
            Some("Clipboard Image"),
            "20260507-160001",
        )
        .expect("prepare docker image paste");

        assert_eq!(
            prepared.storage_path,
            temp.path()
                .join(".gwt")
                .join("paste-images")
                .join("20260507-160001-clipboard-image.jpg")
        );
        assert_eq!(
            prepared.agent_path,
            "/workspace/project/.gwt/paste-images/20260507-160001-clipboard-image.jpg"
        );
        assert_eq!(
            prepared.prompt_text,
            "Image file: /workspace/project/.gwt/paste-images/20260507-160001-clipboard-image.jpg"
        );
    }

    #[test]
    fn image_paste_prepare_rejects_unsupported_mime_and_empty_payload() {
        let temp = tempdir().expect("tempdir");
        let payload = base64::engine::general_purpose::STANDARD.encode(b"gif-bytes");

        let unsupported = super::prepare_image_paste_file(
            temp.path(),
            "/workspace/project",
            &payload,
            "image/gif",
            Some("unsupported.gif"),
            "20260507-160002",
        );
        assert!(matches!(
            unsupported,
            Err(super::ImagePasteError::UnsupportedMimeType(mime)) if mime == "image/gif"
        ));

        let empty = super::prepare_image_paste_file(
            temp.path(),
            "/workspace/project",
            "",
            "image/png",
            None,
            "20260507-160003",
        );
        assert!(matches!(empty, Err(super::ImagePasteError::EmptyPayload)));
    }

    #[test]
    fn image_paste_event_saves_file_under_worktree() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab_id = "tab-1";
        let raw_window_id = "agent-1";
        let window_id = combined_window_id(tab_id, raw_window_id);
        let tab = sample_project_tab_with_window_at(
            tab_id,
            raw_window_id,
            worktree.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let (mut runtime, _events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "feature/image-paste".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: worktree.clone(),
                agent_project_root: worktree.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: tab_id.to_string(),
            },
        );
        let payload = base64::engine::general_purpose::STANDARD.encode(b"webp-bytes");
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "paste_image",
            "id": window_id,
            "data_base64": payload,
            "mime_type": "image/webp",
            "filename": "capture.webp"
        }))
        .expect("deserialize paste image event");

        let events = runtime.handle_frontend_event("client-1".to_string(), event);

        assert!(events.is_empty());
        let paste_dir = worktree.join(".gwt").join("paste-images");
        let files = fs::read_dir(&paste_dir)
            .expect("read paste dir")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect paste files");
        assert_eq!(files.len(), 1, "expected one saved image");
        let saved_path = files[0].path();
        assert_eq!(
            saved_path.extension().and_then(|ext| ext.to_str()),
            Some("webp")
        );
        assert_eq!(
            fs::read(saved_path).expect("read saved image"),
            b"webp-bytes"
        );
    }

    #[test]
    fn image_paste_event_ignores_non_agent_terminal_window() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab_id = "tab-1";
        let raw_window_id = "shell-1";
        let window_id = combined_window_id(tab_id, raw_window_id);
        let tab = sample_project_tab_with_window_at(
            tab_id,
            raw_window_id,
            worktree.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Running,
        );
        let (mut runtime, _events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some(tab_id));
        let payload = base64::engine::general_purpose::STANDARD.encode(b"png-bytes");
        let event: FrontendEvent = serde_json::from_value(serde_json::json!({
            "kind": "paste_image",
            "id": window_id,
            "data_base64": payload,
            "mime_type": "image/png",
            "filename": "capture.png"
        }))
        .expect("deserialize paste image event");

        let events = runtime.handle_frontend_event("client-1".to_string(), event);

        assert!(events.is_empty());
        assert!(
            !worktree.join(".gwt").join("paste-images").exists(),
            "non-agent terminal paste must not create image files"
        );
    }

    fn runtime_hook_state(status: &str, session_id: &str) -> gwt::RuntimeHookEvent {
        runtime_hook_state_for_event(status, "Stop", session_id)
    }

    fn runtime_hook_state_for_event(
        status: &str,
        source_event: &str,
        session_id: &str,
    ) -> gwt::RuntimeHookEvent {
        gwt::RuntimeHookEvent {
            kind: gwt::RuntimeHookEventKind::RuntimeState,
            source_event: Some(source_event.to_string()),
            gwt_session_id: Some(session_id.to_string()),
            agent_session_id: Some("agent-session-1".to_string()),
            project_root: Some("E:/gwt/test-repo".to_string()),
            branch: Some("feature/test".to_string()),
            status: Some(status.to_string()),
            tool_name: None,
            message: None,
            occurred_at: "2026-04-25T00:00:00Z".to_string(),
        }
    }

    fn sample_runtime(
        temp_root: &Path,
        tabs: Vec<ProjectTabRuntime>,
        active_tab_id: Option<&str>,
    ) -> AppRuntime {
        sample_runtime_with_events(temp_root, tabs, active_tab_id).0
    }

    fn sample_runtime_with_events(
        temp_root: &Path,
        tabs: Vec<ProjectTabRuntime>,
        active_tab_id: Option<&str>,
    ) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, _events) = AppEventProxy::stub();
        let sessions_dir = temp_root.join("sessions");
        let log_dir = temp_root.join("logs");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::create_dir_all(&log_dir).expect("create log dir");
        let launch_wizard_cache =
            LaunchWizardMemoryCache::load_with_agent_options(&sessions_dir, sample_agent_options());
        let pty_writers: PtyWriterRegistry = Arc::new(RwLock::new(HashMap::new()));
        let blocking_tasks = BlockingTaskSpawner::thread();
        let persist_dispatcher = super::persist_dispatcher::PersistDispatcher::new(&blocking_tasks);
        let mut runtime = AppRuntime {
            tabs,
            active_tab_id: active_tab_id.map(str::to_owned),
            recent_projects: Vec::new(),
            profile_selections: HashMap::new(),
            profile_config_path: Some(temp_root.join("profile-config.toml")),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            board_all_view_windows: std::collections::HashSet::new(),
            session_state_path: temp_root.join("session-state.json"),
            log_dir,
            proxy,
            blocking_tasks,
            sessions_dir,
            launch_wizard_cache,
            launch_wizard: None,
            pending_workspace_resume_contexts: HashMap::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::<String, ActiveAgentSession>::new(),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_cache_dir(),
            pending_update: None,
            pty_writers,
            persist_dispatcher,
            file_tree_worktree_roots: HashMap::new(),
            server_url: None,
        };
        runtime.rebuild_window_lookup();
        runtime.seed_window_pty_statuses();
        (runtime, _events)
    }

    fn wait_for_recorded_event(
        label: &str,
        events: &Arc<Mutex<Vec<UserEvent>>>,
        predicate: impl Fn(&[UserEvent]) -> bool,
    ) {
        for _ in 0..800 {
            {
                let events = events.lock().expect("event log");
                if predicate(&events) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let snapshot = events.lock().expect("event log").clone();
        panic!("timed out waiting for {label}: {snapshot:?}");
    }

    fn wait_for_path(label: &str, path: &Path) {
        for _ in 0..800 {
            if path.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for {label}: {}", path.display());
    }

    #[test]
    fn project_index_bootstrap_runs_in_background_without_blocking_launch() {
        let temp = tempdir().expect("tempdir");
        let (proxy, events) = AppEventProxy::stub();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let spawn_started = Instant::now();
        let service = crate::project_index_bootstrap::ProjectIndexBootstrapService::new_for_test();

        let spawned = service.spawn_with(
            proxy,
            temp.path().to_path_buf(),
            move |_project_root| {
                started_tx.send(()).expect("signal bootstrap start");
                release_rx
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release bootstrap");
                Ok(())
            },
            |_project_root| {
                gwt::ProjectIndexStatusView::new(
                    gwt::ProjectIndexStatusState::Ready,
                    "test bootstrap complete",
                )
            },
        );
        let spawn_elapsed = spawn_started.elapsed();

        assert_eq!(
            spawned,
            crate::project_index_bootstrap::ProjectIndexBootstrapRequest::Spawned
        );
        assert!(
            spawn_elapsed < Duration::from_millis(250),
            "spawning bootstrap must return before the slow bootstrap body completes"
        );
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("background bootstrap should start promptly");
        assert!(
            events.lock().expect("event log").is_empty(),
            "no status should be emitted before the slow bootstrap completes"
        );

        release_tx.send(()).expect("release bootstrap");
        wait_for_recorded_event("project index status", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::ProjectIndexStatus {
                        project_root,
                        status,
                    } if project_root == &dunce::canonicalize(temp.path())
                            .unwrap_or_else(|_| temp.path().to_path_buf())
                            .display()
                            .to_string()
                        && status.state == gwt::ProjectIndexStatusState::Ready
                        && status.detail == "test bootstrap complete"
                )
            })
        });
    }

    #[test]
    fn agent_launch_success_dispatches_launch_complete_before_project_index_status() {
        let temp = tempdir().expect("tempdir");
        let (proxy, events) = AppEventProxy::stub();
        let completion: AgentLaunchCompletion = (
            ProcessLaunch {
                command: "agent".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(temp.path().to_path_buf()),
            },
            "session-1".to_string(),
            "feature/test".to_string(),
            "Codex".to_string(),
            temp.path().to_path_buf(),
            gwt_agent::AgentId::Codex,
            None,
            None,
            gwt_agent::LaunchRuntimeTarget::Host,
            temp.path().display().to_string(),
        );

        dispatch_agent_launch_success(
            proxy,
            "tab-1::agent-1".to_string(),
            completion,
            |proxy, project_root| {
                proxy.send(UserEvent::ProjectIndexStatus {
                    project_root: project_root.display().to_string(),
                    status: gwt::ProjectIndexStatusView::new(
                        gwt::ProjectIndexStatusState::Ready,
                        "ready",
                    ),
                });
            },
        );

        let recorded = events.lock().expect("events");
        assert!(
            matches!(recorded.first(), Some(UserEvent::LaunchComplete { .. })),
            "LaunchComplete must be emitted first"
        );
        assert!(
            matches!(
                recorded.get(1),
                Some(UserEvent::ProjectIndexStatus {
                    project_root,
                    status,
                }) if project_root == &temp.path().display().to_string()
                    && status.state == gwt::ProjectIndexStatusState::Ready
            ),
            "ProjectIndexStatus must follow LaunchComplete and carry project root"
        );
    }

    fn sample_launch_wizard_session(tab_id: &str, project_root: &Path) -> LaunchWizardSession {
        LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: "wizard-1".to_string(),
            wizard: LaunchWizardState::open_loading(
                LaunchWizardContext {
                    selected_branch: BranchListEntry {
                        name: "feature/demo".to_string(),
                        scope: BranchScope::Local,
                        is_head: false,
                        upstream: None,
                        ahead: 0,
                        behind: 0,
                        last_commit_date: None,
                        cleanup_ready: true,
                        cleanup: BranchCleanupInfo::default(),
                        resume: gwt::BranchResumeInfo::unavailable(),
                    },
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: None,
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                    linked_issue_kind: None,
                },
                Vec::new(),
            ),
            workspace_resume_context: None,
        }
    }

    fn sample_agent_options() -> Vec<gwt::AgentOption> {
        vec![gwt::AgentOption {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: true,
            installed_version: Some("latest".to_string()),
            versions: vec!["latest".to_string()],
            custom_agent: None,
        }]
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let status = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn run_git_with_paths(args: &[&str], paths: &[&Path]) {
        let mut command = gwt_core::process::hidden_command("git");
        command.args(args);
        for path in paths {
            command.arg(path);
        }
        let status = command.status().expect("run git with paths");
        assert!(
            status.success(),
            "git {args:?} {paths:?} failed with {status}"
        );
    }

    fn init_git_clone_with_origin(repo: &Path) -> PathBuf {
        let root = repo.parent().expect("repo parent");
        let seed = root.join("seed");
        let origin = root.join("origin.git");
        fs::create_dir_all(&seed).expect("create seed");
        run_git(&seed, &["init", "-q", "-b", "develop"]);
        run_git(&seed, &["config", "user.name", "Codex"]);
        run_git(&seed, &["config", "user.email", "codex@example.com"]);
        fs::write(seed.join("README.md"), "repo\n").expect("seed readme");
        run_git(&seed, &["add", "README.md"]);
        run_git(&seed, &["commit", "-qm", "init"]);
        run_git_with_paths(&["clone", "--bare"], &[&seed, &origin]);
        run_git_with_paths(&["clone"], &[&origin, repo]);
        run_git(repo, &["config", "user.name", "Codex"]);
        run_git(repo, &["config", "user.email", "codex@example.com"]);
        run_git(repo, &["remote", "set-head", "origin", "-a"]);
        origin
    }

    fn append_workspace_resume_journal(
        repo: &Path,
        journal_id: &str,
        project_root: PathBuf,
        owner: &str,
        summary: &str,
    ) {
        let path = gwt_core::paths::gwt_workspace_journal_path_for_repo_path(repo);
        let entry = gwt_core::workspace_projection::WorkspaceJournalEntry {
            id: journal_id.to_string(),
            project_root,
            title: Some("Suspended review".to_string()),
            status_category: Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Idle),
            status_text: Some("Suspended".to_string()),
            owner: Some(owner.to_string()),
            next_action: Some("Resume the review".to_string()),
            summary: Some(summary.to_string()),
            agent_session_id: None,
            agent_current_focus: None,
            agent_title_summary: Some("Suspended review".to_string()),
            updated_at: chrono::Utc::now(),
        };
        gwt_core::workspace_projection::append_workspace_journal_entry_to_path(&path, &entry)
            .expect("append journal");
    }

    fn sample_no_agent_launch_wizard_session(
        tab_id: &str,
        project_root: &Path,
    ) -> LaunchWizardSession {
        LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: "wizard-unavailable-agent".to_string(),
            wizard: LaunchWizardState::open_with(
                LaunchWizardContext {
                    selected_branch: BranchListEntry {
                        name: "feature/demo".to_string(),
                        scope: BranchScope::Local,
                        is_head: false,
                        upstream: None,
                        ahead: 0,
                        behind: 0,
                        last_commit_date: None,
                        cleanup_ready: true,
                        cleanup: BranchCleanupInfo::default(),
                        resume: gwt::BranchResumeInfo::unavailable(),
                    },
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: Some(project_root.to_path_buf()),
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                    linked_issue_kind: None,
                },
                Vec::new(),
                Vec::new(),
            ),
            workspace_resume_context: None,
        }
    }

    fn sample_ready_agent_launch_wizard_session(
        tab_id: &str,
        project_root: &Path,
    ) -> LaunchWizardSession {
        LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: "wizard-ready-agent".to_string(),
            wizard: LaunchWizardState::open_with(
                LaunchWizardContext {
                    selected_branch: BranchListEntry {
                        name: "feature/demo".to_string(),
                        scope: BranchScope::Local,
                        is_head: false,
                        upstream: None,
                        ahead: 0,
                        behind: 0,
                        last_commit_date: None,
                        cleanup_ready: true,
                        cleanup: BranchCleanupInfo::default(),
                        resume: gwt::BranchResumeInfo::unavailable(),
                    },
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: Some(project_root.to_path_buf()),
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                    linked_issue_kind: None,
                },
                sample_agent_options(),
                Vec::new(),
            ),
            workspace_resume_context: None,
        }
    }

    #[test]
    fn app_runtime_frontend_ready_replies_only_to_requesting_client_and_starts_with_workspace() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");
        runtime
            .window_details
            .insert(window_id.clone(), "Shell ready".to_string());
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        runtime.pending_update = Some(gwt_core::update::UpdateState::UpToDate { checked_at: None });

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

        assert!(matches!(
            events.first(),
            Some(event)
                if matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "client-1")
                    && matches!(event.event, BackendEvent::WorkspaceState { .. })
        ));
        assert!(events.iter().all(|event| matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id
                    && *status == WindowProcessStatus::Ready
                    && detail.as_deref() == Some("Shell ready")
        )));
        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::LaunchWizardState { wizard: Some(_) }
        )));
        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::UpdateState(gwt_core::update::UpdateState::UpToDate { .. })
        )));
    }

    #[test]
    fn app_runtime_apply_update_uses_pending_available_update_state() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::Git,
            &[WindowPreset::Shell],
        );
        let (mut runtime, events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
            current: "9.20.1".to_string(),
            latest: "9.20.2".to_string(),
            release_url: "https://example.invalid/releases/v9.20.2".to_string(),
            asset_url: Some("https://example.invalid/gwt-macos-universal.dmg".to_string()),
            checked_at: chrono::Utc::now(),
        });

        let outbound =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

        assert!(outbound.is_empty(), "apply worker dispatch is internal");
        wait_for_recorded_event("pending update apply", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::ApplyUpdate {
                        client_id,
                        state: gwt_core::update::UpdateState::Available {
                            latest,
                            asset_url: Some(asset_url),
                            ..
                        },
                    } if client_id == "client-1"
                        && latest == "9.20.2"
                        && asset_url == "https://example.invalid/gwt-macos-universal.dmg"
                )
            })
        });
    }

    #[test]
    fn app_runtime_apply_update_without_applicable_pending_update_reports_error() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::Git,
            &[WindowPreset::Shell],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.pending_update = Some(gwt_core::update::UpdateState::Available {
            current: "9.20.1".to_string(),
            latest: "9.20.2".to_string(),
            release_url: "https://example.invalid/releases/v9.20.2".to_string(),
            asset_url: None,
            checked_at: chrono::Utc::now(),
        });

        let outbound =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::ApplyUpdate);

        assert!(outbound.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::UpdateApplyError { message: Some(message), .. }
                    if message.contains("No applicable update asset")
            )
        }));
    }

    #[test]
    fn app_runtime_frontend_ready_replays_terminal_snapshot_only_to_requesting_client() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };
        let mut pane = Pane::new(
            window_id.clone(),
            command,
            args,
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("pane");
        pane.process_bytes(b"hello from frontend ready\n");
        runtime.runtimes.insert(
            window_id.clone(),
            WindowRuntime {
                pane: Arc::new(Mutex::new(pane)),
                output_thread: None,
                status_thread: None,
            },
        );

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

        assert!(events.iter().all(|event| matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        )));
        let snapshot = events.iter().find_map(|event| match &event.event {
            BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => {
                Some(data_base64)
            }
            _ => None,
        });
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(snapshot.expect("terminal snapshot event"))
            .expect("decode terminal snapshot");
        assert!(String::from_utf8_lossy(&decoded).contains("hello from frontend ready"));
    }

    #[test]
    fn app_runtime_frontend_ready_replays_terminal_snapshot_with_sgr_attributes() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };
        let mut pane = Pane::new(
            window_id.clone(),
            command,
            args,
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("pane");
        // Write red foreground + bold "ALERT" then reset, then default-color text.
        pane.process_bytes(b"\x1b[31;1mALERT\x1b[0m normal\n");

        runtime.runtimes.insert(
            window_id.clone(),
            WindowRuntime {
                pane: Arc::new(Mutex::new(pane)),
                output_thread: None,
                status_thread: None,
            },
        );

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

        let snapshot = events.iter().find_map(|event| match &event.event {
            BackendEvent::TerminalSnapshot { id, data_base64 } if id == &window_id => {
                Some(data_base64)
            }
            _ => None,
        });
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(snapshot.expect("terminal snapshot event"))
            .expect("decode terminal snapshot");
        // Visible text must be present.
        assert!(
            String::from_utf8_lossy(&decoded).contains("ALERT"),
            "expected ALERT text in snapshot bytes, got: {:?}",
            String::from_utf8_lossy(&decoded)
        );
        // SGR escape sequence introducing a styled run (CSI ... m) must be present
        // so that xterm.js can replay foreground / bold / etc. from the snapshot.
        let has_sgr = decoded.windows(2).enumerate().any(|(idx, win)| {
            win == [0x1b, b'['] && {
                let tail = &decoded[idx + 2..];
                tail.iter().take(16).any(|b| *b == b'm')
            }
        });
        assert!(
            has_sgr,
            "expected SGR escape (CSI ... m) in TerminalSnapshot bytes so xterm.js can replay color/style; raw snapshot bytes: {:?}",
            decoded
        );
    }

    #[test]
    fn app_runtime_dock_window_tab_resizes_group_runtimes() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().to_path_buf(),
            ProjectKind::Git,
            &[WindowPreset::Shell, WindowPreset::Claude],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let shell_id = combined_window_id("tab-1", "shell-1");
        let claude_id = combined_window_id("tab-1", "claude-1");
        for window_id in [&shell_id, &claude_id] {
            let pane = Pane::new(
                window_id.clone(),
                if cfg!(windows) { "cmd" } else { "/bin/sh" }.to_string(),
                if cfg!(windows) {
                    vec![
                        "/d".to_string(),
                        "/s".to_string(),
                        "/c".to_string(),
                        "exit /b 0".to_string(),
                    ]
                } else {
                    vec!["-lc".to_string(), "exit 0".to_string()]
                },
                80,
                24,
                HashMap::new(),
                None,
            )
            .expect("pane");
            runtime.runtimes.insert(
                window_id.clone(),
                WindowRuntime {
                    pane: Arc::new(Mutex::new(pane)),
                    output_thread: None,
                    status_thread: None,
                },
            );
        }

        let target_geometry = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window("claude-1")
            .expect("claude")
            .geometry
            .clone();
        let (expected_cols, expected_rows) = geometry_to_pty_size(&target_geometry);

        let events = runtime.dock_window_tab_events(&shell_id, &claude_id);

        assert_eq!(events.len(), 1);
        for window_id in [&shell_id, &claude_id] {
            let pane = runtime
                .runtimes
                .get(window_id)
                .expect("runtime")
                .pane
                .lock()
                .expect("pane");
            assert_eq!(pane.screen().size(), (expected_rows, expected_cols));
        }
    }

    #[test]
    fn app_runtime_frontend_ready_replays_active_work_projection_separately_from_workspace() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.active_agent_sessions.insert(
            "tab-1::agent-1".to_string(),
            ActiveAgentSession {
                window_id: "tab-1::agent-1".to_string(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260504-1234".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.join("../repo-work-20260504-1234"),
                agent_project_root: repo
                    .join("../repo-work-20260504-1234")
                    .display()
                    .to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::FrontendReady);

        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::WorkspaceState { .. })
        ));
        let projection = events.iter().find_map(|event| match &event.event {
            BackendEvent::ActiveWorkProjection { projection } => Some(projection),
            _ => None,
        });
        let projection = projection.expect("active work projection event");
        assert_eq!(projection.status_category, "active");
        assert_eq!(projection.active_agents, 1);
        assert_eq!(projection.branch.as_deref(), Some("work/20260504-1234"));
        assert_eq!(projection.agents.len(), 1);
        assert_eq!(projection.agents[0].session_id, "session-1");
        assert_eq!(projection.agents[0].display_name, "Codex");
        assert_eq!(projection.agents[0].status_category, "active");
        assert_eq!(
            projection.agents[0].branch.as_deref(),
            Some("work/20260504-1234")
        );
        assert!(
            events.iter().all(|event| matches!(
                &event.target,
                DispatchTarget::Client(client_id) if client_id == "client-1"
            )),
            "frontend-ready projection replay must remain client-scoped"
        );
    }

    #[test]
    fn app_runtime_select_project_tab_broadcasts_workspace_before_clearing_wizard() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let other = temp.path().join("other");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&other).expect("create other");
        let tabs = vec![
            sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::NonRepo,
                &[WindowPreset::Branches],
            ),
            sample_project_tab(
                "tab-2",
                "Other",
                other,
                ProjectKind::NonRepo,
                &[WindowPreset::FileTree],
            ),
        ];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

        let events = runtime.select_project_tab_events("tab-2");

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(matches!(events[1].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[1].event,
            BackendEvent::ActiveWorkProjection { .. }
        ));
        assert!(matches!(events[2].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[2].event,
            BackendEvent::LaunchWizardState { wizard: None }
        ));
    }

    #[test]
    fn app_runtime_select_project_tab_emits_active_work_projection_for_new_active_tab() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let other = temp.path().join("other");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&other).expect("create other");
        let tabs = vec![
            sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
            sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
        ];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

        let events = runtime.select_project_tab_events("tab-2");

        let projection = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::ActiveWorkProjection { projection } => Some(projection),
                _ => None,
            })
            .expect("ActiveWorkProjection broadcast for newly selected tab");
        assert_eq!(
            projection.id, "tab-2",
            "projection must reflect the newly active tab"
        );
    }

    #[test]
    fn app_runtime_close_project_tab_emits_active_work_projection_when_active_changes() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let other = temp.path().join("other");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&other).expect("create other");
        let tabs = vec![
            sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]),
            sample_project_tab("tab-2", "Other", other, ProjectKind::NonRepo, &[]),
        ];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

        let events = runtime.close_project_tab_events("tab-1");

        let projection = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::ActiveWorkProjection { projection } => Some(projection),
                _ => None,
            })
            .expect("ActiveWorkProjection broadcast after closing the active tab");
        assert_eq!(
            projection.id, "tab-2",
            "projection must reflect the new active tab after close"
        );
    }

    #[test]
    fn app_runtime_open_project_path_emits_active_work_projection_for_new_tab() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tabs = vec![sample_project_tab(
            "tab-1",
            "Repo",
            repo,
            ProjectKind::NonRepo,
            &[],
        )];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));

        let other = temp.path().join("other-project");
        fs::create_dir_all(&other).expect("create other");

        let events = runtime.open_project_path_events(other.clone());

        assert!(
            events
                .iter()
                .any(|event| matches!(&event.event, BackendEvent::ActiveWorkProjection { .. })),
            "opening a new project must emit ActiveWorkProjection for the new active tab"
        );
    }

    #[test]
    fn app_runtime_runtime_status_uses_lightweight_events_for_non_structural_status() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");

        let events = runtime.handle_runtime_status(
            window_id.clone(),
            WindowProcessStatus::Error,
            Some("boom".to_string()),
        );

        assert_eq!(events.len(), 2);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "non-structural runtime status changes must not force a full workspace_state"
        );
        assert!(matches!(events[0].target, DispatchTarget::Broadcast));
        assert!(matches!(
            &events[0].event,
            BackendEvent::WindowState { window_id: id, state }
                if id == &window_id && *state == WindowProcessStatus::Error
        ));
        assert!(matches!(events[1].target, DispatchTarget::Broadcast));
        assert!(matches!(
            &events[1].event,
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id
                    && *status == WindowProcessStatus::Error
                    && detail.as_deref() == Some("boom")
        ));
    }

    #[test]
    fn app_runtime_open_launch_wizard_uses_cached_previous_profile_without_hydrating() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let mut session = gwt_agent::Session::new(&repo, "feature/demo", gwt_agent::AgentId::Codex);
        session.model = Some("gpt-5.4".to_string());
        session.reasoning_level = Some("high".to_string());
        session.tool_version = Some("latest".to_string());
        session.session_mode = gwt_agent::SessionMode::Continue;
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.save(&sessions_dir).expect("save session");

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime
            .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None, None)
            .expect("open launch wizard");

        let view = runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .view();
        assert!(!view.is_hydrating);
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.4");
        assert_eq!(view.selected_reasoning, "high");
        assert_eq!(view.selected_version, "latest");
        assert_eq!(view.selected_execution_mode, "continue");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);
    }

    #[test]
    fn app_runtime_open_launch_wizard_does_not_probe_branch_worktree_for_docker_context() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        run_git(&repo, &["config", "user.name", "Codex"]);
        run_git(&repo, &["config", "user.email", "codex@example.com"]);
        fs::write(repo.join("README.md"), "repo\n").expect("readme");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-qm", "init"]);
        run_git(&repo, &["branch", "feature/docker"]);

        let branch_worktree = temp.path().join("repo-feature-docker");
        let branch_worktree_arg = branch_worktree.to_string_lossy().to_string();
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                &branch_worktree_arg,
                "feature/docker",
            ],
        );
        fs::write(
            branch_worktree.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.20\n",
        )
        .expect("compose");

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime
            .open_launch_wizard_for_branch("tab-1", &repo, "feature/docker", None, None)
            .expect("open launch wizard");

        let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
        assert!(wizard.context.worktree_path.is_none());
        assert!(same_worktree_path(&wizard.context.quick_start_root, &repo));
        let view = wizard.view();
        assert!(!view.runtime_context_resolved);
        assert!(!view.show_runtime_target);
        assert!(view.selected_docker_service.is_none());
    }

    #[test]
    fn app_runtime_launch_wizard_continue_resolves_runtime_context_from_worktree() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        run_git(&repo, &["config", "user.name", "Codex"]);
        run_git(&repo, &["config", "user.email", "codex@example.com"]);
        fs::write(repo.join("README.md"), "repo\n").expect("readme");
        fs::write(
            repo.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.20\n",
        )
        .expect("compose");
        run_git(&repo, &["add", "README.md", "docker-compose.yml"]);
        run_git(&repo, &["commit", "-qm", "init"]);

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

        runtime
            .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
            .expect("open launch wizard");
        assert!(
            !runtime
                .launch_wizard
                .as_ref()
                .expect("wizard")
                .wizard
                .view()
                .runtime_context_resolved
        );

        let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event,
            BackendEvent::LaunchWizardState { wizard: Some(_) }
        ));
        let pending_view = runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view();
        assert!(pending_view.runtime_resolution_pending);
        assert!(!pending_view.runtime_context_resolved);
        assert_eq!(pending_view.primary_action_label, "Preparing...");

        wait_for_recorded_event(
            "launch wizard runtime resolution",
            &recorded_events,
            |events| {
                events
                    .iter()
                    .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            },
        );
        let resolved_event = {
            let mut events = recorded_events.lock().expect("event log");
            events
                .iter()
                .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
                .map(|index| events.remove(index))
                .expect("runtime resolved event")
        };
        let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
            unreachable!("matched above")
        };
        let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
        assert_eq!(resolved_events.len(), 1);

        let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
        assert!(wizard
            .context
            .worktree_path
            .as_ref()
            .is_some_and(|path| same_worktree_path(path, &repo)));
        let view = wizard.view();
        assert!(!view.runtime_resolution_pending);
        assert!(view.runtime_context_resolved);
        assert!(view.show_runtime_target);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
    }

    #[test]
    fn app_runtime_launch_wizard_continue_does_not_materialize_missing_worktree() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let _origin = init_git_clone_with_origin(&repo);
        fs::write(
            repo.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.20\n",
        )
        .expect("compose");
        run_git(&repo, &["add", "docker-compose.yml"]);
        run_git(&repo, &["commit", "-qm", "add compose"]);
        run_git(&repo, &["push", "origin", "develop"]);

        let branch_name = "work/runtime-deferral";
        let expected_worktree = gwt_git::worktree::sibling_worktree_path(&repo, branch_name);
        assert!(
            !expected_worktree.exists(),
            "fixture branch worktree should start absent"
        );

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

        runtime
            .open_launch_wizard_for_branch("tab-1", &repo, branch_name, None, None)
            .expect("open launch wizard");

        let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
        assert_eq!(events.len(), 1);
        let pending_view = runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view();
        assert!(pending_view.runtime_resolution_pending);
        assert_eq!(
            pending_view.runtime_resolution_message.as_deref(),
            Some("Preparing runtime context...")
        );

        wait_for_recorded_event(
            "launch wizard runtime deferral",
            &recorded_events,
            |events| {
                events
                    .iter()
                    .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
            },
        );
        let resolved_event = {
            let mut events = recorded_events.lock().expect("event log");
            events
                .iter()
                .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
                .map(|index| events.remove(index))
                .expect("runtime resolved event")
        };
        let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
            unreachable!("matched above")
        };
        let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
        assert_eq!(resolved_events.len(), 1);

        let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
        assert!(
            wizard.context.worktree_path.is_none(),
            "runtime confirmation should not resolve a newly-created target worktree"
        );
        assert!(
            !expected_worktree.exists(),
            "Runtime confirmation must not create {expected_worktree:?}"
        );
        let view = wizard.view();
        assert!(!view.runtime_resolution_pending);
        assert!(view.runtime_context_resolved);
        assert!(view.show_runtime_target);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
    }

    #[test]
    fn app_runtime_launch_wizard_continue_falls_back_to_host_without_resolved_docker_context() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        run_git(&repo, &["init", "-q", "-b", "develop"]);
        run_git(&repo, &["config", "user.name", "Codex"]);
        run_git(&repo, &["config", "user.email", "codex@example.com"]);
        fs::write(repo.join("README.md"), "repo\n").expect("readme");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-qm", "init"]);

        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let mut session = gwt_agent::Session::new(&repo, "develop", gwt_agent::AgentId::Codex);
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("app".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.save(&sessions_dir).expect("save session");

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

        runtime
            .open_launch_wizard_for_branch("tab-1", &repo, "develop", None, None)
            .expect("open launch wizard");
        let phase_one = runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view();
        assert!(!phase_one.runtime_context_resolved);
        assert_eq!(phase_one.selected_runtime_target, "host");
        assert!(!phase_one.show_runtime_target);

        let events = runtime.handle_launch_wizard_action(LaunchWizardAction::Submit, None);
        assert_eq!(events.len(), 1);
        let pending_view = runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view();
        assert!(pending_view.runtime_resolution_pending);
        assert!(!pending_view.runtime_context_resolved);

        wait_for_recorded_event("launch wizard host fallback", &recorded_events, |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
        });
        let resolved_event = {
            let mut events = recorded_events.lock().expect("event log");
            events
                .iter()
                .position(|event| matches!(event, UserEvent::LaunchWizardRuntimeResolved { .. }))
                .map(|index| events.remove(index))
                .expect("runtime resolved event")
        };
        let UserEvent::LaunchWizardRuntimeResolved { wizard_id, result } = resolved_event else {
            unreachable!("matched above")
        };
        let resolved_events = runtime.handle_launch_wizard_runtime_resolved(wizard_id, *result);
        assert_eq!(resolved_events.len(), 1);

        let view = runtime
            .launch_wizard
            .as_ref()
            .expect("wizard")
            .wizard
            .view();
        assert!(!view.runtime_resolution_pending);
        assert!(view.runtime_context_resolved);
        assert_eq!(view.selected_runtime_target, "host");
        assert!(!view.show_runtime_target);
        assert!(view.selected_docker_service.is_none());
    }

    #[test]
    fn app_runtime_workspace_add_agent_opens_branch_launch_without_branches_window() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.active_agent_sessions.insert(
            "tab-1::agent-1".to_string(),
            ActiveAgentSession {
                window_id: "tab-1::agent-1".to_string(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260504-1234".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.join("../repo-work-20260504-1234"),
                agent_project_root: repo
                    .join("../repo-work-20260504-1234")
                    .display()
                    .to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::OpenActiveWorkLaunchWizard {
                branch_name: "work/20260504-1234".to_string(),
                linked_issue_number: None,
            },
        );

        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardState { wizard: Some(_) })
        ));
        let view = runtime
            .launch_wizard
            .as_ref()
            .expect("active work launch wizard")
            .wizard
            .view();
        assert_eq!(view.mode, gwt::LaunchWizardMode::Branch);
        assert_eq!(view.branch_name, "work/20260504-1234");
        assert!(view.show_start_methods);
        assert!(!view.show_branch_controls);
        assert_eq!(view.live_sessions.len(), 1);
        assert_eq!(view.live_sessions[0].name, "Codex");

        let _ = runtime.handle_launch_wizard_action(
            gwt::LaunchWizardAction::UseStartMethod {
                method: gwt::LaunchWizardStartMethodKind::ConfigureAndStart,
            },
            None,
        );
        let configured_view = runtime
            .launch_wizard
            .as_ref()
            .expect("configured active work launch wizard")
            .wizard
            .view();
        assert!(!configured_view.show_start_methods);
        assert!(configured_view.show_branch_controls);
    }

    #[test]
    fn app_runtime_live_sessions_report_composed_idle_runtime_status() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260504-1234".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: PathBuf::from("E:/gwt/test-repo"),
                agent_project_root: "E:/gwt/test-repo".to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        runtime.handle_runtime_hook_event(runtime_hook_state("Idle", "session-1"));
        let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
    }

    #[test]
    fn app_runtime_live_sessions_report_idle_after_launch_before_first_hook() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260504-1234".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: PathBuf::from("E:/gwt/test-repo"),
                agent_project_root: "E:/gwt/test-repo".to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);

        runtime.handle_runtime_hook_event(runtime_hook_state_for_event(
            "Idle",
            "SessionStart",
            "session-1",
        ));
        let sessions = runtime.live_sessions_for_branch("tab-1", "work/20260504-1234");

        assert_eq!(sessions[0].runtime_status, WindowProcessStatus::Idle);
    }

    #[test]
    fn app_runtime_workspace_state_reports_idle_for_launched_agent_without_hook_state() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260504-1234".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: PathBuf::from("E:/gwt/test-repo"),
                agent_project_root: "E:/gwt/test-repo".to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let view = runtime.app_state_view();
        let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
        let window = tab
            .workspace
            .windows
            .iter()
            .find(|window| window.id == "tab-1::agent-1")
            .unwrap();

        assert_eq!(window.status, WindowProcessStatus::Idle);
    }

    #[test]
    fn app_runtime_workspace_state_normalizes_pre_lifecycle_agent_windows() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime.app_state_view();
        let tab = view.tabs.iter().find(|tab| tab.id == "tab-1").unwrap();
        let window = tab
            .workspace
            .windows
            .iter()
            .find(|window| window.id == "tab-1::agent-1")
            .unwrap();

        assert_eq!(window.status, WindowProcessStatus::NotStarted);
        assert_eq!(tab.running_agent_count, 0);
        assert!(tab.running_agents.is_empty());
    }

    #[test]
    fn app_runtime_window_list_normalizes_pre_lifecycle_agent_windows() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let BackendEvent::WindowList { windows } = runtime.list_windows_event() else {
            panic!("expected window list");
        };
        let window = windows
            .iter()
            .find(|window| window.id == "tab-1::agent-1")
            .unwrap();

        assert_eq!(window.status, WindowProcessStatus::NotStarted);
    }

    #[test]
    fn app_runtime_open_start_work_ensures_remote_develop_without_creating_work_branch() {
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let origin = init_git_clone_with_origin(&repo);
        run_git(&repo, &["checkout", "-qb", "main"]);
        run_git(&repo, &["push", "origin", "main"]);
        run_git(&origin, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        run_git(&repo, &["checkout", "develop"]);
        run_git(&repo, &["remote", "set-head", "origin", "-a"]);
        run_git(&origin, &["branch", "-D", "develop"]);

        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::OpenStartWork);

        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardState { wizard: Some(_) })
        ));
        let view = runtime
            .launch_wizard
            .as_ref()
            .expect("start work wizard")
            .wizard
            .view();
        assert_eq!(view.mode, gwt::LaunchWizardMode::StartWork);
        assert_eq!(view.title, "Start Work");
        assert!(!view.show_branch_controls);
        assert_eq!(view.selected_branch_name, "origin/develop");
        assert!(view.branch_name.starts_with("work/"));

        let develop = gwt_core::process::hidden_command("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/develop",
            ])
            .current_dir(&repo)
            .status()
            .expect("check origin/develop");
        assert!(
            develop.success(),
            "opening Start Work should restore origin/develop from the remote default branch"
        );

        let refs = gwt_core::process::hidden_command("git")
            .args([
                "for-each-ref",
                "refs/heads/work",
                "refs/remotes/origin/work",
            ])
            .current_dir(&repo)
            .output()
            .expect("list work refs");
        assert!(refs.status.success(), "git for-each-ref failed");
        assert!(
            refs.stdout.is_empty(),
            "opening Start Work must not create branch refs"
        );

        let cancel_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LaunchWizardAction {
                action: LaunchWizardAction::Cancel,
                bounds: None,
            },
        );
        assert!(matches!(
            cancel_events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardState { wizard: None })
        ));
        let refs_after_cancel = gwt_core::process::hidden_command("git")
            .args([
                "for-each-ref",
                "refs/heads/work",
                "refs/remotes/origin/work",
            ])
            .current_dir(&repo)
            .output()
            .expect("list work refs after cancel");
        assert!(refs_after_cancel.status.success());
        assert!(
            refs_after_cancel.stdout.is_empty(),
            "cancelling Start Work must not create branch refs"
        );
    }

    #[test]
    fn app_runtime_open_start_work_failure_surfaces_launch_wizard_open_error() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo,
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events =
            runtime.handle_frontend_event("client-1".to_string(), FrontendEvent::OpenStartWork);

        assert!(runtime.launch_wizard.is_none());
        assert!(matches!(
            events.first().map(|event| &event.target),
            Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
        ));
        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardOpenError { title, message })
                if title == "Start Work" && !message.is_empty()
        ));
    }

    #[test]
    fn app_runtime_open_launch_wizard_failure_surfaces_launch_wizard_open_error() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            temp.path().join("repo"),
            ProjectKind::Git,
            &[WindowPreset::Branches],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::OpenLaunchWizard {
                id: "missing-window".to_string(),
                branch_name: "main".to_string(),
                linked_issue_number: None,
            },
        );

        assert!(runtime.launch_wizard.is_none());
        assert!(matches!(
            events.first().map(|event| &event.target),
            Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
        ));
        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardOpenError { title, message })
                if title == "Launch Agent" && message == "Window not found"
        ));
    }

    #[test]
    fn app_runtime_resume_workspace_failure_surfaces_launch_wizard_open_error() {
        // SPEC-2359 / Issue #2757: Resume クリックで `resume_workspace_events`
        // が早期 return / Start Work fallback 失敗を起こした場合、frontend で
        // 可視な `LaunchWizardOpenError` を return しなければならない。
        // 旧経路は `ProjectOpenError` を broadcast していたが、project 開放中は
        // `renderProjectPicker` が hidden なので silent failure になっていた。
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo,
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Current,
                journal_id: None,
            },
        );

        assert!(runtime.launch_wizard.is_none());
        assert!(
            matches!(
                events.first().map(|event| &event.target),
                Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
            ),
            "Resume failure must be replied to the originating client, not broadcast as ProjectOpenError"
        );
        assert!(
            matches!(
                events.first().map(|event| &event.event),
                Some(BackendEvent::LaunchWizardOpenError { title, message })
                    if title == "Resume Workspace" && !message.is_empty()
            ),
            "Resume failure must surface as LaunchWizardOpenError so Workspace Overview can render a visible overlay"
        );
    }

    #[test]
    fn app_runtime_resume_workspace_without_active_tab_returns_launch_wizard_open_error() {
        // Same contract for the `Open a project before resuming work` early return.
        let temp = tempdir().expect("tempdir");
        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Current,
                journal_id: None,
            },
        );

        assert!(matches!(
            events.first().map(|event| &event.target),
            Some(DispatchTarget::Client(client_id)) if client_id == "client-1"
        ));
        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::LaunchWizardOpenError { title, message })
                if title == "Resume Workspace"
                    && message == "Open a project before resuming work"
        ));
    }

    #[test]
    fn app_runtime_custom_agent_cache_refresh_rebroadcasts_open_wizard_state() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

        let events = runtime.custom_agent_reply_with_cache_refresh(
            "client-1".to_string(),
            BackendEvent::CustomAgentDeleted {
                agent_id: "custom-agent".to_string(),
            },
        );

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].event,
            BackendEvent::CustomAgentDeleted { .. }
        ));
        assert!(matches!(
            events[1].event,
            BackendEvent::LaunchWizardState { wizard: Some(_) }
        ));
    }

    #[test]
    fn app_runtime_launch_wizard_submit_failure_emits_structured_error_log() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[WindowPreset::Branches],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

        let events = capture_tracing_events(|| {
            let _ = runtime
                .handle_launch_wizard_action(LaunchWizardAction::Submit, Some(canvas_bounds()));
        });

        let event = events
            .iter()
            .find(|event| {
                event.level == Level::ERROR
                    && event.target == "gwt::agent_launch"
                    && event.fields.get("stage").map(String::as_str) == Some("wizard_submit")
            })
            .expect("launch wizard submit failure log");
        assert_eq!(
            event.fields.get("wizard_id").map(String::as_str),
            Some("wizard-unavailable-agent")
        );
        assert_eq!(
            event.fields.get("tab_id").map(String::as_str),
            Some("tab-1")
        );
        assert_eq!(
            event.fields.get("selected_agent_id").map(String::as_str),
            Some("")
        );
        assert_eq!(
            event.fields.get("requested_agent_id").map(String::as_str),
            Some("none")
        );
        assert_eq!(
            event
                .fields
                .get("selected_launch_target")
                .map(String::as_str),
            Some("agent")
        );
        assert_eq!(
            event.fields.get("error").map(String::as_str),
            Some("Agent option is unavailable")
        );
    }

    #[test]
    fn app_runtime_launch_wizard_set_agent_failure_logs_requested_agent() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[WindowPreset::Branches],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.launch_wizard = Some(sample_no_agent_launch_wizard_session("tab-1", &repo));

        let events = capture_tracing_events(|| {
            let _ = runtime.handle_launch_wizard_action(
                LaunchWizardAction::SetAgent {
                    agent_id: "codex".to_string(),
                },
                Some(canvas_bounds()),
            );
        });

        let event = events
            .iter()
            .find(|event| {
                event.level == Level::ERROR
                    && event.target == "gwt::agent_launch"
                    && event.fields.get("stage").map(String::as_str) == Some("agent_select")
            })
            .expect("set agent failure log");
        assert_eq!(
            event.fields.get("requested_agent_id").map(String::as_str),
            Some("codex")
        );
        assert_eq!(
            event
                .fields
                .get("selected_runtime_target")
                .map(String::as_str),
            Some("host")
        );
        assert_eq!(
            event
                .fields
                .get("selected_tool_version")
                .map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn app_runtime_agent_launch_completion_failure_emits_structured_error_log() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");

        let events = capture_tracing_events(|| {
            let _ = runtime.handle_launch_complete(
                window_id.clone(),
                Err("launch failed before process spawn".to_string()),
            );
        });

        let event = events
            .iter()
            .find(|event| {
                event.level == Level::ERROR
                    && event.target == "gwt::agent_launch"
                    && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
            })
            .expect("agent launch completion failure log");
        assert_eq!(
            event.fields.get("window_id").map(String::as_str),
            Some(window_id.as_str())
        );
        assert_eq!(
            event.fields.get("tab_id").map(String::as_str),
            Some("tab-1")
        );
        assert_eq!(
            event.fields.get("error").map(String::as_str),
            Some("launch failed before process spawn")
        );
    }

    #[test]
    fn app_runtime_launch_wizard_submit_emits_agent_window_launching_status() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LaunchWizardAction {
                action: LaunchWizardAction::Submit,
                bounds: Some(canvas_bounds()),
            },
        );

        let workspace = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::WorkspaceState { workspace } => Some(workspace),
                _ => None,
            })
            .expect("workspace state after wizard submit");
        let agent_window = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == "tab-1")
            .and_then(|tab| {
                tab.workspace
                    .windows
                    .iter()
                    .find(|window| window.preset == WindowPreset::Agent)
            })
            .expect("agent placeholder window");
        assert_eq!(agent_window.title, "Codex");
        assert_eq!(agent_window.agent_id.as_deref(), Some("codex"));

        let launch_status = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::TerminalStatus { id, detail, .. }
                    if detail.as_deref() == Some("Launching...") =>
                {
                    Some(id)
                }
                _ => None,
            })
            .expect("launching terminal status");
        assert!(launch_status.ends_with("::agent-1"));
        assert!(events.iter().any(|event| {
            matches!(
                event.event,
                BackendEvent::LaunchWizardState { wizard: None }
            )
        }));
    }

    #[test]
    fn app_runtime_launch_complete_missing_wizard_window_surfaces_open_error() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.launch_wizard = Some(sample_ready_agent_launch_wizard_session("tab-1", &repo));

        let submit_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LaunchWizardAction {
                action: LaunchWizardAction::Submit,
                bounds: Some(canvas_bounds()),
            },
        );
        let window_id = submit_events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::TerminalStatus { id, detail, .. }
                    if detail.as_deref() == Some("Launching...") =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .expect("wizard launch window id");
        let address = runtime
            .window_lookup
            .remove(&window_id)
            .expect("registered agent window");
        let tab = runtime.tab_mut(&address.tab_id).expect("tab");
        assert!(tab.workspace.close_window(&address.raw_id));

        let completion_events =
            runtime.handle_launch_complete(window_id, Err("Window not found".to_string()));

        assert!(completion_events.iter().any(|event| {
            matches!(
                (&event.target, &event.event),
                (
                    DispatchTarget::Client(client_id),
                    BackendEvent::LaunchWizardOpenError { title, message }
                ) if client_id == "client-1"
                    && title == "Launch Agent"
                    && message == "Window not found"
            )
        }));
    }

    #[test]
    fn app_runtime_start_work_launch_completion_registers_unassigned_agent() {
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("repo-work-20260504-1234");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            repo.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };

        let _events = runtime.handle_launch_complete(
            window_id,
            Ok((
                ProcessLaunch {
                    command,
                    args,
                    env: HashMap::new(),
                    remove_env: Vec::new(),
                    cwd: Some(worktree.clone()),
                },
                "session-1".to_string(),
                "work/20260504-1234".to_string(),
                "Codex".to_string(),
                worktree.clone(),
                gwt_agent::AgentId::Codex,
                None,
                Some("origin/main".to_string()),
                gwt_agent::LaunchRuntimeTarget::Host,
                worktree.display().to_string(),
            )),
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(
            projection.status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Unknown,
            "Start Work must not make an unassigned Agent an active Workspace"
        );
        assert!(projection.git_details.is_none());
        assert_eq!(projection.agents.len(), 1);
        assert_eq!(projection.agents[0].session_id, "session-1");
        assert_eq!(
            projection.agents[0].affiliation_status,
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
        );
        assert_eq!(
            projection.agents[0].branch.as_deref(),
            Some("work/20260504-1234")
        );
        assert_eq!(
            projection.agents[0].worktree_path.as_deref(),
            Some(worktree.as_path())
        );
        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items");
        assert!(
            work_items.is_none(),
            "Start Work launch must not create Workspace history before explicit assignment"
        );
    }

    #[test]
    fn app_runtime_non_work_launch_registers_unassigned_agent() {
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            repo.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };

        let _events = runtime.handle_launch_complete(
            window_id,
            Ok((
                ProcessLaunch {
                    command,
                    args,
                    env: HashMap::new(),
                    remove_env: Vec::new(),
                    cwd: Some(repo.clone()),
                },
                "session-develop".to_string(),
                "develop".to_string(),
                "Codex".to_string(),
                repo.clone(),
                gwt_agent::AgentId::Codex,
                None,
                Some("origin/develop".to_string()),
                gwt_agent::LaunchRuntimeTarget::Host,
                repo.display().to_string(),
            )),
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(projection.agents.len(), 1);
        assert_eq!(projection.agents[0].session_id, "session-develop");
        assert_eq!(
            projection.agents[0].affiliation_status,
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
        );
        assert_eq!(projection.agents[0].branch.as_deref(), Some("develop"));
    }

    #[test]
    fn app_runtime_workspace_resume_launch_completion_carries_context_to_projection() {
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("repo-work-20260507-0001");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "agent-1",
            repo.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.pending_workspace_resume_contexts.insert(
            window_id.clone(),
            WorkspaceResumeContext {
                title: Some("Suspended review".to_string()),
                owner: Some("SPEC-2359".to_string()),
                summary: Some("Resume the suspended Workspace card.".to_string()),
                next_action: Some("Resume the review".to_string()),
            },
        );
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/d".to_string(),
                    "/s".to_string(),
                    "/c".to_string(),
                    "exit /b 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };

        let _events = runtime.handle_launch_complete(
            window_id,
            Ok((
                ProcessLaunch {
                    command,
                    args,
                    env: HashMap::new(),
                    remove_env: Vec::new(),
                    cwd: Some(worktree.clone()),
                },
                "session-1".to_string(),
                "work/20260507-0001".to_string(),
                "Codex".to_string(),
                worktree.clone(),
                gwt_agent::AgentId::Codex,
                Some(2359),
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                worktree.display().to_string(),
            )),
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let details = projection.git_details.expect("git details");
        assert_eq!(projection.title, "Suspended review");
        assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(
            projection.summary.as_deref(),
            Some("Resume the suspended Workspace card.")
        );
        assert_eq!(projection.next_action.as_deref(), Some("Resume the review"));
        assert_eq!(details.branch.as_deref(), Some("work/20260507-0001"));
        assert_eq!(details.worktree_path.as_deref(), Some(worktree.as_path()));
        assert!(details.created_by_start_work);
        assert_eq!(projection.agents.len(), 1);
        assert_eq!(projection.agents[0].session_id, "session-1");
        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items")
            .expect("work items");
        assert_eq!(
            work_items.work_items[0].events[0].kind,
            gwt_core::workspace_projection::WorkspaceWorkEventKind::Resume
        );
    }

    #[test]
    fn app_runtime_start_work_launch_completion_registers_multiple_unassigned_agents() {
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree_one = temp.path().join("repo-work-20260504-1234");
        let worktree_two = temp.path().join("repo-work-20260504-1235");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree_one).expect("create worktree one");
        fs::create_dir_all(&worktree_two).expect("create worktree two");
        let mut persisted = empty_workspace_state();
        persisted.windows.push(sample_window(
            "agent-1",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        ));
        persisted.windows.push(sample_window(
            "agent-2",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        ));
        persisted.next_z_index = 3;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let launch = |cwd: PathBuf| {
            let (command, args) = if cfg!(windows) {
                (
                    "cmd".to_string(),
                    vec![
                        "/d".to_string(),
                        "/s".to_string(),
                        "/c".to_string(),
                        "exit /b 0".to_string(),
                    ],
                )
            } else {
                (
                    "/bin/sh".to_string(),
                    vec!["-lc".to_string(), "exit 0".to_string()],
                )
            };
            ProcessLaunch {
                command,
                args,
                env: HashMap::new(),
                remove_env: Vec::new(),
                cwd: Some(cwd),
            }
        };

        let _first_events = runtime.handle_launch_complete(
            combined_window_id("tab-1", "agent-1"),
            Ok((
                launch(worktree_one.clone()),
                "session-1".to_string(),
                "work/20260504-1234".to_string(),
                "Codex 1".to_string(),
                worktree_one.clone(),
                gwt_agent::AgentId::Codex,
                None,
                Some("origin/main".to_string()),
                gwt_agent::LaunchRuntimeTarget::Host,
                worktree_one.display().to_string(),
            )),
        );
        let second_events = runtime.handle_launch_complete(
            combined_window_id("tab-1", "agent-2"),
            Ok((
                launch(worktree_two.clone()),
                "session-2".to_string(),
                "work/20260504-1235".to_string(),
                "Codex 2".to_string(),
                worktree_two.clone(),
                gwt_agent::AgentId::Codex,
                None,
                Some("origin/main".to_string()),
                gwt_agent::LaunchRuntimeTarget::Host,
                worktree_two.display().to_string(),
            )),
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let session_ids = projection
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(projection.agents.len(), 2);
        assert!(session_ids.contains("session-1"));
        assert!(session_ids.contains("session-2"));
        assert!(projection
            .agents
            .iter()
            .all(gwt_core::workspace_projection::WorkspaceAgentSummary::is_unassigned));
        assert!(second_events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } if projection.active_agents == 0
                && projection.agents.is_empty()
                && projection.unassigned_agents.len() == 2
                && projection.branch.is_none()
        )));
    }

    #[test]
    fn app_runtime_launch_failure_log_redacts_sensitive_error_values() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "agent-1",
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");

        let events = capture_tracing_events(|| {
            let _ = runtime.handle_launch_complete(
                window_id,
                Err("failed OPENAI_API_KEY=sk-test --api-key sk-other GWT_HOOK_TOKEN=hook-secret --token plain-token".to_string()),
            );
        });

        let event = events
            .iter()
            .find(|event| {
                event.level == Level::ERROR
                    && event.target == "gwt::agent_launch"
                    && event.fields.get("stage").map(String::as_str) == Some("launch_complete")
            })
            .expect("redacted launch completion failure log");
        let error = event.fields.get("error").expect("error field");
        assert!(!error.contains("sk-test"));
        assert!(!error.contains("sk-other"));
        assert!(!error.contains("hook-secret"));
        assert!(!error.contains("plain-token"));
        assert!(error.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(error.contains("--api-key [REDACTED]"));
        assert!(error.contains("GWT_HOOK_TOKEN=[REDACTED]"));
        assert!(error.contains("--token [REDACTED]"));
    }

    #[test]
    fn app_runtime_runtime_status_stopped_auto_closes_active_agent_window() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "codex-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            sample_active_agent_session("tab-1", &window_id),
        );

        let events = runtime.handle_runtime_status(
            window_id.clone(),
            WindowProcessStatus::Stopped,
            Some("Process exited".to_string()),
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(!runtime.active_agent_sessions.contains_key(&window_id));
        assert!(!runtime.window_lookup.contains_key(&window_id));
        assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
    }

    #[test]
    fn app_runtime_active_work_projection_filters_stale_saved_agents_when_no_agent_is_live() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection.status_text = "Old agent is running".to_string();
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "stale-session".to_string(),
                window_id: Some("tab-1::agent-1".to_string()),
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Old focus".to_string()),
                title_summary: None,
                worktree_path: None,
                branch: Some("work/old".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save stale projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(view.active_agents, 0);
        assert_eq!(view.blocked_agents, 0);
        assert!(view.agents.is_empty());
        assert_eq!(view.status_category, "idle");
        assert_eq!(view.status_text, "No active work");
    }

    #[test]
    fn app_runtime_active_work_projection_resets_stale_current_identity_when_no_agent_is_live() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.title = "PR-2525".to_string();
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection.status_text = "Old PR is active".to_string();
        projection.summary = Some("Old PR summary".to_string());
        projection.owner = Some("PR-2525".to_string());
        projection.next_action = Some("Review old PR".to_string());
        projection.board_refs = vec!["board-old".to_string()];
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/old".to_string()),
            worktree_path: Some(repo.join("work-old")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: Some(2525),
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "stale-session".to_string(),
                window_id: Some("tab-1::agent-1".to_string()),
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Old focus".to_string()),
                title_summary: Some("Old title".to_string()),
                worktree_path: None,
                branch: Some("work/old".to_string()),
                last_board_entry_id: Some("board-old".to_string()),
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save stale projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(view.title, "Repo workspace");
        assert_eq!(view.status_category, "idle");
        assert_eq!(view.status_text, "No active work");
        assert_eq!(view.summary, None);
        assert_eq!(view.owner, None);
        assert_eq!(view.next_action, None);
        assert_eq!(view.branch, None);
        assert_eq!(view.worktree_path, None);
        assert_eq!(view.pr_number, None);
        assert!(view.board_refs.is_empty());
    }

    #[test]
    fn app_runtime_active_work_projection_filters_stale_agent_when_window_id_is_reused() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let window_id = "tab-1::agent-1";
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection.status_text = "Old agent is running".to_string();
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "stale-session".to_string(),
                window_id: Some(window_id.to_string()),
                agent_id: "codex".to_string(),
                display_name: "Old Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Old focus".to_string()),
                title_summary: Some("Old title".to_string()),
                worktree_path: None,
                branch: Some("work/old".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save stale projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.active_agent_sessions.insert(
            window_id.to_string(),
            ActiveAgentSession {
                window_id: window_id.to_string(),
                session_id: "live-session".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/live".to_string(),
                display_name: "Live Codex".to_string(),
                worktree_path: repo.join("../repo-work-live"),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(view.active_agents, 1);
        assert_eq!(view.blocked_agents, 0);
        assert_eq!(view.agents.len(), 1);
        assert_eq!(view.agents[0].session_id, "live-session");
        assert_eq!(view.agents[0].display_name, "Live Codex");
        assert!(!view
            .agents
            .iter()
            .any(|agent| agent.session_id == "stale-session"));
    }

    #[test]
    fn app_runtime_active_work_projection_includes_recent_workspace_journal_entries() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        gwt_core::workspace_projection::update_workspace_projection_with_journal(
            &repo,
            gwt_core::workspace_projection::WorkspaceProjectionUpdate {
                title: Some("Workspace Overview".to_string()),
                status_category: Some(
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Idle,
                ),
                status_text: Some("Ready for review".to_string()),
                owner: Some("SPEC-2359".to_string()),
                next_action: Some("Review summary".to_string()),
                summary: Some("Overview summary is persisted.".to_string()),
                agent_session_id: None,
                agent_current_focus: None,
                agent_title_summary: None,
            },
        )
        .expect("workspace update");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(
            view.summary.as_deref(),
            Some("Overview summary is persisted.")
        );
        assert_eq!(view.journal_entries.len(), 1);
        assert_eq!(
            view.journal_entries[0].summary.as_deref(),
            Some("Overview summary is persisted.")
        );
        assert_eq!(
            view.journal_entries[0].next_action.as_deref(),
            Some("Review summary")
        );
    }

    #[test]
    fn app_runtime_resume_workspace_journal_reuses_existing_branch_as_execution_container() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let branch = "work/20260507-0001";
        run_git(&repo, &["branch", branch]);
        gwt_core::workspace_projection::save_workspace_projection(
            &repo,
            &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
        )
        .expect("save projection");
        append_workspace_resume_journal(
            &repo,
            "journal-reuse",
            temp.path().join("work").join("20260507-0001"),
            "SPEC-2359",
            "Resume the suspended Workspace card.",
        );
        assert_eq!(
            super::workspace_resume_branch_from_journal_project_root(
                &temp.path().join("work").join("20260507-0001"),
                &repo
            )
            .as_deref(),
            Some(branch)
        );
        assert!(super::workspace_resume_branch_exists(&repo, branch));
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Journal,
                journal_id: Some("journal-reuse".to_string()),
            },
        );

        let session = runtime.launch_wizard.as_ref().expect("launch wizard");
        let view = session.wizard.view();
        assert_eq!(view.title, "Launch Agent");
        assert_eq!(view.branch_name, branch);
        let context = session
            .workspace_resume_context
            .as_ref()
            .expect("workspace resume context");
        assert_eq!(context.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(
            context.summary.as_deref(),
            Some("Resume the suspended Workspace card.")
        );
    }

    // SPEC-2359 US-42 — Workspace Resume Picker tests.

    fn write_resumable_session_for_test(
        sessions_dir: &Path,
        session_id: &str,
        repo: &Path,
        branch: &str,
        agent_id: gwt_agent::AgentId,
        agent_session_id: Option<&str>,
    ) {
        let mut session = gwt_agent::Session::new(repo, branch, agent_id);
        session.id = session_id.to_string();
        session.display_name = "Codex".to_string();
        session.tool_version = Some("installed".to_string());
        session.agent_session_id = agent_session_id.map(str::to_string);
        std::fs::create_dir_all(sessions_dir).expect("sessions dir");
        session.save(sessions_dir).expect("session toml");
    }

    fn projection_with_assigned_agent(
        repo: &Path,
        session_id: &str,
    ) -> gwt_core::workspace_projection::WorkspaceProjection {
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
        projection.title = "Workspace with Resume candidate".to_string();
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection
            .agents
            .push(workspace_agent_summary_for_test(session_id, None));
        projection
    }

    #[test]
    fn app_runtime_list_resumable_agents_returns_assigned_with_session_toml() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let session_id = "session-resumable-1";
        let projection = projection_with_assigned_agent(&repo, session_id);
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let sessions_dir = temp.path().join("sessions");
        write_resumable_session_for_test(
            &sessions_dir,
            session_id,
            &repo,
            "work/test",
            gwt_agent::AgentId::Codex,
            Some("prior-codex-uuid"),
        );

        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ListResumableAgents { workspace_id: None },
        );

        let event = events
            .first()
            .expect("backend should respond to ListResumableAgents");
        assert!(matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        ));
        match &event.event {
            BackendEvent::WorkspaceResumableAgents { agents, .. } => {
                assert_eq!(agents.len(), 1, "single assigned agent must surface");
                assert_eq!(agents[0].session_id, session_id);
                assert!(matches!(
                    agents[0].resume_kind,
                    gwt::ResumableAgentResumeKind::Session
                ));
            }
            other => panic!("unexpected backend event: {other:?}"),
        }
    }

    #[test]
    fn app_runtime_list_resumable_agents_includes_unassigned_agents_with_session_toml() {
        // SPEC-2359 US-42 follow-up: production projections often store
        // agents with `affiliation_status = unassigned` (no explicit
        // `workspace join` step). Resume Picker must still offer them as
        // candidates when a Session toml is on disk; otherwise users see
        // "No resumable agents" for every Workspace they did not
        // manually ensure / join.
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let session_id = "session-unassigned-1";
        let mut projection = projection_with_assigned_agent(&repo, session_id);
        projection.agents[0].affiliation_status =
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let sessions_dir = temp.path().join("sessions");
        write_resumable_session_for_test(
            &sessions_dir,
            session_id,
            &repo,
            "work/test",
            gwt_agent::AgentId::Codex,
            Some("agent-session-uuid"),
        );

        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ListResumableAgents { workspace_id: None },
        );

        match events.first().map(|outbound| &outbound.event) {
            Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
                assert_eq!(
                    agents.len(),
                    1,
                    "Unassigned agent with a backing Session toml must still surface",
                );
                assert_eq!(agents[0].session_id, session_id);
            }
            other => panic!("unexpected backend event: {other:?}"),
        }
    }

    #[test]
    fn app_runtime_list_resumable_agents_excludes_live_session_ids() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let session_id = "session-live-1";
        let projection = projection_with_assigned_agent(&repo, session_id);
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let sessions_dir = temp.path().join("sessions");
        write_resumable_session_for_test(
            &sessions_dir,
            session_id,
            &repo,
            "work/test",
            gwt_agent::AgentId::Codex,
            Some("agent-session-uuid"),
        );

        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        // Register the same session id as live so the picker skips it.
        let mut live = sample_active_agent_session("tab-1", "window-1");
        live.session_id = session_id.to_string();
        runtime
            .active_agent_sessions
            .insert("window-1".to_string(), live);

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ListResumableAgents { workspace_id: None },
        );

        match events.first().map(|outbound| &outbound.event) {
            Some(BackendEvent::WorkspaceResumableAgents { agents, .. }) => {
                assert!(
                    agents.is_empty(),
                    "live session must not appear in the Resume picker"
                );
            }
            other => panic!("unexpected backend event: {other:?}"),
        }
    }

    #[test]
    fn app_runtime_resume_workspace_agent_replies_error_when_session_toml_missing() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspaceAgent {
                session_id: "missing-session".to_string(),
                bounds: canvas_bounds(),
            },
        );

        let event = events
            .first()
            .expect("ResumeWorkspaceAgent must reply on missing session");
        assert!(matches!(
            &event.target,
            DispatchTarget::Client(client_id) if client_id == "client-1"
        ));
        assert!(matches!(
            &event.event,
            BackendEvent::WorkspaceResumeAgentError { session_id, message }
                if session_id == "missing-session" && !message.is_empty()
        ));
    }

    #[test]
    fn app_runtime_latest_branch_resume_picks_newest_resumable_session() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");

        let mut older =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        older.id = "session-older".to_string();
        older.agent_session_id = Some("native-older".to_string());
        older.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 9, 0, 0).unwrap();
        older.updated_at = older.last_activity_at;
        older.created_at = older.last_activity_at;
        older.save(&sessions_dir).expect("save older session");

        let mut newer =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        newer.id = "session-newer".to_string();
        newer.agent_session_id = Some("native-newer".to_string());
        newer.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 10, 0, 0).unwrap();
        newer.updated_at = newer.last_activity_at;
        newer.created_at = newer.last_activity_at;
        newer.save(&sessions_dir).expect("save newer session");

        let mut metadata_only =
            gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
        metadata_only.id = "session-metadata-only".to_string();
        metadata_only.agent_session_id = None;
        metadata_only.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, 11, 0, 0).unwrap();
        metadata_only.updated_at = metadata_only.last_activity_at;
        metadata_only.created_at = metadata_only.last_activity_at;
        metadata_only
            .save(&sessions_dir)
            .expect("save metadata-only session");

        let runtime = sample_runtime(temp.path(), Vec::new(), None);

        let selected = runtime
            .latest_resumable_branch_session(&repo, "work/manual-resume")
            .expect("latest resumable session");

        assert_eq!(selected.id, "session-newer");
        assert_eq!(selected.agent_session_id.as_deref(), Some("native-newer"));
    }

    #[test]
    fn app_runtime_open_launch_wizard_shows_only_latest_resume_and_focus_methods() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");

        for (session_id, native_session_id, hour) in [
            ("session-older", "native-older", 9),
            ("session-newer", "native-newer", 10),
        ] {
            let mut session =
                gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.agent_session_id = Some(native_session_id.to_string());
            session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
            session.updated_at = session.last_activity_at;
            session.created_at = session.last_activity_at;
            session.save(&sessions_dir).expect("save session");
        }
        for (session_id, hour) in [("session-live-older", 11), ("session-live-newer", 12)] {
            let mut session =
                gwt_agent::Session::new(&repo, "work/manual-resume", gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.agent_session_id = None;
            session.last_activity_at = Utc.with_ymd_and_hms(2026, 5, 21, hour, 0, 0).unwrap();
            session.updated_at = session.last_activity_at;
            session.created_at = session.last_activity_at;
            session
                .save(&sessions_dir)
                .expect("save live session metadata");
        }

        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "branches-1",
            repo,
            WindowPreset::Branches,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "branches-1");
        for (window, session, display) in [
            ("agent-older", "session-live-older", "Codex Older"),
            ("agent-newer", "session-live-newer", "Codex Newer"),
        ] {
            let agent_window_id = combined_window_id("tab-1", window);
            runtime.active_agent_sessions.insert(
                agent_window_id.clone(),
                ActiveAgentSession {
                    window_id: agent_window_id,
                    session_id: session.to_string(),
                    agent_id: "codex".to_string(),
                    branch_name: "work/manual-resume".to_string(),
                    display_name: display.to_string(),
                    worktree_path: PathBuf::from("/tmp/repo"),
                    agent_project_root: "/tmp/repo".to_string(),
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                    tab_id: "tab-1".to_string(),
                },
            );
        }

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::OpenLaunchWizard {
                id: window_id,
                branch_name: "work/manual-resume".to_string(),
                linked_issue_number: None,
            },
        );

        let view = runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard")
            .wizard
            .view();
        assert_eq!(
            view.quick_start_entries
                .iter()
                .map(|entry| entry.resume_session_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("native-newer")]
        );
        let continue_method = view
            .start_methods
            .iter()
            .find(|method| method.kind == "continue_last_session")
            .expect("continue method");
        assert!(
            continue_method
                .detail
                .as_deref()
                .unwrap_or("")
                .contains("native-newer"),
            "continue method should describe the latest resumable session"
        );
        let focus_method = view
            .start_methods
            .iter()
            .find(|method| method.kind == "focus_running_session")
            .expect("focus method");
        assert!(
            focus_method.summary.contains("Codex Newer"),
            "focus method should target the latest live session"
        );
    }

    #[test]
    fn app_runtime_resume_branch_latest_returns_branch_error_when_no_resumable_session_exists() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "branches-1",
            repo,
            WindowPreset::Branches,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "branches-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeBranchLatestAgent {
                id: window_id.clone(),
                branch_name: "work/manual-resume".to_string(),
                bounds: canvas_bounds(),
            },
        );

        assert!(matches!(
            events.first().map(|event| &event.event),
            Some(BackendEvent::BranchError { id, message })
                if id == &window_id && message.contains("No resumable session")
        ));
    }

    #[test]
    fn app_runtime_bootstrap_auto_resumes_clean_waiting_input_session() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("auto-resume");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/auto-resume",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-auto",
            "Auto Resume",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
        for (session_id, native_session_id) in [
            ("session-auto-one", "native-session-one"),
            ("session-auto-two", "native-session-two"),
        ] {
            let mut session =
                gwt_agent::Session::new(&worktree, "work/auto-resume", gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.agent_session_id = Some(native_session_id.to_string());
            session.restore_window_on_startup = true;
            session.record_hook_event("Stop");
            session.record_completed_stop();
            session
                .save(&runtime.sessions_dir)
                .expect("save resumable session");
        }

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        assert_eq!(
            runtime.tabs.len(),
            1,
            "bootstrap should reopen the worktree tab"
        );
        assert!(same_worktree_path(&runtime.tabs[0].project_root, &worktree));
        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        assert_eq!(
            agent_windows, 2,
            "all exact-resumable sessions for a worktree should restart"
        );
    }

    #[test]
    fn app_runtime_bootstrap_queues_startup_auto_resume_until_canvas_ready() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("queued-auto-resume");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/queued-auto-resume",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-auto",
            "Auto Resume",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/queued-auto-resume",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-queued-auto".to_string();
        session.agent_session_id = Some("native-queued-auto".to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");

        runtime.bootstrap();

        assert!(
            runtime.tabs[0]
                .workspace
                .persisted()
                .windows
                .iter()
                .all(|window| window.preset != WindowPreset::Agent),
            "bootstrap should wait for the frontend canvas bounds before placing restored windows"
        );

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        assert_eq!(agent_windows, 1);
        assert_eq!(runtime.pending_auto_resume_sources.len(), 1);
    }

    #[test]
    fn app_runtime_startup_auto_resume_uses_centered_stack_bounds() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("centered-stack");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/centered-stack",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-auto",
            "Auto Resume",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
        for (index, native_session_id) in
            ["native-stack-one", "native-stack-two", "native-stack-three"]
                .into_iter()
                .enumerate()
        {
            let mut session = gwt_agent::Session::new(
                &worktree,
                "work/centered-stack",
                gwt_agent::AgentId::Codex,
            );
            session.id = format!("session-centered-stack-{index}");
            session.agent_session_id = Some(native_session_id.to_string());
            session.restore_window_on_startup = true;
            session.record_hook_event("Stop");
            session.record_completed_stop();
            session.last_activity_at = chrono::Utc::now() - chrono::Duration::seconds(index as i64);
            session.updated_at = session.last_activity_at;
            session
                .save(&runtime.sessions_dir)
                .expect("save resumable session");
        }

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let geometries = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .map(|window| window.geometry.clone())
            .collect::<Vec<_>>();
        assert_eq!(geometries.len(), 3);
        assert_eq!(
            geometries
                .iter()
                .map(|geometry| (geometry.x, geometry.y, geometry.width, geometry.height))
                .collect::<Vec<_>>(),
            vec![
                (312.0, 216.0, 720.0, 420.0),
                (340.0, 240.0, 720.0, 420.0),
                (368.0, 264.0, 720.0, 420.0),
            ],
            "restored agent windows should form a stack centered in the startup canvas"
        );
    }

    #[test]
    fn app_runtime_startup_auto_resume_excludes_closed_stopped_windows() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("restore-flag");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/restore-flag",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-auto",
            "Auto Resume",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
        for (session_id, native_session_id, restore_window_on_startup, stopped) in [
            ("session-open-window", "native-open-window", true, false),
            ("session-closed-window", "native-closed-window", false, true),
        ] {
            let mut session =
                gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
            session.id = session_id.to_string();
            session.agent_session_id = Some(native_session_id.to_string());
            session.restore_window_on_startup = restore_window_on_startup;
            session.record_hook_event("Stop");
            session.record_completed_stop();
            if stopped {
                session.update_status(gwt_agent::AgentStatus::Stopped);
            }
            session
                .save(&runtime.sessions_dir)
                .expect("save resumable session");
        }

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let resumed_sources = runtime
            .pending_auto_resume_sources
            .values()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            resumed_sources,
            std::collections::HashSet::from(["session-open-window".to_string()])
        );
    }

    #[test]
    fn app_runtime_startup_auto_resume_includes_legacy_non_stopped_sessions() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("legacy-restore");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/legacy-restore",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-auto",
            "Auto Resume",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-auto"));
        let mut session =
            gwt_agent::Session::new(&worktree, "work/legacy-restore", gwt_agent::AgentId::Codex);
        session.id = "session-legacy-open-window".to_string();
        session.agent_session_id = Some("native-legacy-open-window".to_string());
        session.restore_window_on_startup = false;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save legacy open session");

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let resumed_sources = runtime
            .pending_auto_resume_sources
            .values()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            resumed_sources,
            std::collections::HashSet::from(["session-legacy-open-window".to_string()]),
            "legacy sessions without the new restore flag should still restore when they were not explicitly stopped"
        );
    }

    #[test]
    fn app_runtime_close_agent_window_clears_startup_restore_eligibility() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab_id = "tab-1";
        let raw_window_id = "agent-1";
        let window_id = combined_window_id(tab_id, raw_window_id);
        let tab = sample_project_tab_with_window_at(
            tab_id,
            raw_window_id,
            worktree.clone(),
            WindowPreset::Agent,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some(tab_id));
        let mut session =
            gwt_agent::Session::new(&worktree, "work/restore-flag", gwt_agent::AgentId::Codex);
        session.id = "session-close-clears-restore".to_string();
        session.restore_window_on_startup = true;
        session
            .save(&runtime.sessions_dir)
            .expect("save active session");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: session.id.clone(),
                agent_id: "codex".to_string(),
                branch_name: "work/restore-flag".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: worktree.clone(),
                agent_project_root: worktree.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: tab_id.to_string(),
            },
        );

        runtime.close_window_events(&window_id);

        let loaded = gwt_agent::Session::load(
            &runtime
                .sessions_dir
                .join("session-close-clears-restore.toml"),
        )
        .expect("load session");
        assert!(!loaded.restore_window_on_startup);
    }

    #[test]
    fn app_runtime_bootstrap_auto_resumes_same_repo_worktree_session_from_restored_project_tab() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("same-repo-session");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/same-repo-session",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab("tab-repo", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/same-repo-session",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-same-repo-worktree".to_string();
        session.agent_session_id = Some("native-same-repo-worktree".to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        assert_eq!(
            runtime.tabs.len(),
            1,
            "bootstrap should keep the restored project tab instead of opening a hidden worktree tab"
        );
        assert!(same_worktree_path(&runtime.tabs[0].project_root, &repo));
        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        assert_eq!(
            agent_windows, 1,
            "resumable sessions from local worktrees in the restored repo should restart inside the project tab"
        );
    }

    #[test]
    fn app_runtime_bootstrap_ignores_same_repo_worktree_session_without_lifecycle() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("no-lifecycle-session");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/no-lifecycle-session",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/no-lifecycle-session",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-same-repo-no-lifecycle".to_string();
        session.agent_session_id = Some("native-no-lifecycle".to_string());
        session.restore_window_on_startup = true;
        session.update_status(gwt_agent::AgentStatus::Running);
        session
            .save(&runtime.sessions_dir)
            .expect("save stale session");

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        assert_eq!(
            agent_windows, 0,
            "same-repo fallback must still require lifecycle evidence so old session history does not mass launch"
        );
    }

    #[test]
    fn app_runtime_bootstrap_ignores_same_repo_worktree_session_with_placeholder_resume_id() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("placeholder-session");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/placeholder-session",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab("tab-repo", "Repo", repo, ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-repo"));
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/placeholder-session",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-same-repo-placeholder".to_string();
        session.agent_session_id = Some("agent-session".to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save placeholder session");

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        let agent_windows = runtime.tabs[0]
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Agent)
            .count();
        assert_eq!(
            agent_windows, 0,
            "placeholder Codex hook ids must not launch `codex resume agent-session`"
        );
    }

    #[test]
    fn app_runtime_bootstrap_does_not_auto_resume_sessions_outside_restored_tabs() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("unlisted-auto-resume");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/unlisted-auto-resume",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
        let mut session = gwt_agent::Session::new(
            &worktree,
            "work/unlisted-auto-resume",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-unlisted-auto".to_string();
        session.agent_session_id = Some("native-unlisted-auto".to_string());
        session.restore_window_on_startup = true;
        session.record_hook_event("Stop");
        session.record_completed_stop();
        session
            .save(&runtime.sessions_dir)
            .expect("save resumable session");

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        assert!(
            runtime.tabs.is_empty(),
            "bootstrap must not open project tabs from old session TOMLs that were not restored from session.json"
        );
        assert!(
            runtime.pending_auto_resume_sources.is_empty(),
            "unlisted sessions must remain manual resume candidates instead of launching hidden agent windows"
        );
    }

    #[test]
    fn app_runtime_bootstrap_auto_resume_dedupes_and_skips_stale_without_count_cap() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let worktree = temp.path().join("worktrees").join("auto-resume-guard");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "work/auto-resume-guard",
                worktree.to_str().expect("worktree path"),
            ],
        );
        let tab = sample_project_tab(
            "tab-worktree",
            "Worktree",
            worktree.clone(),
            ProjectKind::Git,
            &[],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-worktree"));
        let now = chrono::Utc::now();
        let cases = [
            ("session-fresh-1", "native-one", 1_i64),
            ("session-duplicate-native-one", "native-one", 2_i64),
            ("session-fresh-2", "native-two", 3_i64),
            ("session-fresh-3", "native-three", 4_i64),
            ("session-fresh-4", "native-four", 5_i64),
            ("session-stale", "native-stale", 60 * 60 * 48_i64),
        ];
        for (session_id, native_session_id, age_secs) in cases {
            let mut session = gwt_agent::Session::new(
                &worktree,
                "work/auto-resume-guard",
                gwt_agent::AgentId::Codex,
            );
            session.id = session_id.to_string();
            session.agent_session_id = Some(native_session_id.to_string());
            session.restore_window_on_startup = true;
            session.record_hook_event("Stop");
            session.record_completed_stop();
            session.last_activity_at = now - chrono::Duration::seconds(age_secs);
            session.updated_at = session.last_activity_at;
            session
                .save(&runtime.sessions_dir)
                .expect("save resumable session");
        }

        runtime.bootstrap();
        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::StartupAutoResumeReady {
                bounds: canvas_bounds(),
            },
        );

        assert_eq!(
            runtime.pending_auto_resume_sources.len(),
            4,
            "startup auto-resume must restore every fresh unique exact-resumable session"
        );
        let resumed_sources = runtime
            .pending_auto_resume_sources
            .values()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        assert!(
            !resumed_sources.contains("session-duplicate-native-one"),
            "duplicate native agent session ids must not launch twice"
        );
        assert!(
            !resumed_sources.contains("session-stale"),
            "stale persisted sessions must stay available for manual resume instead of auto-launching"
        );
        assert!(
            resumed_sources.contains("session-fresh-4"),
            "startup auto-resume must not drop fresh unique sessions due to an arbitrary count cap"
        );
    }

    #[test]
    fn app_runtime_resume_workspace_journal_populates_quick_start_entries_from_prior_sessions() {
        // SPEC-2359 US-44 (Issue #2757) follow-on: when the user clicks
        // `Resume` on a Workspace journal card whose branch already has a
        // prior session on disk, the Launch Wizard should expose that prior
        // session through the Quick Start panel immediately. Without this,
        // the Quick Start panel only fills after the user completes the
        // runtime resolution step, which forces a multi-click resume path
        // even though the resumable session metadata is already known.
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let branch = "work/20260518-resume-qs";
        run_git(&repo, &["branch", branch]);
        gwt_core::workspace_projection::save_workspace_projection(
            &repo,
            &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
        )
        .expect("save projection");
        append_workspace_resume_journal(
            &repo,
            "journal-quickstart",
            temp.path().join("work").join("20260518-resume-qs"),
            "SPEC-2359",
            "Resume with prior Codex session available",
        );

        // Pre-seed a Session toml that matches the resumable branch so the
        // wizard cache exposes it through Quick Start entries.
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("sessions dir");
        let mut session = gwt_agent::Session::new(&repo, branch, gwt_agent::AgentId::Codex);
        session.display_name = "Codex".to_string();
        session.agent_session_id = Some("prior-codex-uuid".to_string());
        session.tool_version = Some("installed".to_string());
        session.save(&sessions_dir).expect("save session toml");

        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Journal,
                journal_id: Some("journal-quickstart".to_string()),
            },
        );

        let session_ref = runtime
            .launch_wizard
            .as_ref()
            .expect("launch wizard opened");
        let view = session_ref.wizard.view();
        assert_eq!(view.branch_name, branch);
        assert!(
            !view.quick_start_entries.is_empty(),
            "Resume must pre-populate Quick Start entries so the prior session is immediately resumable; saw empty Quick Start panel"
        );
        assert!(
            view.quick_start_entries
                .iter()
                .any(|entry| entry.resume_session_id.as_deref() == Some("prior-codex-uuid")),
            "Expected the prior Codex session to appear in Quick Start entries with its resume_session_id surfaced"
        );
    }

    #[test]
    fn app_runtime_resume_workspace_journal_falls_back_to_new_work_branch_when_branch_is_missing() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        gwt_core::workspace_projection::save_workspace_projection(
            &repo,
            &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
        )
        .expect("save projection");
        append_workspace_resume_journal(
            &repo,
            "journal-new-work",
            temp.path().join("work").join("20260507-0002"),
            "Issue #2359",
            "Carry this suspended context into a new work branch.",
        );
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Journal,
                journal_id: Some("journal-new-work".to_string()),
            },
        );

        let session = runtime.launch_wizard.as_ref().expect("launch wizard");
        let view = session.wizard.view();
        assert_eq!(view.title, "Start Work");
        assert!(view.branch_name.starts_with("work/"));
        assert_ne!(view.branch_name, "work/20260507-0002");
        assert_eq!(view.linked_issue_number, Some(2359));
        let context = session
            .workspace_resume_context
            .as_ref()
            .expect("workspace resume context");
        assert_eq!(context.owner.as_deref(), Some("Issue #2359"));
        assert_eq!(
            context.summary.as_deref(),
            Some("Carry this suspended context into a new work branch.")
        );
    }

    #[test]
    fn app_runtime_resume_workspace_current_ignores_idle_stale_git_details() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);
        let stale_branch = "work/20260507-stale";
        run_git(&repo, &["branch", stale_branch]);
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.title = "Stale Workspace".to_string();
        projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Idle;
        projection.status_text = "No active work".to_string();
        projection.summary = Some("Old work should not be resumed".to_string());
        projection.owner = Some("Issue #2359".to_string());
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some(stale_branch.to_string()),
            worktree_path: Some(temp.path().join("work/20260507-stale")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Current,
                journal_id: None,
            },
        );

        let session = runtime.launch_wizard.as_ref().expect("launch wizard");
        let view = session.wizard.view();
        assert_eq!(view.title, "Start Work");
        assert!(view.branch_name.starts_with("work/"));
        assert_ne!(view.branch_name, stale_branch);
        let context = session
            .workspace_resume_context
            .as_ref()
            .expect("workspace resume context");
        assert_eq!(context.title.as_deref(), Some("Repo workspace"));
        assert_eq!(context.owner, None);
        assert_eq!(context.summary, None);
    }

    #[test]
    fn app_runtime_resume_workspace_journal_derives_feature_branch_under_work_named_repo_parent() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("work").join("repo");
        init_git_clone_with_origin(&repo);
        let branch = "feature/resume-existing";
        run_git(&repo, &["branch", branch]);
        gwt_core::workspace_projection::save_workspace_projection(
            &repo,
            &gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo),
        )
        .expect("save projection");
        append_workspace_resume_journal(
            &repo,
            "journal-feature",
            temp.path()
                .join("work")
                .join("feature")
                .join("resume-existing"),
            "Issue #2359",
            "Resume a non-work branch from a deleted worktree path.",
        );
        assert_eq!(
            super::workspace_resume_branch_from_journal_project_root(
                &temp
                    .path()
                    .join("work")
                    .join("feature")
                    .join("resume-existing"),
                &repo
            )
            .as_deref(),
            Some(branch)
        );
        assert!(super::workspace_resume_branch_exists(&repo, branch));
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::ResumeWorkspace {
                source: gwt::WorkspaceResumeSource::Journal,
                journal_id: Some("journal-feature".to_string()),
            },
        );

        let session = runtime.launch_wizard.as_ref().expect("launch wizard");
        let view = session.wizard.view();
        assert_eq!(view.title, "Launch Agent");
        assert_eq!(view.branch_name, branch);
    }

    #[test]
    fn app_runtime_active_work_projection_exposes_done_workspace_cleanup_candidate() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0200".to_string()),
            worktree_path: Some(repo.join("work/20260507-0200")),
            base_branch: Some("origin/main".to_string()),
            pr_number: Some(2525),
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");
        let candidate = view.cleanup_candidate.expect("cleanup candidate");

        assert_eq!(candidate.branch, "work/20260507-0200");
        assert_eq!(candidate.reason, "workspace_done");
        assert!(!candidate.default_delete_remote);
    }

    #[test]
    fn app_runtime_active_work_projection_does_not_spawn_git_for_cleanup_candidate() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let fake_bin = temp.path().join("fake-bin");
        fs::create_dir_all(&fake_bin).expect("create fake bin");
        let fake_git = write_fake_git_recorder(&fake_bin);
        let git_log = temp.path().join("git-invocations.log");
        let _path = prepend_tool_parent_to_path(&fake_git);
        let _git_log = ScopedEnvVar::set("GWT_FAKE_GIT_LOG", &git_log);
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0200".to_string()),
            worktree_path: Some(repo.join("work/20260507-0200")),
            base_branch: Some("origin/main".to_string()),
            pr_number: Some(2525),
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert!(view.cleanup_candidate.is_some());
        let invocations = fs::read_to_string(&git_log).unwrap_or_default();
        assert!(
            invocations.trim().is_empty(),
            "active-work projection must not spawn git on the GUI hot path; invocations:\n{invocations}"
        );
    }

    #[test]
    fn workspace_cleanup_failure_does_not_emit_done_work_item() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let branch = "work/missing";
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some(branch.to_string()),
            worktree_path: Some(repo.join("work/missing")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: Some(2828),
            pr_state: Some("MERGED".to_string()),
            pr_url: Some("https://github.com/akiojin/gwt/pull/2828".to_string()),
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        let work_item_id = projection.id.clone();
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let start = gwt_core::workspace_projection::WorkspaceWorkEvent::new(
            gwt_core::workspace_projection::WorkspaceWorkEventKind::Start,
            &work_item_id,
            chrono::Utc::now(),
        );
        gwt_core::workspace_projection::record_workspace_work_event(&repo, start)
            .expect("record start");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

        let immediate_events =
            runtime.run_workspace_cleanup_events("client-1", branch, false, false);

        assert!(immediate_events.is_empty());
        wait_for_recorded_event("workspace cleanup failure", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(outbound_events)
                        if outbound_events.iter().any(|outbound| matches!(
                            outbound.event,
                            BackendEvent::BranchCleanupResult { .. }
                                | BackendEvent::BranchError { .. }
                        ))
                )
            })
        });
        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items")
            .expect("work items");
        let item = work_items
            .work_items
            .iter()
            .find(|item| item.id == work_item_id)
            .expect("work item");

        assert!(
            !item
                .events
                .iter()
                .any(|event| event.kind
                    == gwt_core::workspace_projection::WorkspaceWorkEventKind::Done),
            "failed cleanup must not mark the Workspace work item done"
        );
    }

    #[test]
    fn app_runtime_active_work_projection_exposes_saved_pr_metadata_without_live_agents() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.title = "Active Work PR display".to_string();
        projection.status_text = "No active work".to_string();
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0808".to_string()),
            worktree_path: Some(repo.join("work/20260507-0808")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: Some(2538),
            pr_state: Some("OPEN".to_string()),
            pr_url: Some("https://github.com/akiojin/gwt/pull/2538".to_string()),
            pr_created_at: Some("2026-05-07T08:20:00Z".parse().expect("pr created_at")),
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(view.active_agents, 0);
        assert_eq!(view.pr_number, Some(2538));
        assert_eq!(view.pr_state.as_deref(), Some("OPEN"));
        assert_eq!(
            view.pr_url.as_deref(),
            Some("https://github.com/akiojin/gwt/pull/2538")
        );
        assert_eq!(
            view.pr_created_at.as_deref(),
            Some("2026-05-07T08:20:00+00:00")
        );
    }

    #[test]
    fn app_runtime_active_work_projection_hides_cleanup_candidate_for_live_agent_branch() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "codex-1",
            repo.clone(),
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category = gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260507-0200".to_string()),
            worktree_path: Some(repo.join("work/20260507-0200")),
            base_branch: Some("origin/main".to_string()),
            pr_number: Some(2525),
            pr_state: Some("merged".to_string()),
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id,
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260507-0200".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.join("work/20260507-0200"),
                agent_project_root: repo.join("work/20260507-0200").display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let view = runtime
            .active_work_projection_for_tab("tab-1", &runtime.tabs[0])
            .expect("projection view");

        assert_eq!(view.cleanup_candidate, None);
    }

    #[test]
    fn app_runtime_stopped_agent_cleans_saved_projection_and_broadcasts_active_work_idle() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "codex-1",
            repo.clone(),
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        let session = ActiveAgentSession {
            window_id: window_id.clone(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260506-1652".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: temp.path().join("work/20260506-1652"),
            agent_project_root: temp.path().join("work/20260506-1652").display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        };
        runtime
            .active_agent_sessions
            .insert(window_id.clone(), session.clone());
        save_start_work_workspace_projection(&repo, &session, "origin/main", None, None)
            .expect("save projection");

        let events = runtime.handle_runtime_status(
            window_id.clone(),
            WindowProcessStatus::Stopped,
            Some("Process exited".to_string()),
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert!(projection.agents.is_empty());
        assert_eq!(
            projection.status_category,
            gwt_core::workspace_projection::WorkspaceStatusCategory::Idle
        );
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } if projection.active_agents == 0
                && projection.agents.is_empty()
                && projection.status_category == "idle"
        )));
    }

    #[test]
    fn app_runtime_status_thread_reports_process_exit_without_reader_eof() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Running,
        );
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");
        let captured_events = match &runtime.proxy {
            AppEventProxy::Stub(events) => events.clone(),
            AppEventProxy::Real(_) => panic!("sample runtime must use stub proxy"),
        };
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/D".to_string(),
                    "/S".to_string(),
                    "/C".to_string(),
                    "exit /B 0".to_string(),
                ],
            )
        } else {
            (
                "/bin/sh".to_string(),
                vec!["-lc".to_string(), "exit 0".to_string()],
            )
        };
        let pane = Arc::new(Mutex::new(
            Pane::new(
                window_id.clone(),
                command,
                args,
                80,
                24,
                HashMap::new(),
                None,
            )
            .expect("pane"),
        ));
        if cfg!(windows) {
            // Windows ConPTY may wait for this CPR response before exposing exit state.
            if let Ok(pane) = pane.lock() {
                let _ = pane.pty().write_input(b"\x1b[1;1R");
            }
        }
        let status_thread = runtime.spawn_status_thread(window_id.clone(), pane.clone());

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed_status = None;
        while Instant::now() < deadline {
            if let Ok(events) = captured_events.lock() {
                observed_status = events.iter().find_map(|event| match event {
                    UserEvent::RuntimeStatus { id, status, detail }
                        if id == &window_id && *status == WindowProcessStatus::Stopped =>
                    {
                        Some(detail.clone())
                    }
                    _ => None,
                });
            }
            if observed_status.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        if observed_status.is_none() {
            if let Ok(pane) = pane.lock() {
                let _ = pane.kill();
            }
        }
        let _ = status_thread.join();

        assert_eq!(observed_status.flatten().as_deref(), Some("Process exited"));
    }

    #[test]
    fn app_runtime_runtime_hook_stopped_auto_closes_active_agent_window() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "codex-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            sample_active_agent_session("tab-1", &window_id),
        );

        let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].event,
            BackendEvent::RuntimeHookEvent { .. }
        ));
        assert!(matches!(
            events[1].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(!runtime.active_agent_sessions.contains_key(&window_id));
        assert!(!runtime.window_lookup.contains_key(&window_id));
        assert!(runtime.tabs[0].workspace.window("codex-1").is_none());
    }

    #[test]
    fn app_runtime_workspace_projection_surface_helper_groups_state_and_active_work_events() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "codex-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            sample_active_agent_session("tab-1", &window_id),
        );

        let mut events = Vec::new();
        runtime.push_workspace_and_active_work_projection_broadcasts(&mut events);

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(matches!(
            events[1].event,
            BackendEvent::ActiveWorkProjection { .. }
        ));
    }

    #[test]
    fn app_runtime_runtime_hook_stopped_without_active_session_keeps_window_open() {
        let temp = tempdir().expect("tempdir");
        let tab = sample_project_tab_with_window(
            "tab-1",
            "codex-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");

        let events = runtime.handle_runtime_hook_event(runtime_hook_state("Stopped", "session-1"));

        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event,
            BackendEvent::RuntimeHookEvent { .. }
        ));
        assert!(runtime.window_lookup.contains_key(&window_id));
        assert!(runtime.tabs[0].workspace.window("codex-1").is_some());
    }

    #[test]
    fn app_runtime_start_window_registers_running_process_runtime_and_pty_writer() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo,
            ProjectKind::NonRepo,
            &[WindowPreset::Shell],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window = runtime.tabs[0].workspace.persisted().windows[0].clone();
        let window_id = combined_window_id("tab-1", &window.id);

        let events =
            runtime.start_window("tab-1", &window.id, window.preset, window.geometry.clone());

        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::WindowState { window_id: id, state }
                if id == &window_id && *state == WindowProcessStatus::Running
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            BackendEvent::TerminalStatus { id, status, detail }
                if id == &window_id
                    && *status == WindowProcessStatus::Running
                    && detail.is_none()
        )));
        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Running)
        );
        assert!(runtime.runtimes.contains_key(&window_id));
        assert!(runtime
            .pty_writers
            .read()
            .expect("pty writer registry")
            .contains_key(&window_id));

        runtime.stop_window_runtime(&window_id);
    }

    #[test]
    fn app_runtime_viewport_and_geometry_updates_persist_workspace_state() {
        // Persistence flows through `workspace_state_path()` which is
        // HOME-based, so we must serialize against other HOME-touching
        // tests and pin HOME to this test's tempdir for the duration.
        // Without this guard, parallel tests that mutate HOME race
        // with our persist + load pair and the workspace file ends up
        // missing (or pointing at another test's already-cleaned
        // tempdir).
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "shell-1",
            repo.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");

        assert_eq!(
            runtime
                .update_viewport_events(gwt::CanvasViewport {
                    x: 12.0,
                    y: 34.0,
                    zoom: 1.25,
                })
                .len(),
            1
        );
        assert_eq!(
            runtime
                .update_window_geometry_events(
                    &window_id,
                    WindowGeometry {
                        x: 56.0,
                        y: 78.0,
                        width: 720.0,
                        height: 480.0,
                    },
                    100,
                    30,
                    None,
                )
                .len(),
            1
        );

        assert!(
            runtime
                .persist_dispatcher
                .wait_idle(std::time::Duration::from_secs(5)),
            "persist dispatcher should drain before disk readback",
        );
        let session = load_session_state(&temp.path().join("session-state.json"))
            .expect("load persisted session state");
        assert_eq!(session.active_tab_id.as_deref(), Some("tab-1"));
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].project_root, repo);

        let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
        assert_eq!(workspace.viewport.x, 12.0);
        assert_eq!(workspace.viewport.y, 34.0);
        assert_eq!(workspace.viewport.zoom, 1.25);
        let window = workspace
            .windows
            .iter()
            .find(|window| window.id == "shell-1")
            .expect("persisted window");
        assert_eq!(window.geometry.x, 56.0);
        assert_eq!(window.geometry.y, 78.0);
        assert_eq!(window.geometry.width, 720.0);
        assert_eq!(window.geometry.height, 480.0);
    }

    #[test]
    fn app_runtime_geometry_update_rejects_stale_base_revision() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "shell-1",
            repo.clone(),
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");

        assert_eq!(
            runtime
                .update_window_geometry_events(
                    &window_id,
                    WindowGeometry {
                        x: 56.0,
                        y: 78.0,
                        width: 720.0,
                        height: 480.0,
                    },
                    100,
                    30,
                    Some(0),
                )
                .len(),
            1
        );

        assert_eq!(
            runtime
                .update_window_geometry_events(
                    &window_id,
                    WindowGeometry {
                        x: 90.0,
                        y: 120.0,
                        width: 960.0,
                        height: 640.0,
                    },
                    120,
                    40,
                    Some(0),
                )
                .len(),
            1,
            "stale updates should return the current workspace state so the frontend can resync"
        );

        assert!(
            runtime
                .persist_dispatcher
                .wait_idle(std::time::Duration::from_secs(5)),
            "persist dispatcher should drain before disk readback",
        );
        let workspace = load_restored_workspace_state(&repo).expect("load persisted workspace");
        let window = workspace
            .windows
            .iter()
            .find(|window| window.id == "shell-1")
            .expect("persisted window");
        assert_eq!(window.geometry.x, 56.0);
        assert_eq!(window.geometry.y, 78.0);
        assert_eq!(window.geometry.width, 720.0);
        assert_eq!(window.geometry.height, 480.0);
        assert_eq!(window.geometry_revision, 1);
    }

    #[test]
    fn app_runtime_load_board_replies_with_repo_scoped_snapshot() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Need review",
                Some("running".to_string()),
                None,
                vec!["coordination".to_string()],
                vec!["2018".to_string()],
            ),
        )
        .expect("seed board snapshot");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadBoard {
                id: window_id.clone(),
                all: false,
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardEntries { id, entries, .. },
            }] if client_id == "client-1"
                && id == &window_id
                && entries.len() == 1
                && entries[0].body == "Need review"
        ));
    }

    #[test]
    fn app_runtime_load_board_defaults_to_current_workspace_audience() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.id = "workspace-current".to_string();
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "session-current".to_string(),
                window_id: None,
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Board audience".to_string()),
                title_summary: Some("Board audience".to_string()),
                worktree_path: Some(repo.clone()),
                branch: Some("work/board-audience".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: Some("workspace-current".to_string()),
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "broadcast entry",
                None,
                None,
                vec![],
                vec![],
            ),
        )
        .expect("seed broadcast");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "current workspace entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-current"]),
        )
        .expect("seed current");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "other workspace entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-other"]),
        )
        .expect("seed other");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadBoard {
                id: window_id.clone(),
                all: false,
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                event: BackendEvent::BoardEntries { entries, .. },
                ..
            }] if entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>()
                == vec!["broadcast entry", "current workspace entry"]
        ));
    }

    #[test]
    fn app_runtime_load_board_history_replies_with_older_page() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        for idx in 0..4 {
            let mut entry = BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                format!("entry-{idx}"),
                None,
                None,
                vec![],
                vec![],
            );
            entry.id = format!("entry-{idx}");
            entry.created_at = chrono::Utc::now() + chrono::Duration::seconds(idx);
            entry.updated_at = entry.created_at;
            post_entry(&repo, entry).expect("seed board entry");
        }
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadBoardHistory {
                id: window_id.clone(),
                before_entry_id: Some("entry-3".to_string()),
                limit: 2,
                all: false,
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardHistoryPage {
                    id,
                    entries,
                    has_more_before,
                },
            }] if client_id == "client-1"
                && id == &window_id
                && entries.iter().map(|entry| entry.body.as_str()).collect::<Vec<_>>() == vec!["entry-1", "entry-2"]
                && *has_more_before
        ));
    }

    #[test]
    fn app_runtime_open_board_origin_agent_focuses_live_origin_session_window() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let board_raw_id = tab
            .workspace
            .add_window(WindowPreset::Board, canvas_bounds())
            .id;
        let agent_raw_id = tab
            .workspace
            .add_window(WindowPreset::Agent, canvas_bounds())
            .id;
        let board_window_id = combined_window_id("tab-1", &board_raw_id);
        let agent_window_id = combined_window_id("tab-1", &agent_raw_id);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.active_agent_sessions.insert(
            agent_window_id.clone(),
            ActiveAgentSession {
                window_id: agent_window_id.clone(),
                session_id: "session-origin".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/board-origin".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::OpenBoardOriginAgent {
                id: board_window_id,
                origin_session_id: "session-origin".to_string(),
                bounds: Some(canvas_bounds()),
            },
        );

        let workspace = runtime.tab("tab-1").expect("tab").workspace.persisted();
        let focused = workspace
            .windows
            .iter()
            .max_by_key(|window| window.z_index)
            .expect("focused window");
        assert_eq!(focused.id, agent_raw_id);
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                event: BackendEvent::WorkspaceState { .. },
                ..
            }
        )));
    }

    #[test]
    fn board_origin_agent_resume_config_uses_exact_saved_session() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let runtime = sample_runtime(temp.path(), Vec::new(), None);
        let mut session =
            gwt_agent::Session::new(&repo, "work/board-origin", gwt_agent::AgentId::Codex);
        session.id = "session-origin".to_string();
        session.agent_session_id = Some("codex-resume-123".to_string());
        session.model = Some("gpt-5.4".to_string());
        session.reasoning_level = Some("high".to_string());
        session.tool_version = Some("latest".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.save(&runtime.sessions_dir).expect("save session");

        let config = runtime
            .board_origin_agent_resume_config("session-origin")
            .expect("resume config");

        assert_eq!(config.branch.as_deref(), Some("work/board-origin"));
        assert_eq!(config.working_dir.as_deref(), Some(repo.as_path()));
        assert_eq!(
            config.resume_session_id.as_deref(),
            Some("codex-resume-123")
        );
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(config.model.as_deref(), Some("gpt-5.4"));
        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert!(config.skip_permissions);
        assert!(config.codex_fast_mode);
    }

    #[test]
    fn board_origin_agent_resume_config_supports_builtin_agent_descriptors() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let runtime = sample_runtime(temp.path(), Vec::new(), None);

        for agent_id in [gwt_agent::AgentId::OpenClaw, gwt_agent::AgentId::Hermes] {
            let session_id = format!("session-{}", agent_id.command());
            let resume_id = format!("resume-{}", agent_id.command());
            let mut session = gwt_agent::Session::new(
                &repo,
                format!("work/{}", agent_id.command()),
                agent_id.clone(),
            );
            session.id = session_id.clone();
            session.agent_session_id = Some(resume_id.clone());
            session.save(&runtime.sessions_dir).expect("save session");

            let config = runtime
                .board_origin_agent_resume_config(&session_id)
                .expect("resume config");

            assert_eq!(config.agent_id, agent_id);
            assert_eq!(
                config.resume_session_id.as_deref(),
                Some(resume_id.as_str())
            );
            assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        }
    }

    #[test]
    fn app_runtime_open_board_origin_agent_rejects_missing_exact_resume_session() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let board_window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::OpenBoardOriginAgent {
                id: board_window_id.clone(),
                origin_session_id: "missing-session".to_string(),
                bounds: Some(canvas_bounds()),
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardError { id, message },
            }] if client_id == "client-1"
                && id == &board_window_id
                && message.contains("missing-session")
        ));
    }

    #[test]
    fn app_runtime_load_knowledge_bridge_replies_with_cache_backed_issue_and_spec_views() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);

        let cache = Cache::new(issue_cache_root(&repo));
        cache
            .write_snapshot(&sample_issue_snapshot(
                42,
                "Issue bridge",
                &["bug"],
                "Issue body",
                "2026-04-20T10:00:00Z",
            ))
            .expect("write issue snapshot");
        cache
            .write_snapshot(&sample_issue_snapshot(
                1930,
                "SPEC-1930: Cache-backed SPEC bridge",
                &["gwt-spec", "phase/implementation"],
                concat!(
                    "<!-- gwt-spec id=1930 version=1 -->\n",
                    "<!-- sections:\n",
                    "spec=body\n",
                    "tasks=body\n",
                    "-->\n\n",
                    "<!-- artifact:spec BEGIN -->\n",
                    "# SPEC bridge\n",
                    "## Summary\n",
                    "Cache-backed issue view\n",
                    "<!-- artifact:spec END -->\n\n",
                    "<!-- artifact:tasks BEGIN -->\n",
                    "- [x] T-001\n",
                    "<!-- artifact:tasks END -->\n"
                ),
                "2026-04-20T09:00:00Z",
            ))
            .expect("write spec snapshot");
        write_issue_link_store(
            &repo,
            HashMap::from([("feature/issue-bridge".to_string(), 42)]),
        );

        let mut persisted = empty_workspace_state();
        persisted.windows.push(sample_window(
            "issue-1",
            WindowPreset::Issue,
            WindowProcessStatus::Ready,
        ));
        persisted.windows.push(sample_window(
            "spec-1",
            WindowPreset::Spec,
            WindowProcessStatus::Ready,
        ));
        persisted.next_z_index = 3;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo,
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let issue_events = runtime.load_knowledge_bridge_events(
            "client-1",
            KnowledgeLoadRequest {
                id: &combined_window_id("tab-1", "issue-1"),
                kind: gwt::KnowledgeKind::Issue,
                request_id: None,
                selected_number: Some(42),
                refresh: false,
            },
        );
        assert_eq!(issue_events.len(), 2);
        assert!(matches!(
            &issue_events[0].event,
            BackendEvent::KnowledgeEntries {
                knowledge_kind,
                entries,
                selected_number,
                refresh_enabled,
                ..
            } if *knowledge_kind == gwt::KnowledgeKind::Issue
                && entries.len() == 1
                && entries[0].number == 42
                && entries[0].linked_branch_count == 1
                && *selected_number == Some(42)
                && *refresh_enabled
        ));
        assert!(matches!(
            &issue_events[1].event,
            BackendEvent::KnowledgeDetail { detail, .. }
                if detail.launch_issue_number == Some(42)
                    && detail.sections.iter().any(|section| section.title == "Linked branches"
                        && section.body.contains("feature/issue-bridge"))
        ));

        let spec_events = runtime.load_knowledge_bridge_events(
            "client-1",
            KnowledgeLoadRequest {
                id: &combined_window_id("tab-1", "spec-1"),
                kind: gwt::KnowledgeKind::Spec,
                request_id: None,
                selected_number: Some(1930),
                refresh: false,
            },
        );
        assert_eq!(spec_events.len(), 2);
        assert!(matches!(
            &spec_events[0].event,
            BackendEvent::KnowledgeEntries {
                knowledge_kind,
                entries,
                selected_number,
                refresh_enabled,
                ..
            } if *knowledge_kind == gwt::KnowledgeKind::Spec
                && entries.len() == 1
                && entries[0].number == 1930
                && *selected_number == Some(1930)
                && *refresh_enabled
        ));
        assert!(matches!(
            &spec_events[1].event,
            BackendEvent::KnowledgeDetail { detail, .. }
                if detail.sections.iter().any(|section| section.title == "spec"
                    && section.body.contains("Cache-backed issue view"))
        ));
    }

    #[test]
    fn app_runtime_knowledge_search_errors_for_wrong_surface() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);

        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "shell-1",
            repo,
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        );
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "shell-1");
        let events = runtime.search_knowledge_bridge_events(
            "client-1",
            KnowledgeSearchRequest {
                id: &window_id,
                kind: gwt::KnowledgeKind::Issue,
                query: "semantic query",
                request_id: 9,
                selected_number: None,
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::KnowledgeError {
                    knowledge_kind,
                    message,
                    ..
                },
            }] if client_id == "client-1"
                && *knowledge_kind == gwt::KnowledgeKind::Issue
                && message == "Window is not a knowledge bridge"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn app_runtime_knowledge_search_replies_through_async_dispatch() {
        let _lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        write_fake_project_index_runtime(temp.path());

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);

        let cache = Cache::new(issue_cache_root(&repo));
        cache
            .write_snapshot(&sample_issue_snapshot(
                42,
                "Async semantic issue",
                &["bug"],
                "Search result body",
                "2026-04-20T10:00:00Z",
            ))
            .expect("write issue snapshot");

        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "issue-1",
            repo,
            WindowPreset::Issue,
            WindowProcessStatus::Ready,
        );
        let (mut runtime, events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "issue-1");

        let immediate_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SearchKnowledgeBridge {
                id: window_id.clone(),
                knowledge_kind: gwt::KnowledgeKind::Issue,
                query: "semantic query".to_string(),
                request_id: 9,
                selected_number: None,
            },
        );

        assert!(
            immediate_events.is_empty(),
            "semantic search must not reply on the frontend event loop"
        );
        wait_for_recorded_event("knowledge search dispatch", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.target,
                                DispatchTarget::Client(client_id) if client_id == "client-1"
                            ) && matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeSearchResults {
                                    id,
                                    knowledge_kind,
                                    query,
                                    request_id,
                                    entries,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && query == "semantic query"
                                    && *request_id == 9
                                    && entries.len() == 1
                                    && entries[0].number == 42
                            )
                        })
                )
            })
        });
    }

    #[test]
    fn app_runtime_manual_knowledge_refresh_replies_through_async_dispatch() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "issue-1",
            repo,
            WindowPreset::Issue,
            WindowProcessStatus::Ready,
        );
        let (mut runtime, events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "issue-1");

        let immediate_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadKnowledgeBridge {
                id: window_id.clone(),
                knowledge_kind: gwt::KnowledgeKind::Issue,
                request_id: Some(31),
                selected_number: Some(43),
                refresh: true,
            },
        );

        assert!(
            immediate_events.is_empty(),
            "manual refresh must not block the frontend event loop"
        );
        wait_for_recorded_event("manual knowledge refresh dispatch", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.target,
                                DispatchTarget::Client(client_id) if client_id == "client-1"
                            ) && matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeEntries {
                                    id,
                                    knowledge_kind,
                                    request_id,
                                    entries,
                                    selected_number,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && *request_id == Some(31)
                                    && *selected_number == Some(43)
                                    && entries.len() == 1
                                    && entries[0].number == 43
                            )
                        }) && dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeDetail {
                                    id,
                                    request_id,
                                    detail,
                                    ..
                                } if id == &window_id
                                    && *request_id == Some(31)
                                    && detail.number == Some(43)
                            )
                        })
                )
            })
        });
    }

    #[test]
    fn app_runtime_manual_knowledge_refresh_error_preserves_request_context() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "issue-1",
            repo,
            WindowPreset::Issue,
            WindowProcessStatus::Ready,
        );
        let (mut runtime, events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "issue-1");

        let immediate_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadKnowledgeBridge {
                id: window_id.clone(),
                knowledge_kind: gwt::KnowledgeKind::Issue,
                request_id: Some(32),
                selected_number: None,
                refresh: true,
            },
        );

        assert!(
            immediate_events.is_empty(),
            "manual refresh errors must be reported asynchronously"
        );
        wait_for_recorded_event("manual knowledge refresh error", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| {
                            matches!(
                                &outbound.event,
                                BackendEvent::KnowledgeError {
                                    id,
                                    knowledge_kind,
                                    request_id,
                                    message,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && *request_id == Some(32)
                                    && message.contains("gh refresh failed")
                            )
                        })
                )
            })
        });
    }

    #[test]
    fn app_runtime_background_knowledge_refresh_silent_paths_do_not_dispatch() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let fake_gh = write_fake_gh_issue_list(temp.path());
        let _path = prepend_fake_gh_to_path(&fake_gh);
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);
        let marker = temp.path().join("fake-gh-called");
        let _marker = ScopedEnvVar::set("GWT_FAKE_GH_MARKER", &marker);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "issue-1",
            repo.clone(),
            WindowPreset::Issue,
            WindowProcessStatus::Ready,
        );
        let (runtime, events) = sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "issue-1");

        let mode_guard = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "fail");
        runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
            client_id: "client-1".to_string(),
            id: window_id.clone(),
            project_root: repo,
            kind: gwt::KnowledgeKind::Issue,
            request_id: Some(33),
            selected_number: None,
            force: false,
        });
        wait_for_path("stale knowledge refresh gh invocation", &marker);
        assert!(
            events.lock().expect("event log").is_empty(),
            "background refresh errors should not overwrite the current cache view"
        );

        fs::remove_file(&marker).expect("remove marker");
        drop(mode_guard);
        let _mode = ScopedEnvVar::set("GWT_FAKE_GH_MODE", "ok");

        runtime.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
            client_id: "client-1".to_string(),
            id: window_id,
            project_root: temp.path().join("missing-repo"),
            kind: gwt::KnowledgeKind::Issue,
            request_id: Some(34),
            selected_number: Some(43),
            force: false,
        });
        thread::sleep(Duration::from_millis(250));
        assert!(
            events.lock().expect("event log").is_empty(),
            "noop background refresh should return silently without dispatch"
        );
    }

    #[test]
    fn app_runtime_load_knowledge_bridge_keeps_pr_surface_disabled_until_cache_support_exists() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);

        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "pr-1",
            repo,
            WindowPreset::Pr,
            WindowProcessStatus::Ready,
        );
        let runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.load_knowledge_bridge_events(
            "client-1",
            KnowledgeLoadRequest {
                id: &combined_window_id("tab-1", "pr-1"),
                kind: gwt::KnowledgeKind::Pr,
                request_id: None,
                selected_number: None,
                refresh: false,
            },
        );

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0].event,
            BackendEvent::KnowledgeEntries {
                knowledge_kind,
                entries,
                refresh_enabled,
                empty_message,
                ..
            } if *knowledge_kind == gwt::KnowledgeKind::Pr
                && entries.is_empty()
                && !*refresh_enabled
                && empty_message.as_deref().is_some_and(|message| message.contains("cache-backed PR list support"))
        ));
        assert!(matches!(
            &events[1].event,
            BackendEvent::KnowledgeDetail { detail, .. }
                if detail.sections.iter().any(|section| section.body.contains("cache-backed PR list support"))
        ));
    }

    #[test]
    fn app_runtime_load_profile_replies_with_config_backed_snapshot() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("profile-config.toml");
        let mut settings = Settings::default();
        settings
            .profiles
            .add(Profile::new("dev"))
            .expect("add profile");
        settings.profiles.switch("dev").expect("switch active");
        settings
            .profiles
            .set_env_var("dev", "API_KEY", "override")
            .expect("set env var");
        write_profile_config(&config_path, &settings);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "profile-1",
            repo,
            WindowPreset::Profile,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "profile-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadProfile {
                id: window_id.clone(),
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::ProfileSnapshot { id, snapshot },
            }] if client_id == "client-1"
                && id == &window_id
                && snapshot.active_profile == "dev"
                && snapshot.selected_profile == "dev"
                && snapshot.profiles.iter().any(|profile|
                    profile.name == "dev"
                        && profile.is_active
                        && profile.env_vars.iter().any(|entry|
                            entry.key == "API_KEY" && entry.value == "override"
                        )
                )
        ));
    }

    #[test]
    fn app_runtime_select_and_save_profile_broadcasts_snapshot_to_profile_windows() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("profile-config.toml");
        let mut settings = Settings::default();
        settings
            .profiles
            .add(Profile::new("dev"))
            .expect("add profile");
        write_profile_config(&config_path, &settings);

        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut persisted = empty_workspace_state();
        persisted.windows.push(sample_window(
            "profile-1",
            WindowPreset::Profile,
            WindowProcessStatus::Ready,
        ));
        persisted.windows.push(sample_window(
            "profile-2",
            WindowPreset::Profile,
            WindowProcessStatus::Ready,
        ));
        persisted.next_z_index = 3;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo,
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let current_window_id = combined_window_id("tab-1", "profile-1");
        let sibling_window_id = combined_window_id("tab-1", "profile-2");

        let select_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SelectProfile {
                id: current_window_id.clone(),
                profile_name: "dev".to_string(),
            },
        );
        assert!(matches!(
            &select_events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::ProfileSnapshot { id, snapshot },
            }] if client_id == "client-1"
                && id == &current_window_id
                && snapshot.selected_profile == "dev"
        ));

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SaveProfile {
                id: current_window_id.clone(),
                current_name: "dev".to_string(),
                name: "review".to_string(),
                description: "Review profile".to_string(),
                env_vars: vec![ProfileEnvEntryView {
                    key: "API_KEY".to_string(),
                    value: "override".to_string(),
                }],
                disabled_env: vec!["SECRET".to_string()],
            },
        );

        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ProfileSnapshot { id, snapshot },
            } if id == &current_window_id
                && snapshot.selected_profile == "review"
                && snapshot.active_profile == "default"
                && snapshot.profiles.iter().any(|profile|
                    profile.name == "review"
                        && profile.env_vars.iter().any(|entry|
                            entry.key == "API_KEY" && entry.value == "override"
                        )
                )
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ProfileSnapshot { id, snapshot },
            } if id == &sibling_window_id
                && snapshot.selected_profile == "default"
                && snapshot.profiles.iter().any(|profile| profile.name == "review")
        )));

        let saved = Settings::load_from_path(&config_path).expect("load saved config");
        assert!(saved
            .profiles
            .profiles
            .iter()
            .any(|profile| profile.name == "review" && profile.description == "Review profile"));
    }

    #[test]
    fn app_runtime_logs_profile_save_user_action_without_env_values() {
        let temp = tempdir().expect("tempdir");
        let _config_home = ScopedEnvVar::set("GWT_CONFIG_HOME", temp.path());
        Settings::default()
            .save(&temp.path().join("config.toml"))
            .expect("save settings");

        let mut runtime = sample_runtime(temp.path(), vec![], None);
        let events = capture_tracing_events(|| {
            let _ = runtime.handle_frontend_event(
                "client-1".to_string(),
                FrontendEvent::SaveProfile {
                    id: "profile-window".to_string(),
                    current_name: "default".to_string(),
                    name: "default".to_string(),
                    description: String::new(),
                    env_vars: vec![ProfileEnvEntryView {
                        key: "Test".to_string(),
                        value: "must-not-leak".to_string(),
                    }],
                    disabled_env: vec![],
                },
            );
        });

        let action = events
            .iter()
            .find(|event| event.target == "gwt_ui_action")
            .expect("profile save user action log");
        assert_eq!(action.level, Level::INFO);
        assert_eq!(
            action.fields.get("action").map(String::as_str),
            Some("save_profile")
        );
        assert_eq!(
            action.fields.get("profile_name").map(String::as_str),
            Some("default")
        );
        assert_eq!(
            action.fields.get("env_keys").map(String::as_str),
            Some("Test")
        );
        assert_eq!(
            action.fields.get("env_var_count").map(String::as_str),
            Some("1")
        );
        assert!(
            !action
                .fields
                .values()
                .any(|value| value.contains("must-not-leak")),
            "env values must not be written to the user action log: {action:?}"
        );
    }

    #[test]
    fn frontend_user_action_redacts_backend_test_url_secrets() {
        let custom_agent_log =
            super::frontend_user_action_log(&FrontendEvent::TestBackendConnection {
                base_url: "https://user:pass@example.com/v1?token=secret#frag".to_string(),
                api_key: "api-key-must-not-leak".to_string(),
            })
            .expect("custom agent backend test action log");
        assert_eq!(custom_agent_log.ui_target, "https://example.com");

        let builtin_agent_log =
            super::frontend_user_action_log(&FrontendEvent::TestAgentBackendConnection {
                agent: gwt_agent::BuiltinAgentId::Codex,
                base_url: "http://token@example.net:11434/openai?signed=secret".to_string(),
                api_key: "agent-key-must-not-leak".to_string(),
            })
            .expect("builtin agent backend test action log");
        assert_eq!(builtin_agent_log.ui_target, "http://example.net:11434");

        let logged_values = [
            custom_agent_log.ui_target.as_str(),
            builtin_agent_log.ui_target.as_str(),
        ];
        assert!(
            !logged_values.iter().any(|value| value.contains("user")
                || value.contains("pass")
                || value.contains("token")
                || value.contains("secret")),
            "backend test URLs must not leak credentials or query strings: {logged_values:?}"
        );
    }

    #[test]
    fn frontend_user_action_logs_project_index_search_without_query_values() {
        let log = super::frontend_user_action_log(&FrontendEvent::SearchProjectIndex {
            id: "index-window".to_string(),
            query: "secret query".to_string(),
            request_id: 7,
            scopes: vec![
                gwt::IndexSearchScope::Issues,
                gwt::IndexSearchScope::FilesDocs,
            ],
            worktree_hash: Some("worktree-hash".to_string()),
            match_mode: gwt::IndexSearchMatchMode::AllTerms,
        })
        .expect("project index search user action log");

        assert_eq!(log.action, "search_project_index");
        assert_eq!(log.surface, "index");
        assert_eq!(log.window_id, "index-window");
        assert_eq!(log.mode, "files-docs,issues");
        assert_eq!(log.agent_id, "worktree-hash");
        assert_eq!(log.count, "secret query".len());

        let logged_values = [
            log.window_id.as_str(),
            log.ui_target.as_str(),
            log.profile_name.as_str(),
            log.env_keys.as_str(),
            log.agent_id.as_str(),
            log.mode.as_str(),
        ];
        assert!(
            !logged_values.iter().any(|value| value.contains("secret")),
            "project index search query must not be written to the user action log: {logged_values:?}"
        );
    }

    #[test]
    fn app_runtime_load_logs_replies_with_current_log_snapshot() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "logs-1",
            repo,
            WindowPreset::Logs,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "logs-1");
        let log_path = current_log_file(&runtime.log_dir);
        // Write the canonical on-disk JSONL shape produced by
        // `tracing_subscriber::fmt::layer().json()` (see
        // `crates/gwt-core/src/logging/fmt_layer.rs`) so the reader exercises
        // the production format end-to-end (SPEC-1924 FR-035).
        fs::write(
            &log_path,
            "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
             \"level\":\"WARN\",\
             \"fields\":{\"message\":\"runtime stalled\",\"detail\":\"retrying read\"},\
             \"target\":\"pty\"}\n",
        )
        .expect("write log snapshot");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadLogs {
                id: window_id.clone(),
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::LogEntries { id, entries },
            }] if client_id == "client-1"
                && id == &window_id
                && entries.len() == 1
                && entries[0].message == "runtime stalled"
                && entries[0].detail.as_deref() == Some("retrying read")
                && entries[0].source == "pty"
                && matches!(entries[0].severity, LogLevel::Warn)
        ));
    }

    /// SPEC-1924 US-14 / FR-036 / SC-010 — when canonical log file contains
    /// malformed lines, the Logs window receives the surviving entries plus
    /// exactly one Warning notice via `LogEntryAppended`.
    #[test]
    fn app_runtime_load_logs_emits_warning_for_skipped_lines() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "logs-1",
            repo,
            WindowPreset::Logs,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "logs-1");
        let log_path = current_log_file(&runtime.log_dir);
        let good = "{\"timestamp\":\"2026-05-20T09:00:00.000000+00:00\",\
            \"level\":\"INFO\",\"fields\":{\"message\":\"ok\"},\"target\":\"gwt\"}";
        let malformed = "{\"foo\":\"bar\"}";
        fs::write(&log_path, format!("{good}\n{malformed}\n{good}\n")).expect("write log snapshot");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadLogs {
                id: window_id.clone(),
            },
        );

        assert_eq!(
            events.len(),
            2,
            "expected LogEntries + LogEntryAppended for skipped notice, got {:?}",
            events
        );

        let entries_match = matches!(
            &events[0],
            OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::LogEntries { id, entries },
            } if client_id == "client-1"
                && id == &window_id
                && entries.len() == 2
                && entries.iter().all(|e| e.message == "ok")
        );
        assert!(
            entries_match,
            "first event must be LogEntries: {:?}",
            events[0]
        );

        let warning_match = matches!(
            &events[1],
            OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::LogEntryAppended { entry },
            } if client_id == "client-1"
                && entry.severity == LogLevel::Warn
                && entry.source == "gwt_core::logging::reader"
                && entry.message.contains("Skipped 1 malformed line")
        );
        assert!(
            warning_match,
            "second event must be a Warn LogEntryAppended notice: {:?}",
            events[1]
        );
    }

    #[test]
    fn app_runtime_save_ui_trace_replies_with_artifact_path() {
        let temp = tempdir().expect("tempdir");
        let mut runtime = sample_runtime(temp.path(), vec![], None);

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::SaveUiTrace {
                trace: serde_json::from_value::<UiTracePayload>(serde_json::json!({
                    "session_id": "trace-1",
                    "entries": [
                        { "kind": "trace_start", "ts": 1 }
                    ]
                }))
                .expect("typed ui trace payload"),
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::UiTraceSaved { path, entries },
            }] if client_id == "client-1" && *entries == 1 && Path::new(path).exists()
        ));
    }

    #[test]
    fn app_runtime_post_board_entry_persists_reply_topics_and_owners() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let parent = post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Question,
                "Can someone verify this?",
                None,
                None,
                vec!["coordination".to_string()],
                vec!["2018".to_string()],
            ),
        )
        .expect("seed board parent")
        .board
        .entries
        .into_iter()
        .next()
        .expect("parent entry");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo.clone(),
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::PostBoardEntry {
                id: window_id.clone(),
                entry_kind: BoardEntryKind::Next,
                body: "I will take the next slice".to_string(),
                parent_id: Some(parent.id.clone()),
                topics: vec!["coordination".to_string(), "phase-1b".to_string()],
                owners: vec!["2018".to_string()],
                targets: Vec::new(),
                mentions: vec![
                    BoardMention::new(BoardMentionTargetKind::User, "akiojin").with_label("Akio")
                ],
            },
        );

        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardEntries { id, entries, .. },
            } if client_id == "client-1"
                && id == &window_id
                && entries.iter().any(|entry|
                    entry.body == "I will take the next slice"
                    && entry.parent_id.as_deref() == Some(parent.id.as_str())
                    && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
                    && entry.related_owners == vec!["2018".to_string()]
                    && entry.mentions.len() == 1
                    && entry.mentions[0].typed_key() == "user:akiojin"
                )
        )));

        let snapshot = load_snapshot(&repo).expect("load board snapshot");
        assert!(snapshot.board.entries.iter().any(|entry| entry.body
            == "I will take the next slice"
            && entry.parent_id.as_deref() == Some(parent.id.as_str())
            && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
            && entry.related_owners == vec!["2018".to_string()]
            && entry.mentions.len() == 1
            && entry.mentions[0].typed_key() == "user:akiojin"));
    }

    #[test]
    fn app_runtime_post_board_entry_accepts_reply_to_history_parent() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let parent_id = "history-parent".to_string();
        let events_path = coordination_events_path(&repo);
        fs::create_dir_all(
            events_path
                .parent()
                .expect("coordination event log has parent"),
        )
        .expect("create coordination dir");
        let mut events = fs::File::create(&events_path).expect("create legacy event log");
        for idx in 0..505 {
            let mut entry = BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                format!("history entry {idx}"),
                None,
                None,
                vec![],
                vec![],
            );
            if idx == 0 {
                entry.id = parent_id.clone();
            }
            serde_json::to_writer(&mut events, &CoordinationEvent::MessageAppended { entry })
                .expect("write board seed event");
            events.write_all(b"\n").expect("write board seed newline");
        }
        events.flush().expect("flush board seed events");
        let snapshot = load_snapshot(&repo).expect("load board snapshot");
        assert!(!snapshot
            .board
            .entries
            .iter()
            .any(|entry| entry.id == parent_id));
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo.clone(),
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::PostBoardEntry {
                id: window_id.clone(),
                entry_kind: BoardEntryKind::Next,
                body: "Reply to older context".to_string(),
                parent_id: Some(parent_id.clone()),
                topics: vec![],
                owners: vec![],
                targets: Vec::new(),
                mentions: Vec::new(),
            },
        );

        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardEntries { id, entries, .. },
            } if client_id == "client-1"
                && id == &window_id
                && entries.iter().any(|entry|
                    entry.body == "Reply to older context"
                    && entry.parent_id.as_deref() == Some(parent_id.as_str())
                )
        )));
    }

    #[test]
    fn app_runtime_post_board_entry_updates_workspace_projection_current_state() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo.clone(),
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "board-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::PostBoardEntry {
                id: window_id,
                entry_kind: BoardEntryKind::Next,
                body: "Run final verification".to_string(),
                parent_id: None,
                topics: vec!["start-work".to_string()],
                owners: vec!["SPEC-2359".to_string()],
                targets: Vec::new(),
                mentions: Vec::new(),
            },
        );

        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let board_entry_id = projection
            .board_refs
            .first()
            .expect("workspace board ref")
            .clone();

        assert_eq!(
            projection.next_action.as_deref(),
            Some("Run final verification")
        );
        assert_eq!(projection.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(projection.board_refs.len(), 1);
        let work_items = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load work items")
            .expect("work items");
        assert_eq!(
            work_items.work_items[0].board_refs,
            vec![board_entry_id.clone()]
        );
        assert_eq!(
            work_items.work_items[0].events[0].kind,
            gwt_core::workspace_projection::WorkspaceWorkEventKind::Update
        );
        let projected_event = events
            .iter()
            .find_map(|event| match event {
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::ActiveWorkProjection { projection },
                } => Some(projection),
                _ => None,
            })
            .expect("active work projection broadcast");
        assert_eq!(projected_event.board_refs, vec![board_entry_id.clone()]);
        assert_eq!(projected_event.active_agents, 0);
        assert_eq!(projected_event.status_category, "idle");
        assert_eq!(projected_event.next_action, None);
    }

    #[test]
    fn app_runtime_board_milestone_from_unassigned_origin_does_not_create_workspace_history() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "board-1",
            repo.clone(),
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "session-unassigned".to_string(),
                window_id: None,
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some("Investigate Workspace materialization".to_string()),
                title_summary: Some("Workspace materialization".to_string()),
                worktree_path: None,
                branch: Some("work/unassigned".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let entry = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Claim,
            "Unassigned claim without materialization must not pollute current Workspace history.",
            None,
            None,
            vec!["workspace-materialization".to_string()],
            vec!["2359".to_string()],
        )
        .with_title_summary("Workspace materialization")
        .with_origin_session_id("session-unassigned");

        runtime.record_workspace_board_milestone_event("tab-1", &repo, &entry);

        let saved = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-unassigned")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned
        );
        assert!(
            gwt_core::workspace_projection::load_workspace_work_items(&repo)
                .expect("load workspace history")
                .is_none(),
            "Unassigned origin Board entries must not append to unrelated Workspace history"
        );
    }

    #[test]
    fn app_runtime_active_work_projection_preserves_blocked_agent_board_state() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("repo-work-20260504-1234");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let session = ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        };
        runtime
            .active_agent_sessions
            .insert(session.window_id.clone(), session.clone());
        save_assigned_workspace_projection_for_test(&repo, &session)
            .expect("save initial projection");
        let blocked = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Blocked,
            "Waiting for API credentials",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1")
        .with_origin_agent_id("codex")
        .with_origin_branch("work/20260504-1234");

        let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
        let event = events
            .iter()
            .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
            .cloned()
            .expect("active projection broadcast");

        assert!(matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } if projection.status_category == "blocked"
                && projection.blocked_agents == 1
                && projection.agents.iter().any(|agent|
                    agent.session_id == "session-1"
                        && agent.status_category == "blocked"
                        && agent.last_board_entry_id.as_deref() == Some(blocked.id.as_str())
                )
                && projection.board_refs == vec![blocked.id.clone()]
                && projection.next_action.as_deref() == Some("Resolve blocker")
        ));
    }

    #[test]
    fn app_runtime_active_work_projection_prioritizes_handoff_agents() {
        use gwt_core::workspace_projection::{
            WorkspaceAgentSummary, WorkspaceProjection, WorkspaceStatusCategory,
        };

        let mut projection = WorkspaceProjection::default_for_project("/repo");
        let now = chrono::Utc::now();
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "session-active".to_string(),
            window_id: Some("tab-1::agent-active".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Alpha".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Implementing tests".to_string()),
            title_summary: None,
            worktree_path: None,
            branch: Some("work/20260504-1234".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: now,
        });
        projection.agents.push(WorkspaceAgentSummary {
            session_id: "session-handoff".to_string(),
            window_id: Some("tab-1::agent-handoff".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Zulu".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Review visual state coverage".to_string()),
            title_summary: None,
            worktree_path: None,
            branch: Some("work/20260504-1234".to_string()),
            last_board_entry_id: Some("board-handoff".to_string()),
            last_board_entry_kind: Some(BoardEntryKind::Handoff),
            coordination_scope: Some("SPEC-2359 / workspace-ux".to_string()),
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: now,
        });

        let view = active_work_projection_from_saved(projection);

        assert_eq!(view.agents[0].session_id, "session-handoff");
        assert_eq!(
            view.agents[0].last_board_entry_kind.as_deref(),
            Some("handoff")
        );
        assert_eq!(
            view.agents[0].coordination_scope.as_deref(),
            Some("SPEC-2359 / workspace-ux")
        );
    }

    #[test]
    fn app_runtime_active_work_projection_recovers_blocked_agent_after_status_milestone() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("repo-work-20260504-1234");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let session = ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        };
        runtime
            .active_agent_sessions
            .insert(session.window_id.clone(), session.clone());
        save_assigned_workspace_projection_for_test(&repo, &session)
            .expect("save initial projection");
        let blocked = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Blocked,
            "Waiting for API credentials",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1");
        runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
        let status = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "API credentials configured",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1");

        let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &status);
        let event = events
            .iter()
            .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
            .cloned()
            .expect("active projection broadcast");

        assert!(matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } if projection.status_category == "active"
                && projection.active_agents == 1
                && projection.blocked_agents == 0
                && projection.branch.as_deref() == Some("work/20260504-1234")
        ));
    }

    #[test]
    fn app_runtime_active_work_projection_keeps_blocked_agent_after_next_milestone() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("repo-work-20260504-1234");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&worktree).expect("create worktree");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Board],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let session = ActiveAgentSession {
            window_id: "tab-1::agent-1".to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "work/20260504-1234".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: worktree.clone(),
            agent_project_root: worktree.display().to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            tab_id: "tab-1".to_string(),
        };
        runtime
            .active_agent_sessions
            .insert(session.window_id.clone(), session.clone());
        save_assigned_workspace_projection_for_test(&repo, &session)
            .expect("save initial projection");
        let blocked = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Blocked,
            "Waiting for API credentials",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1");
        runtime.record_workspace_board_milestone_event("tab-1", &repo, &blocked);
        let next = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Next,
            "Try alternate credential source",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1");

        let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &next);
        let event = events
            .iter()
            .find(|e| matches!(e.event, BackendEvent::ActiveWorkProjection { .. }))
            .cloned()
            .expect("blocked projection broadcast");

        assert!(matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::ActiveWorkProjection { projection },
            } if projection.status_category == "blocked"
                && projection.active_agents == 0
                && projection.blocked_agents == 1
                && projection.status_text == "Waiting for API credentials"
                && projection.next_action.as_deref() == Some("Try alternate credential source")
                && projection.branch.as_deref() == Some("work/20260504-1234")
        ));
    }

    #[test]
    fn app_runtime_agent_window_initial_title_uses_linked_issue_title() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        Cache::new(issue_cache_root(&repo))
            .write_snapshot(&sample_issue_snapshot(
                2359,
                "SPEC: Workspace purpose titles",
                &["gwt-spec"],
                "Spec body",
                "2026-05-06T00:00:00Z",
            ))
            .expect("write issue cache");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260506-0736")
            .linked_issue_number(2359)
            .build();

        runtime
            .spawn_agent_window("tab-1", config, canvas_bounds(), None)
            .expect("spawn agent window");

        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.preset == WindowPreset::Agent)
            .expect("agent window");
        assert_eq!(
            agent_window.purpose_title.as_deref(),
            Some("SPEC: Workspace purpose titles")
        );
        assert_eq!(agent_window.title, "Codex");
    }

    #[test]
    fn app_runtime_agent_window_initial_state_broadcast_includes_agent_id() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode).build();

        let events = runtime
            .spawn_agent_window("tab-1", config, canvas_bounds(), None)
            .expect("spawn agent window");

        let workspace = events
            .iter()
            .find_map(|event| match &event.event {
                BackendEvent::WorkspaceState { workspace } => Some(workspace),
                _ => None,
            })
            .expect("initial WorkspaceState broadcast");
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == "tab-1")
            .expect("tab in WorkspaceState");
        let agent_window = tab
            .workspace
            .windows
            .iter()
            .find(|window| window.preset == WindowPreset::Agent)
            .expect("agent window in WorkspaceState");

        assert_eq!(agent_window.title, "Claude Code");
        assert_eq!(agent_window.agent_id.as_deref(), Some("claude"));
    }

    #[test]
    fn app_runtime_agent_window_initial_title_uses_branch_issue_link_title() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        Cache::new(issue_cache_root(&repo))
            .write_snapshot(&sample_issue_snapshot(
                2468,
                "SPEC: Branch linked purpose",
                &["gwt-spec"],
                "Spec body",
                "2026-05-06T00:00:00Z",
            ))
            .expect("write issue cache");
        write_issue_link_store(
            &repo,
            HashMap::from([("work/20260506-1257".to_string(), 2468)]),
        );
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260506-1257")
            .build();

        runtime
            .spawn_agent_window("tab-1", config, canvas_bounds(), None)
            .expect("spawn agent window");

        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.preset == WindowPreset::Agent)
            .expect("agent window");
        assert_eq!(
            agent_window.purpose_title.as_deref(),
            Some("SPEC: Branch linked purpose")
        );
        assert_eq!(agent_window.title, "Codex");
    }

    #[test]
    fn app_runtime_agent_window_initial_title_falls_back_to_projection_owner() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.owner = Some("SPEC-2008".to_string());
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/20260506-1257".to_string()),
            worktree_path: Some(repo.join("work/20260506-1257")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: None,
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260506-1257")
            .build();

        runtime
            .spawn_agent_window("tab-1", config, canvas_bounds(), None)
            .expect("spawn agent window");

        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.preset == WindowPreset::Agent)
            .expect("agent window");
        assert_eq!(agent_window.purpose_title.as_deref(), Some("SPEC-2008"));
        assert_eq!(agent_window.title, "Codex");
    }

    #[test]
    fn app_runtime_agent_window_initial_title_ignores_projection_owner_for_other_branch() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        init_repo(&repo);
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.owner = Some("PR-2525".to_string());
        projection.git_details = Some(gwt_core::workspace_projection::GitDetails {
            branch: Some("work/old".to_string()),
            worktree_path: Some(repo.join("work-old")),
            base_branch: Some("origin/develop".to_string()),
            pr_number: Some(2525),
            pr_state: None,
            pr_url: None,
            pr_created_at: None,
            created_by_start_work: true,
            created_at: chrono::Utc::now(),
        });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .branch("work/20260507-0714")
            .build();

        runtime
            .spawn_agent_window("tab-1", config, canvas_bounds(), None)
            .expect("spawn agent window");

        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|window| window.preset == WindowPreset::Agent)
            .expect("agent window");
        assert_eq!(agent_window.purpose_title, None);
        assert_eq!(agent_window.title, "Codex");
    }

    #[test]
    fn app_runtime_board_milestone_updates_same_session_agent_window_dynamic_title_only() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent_one =
            sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent_one.title = "Codex".to_string();
        agent_one.purpose_title = Some("Initial purpose".to_string());
        let mut agent_two =
            sample_window("agent-2", WindowPreset::Agent, WindowProcessStatus::Running);
        agent_two.title = "Claude".to_string();
        agent_two.purpose_title = Some("Other purpose".to_string());
        tab_workspace.windows.push(agent_one);
        tab_workspace.windows.push(agent_two);
        tab_workspace.next_z_index = 3;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let first_window_id = combined_window_id("tab-1", "agent-1");
        let second_window_id = combined_window_id("tab-1", "agent-2");
        runtime.active_agent_sessions.insert(
            first_window_id.clone(),
            ActiveAgentSession {
                window_id: first_window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260506-0736".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        runtime.active_agent_sessions.insert(
            second_window_id,
            ActiveAgentSession {
                window_id: combined_window_id("tab-1", "agent-2"),
                session_id: "session-2".to_string(),
                agent_id: "claude".to_string(),
                branch_name: "work/20260506-0737".to_string(),
                display_name: "Claude".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        save_start_work_workspace_projection(
            &repo,
            runtime
                .active_agent_sessions
                .get(&first_window_id)
                .expect("first session"),
            "develop",
            None,
            None,
        )
        .expect("save projection");
        let milestone = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Implement dynamic title sync with detailed workspace context",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1")
        .with_title_summary("Implement dynamic title sync");

        runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

        let tab = runtime.tab("tab-1").expect("tab");
        assert_eq!(
            tab.workspace
                .window("agent-1")
                .expect("agent 1")
                .dynamic_title
                .as_deref(),
            Some("Implement dynamic title sync")
        );
        assert_eq!(
            tab.workspace
                .window("agent-1")
                .expect("agent 1")
                .dynamic_title_detail
                .as_deref(),
            Some("Implement dynamic title sync with detailed workspace context")
        );
        assert_eq!(
            tab.workspace
                .window("agent-2")
                .expect("agent 2")
                .dynamic_title
                .as_deref(),
            None
        );
    }

    /// Phase U-5 (SPEC-2359 US-38, FR-125, FR-126): a Board post that carries
    /// `title_summary` must broadcast both `WorkspaceState` (so the pane
    /// heading rehydrates on WS reconnect / GUI reload) and
    /// `ActiveWorkProjection` (Active Work card) in the same batch. Prior to
    /// Phase U-5 the Board path mutated `dynamic_title` in memory but
    /// silently skipped the `WorkspaceState` broadcast, leaving the pane
    /// heading stale after reconnect.
    #[test]
    fn app_runtime_board_milestone_broadcasts_workspace_state_for_title_sync() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        agent.purpose_title = Some("Initial purpose".to_string());
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260513-0343".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        save_start_work_workspace_projection(
            &repo,
            runtime
                .active_agent_sessions
                .get(&window_id)
                .expect("session"),
            "develop",
            None,
            None,
        )
        .expect("save projection");
        let milestone = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Implementing Phase U-5 Board path title sync hardening",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1")
        .with_title_summary("Implementing Phase U-5");

        let events = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "expected WorkspaceState broadcast from Board path so pane heading refreshes on reconnect: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "expected ActiveWorkProjection broadcast from Board path: {events:?}"
        );
    }

    /// Phase U-5 (SPEC-2359 US-38, FR-129, FR-130): the WebSocket reconnect
    /// path goes through `FrontendEvent::FrontendReady` → `frontend_sync_events`.
    /// The replied `WorkspaceState` must carry each window's `dynamic_title`
    /// and `dynamic_title_detail` so the frontend's `windowDisplayTitle()` can
    /// rehydrate the pane heading without waiting for another mutation. This
    /// test fails if anyone strips `dynamic_title` from the projected
    /// `WorkspaceView`, regressing the reconnect contract.
    #[test]
    fn frontend_sync_events_preserves_window_dynamic_title_for_reconnect_rehydrate() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        agent.purpose_title = Some("Initial purpose".to_string());
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let tab_mut = runtime.tab_mut("tab-1").expect("tab mut");
        tab_mut.workspace.set_dynamic_title_with_detail(
            "agent-1",
            Some("Phase U-5 rehydrate target".to_string()),
            Some("simulated stale state before reconnect".to_string()),
        );

        let events = runtime.frontend_sync_events("client-1");

        let workspace_event = events
            .iter()
            .find(|event| matches!(event.event, BackendEvent::WorkspaceState { .. }))
            .expect("WorkspaceState reply for FrontendReady");
        let workspace = match &workspace_event.event {
            BackendEvent::WorkspaceState { workspace } => workspace,
            _ => unreachable!(),
        };
        let projected_window = workspace
            .tabs
            .iter()
            .find(|tab| tab.id == "tab-1")
            .and_then(|tab| {
                tab.workspace
                    .windows
                    .iter()
                    .find(|window| window.id == combined_window_id("tab-1", "agent-1"))
            })
            .expect("agent window in projected WorkspaceState");
        assert_eq!(
            projected_window.dynamic_title.as_deref(),
            Some("Phase U-5 rehydrate target"),
            "frontend_sync_events must include dynamic_title so reconnect rehydrate restores pane heading"
        );
        assert_eq!(
            projected_window.dynamic_title_detail.as_deref(),
            Some("simulated stale state before reconnect"),
            "frontend_sync_events must include dynamic_title_detail so tooltip survives reconnect"
        );
    }

    /// Phase U-5: re-asserts the diff gate from
    /// `apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs`
    /// at the Board entrypoint. Re-posting an identical milestone (same
    /// `title_summary` + same body for `current_focus`) must not emit a
    /// duplicate `WorkspaceState` broadcast on busy projections.
    #[test]
    fn app_runtime_board_milestone_skips_workspace_state_on_identical_resync() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260513-0343".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        save_start_work_workspace_projection(
            &repo,
            runtime
                .active_agent_sessions
                .get(&window_id)
                .expect("session"),
            "develop",
            None,
            None,
        )
        .expect("save projection");
        let milestone = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "Stable body for current_focus",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1")
        .with_title_summary("Stable title");

        let first = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
        assert!(
            first
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "first Board post should broadcast WorkspaceState: {first:?}"
        );

        let second = runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);
        assert!(
            !second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "second Board post with identical title_summary must not duplicate WorkspaceState: {second:?}"
        );
        assert!(
            second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "ActiveWorkProjection should still broadcast on identical resync: {second:?}"
        );
    }

    #[test]
    fn app_runtime_board_milestone_uses_short_title_summary_not_long_body_for_window_title() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        agent.purpose_title = Some("Initial purpose".to_string());
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260507-0227".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        save_start_work_workspace_projection(
            &repo,
            runtime
                .active_agent_sessions
                .get(&window_id)
                .expect("session"),
            "develop",
            None,
            None,
        )
        .expect("save projection");
        let long_body = "Implementing the title-summary contract across Board, Workspace, runtime synchronization, CLI parsing, hook reminders, and frontend titlebar rendering";
        let mut entry_value = serde_json::to_value(
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                long_body,
                None,
                None,
                vec!["start-work".to_string()],
                vec!["SPEC-2359".to_string()],
            )
            .with_origin_session_id("session-1"),
        )
        .expect("entry json");
        entry_value["title_summary"] = serde_json::json!("Title summary contract");
        let milestone: BoardEntry = serde_json::from_value(entry_value).expect("milestone");

        runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

        let tab = runtime.tab("tab-1").expect("tab");
        assert_eq!(
            tab.workspace
                .window("agent-1")
                .expect("agent")
                .dynamic_title
                .as_deref(),
            Some("Title summary contract")
        );
        assert_eq!(
            tab.workspace
                .window("agent-1")
                .expect("agent")
                .dynamic_title_detail
                .as_deref(),
            Some(long_body)
        );
    }

    #[test]
    fn app_runtime_board_milestone_without_title_summary_keeps_existing_agent_window_title() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        agent.purpose_title = Some("Initial purpose".to_string());
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260507-0227".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        save_start_work_workspace_projection(
            &repo,
            runtime
                .active_agent_sessions
                .get(&window_id)
                .expect("session"),
            "develop",
            None,
            None,
        )
        .expect("save projection");
        let milestone = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Status,
            "This long body should remain detail only and should not become the titlebar text",
            None,
            None,
            vec!["start-work".to_string()],
            vec!["SPEC-2359".to_string()],
        )
        .with_origin_session_id("session-1");

        runtime.record_workspace_board_milestone_event("tab-1", &repo, &milestone);

        let tab = runtime.tab("tab-1").expect("tab");
        assert_eq!(
            tab.workspace
                .window("agent-1")
                .expect("agent")
                .dynamic_title
                .as_deref(),
            None
        );
    }

    #[test]
    fn app_runtime_workspace_projection_change_updates_agent_window_title_summary() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260510-0900".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: repo.display().to_string(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );

        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "session-1".to_string(),
                window_id: Some(window_id),
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: Some(
                    "Implement mandatory Agent title summary updates for Workspace".to_string(),
                ),
                title_summary: Some("Agent title summary guard".to_string()),
                worktree_path: Some(repo.clone()),
                branch: Some("work/20260510-0900".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let events = runtime.handle_workspace_projection_changed_events(&repo);

        assert!(events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })));
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("Agent title summary guard")
        );
        assert_eq!(
            agent_window.dynamic_title_detail.as_deref(),
            Some("Implement mandatory Agent title summary updates for Workspace")
        );
    }

    // ---------------------------------------------------------------------
    // SPEC-2359 US-26 Phase U-1: canonical title-sync orchestration
    // ---------------------------------------------------------------------

    fn apply_title_sync_setup_tab_and_runtime(
        repo: PathBuf,
        active_tab: Option<&str>,
    ) -> (AppRuntime, String) {
        let mut tab_workspace = empty_workspace_state();
        let mut agent = sample_window("agent-1", WindowPreset::Agent, WindowProcessStatus::Running);
        agent.title = "Codex".to_string();
        tab_workspace.windows.push(agent);
        tab_workspace.next_z_index = 2;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        // Need the path so the temp directory survives until the runtime drops.
        let temp_root = repo.parent().expect("repo has parent").to_path_buf();
        let mut runtime = sample_runtime(&temp_root, vec![tab], active_tab);
        let window_id = combined_window_id("tab-1", "agent-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: "session-1".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260510-0900".to_string(),
                display_name: "Codex".to_string(),
                worktree_path: repo,
                agent_project_root: String::new(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        (runtime, window_id)
    }

    fn apply_title_sync_sample_projection(
        repo: &Path,
        window_id: &str,
        title_summary: Option<&str>,
        current_focus: Option<&str>,
    ) -> gwt_core::workspace_projection::WorkspaceProjection {
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(repo);
        projection.status_category =
            gwt_core::workspace_projection::WorkspaceStatusCategory::Active;
        projection
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: "session-1".to_string(),
                window_id: Some(window_id.to_string()),
                agent_id: "codex".to_string(),
                display_name: "Codex".to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: current_focus.map(str::to_string),
                title_summary: title_summary.map(str::to_string),
                worktree_path: Some(repo.to_path_buf()),
                branch: Some("work/20260510-0900".to_string()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: None,
                updated_at: chrono::Utc::now(),
            });
        projection
    }

    #[test]
    fn apply_workspace_projection_title_sync_writes_dynamic_title() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Canonical orchestration"),
            Some("Implement apply_workspace_projection_title_sync"),
        );

        let _events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("Canonical orchestration"),
            "dynamic_title should reflect projection.agents[<i>].title_summary"
        );
        assert_eq!(
            agent_window.dynamic_title_detail.as_deref(),
            Some("Implement apply_workspace_projection_title_sync"),
            "dynamic_title_detail should reflect projection.agents[<i>].current_focus"
        );
    }

    #[test]
    fn apply_workspace_projection_title_sync_emits_active_work_projection_for_active_tab() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // active_work_projection_for_tab loads from disk, so the projection
        // file must exist for the broadcast to populate.
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Phase U-1 broadcast assertion"),
            None,
        );
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "expected ActiveWorkProjection broadcast in events: {events:?}"
        );
    }

    #[test]
    fn apply_workspace_projection_title_sync_returns_no_events_without_active_tab() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) = apply_title_sync_setup_tab_and_runtime(repo.clone(), None);
        let projection =
            apply_title_sync_sample_projection(&repo, &window_id, Some("No active tab"), None);

        let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "without an active tab, ActiveWorkProjection broadcast must be skipped"
        );
        // Even without an active tab, the in-memory dynamic_title should still
        // be synced so the next workspace_state broadcast carries it.
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("No active tab"),
            "dynamic_title must be set in-memory regardless of active_tab routing"
        );
    }

    #[test]
    fn apply_workspace_projection_title_sync_emits_workspace_state_when_dynamic_title_changed() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Phase U-2 WorkspaceState assertion"),
            None,
        );
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

        // Phase U-2 (SPEC-2359 US-26): a workspace update path that mutates
        // an in-memory dynamic_title MUST broadcast WorkspaceState in the
        // same batch so the frontend's `windowData.dynamic_title` and the
        // pane heading `windowDisplayTitle` refresh without waiting for the
        // next hook event or window structure change.
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "expected WorkspaceState broadcast when dynamic_title changed: {events:?}"
        );
    }

    #[test]
    fn apply_workspace_projection_title_sync_skips_workspace_state_when_nothing_changed() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // Drop the active_agent_sessions entry AND erase the projection's
        // window_id so neither the fast path nor the Phase U-3 fallback
        // can resolve a window. The WorkspaceState broadcast should be
        // skipped to avoid forcing a frontend re-render for a no-op update.
        runtime.active_agent_sessions.clear();
        let mut projection =
            apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("No-op"), None);
        projection.agents[0].window_id = None;
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let events = runtime.apply_workspace_projection_title_sync(&repo, &projection);

        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "WorkspaceState must be skipped when in-memory dynamic_title did not change: {events:?}"
        );
    }

    #[test]
    fn apply_workspace_projection_title_sync_skips_workspace_state_when_same_title_resyncs() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Stable title"),
            Some("Stable focus"),
        );
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        // First sync: dynamic_title transitions from None → "Stable title".
        let first = runtime.apply_workspace_projection_title_sync(&repo, &projection);
        assert!(
            first
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "first sync should broadcast WorkspaceState: {first:?}"
        );

        // Second sync with the same projection: nothing diffs, so the
        // WorkspaceState broadcast must be suppressed to avoid forcing a
        // full frontend re-render on busy projections (Codex review P2).
        let second = runtime.apply_workspace_projection_title_sync(&repo, &projection);
        assert!(
            !second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "second sync with identical title must not broadcast WorkspaceState: {second:?}"
        );
        // ActiveWorkProjection still fires (it's idempotent on the
        // frontend; the active card snapshot is harmless to re-send).
        assert!(
            second
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "ActiveWorkProjection should still broadcast on resync: {second:?}"
        );
    }

    #[test]
    fn handle_workspace_projection_changed_events_broadcasts_workspace_state_for_pane_heading() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Pane heading via WorkspaceState"),
            Some("triggered by gwtd workspace update --title-summary"),
        );
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let events = runtime.handle_workspace_projection_changed_events(&repo);

        // The original handler returned only ActiveWorkProjection. Phase
        // U-2 promotes it to also broadcast WorkspaceState in one batch so
        // the pane heading refreshes immediately after `gwtd workspace
        // update --title-summary`.
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "handle_workspace_projection_changed_events must broadcast WorkspaceState: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::ActiveWorkProjection { .. })),
            "ActiveWorkProjection broadcast must still fire: {events:?}"
        );
    }

    #[test]
    fn sync_agent_window_titles_falls_back_to_projection_window_id_when_active_agent_sessions_missing(
    ) {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // Phase U-3: simulate a session that gwt's launch tracking has not
        // registered (e.g. GUI restarted after launch, or session started
        // outside gwt's launch path). The window exists in workspace state
        // and the projection knows its window_id + worktree_path, but
        // active_agent_sessions is empty.
        runtime.active_agent_sessions.clear();
        let projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Phase U-3 backfill via projection window_id"),
            Some("ensures untracked sessions still update pane heading"),
        );

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            changed,
            "Phase U-3: sync must return true when projection-driven fallback resolves the window"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("Phase U-3 backfill via projection window_id"),
            "dynamic_title must propagate even when active_agent_sessions is empty"
        );
        assert_eq!(
            agent_window.dynamic_title_detail.as_deref(),
            Some("ensures untracked sessions still update pane heading")
        );
    }

    #[test]
    fn sync_agent_window_titles_skips_fallback_when_projection_worktree_mismatches() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let other_repo = temp.path().join("other-repo");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&other_repo).expect("create other repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        runtime.active_agent_sessions.clear();
        // Projection claims the agent lives in a *different* worktree.
        // Phase U-3 fallback must refuse to touch local windows in that
        // case, otherwise cross-worktree titles could leak.
        let mut projection = apply_title_sync_sample_projection(
            &repo,
            &window_id,
            Some("Cross-worktree title leak guard"),
            None,
        );
        projection.agents[0].worktree_path = Some(other_repo);

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            !changed,
            "fallback must refuse cross-worktree window updates"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert!(
            agent_window.dynamic_title.is_none(),
            "dynamic_title must stay None when projection.agents[<i>].worktree_path mismatches"
        );
    }

    #[test]
    fn sync_agent_window_titles_skips_fallback_when_projection_window_id_unknown_locally() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        runtime.active_agent_sessions.clear();
        // Projection references a window_id that is NOT present in any
        // local tab. The fallback must short-circuit instead of producing
        // a phantom window update.
        let mut projection =
            apply_title_sync_sample_projection(&repo, "tab-1::agent-1", Some("phantom"), None);
        projection.agents[0].window_id = Some("tab-99::ghost".to_string());

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            !changed,
            "fallback must require the projected window_id to exist locally"
        );
    }

    #[test]
    fn sync_agent_window_titles_returns_false_when_no_resolution_path_exists() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // Drop the active_agent_sessions entry AND erase the projection's
        // window_id so neither the fast path nor the Phase U-3 fallback
        // can resolve this agent's window. The sync must then be a no-op.
        runtime.active_agent_sessions.clear();
        let mut projection = apply_title_sync_sample_projection(
            &repo,
            "tab-1::agent-1",
            Some("Should not propagate"),
            None,
        );
        projection.agents[0].window_id = None;

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            !changed,
            "sync must return false when no in-memory window was touched"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert!(
            agent_window.dynamic_title.is_none(),
            "dynamic_title must stay None when neither active_agent_sessions nor projection window_id resolve"
        );
    }

    // ---------------------------------------------------------------------
    // SPEC-2359 Phase U-4: worktree-only fallback for SessionStart hook
    // registered records (window_id is None, session_id does not match any
    // active_agent_session). Resolves to the unique active session in the
    // same worktree with the same agent_id when one exists.
    // ---------------------------------------------------------------------

    #[test]
    fn sync_agent_window_titles_fast_path_resolves_across_unrelated_project_root() {
        // SPEC-2359 Phase U-7: when the watcher event project_root is for
        // tab A but a session lives in tab B (both share current.json
        // because they're worktrees of the same repo), the fast path must
        // still resolve B's window via session_id alone. Previously the
        // additional `same_worktree_path(session.worktree_path,
        // project_root)` filter prevented this and caused the user-visible
        // pane heading to stay on the agent_id fallback even after
        // `gwtd workspace update --title-summary` succeeded at the data
        // layer.
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let unrelated = temp.path().join("unrelated");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&unrelated).expect("create unrelated");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));

        // Projection identifies the agent by session-1 with a worktree
        // that *does not* match the unrelated project_root we will pass
        // in. The fast path should still resolve.
        let projection = apply_title_sync_sample_projection(
            &repo,
            "tab-1::agent-1",
            Some("Phase U-7 cross-tab fast path"),
            None,
        );

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&unrelated, &projection);

        assert!(
            changed,
            "fast path must resolve by session_id alone, independent of project_root"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("Phase U-7 cross-tab fast path"),
        );
    }

    #[test]
    fn sync_agent_window_titles_falls_back_to_worktree_when_session_id_not_active() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        let mut projection = apply_title_sync_sample_projection(
            &repo,
            "tab-1::agent-1",
            Some("Phase U-4 worktree fallback"),
            Some("SessionStart-hook registered records have no window_id"),
        );
        // Simulate a SessionStart-hook registration: same worktree, same
        // agent_id (codex), but a *different* session_id and no window_id.
        // The fast path won't match by session_id, but the worktree-only
        // fallback should resolve to the in-memory Codex window.
        projection.agents[0].session_id = "out-of-band-session".to_string();
        projection.agents[0].window_id = None;

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            changed,
            "worktree fallback should resolve to the unique active session"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert_eq!(
            agent_window.dynamic_title.as_deref(),
            Some("Phase U-4 worktree fallback"),
        );
    }

    #[test]
    fn sync_agent_window_titles_worktree_fallback_refuses_when_agent_id_mismatches() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, _window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // Active session is `codex`, but the projection record (SessionStart
        // registered) claims `claude`. The fallback must refuse rather than
        // assigning a Claude title to the Codex pane.
        let mut projection = apply_title_sync_sample_projection(
            &repo,
            "tab-1::agent-1",
            Some("Wrong-agent title leak guard"),
            None,
        );
        projection.agents[0].session_id = "out-of-band-claude".to_string();
        projection.agents[0].window_id = None;
        projection.agents[0].agent_id = "claude".to_string();

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            !changed,
            "worktree fallback must require matching agent_id to disambiguate"
        );
        let tab = runtime.tab("tab-1").expect("tab");
        let agent_window = tab.workspace.window("agent-1").expect("agent window");
        assert!(agent_window.dynamic_title.is_none());
    }

    #[test]
    fn sync_agent_window_titles_worktree_fallback_refuses_when_multiple_sessions_match() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, window_id) =
            apply_title_sync_setup_tab_and_runtime(repo.clone(), Some("tab-1"));
        // Add a second Codex session in the same worktree. The fallback
        // must refuse to pick one because the mapping would be ambiguous.
        runtime.active_agent_sessions.insert(
            "tab-1::agent-2".to_string(),
            ActiveAgentSession {
                window_id: "tab-1::agent-2".to_string(),
                session_id: "session-2".to_string(),
                agent_id: "codex".to_string(),
                branch_name: "work/20260510-0900".to_string(),
                display_name: "Codex 2".to_string(),
                worktree_path: repo.clone(),
                agent_project_root: String::new(),
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                tab_id: "tab-1".to_string(),
            },
        );
        let mut projection =
            apply_title_sync_sample_projection(&repo, &window_id, Some("Ambiguity guard"), None);
        projection.agents[0].session_id = "out-of-band-session".to_string();
        projection.agents[0].window_id = None;

        let changed =
            runtime.sync_agent_window_titles_from_workspace_projection(&repo, &projection);

        assert!(
            !changed,
            "worktree fallback must refuse when multiple sessions share the same worktree + agent_id"
        );
    }

    #[test]
    fn app_runtime_runtime_hook_state_does_not_update_agent_window_dynamic_title() {
        let temp = tempdir().expect("tempdir");
        let mut tab = sample_project_tab_with_window(
            "tab-1",
            "codex-1",
            WindowPreset::Codex,
            WindowProcessStatus::Running,
        );
        tab.workspace
            .set_dynamic_title("codex-1", Some("Board milestone focus".to_string()));
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "codex-1");
        runtime.active_agent_sessions.insert(
            window_id.clone(),
            sample_active_agent_session("tab-1", &window_id),
        );

        let events = runtime.handle_runtime_hook_event(runtime_hook_state("Waiting", "session-1"));

        assert!(events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::RuntimeHookEvent { .. })));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event, BackendEvent::WorkspaceState { .. })),
            "non-structural runtime hook state changes must not force a full workspace_state"
        );
        assert!(events
            .iter()
            .any(|event| matches!(event.event, BackendEvent::TerminalStatus { .. })));
        let tab = runtime.tab("tab-1").expect("tab");
        assert_eq!(
            tab.workspace
                .window("codex-1")
                .expect("codex window")
                .dynamic_title
                .as_deref(),
            Some("Board milestone focus")
        );
    }

    #[test]
    fn app_runtime_gui_board_scope_uses_active_workspace_id_not_first_agent_workspace() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.id = "workspace-active".to_string();
        projection.agents.push(workspace_agent_summary_for_test(
            "session-other",
            Some("workspace-other"),
        ));
        projection
            .agents
            .push(workspace_agent_summary_for_test("session-active", None));
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");

        let scope = super::board::gui_default_board_scope_for_project(&repo)
            .expect("resolve GUI board scope");
        let post_audience =
            gwt::board_audience::post_audience_for_gui(&repo, &[]).expect("resolve post audience");

        assert_eq!(
            scope,
            BoardAudienceScope::Workspace("workspace-active".to_string())
        );
        assert_eq!(post_audience, Some(vec!["workspace-active".to_string()]));
    }

    #[test]
    fn app_runtime_board_projection_change_preserves_all_view_for_live_updates() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.id = "workspace-a".to_string();
        projection
            .agents
            .push(workspace_agent_summary_for_test("session-a", None));
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save projection");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Workspace A update",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-a"]),
        )
        .expect("seed workspace A update");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Workspace B update",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-b"]),
        )
        .expect("seed workspace B update");

        let mut tab_workspace = empty_workspace_state();
        tab_workspace.windows.push(sample_window(
            "board-all",
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        ));
        tab_workspace.windows.push(sample_window(
            "board-workspace",
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        ));
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let all_window_id = combined_window_id("tab-1", "board-all");
        let workspace_window_id = combined_window_id("tab-1", "board-workspace");
        let _ = runtime.load_board_events("client-1", &all_window_id, true);
        let _ = runtime.load_board_events("client-1", &workspace_window_id, false);

        let events = runtime.handle_board_projection_changed_events(&repo);

        let all_entries = events
            .iter()
            .find_map(|event| match event {
                OutboundEvent {
                    event: BackendEvent::BoardEntries { id, entries, .. },
                    ..
                } if id == &all_window_id => Some(entries),
                _ => None,
            })
            .expect("all view board entries");
        let workspace_entries = events
            .iter()
            .find_map(|event| match event {
                OutboundEvent {
                    event: BackendEvent::BoardEntries { id, entries, .. },
                    ..
                } if id == &workspace_window_id => Some(entries),
                _ => None,
            })
            .expect("workspace view board entries");

        assert!(
            all_entries
                .iter()
                .any(|entry| entry.body == "Workspace B update"),
            "All view live update must include other Workspace entries"
        );
        assert!(
            !workspace_entries
                .iter()
                .any(|entry| entry.body == "Workspace B update"),
            "Workspace view live update must stay scoped"
        );
    }

    #[test]
    fn app_runtime_board_projection_change_broadcasts_to_matching_board_windows_only() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        let other_repo = temp.path().join("other-repo");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&other_repo).expect("create other repo");
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "External update",
                None,
                None,
                vec![],
                vec![],
            ),
        )
        .expect("seed matching board snapshot");
        post_entry(
            &other_repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "codex",
                BoardEntryKind::Status,
                "Other project update",
                None,
                None,
                vec![],
                vec![],
            ),
        )
        .expect("seed other board snapshot");

        let mut tab_workspace = empty_workspace_state();
        tab_workspace.windows.push(sample_window(
            "board-1",
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        ));
        tab_workspace.windows.push(sample_window(
            "board-2",
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        ));
        tab_workspace.windows.push(sample_window(
            "logs-1",
            WindowPreset::Logs,
            WindowProcessStatus::Ready,
        ));
        tab_workspace.next_z_index = 4;
        let matching_tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(tab_workspace),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };
        let other_tab = sample_project_tab_with_window_at(
            "tab-2",
            "board-3",
            other_repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![matching_tab, other_tab], Some("tab-1"));

        let events = runtime.handle_board_projection_changed_events(&repo);

        assert_eq!(events.len(), 3);
        for expected_id in [
            combined_window_id("tab-1", "board-1"),
            combined_window_id("tab-1", "board-2"),
        ] {
            assert!(events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::BoardEntries { id, entries, .. },
                } if *id == expected_id
                    && entries.len() == 1
                    && entries[0].body == "External update"
            )));
        }
        assert!(!events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                event: BackendEvent::BoardEntries { id, .. },
                ..
            } if *id == combined_window_id("tab-2", "board-3")
        )));
    }

    fn migration_pending_tab(tab_id: &str, project_root: PathBuf) -> ProjectTabRuntime {
        ProjectTabRuntime {
            id: tab_id.to_string(),
            title: "Repo".to_string(),
            project_root,
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(empty_workspace_state()),
            migration_pending: true,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// SPEC-2014 FR-PERF-003: ProjectTabRuntime caches `main_worktree_root`
    /// resolution per tab so the Launch Wizard / Start Work paths do not
    /// re-spawn `git rev-parse --git-common-dir` on every open.
    #[test]
    fn project_tab_runtime_main_worktree_root_caches_resolution() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("repo");
        gwt_core::process::hidden_command("git")
            .args(["init", repo.to_str().unwrap()])
            .output()
            .expect("git init");

        let tab = sample_project_tab("tab-1", "Repo", repo.clone(), ProjectKind::Git, &[]);
        assert!(
            tab.main_worktree_root_cache.get().is_none(),
            "cache must start empty"
        );

        let first = tab.main_worktree_root();
        let cached = tab
            .main_worktree_root_cache
            .get()
            .expect("cache populated after first access")
            .clone();
        assert_eq!(first, cached);

        let second = tab.main_worktree_root();
        assert_eq!(
            first, second,
            "second call must return the cached resolution"
        );
    }

    #[test]
    fn migration_detected_broadcasts_only_for_pending_tabs() {
        let temp = tempdir().expect("tempdir");
        let repo_a = temp.path().join("repo-a");
        let repo_b = temp.path().join("repo-b");
        fs::create_dir_all(&repo_a).expect("repo-a");
        fs::create_dir_all(&repo_b).expect("repo-b");

        let pending = migration_pending_tab("tab-1", repo_a);
        let mut clean = sample_project_tab("tab-2", "Other", repo_b, ProjectKind::Git, &[]);
        clean.migration_pending = false;
        let runtime = sample_runtime(temp.path(), vec![pending, clean], Some("tab-1"));

        let events = runtime.migration_detected_broadcasts();

        assert_eq!(events.len(), 1, "only pending tabs should broadcast");
        assert!(matches!(
            &events[0],
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationDetected { tab_id, .. },
            } if tab_id == "tab-1"
        ));
    }

    #[test]
    fn handle_migration_done_repoints_tab_and_emits_broadcast() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let new_worktree = project.join("develop");
        fs::create_dir_all(&new_worktree).expect("new worktree");

        let tab = migration_pending_tab("tab-1", project);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_migration_done("tab-1", &new_worktree);

        let updated = runtime
            .tabs
            .iter()
            .find(|t| t.id == "tab-1")
            .expect("tab still present");
        let canonical_new =
            dunce::canonicalize(&new_worktree).unwrap_or_else(|_| new_worktree.clone());
        assert_eq!(updated.project_root, canonical_new);
        assert!(!updated.migration_pending, "pending flag must clear");

        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationDone { tab_id, .. },
            } if tab_id == "tab-1"
        )));
    }

    #[test]
    fn handle_migration_error_clears_pending_and_broadcasts_recovery_label() {
        use gwt_core::migration::{MigrationPhase, RecoveryState};
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");

        let tab = migration_pending_tab("tab-1", project);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.handle_migration_error(
            "tab-1",
            MigrationPhase::Bareify,
            "boom".to_string(),
            RecoveryState::RolledBack,
        );

        assert!(
            !runtime
                .tabs
                .iter()
                .find(|t| t.id == "tab-1")
                .unwrap()
                .migration_pending
        );
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationError { tab_id, recovery, phase, .. },
            } if tab_id == "tab-1" && recovery == "rolled_back" && phase == "bareify"
        )));
    }

    #[test]
    fn open_start_work_refuses_while_migration_pending() {
        // SPEC-1934 US-7 / FR-034: Workspace Start Work must not run on a tab
        // whose Normal → Nested Bare+Worktree migration is still pending.
        // Without this gate, the launch path tries to fetch
        // `origin/work/<branch>` on a single-branch refspec and dies with
        // `fatal: invalid reference: origin/work/<branch>`.
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");

        let tab = migration_pending_tab("tab-1", project);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.open_start_work("client-1");

        assert!(
            events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Client(_),
                    event: BackendEvent::LaunchWizardOpenError { title, message },
                } if title == "Start Work"
                    && message == "Complete the project migration before starting work"
            )),
            "Start Work on a migration_pending tab must surface a clear error: {events:?}"
        );
    }

    #[test]
    fn github_repository_search_parser_maps_gh_json_fields() {
        let raw = r#"[
          {
            "fullName": "akiojin/gwt",
            "description": "Git Worktree Manager",
            "url": "https://github.com/akiojin/gwt",
            "defaultBranch": "develop",
            "visibility": "public",
            "updatedAt": "2026-05-13T00:00:00Z"
          }
        ]"#;

        let repositories =
            super::parse_github_repository_search_results(raw).expect("parse gh search json");

        assert_eq!(repositories.len(), 1);
        assert_eq!(repositories[0].full_name, "akiojin/gwt");
        assert_eq!(
            repositories[0].description.as_deref(),
            Some("Git Worktree Manager")
        );
        assert_eq!(repositories[0].url, "https://github.com/akiojin/gwt");
        assert_eq!(repositories[0].default_branch.as_deref(), Some("develop"));
        assert_eq!(repositories[0].visibility.as_deref(), Some("public"));
        assert_eq!(
            repositories[0].updated_at.as_deref(),
            Some("2026-05-13T00:00:00Z")
        );
    }

    #[test]
    fn clone_project_done_opens_workspace_home_and_broadcasts_done() {
        let temp = tempdir().expect("tempdir");
        let workspace_home = temp.path().join("sample");
        let bare_repo = workspace_home.join("sample.git");
        fs::create_dir_all(&workspace_home).expect("workspace home");
        let output = gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
            .output()
            .expect("git init --bare");
        assert!(
            output.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

        let events = runtime.handle_clone_project_done(&workspace_home);

        assert_eq!(runtime.tabs.len(), 1);
        assert_eq!(
            runtime.tabs[0].project_root,
            dunce::canonicalize(&workspace_home).unwrap()
        );
        assert_eq!(runtime.recent_projects.len(), 1);
        assert_eq!(
            runtime.recent_projects[0].path,
            dunce::canonicalize(&workspace_home).unwrap()
        );
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::CloneProjectDone {
                    workspace_home: emitted_workspace_home,
                },
            } if emitted_workspace_home == &workspace_home.display().to_string()
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::WorkspaceState { .. },
            }
        )));
    }

    #[test]
    fn open_project_path_for_worktree_remembers_workspace_home_only() {
        // Issue #2867: open_project_path で worktree path を渡したとき、tab は
        // worktree で開く (direct-pick) が、recent_projects は workspace home
        // (bare repo の親) に正規化されて 1 件だけ残る。同じ workspace の
        // 別 worktree を続けて開いても recent_projects は増えない。
        let temp = tempdir().expect("tempdir");
        let workspace_home = temp.path().join("workspace");
        let bare_repo = workspace_home.join("repo.git");
        fs::create_dir_all(&workspace_home).expect("workspace home");
        let output = gwt_core::process::hidden_command("git")
            .args(["init", "--bare", bare_repo.to_str().expect("bare path")])
            .output()
            .expect("git init --bare");
        assert!(
            output.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bootstrap = workspace_home.join(".bootstrap");
        let clone = gwt_core::process::hidden_command("git")
            .args([
                "clone",
                bare_repo.to_str().unwrap(),
                bootstrap.to_str().unwrap(),
            ])
            .output()
            .expect("git clone");
        assert!(clone.status.success(), "git clone failed");
        for (key, value) in [
            ("user.email", "test@example.com"),
            ("user.name", "Test User"),
        ] {
            let cfg = gwt_core::process::hidden_command("git")
                .args(["config", key, value])
                .current_dir(&bootstrap)
                .output()
                .expect("git config");
            assert!(cfg.status.success(), "git config {key} failed");
        }
        for args in [
            vec!["checkout", "-b", "develop"],
            vec!["commit", "--allow-empty", "-m", "init"],
            vec!["push", "origin", "develop"],
        ] {
            let out = gwt_core::process::hidden_command("git")
                .args(&args)
                .current_dir(&bootstrap)
                .output()
                .expect("git command");
            assert!(out.status.success(), "git {args:?} failed");
        }
        fs::remove_dir_all(&bootstrap).expect("remove bootstrap");

        let develop_worktree = workspace_home.join("develop");
        let feature_worktree = workspace_home.join("feature/alpha");
        for (path, branch_args) in [
            (
                &develop_worktree,
                vec!["worktree", "add", "@PATH@", "develop"],
            ),
            (
                &feature_worktree,
                vec![
                    "worktree",
                    "add",
                    "-b",
                    "feature/alpha",
                    "@PATH@",
                    "develop",
                ],
            ),
        ] {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("worktree parent");
            }
            let path_str = path.to_str().unwrap().to_string();
            let resolved: Vec<&str> = branch_args
                .iter()
                .map(|a| {
                    if *a == "@PATH@" {
                        path_str.as_str()
                    } else {
                        *a
                    }
                })
                .collect();
            let out = gwt_core::process::hidden_command("git")
                .args(&resolved)
                .current_dir(&bare_repo)
                .output()
                .expect("git worktree add");
            assert!(
                out.status.success(),
                "git {resolved:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
        let canonical_home = dunce::canonicalize(&workspace_home).unwrap();

        runtime
            .open_project_path(develop_worktree.clone())
            .expect("open develop worktree");
        assert_eq!(runtime.tabs.len(), 1);
        assert_eq!(
            runtime.tabs[0].project_root,
            dunce::canonicalize(&develop_worktree).unwrap(),
            "tab must open at the chosen worktree (SC-035 direct-pick preserved)"
        );
        assert_eq!(runtime.recent_projects.len(), 1);
        assert_eq!(
            runtime.recent_projects[0].path, canonical_home,
            "recent_projects must collapse to workspace home, not worktree path (Issue #2867)"
        );

        runtime
            .open_project_path(feature_worktree.clone())
            .expect("open feature worktree");
        assert_eq!(
            runtime.recent_projects.len(),
            1,
            "opening another worktree in the same workspace must not add a new recent entry"
        );
        assert_eq!(
            runtime.recent_projects[0].path, canonical_home,
            "second worktree open must keep workspace home as the canonical recent entry"
        );
    }

    #[test]
    fn clone_project_start_validation_uses_clone_project_error_event() {
        let temp = tempdir().expect("tempdir");
        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::CloneProjectStart {
                url: "".to_string(),
                parent_path: "".to_string(),
            },
        );

        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::CloneProjectError { message },
            } if client_id == "client-1" && message.contains("repository URL")
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                event: BackendEvent::ProjectOpenError { .. },
                ..
            }
        )));
    }

    #[test]
    fn skip_migration_events_clears_pending_flag_without_broadcast() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");

        let tab = migration_pending_tab("tab-1", project);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.skip_migration_events("tab-1");
        assert!(events.is_empty(), "skip must not emit events itself");
        assert!(!runtime.tabs[0].migration_pending);
    }

    #[test]
    fn skip_migration_events_keeps_normal_git_and_redetects_on_next_launch() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");
        init_repo(&project);

        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
        let open_events = runtime.open_project_path_events(project.clone());
        let tab_id = runtime.active_tab_id.clone().expect("active tab");

        assert!(open_events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MigrationDetected { .. },
            }
        )));

        let skip_events = runtime.skip_migration_events(&tab_id);
        assert!(skip_events.is_empty(), "skip must not mutate repository");
        assert!(matches!(
            gwt_git::detect_repo_type(&project),
            gwt_git::RepoType::Normal {
                needs_migration: true,
                ..
            }
        ));

        let mut next_runtime = sample_runtime(temp.path(), Vec::new(), None);
        let next_events = next_runtime.open_project_path_events(project);

        assert!(
            next_events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::MigrationDetected { .. },
                }
            )),
            "skip is launch-local; the modal must be shown again next launch"
        );
    }

    #[test]
    fn quit_migration_events_requests_app_quit_without_repository_changes() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");
        init_repo(&project);

        let tab = migration_pending_tab("tab-1", project.clone());
        let (mut runtime, recorded_events) =
            sample_runtime_with_events(temp.path(), vec![tab], Some("tab-1"));

        let events = runtime.quit_migration_events("tab-1");

        assert!(
            events.is_empty(),
            "quit is delivered through the event proxy"
        );
        let recorded_events = recorded_events.lock().expect("recorded events");
        assert!(recorded_events
            .iter()
            .any(|event| matches!(event, UserEvent::QuitApp)));
        assert!(matches!(
            gwt_git::detect_repo_type(&project),
            gwt_git::RepoType::Normal {
                needs_migration: true,
                ..
            }
        ));
    }

    #[test]
    fn open_project_with_existing_migration_backup_emits_recovery_error() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("project dir");
        init_repo(&project);
        fs::create_dir_all(project.join(gwt_core::migration::backup::BACKUP_DIR_NAME))
            .expect("migration backup dir");

        let mut runtime = sample_runtime(temp.path(), Vec::new(), None);
        let events = runtime.open_project_path_events(project.clone());

        assert!(
            events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::MigrationDetected { .. },
                }
            )),
            "Normal Git layout should still open a migration-pending tab"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::MigrationError {
                        phase,
                        recovery,
                        message,
                        ..
                    },
                } if phase == "backup"
                    && recovery == "partial"
                    && message.contains(".gwt-migration-backup")
            )),
            "existing migration backup must be surfaced as a recovery error"
        );
    }

    // SPEC-2041 Phase 19 (FR-065 / CodeRabbit review on PR #2630): renderer-
    // supplied log paths must canonicalize into the gwt update logs root.
    // These tests cover the `validate_update_log_path` pure validator.
    #[test]
    fn validate_update_log_path_accepts_file_inside_logs_root() {
        let logs_root = tempfile::tempdir().expect("logs root tempdir");
        let log_file = logs_root.path().join("update-2026-05-10.log");
        std::fs::write(&log_file, b"{}\n").unwrap();

        let resolved =
            super::validate_update_log_path(log_file.to_str().unwrap(), logs_root.path());
        assert!(
            resolved.is_some(),
            "expected file inside logs_root to validate"
        );
        let resolved = resolved.unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("update-2026-05-10.log"));
    }

    #[test]
    fn validate_update_log_path_rejects_files_outside_logs_root() {
        let logs_root = tempfile::tempdir().expect("logs root tempdir");
        let outside = tempfile::tempdir().expect("outside tempdir");
        let outside_file = outside.path().join("evil.txt");
        std::fs::write(&outside_file, b"steal me").unwrap();

        let resolved =
            super::validate_update_log_path(outside_file.to_str().unwrap(), logs_root.path());
        assert!(resolved.is_none(), "outside-root paths must be rejected");
    }

    #[test]
    fn validate_update_log_path_rejects_url_schemes_and_empty() {
        let logs_root = tempfile::tempdir().expect("logs root tempdir");
        for raw in [
            "",
            "   ",
            "http://evil.example/log",
            "https://evil.example/log",
            "file:///etc/passwd",
        ] {
            assert!(
                super::validate_update_log_path(raw, logs_root.path()).is_none(),
                "expected `{raw}` to be rejected",
            );
        }
    }

    #[test]
    fn validate_update_log_path_rejects_directories() {
        let logs_root = tempfile::tempdir().expect("logs root tempdir");
        // Caller passes the logs root itself; a directory must not be opened
        // as a file.
        let resolved =
            super::validate_update_log_path(logs_root.path().to_str().unwrap(), logs_root.path());
        assert!(resolved.is_none(), "directories must be rejected");
    }

    #[test]
    fn validate_update_log_path_rejects_missing_files() {
        let logs_root = tempfile::tempdir().expect("logs root tempdir");
        let missing = logs_root.path().join("does-not-exist.log");
        let resolved = super::validate_update_log_path(missing.to_str().unwrap(), logs_root.path());
        assert!(resolved.is_none(), "missing files must be rejected");
    }

    // SPEC-2785 FR-E: open_server_url requests must be gated by an exact
    // same-origin match against the embedded server's bound URL. The shared
    // validator function is reused by `AppRuntime::open_server_url_events`
    // so a mismatched origin cannot smuggle an arbitrary URL into the OS
    // opener.
    #[test]
    fn validate_server_url_accepts_exact_bound_url() {
        let allowed = Some("http://127.0.0.1:54321/");
        assert!(super::validate_server_url(
            allowed,
            "http://127.0.0.1:54321/"
        ));
    }

    #[test]
    fn validate_server_url_rejects_different_port() {
        let allowed = Some("http://127.0.0.1:54321/");
        assert!(!super::validate_server_url(
            allowed,
            "http://127.0.0.1:54322/"
        ));
    }

    #[test]
    fn validate_server_url_rejects_different_scheme() {
        let allowed = Some("http://127.0.0.1:54321/");
        assert!(!super::validate_server_url(
            allowed,
            "https://127.0.0.1:54321/"
        ));
    }

    #[test]
    fn validate_server_url_rejects_when_allowed_is_none() {
        assert!(!super::validate_server_url(None, "http://127.0.0.1:54321/"));
    }

    #[test]
    fn validate_server_url_rejects_external_origin() {
        let allowed = Some("http://127.0.0.1:54321/");
        assert!(!super::validate_server_url(allowed, "http://evil.example/"));
    }

    // SPEC-2785 SC-4: `open_server_url_events` returns an empty event list and
    // performs no OS opener side effect when the requested URL does not match
    // the configured server URL. The state mutation guard is `server_url`
    // being None or unequal to the request.
    #[test]
    fn open_server_url_events_rejects_mismatched_origin() {
        let temp = tempdir().expect("tempdir");
        let (mut runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
        runtime.set_server_url("http://127.0.0.1:54321/".to_string());
        let outbound =
            runtime.open_server_url_events("client-1", "http://evil.example/".to_string());
        assert!(
            outbound.is_empty(),
            "mismatched origin must yield no outbound events"
        );
    }

    #[test]
    fn open_server_url_events_rejects_when_server_url_unset() {
        let temp = tempdir().expect("tempdir");
        let (runtime, _events) = sample_runtime_with_events(temp.path(), Vec::new(), None);
        let outbound =
            runtime.open_server_url_events("client-1", "http://127.0.0.1:54321/".to_string());
        assert!(
            outbound.is_empty(),
            "unset server URL must reject any open request"
        );
    }

    #[test]
    fn codex_hook_trust_launch_enabled_registers_host_codex_hooks() {
        let home = tempdir().expect("home tempdir");
        let profile_config_path = home.path().join(".gwt/config.toml");
        let mut settings = Settings::default();
        settings.agent.codex_trust_managed_hooks = Some(true);
        settings.save(&profile_config_path).unwrap();

        let worktree = tempdir().expect("worktree tempdir");
        gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

        let report = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .unwrap()
        .expect("enabled host Codex launch should register trust");

        assert_eq!(report.trusted_entries.len(), 5);
        let codex_config_path = home.path().join(".codex/config.toml");
        let config = fs::read_to_string(&codex_config_path).unwrap();
        assert!(
            config.contains("trusted_hash"),
            "Codex config should contain trusted hashes, got: {config}"
        );
        assert_eq!(report.config_path, codex_config_path);
    }

    #[test]
    fn codex_hook_trust_launch_defaults_to_host_codex_registration_and_false_opts_out() {
        let home = tempdir().expect("home tempdir");
        let profile_config_path = home.path().join(".gwt/config.toml");
        let worktree = tempdir().expect("worktree tempdir");
        gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

        let unset = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .unwrap();
        assert_eq!(
            unset
                .expect("unset config should default to trusting managed Codex hooks")
                .trusted_entries
                .len(),
            5
        );
        fs::remove_file(home.path().join(".codex/config.toml")).unwrap();

        let mut settings = Settings::default();
        settings.agent.codex_trust_managed_hooks = Some(false);
        settings.save(&profile_config_path).unwrap();
        let disabled = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .unwrap();
        assert!(disabled.is_none());

        assert!(
            !home.path().join(".codex/config.toml").exists(),
            "false opt-out must not recreate Codex config"
        );

        settings.agent.codex_trust_managed_hooks = Some(true);
        settings.save(&profile_config_path).unwrap();
        let enabled = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .unwrap();
        assert_eq!(
            enabled
                .expect("true config should register managed Codex hooks")
                .trusted_entries
                .len(),
            5
        );

        let claude = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::ClaudeCode,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        )
        .unwrap();
        assert!(claude.is_none());

        assert!(
            home.path().join(".codex/config.toml").exists(),
            "host default/true paths must create Codex config"
        );
    }

    #[test]
    fn codex_hook_trust_launch_is_warning_only_when_registration_fails() {
        let home = tempdir().expect("home tempdir");
        let profile_config_path = home.path().join(".gwt/config.toml");
        let worktree = tempdir().expect("worktree tempdir");
        gwt_skills::generate_codex_hooks(worktree.path()).unwrap();

        let codex_config_parent = home.path().join(".codex");
        fs::write(&codex_config_parent, "not a directory").unwrap();

        let result = super::maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            worktree.path(),
            &gwt_agent::AgentId::Codex,
            gwt_agent::LaunchRuntimeTarget::Host,
            None,
        );

        assert!(
            result.is_ok(),
            "optional trust registration must not abort launch: {result:?}"
        );
        assert!(
            result.unwrap().is_none(),
            "skip paths must not create Codex config"
        );
    }

    #[test]
    fn workspace_view_for_tab_omits_work_item_history_from_workspace_state() {
        // SPEC-2359 CPU/power follow-up: workspace_state is broadcast frequently
        // and must stay structural. Workspace history/work items are carried by
        // active_work_projection so every window/status update does not serialize
        // the full work item event log.
        let _env_guard = env_test_lock().lock().expect("env lock");
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");

        use chrono::TimeZone as _;
        let completed_at = chrono::Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let work_item = gwt_core::workspace_projection::WorkspaceWorkItem {
            id: "work-item-done".to_string(),
            title: "Test Done Item".to_string(),
            intent: None,
            summary: None,
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Done,
            owner: None,
            created_at: completed_at,
            updated_at: completed_at,
            completed_at: Some(completed_at),
            agents: Vec::new(),
            execution_containers: Vec::new(),
            board_refs: Vec::new(),
            related_work_item_ids: Vec::new(),
            events: Vec::new(),
        };
        let projection = gwt_core::workspace_projection::WorkspaceWorkItemsProjection {
            updated_at: completed_at,
            work_items: vec![work_item],
        };
        let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
        fs::create_dir_all(work_items_path.parent().expect("parent dir"))
            .expect("create workspace dir");
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &work_items_path,
            &projection,
        )
        .expect("save work items projection");

        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(empty_workspace_state()),
            migration_pending: false,
            main_worktree_root_cache: std::sync::Arc::new(std::sync::OnceLock::new()),
        };

        let view = crate::runtime_support::workspace_view_for_tab(&tab);
        assert!(
            view.work_items.is_empty(),
            "WorkspaceView.work_items must stay empty because workspace_state is a hot broadcast path; active_work_projection owns Workspace history"
        );
    }

    // SPEC-1924 US-14 FR-035 / FR-036 / SC-010 / SC-011 — verify the Logs
    // window snapshot reader goes through `gwt_core::logging::read_log_file`
    // and that the synthetic warning event is well-formed when malformed
    // lines are skipped.

    const PROD_LINE_INFO: &str = r#"{"timestamp":"2026-05-20T09:00:00.015355+09:00","level":"INFO","fields":{"message":"PTY resize completed","outcome":"ok"},"target":"gwt::resize::pty"}"#;
    const MALFORMED_LINE: &str = r#"{"foo":"bar"}"#;

    fn write_canonical_log_file(log_dir: &Path, lines: &[&str]) {
        fs::create_dir_all(log_dir).expect("create log dir");
        let log_path = current_log_file(log_dir);
        let mut file = fs::File::create(&log_path).expect("create log file");
        for line in lines {
            file.write_all(line.as_bytes()).expect("write line");
            file.write_all(b"\n").expect("write newline");
        }
    }

    #[test]
    fn load_log_entries_from_dir_returns_outcome_with_no_skipped_lines() {
        let dir = tempdir().expect("tempdir");
        write_canonical_log_file(dir.path(), &[PROD_LINE_INFO]);

        let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

        assert_eq!(outcome.entries.len(), 1);
        assert_eq!(outcome.entries[0].message, "PTY resize completed");
        assert_eq!(outcome.diagnostics.skipped, 0);
    }

    #[test]
    fn load_log_entries_from_dir_counts_skipped_lines() {
        let dir = tempdir().expect("tempdir");
        write_canonical_log_file(
            dir.path(),
            &[PROD_LINE_INFO, MALFORMED_LINE, PROD_LINE_INFO],
        );

        let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

        assert_eq!(outcome.entries.len(), 2);
        assert_eq!(outcome.diagnostics.skipped, 1);
    }

    #[test]
    fn load_log_entries_from_dir_returns_empty_outcome_when_file_missing() {
        let dir = tempdir().expect("tempdir");

        let outcome = super::load_log_entries_from_dir(dir.path()).expect("read ok");

        assert!(outcome.entries.is_empty());
        assert_eq!(outcome.diagnostics.skipped, 0);
    }

    #[test]
    fn skipped_lines_warning_is_warn_severity_and_includes_count_and_path() {
        let diagnostics = gwt_core::logging::ReadDiagnostics {
            path: PathBuf::from("/tmp/gwt.log.2026-05-20"),
            skipped: 3,
        };

        let event = super::skipped_lines_warning(&diagnostics);

        assert_eq!(event.severity, LogLevel::Warn);
        assert_eq!(event.source, "gwt_core::logging::reader");
        assert!(event.message.contains("Skipped 3 malformed lines"));
        assert!(event.message.contains("/tmp/gwt.log.2026-05-20"));
    }

    #[test]
    fn skipped_lines_warning_singular_for_one_line() {
        let diagnostics = gwt_core::logging::ReadDiagnostics {
            path: PathBuf::from("/tmp/x.log"),
            skipped: 1,
        };

        let event = super::skipped_lines_warning(&diagnostics);

        assert!(event.message.contains("Skipped 1 malformed line "));
    }
}

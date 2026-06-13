use super::*;
use gwt::LaunchWizardAction;
use std::io::Write as _;

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

struct RuntimeStopThreads {
    output_thread: Option<JoinHandle<()>>,
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
mod launch_output_mirror;
mod migration;
pub(crate) mod persist_dispatcher;
mod profile;
mod runtime_events;
mod title_sync;
mod ui_trace;
mod window;
mod wizard;
mod workspace;
pub use board::BoardPostRequest;
use profile::ProfileSaveRequest;
use ui_trace::save_ui_trace_to_log_dir;
use workspace::{
    active_agent_summary_from_session, merge_active_sessions_into_projection,
    retain_live_workspace_agents, save_workspace_launch_projection, spawn_workspace_cleanup_async,
    workspace_cleanup_candidate_for_projection, workspace_projection_for_current_resume,
    workspace_projection_owner_title,
};

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
    if session.fast_mode_enabled() {
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
        FrontendEvent::SetClaudeAccountUsageEnabled { enabled } => {
            FrontendUserActionLog::new("set_claude_account_usage_enabled", "usage")
                .mode(if *enabled { "on" } else { "off" })
        }
        FrontendEvent::RefreshUsage => FrontendUserActionLog::new("refresh_usage", "usage"),
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
                | LaunchWizardAction::SetFastMode { enabled }
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
        FrontendEvent::GetBoardAuthStatus => {
            FrontendUserActionLog::new("get_board_auth_status", "settings")
        }
        FrontendEvent::BoardProviderSignIn { provider } => {
            FrontendUserActionLog::new("board_provider_sign_in", "settings").target(provider)
        }
        FrontendEvent::BoardProviderSignOut { provider } => {
            FrontendUserActionLog::new("board_provider_sign_out", "settings").target(provider)
        }
        FrontendEvent::UpdateBoardProviderConfig { provider, .. } => {
            FrontendUserActionLog::new("update_board_provider_config", "settings").target(provider)
        }
        FrontendEvent::UpdateBoardOauthPort { port } => {
            FrontendUserActionLog::new("update_board_oauth_port", "settings")
                .target(port.to_string())
        }
        FrontendEvent::UpdateSystemSettings {
            language,
            codex_trust_managed_hooks,
            ..
        } => FrontendUserActionLog::new("update_system_settings", "settings")
            .target(language)
            .force(codex_trust_managed_hooks.unwrap_or(false)),
        FrontendEvent::GetAutostartStatus => {
            FrontendUserActionLog::new("get_autostart_status", "settings")
        }
        FrontendEvent::UpdateAutostart { enabled } => {
            FrontendUserActionLog::new("update_autostart", "settings").force(*enabled)
        }
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
        FrontendEvent::ApplyUpdateToVersion { version } => {
            FrontendUserActionLog::new("apply_update_to_version", "update").target(version)
        }
        FrontendEvent::CloseWork {
            work_id,
            close_kind,
        } => FrontendUserActionLog::new("close_work", "workspace")
            .target(format!("{work_id} ({close_kind})")),
        // SPEC-3050: log the injection request without its text payload —
        // the injected line lands in the PTY transcript anyway.
        FrontendEvent::PaneSendInput { session_id, .. } => {
            FrontendUserActionLog::new("pane_send_input", "terminal").target(session_id)
        }
        // These events can contain high-volume, high-frequency, or sensitive
        // payloads. They are handled by more specific logs or diagnostics.
        FrontendEvent::StartupAutoResumeReady { .. }
        | FrontendEvent::UpdateViewport { .. }
        | FrontendEvent::UpdateWindowGeometry { .. }
        | FrontendEvent::TerminalInput { .. }
        | FrontendEvent::PasteImage { .. }
        | FrontendEvent::PasteImageUploaded { .. }
        | FrontendEvent::AttachFiles { .. } => return None,
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

fn autostart_status_event_from_result(
    result: Result<
        gwt::cli::tray::autostart::AutostartStatus,
        gwt::cli::tray::autostart::AutostartError,
    >,
) -> BackendEvent {
    match result {
        Ok(status) => BackendEvent::AutostartStatus {
            enabled: status.enabled,
            mechanism: format!("{:?}", status.mechanism),
            install_path: status
                .install_path
                .map(|path| path.to_string_lossy().into_owned()),
        },
        Err(error) => BackendEvent::AutostartError {
            message: error.to_string(),
        },
    }
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

impl AgentWindowPlacement {
    fn bounds(&self) -> WindowGeometry {
        match self {
            Self::Centered(bounds) | Self::Exact(bounds) => bounds.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImagePasteFile {
    pub(crate) bytes: Option<Vec<u8>>,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) remove_source_after_save: bool,
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

const IMAGE_PASTE_PROMPT_PREFIX: &str = "Image file: ";
const FILE_ATTACHMENT_RELATIVE_DIR: &str = ".gwt/drop-files";
static IMAGE_PASTE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedFileAttachment {
    pub(crate) bytes: Option<Vec<u8>>,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) remove_source_after_save: bool,
    pub(crate) storage_path: Option<PathBuf>,
    pub(crate) agent_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileAttachmentError {
    EmptyPath,
    InvalidBase64(String),
    SizeMismatch { declared: u64, actual: u64 },
    TooLarge { size: u64, limit: u64 },
    NotAFile(String),
    ReadFailed(String),
    WriteFailed(String),
    UploadedFileMissing(String),
}

impl std::fmt::Display for FileAttachmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("file attachment path is empty"),
            Self::InvalidBase64(error) => {
                write!(formatter, "invalid file attachment payload: {error}")
            }
            Self::SizeMismatch { declared, actual } => write!(
                formatter,
                "file attachment size mismatch: declared {declared}, decoded {actual}"
            ),
            Self::TooLarge { size, limit } => {
                write!(formatter, "file attachment is too large: {size} > {limit}")
            }
            Self::NotAFile(path) => write!(formatter, "file attachment is not a file: {path}"),
            Self::ReadFailed(error) => write!(formatter, "failed to read file attachment: {error}"),
            Self::WriteFailed(error) => {
                write!(formatter, "failed to save file attachment: {error}")
            }
            Self::UploadedFileMissing(upload_id) => {
                write!(
                    formatter,
                    "uploaded file attachment is missing: {upload_id}"
                )
            }
        }
    }
}

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

fn sanitize_file_attachment_name(filename: &str) -> String {
    let trimmed = filename.trim();
    let raw_name = trimmed
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("file");
    let mut sanitized = String::new();
    let mut previous_dash = false;
    for character in raw_name.chars() {
        let unsafe_character = character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            );
        if unsafe_character || character.is_whitespace() || character == '-' {
            if !previous_dash {
                sanitized.push('-');
                previous_dash = true;
            }
        } else if character.is_ascii() {
            sanitized.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else {
            sanitized.push(character);
            previous_dash = false;
        }
    }
    let sanitized = sanitized.trim_matches(['-', '.', '_']);
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "file".to_string()
    } else if is_reserved_attachment_basename(sanitized) {
        format!("file-{sanitized}")
    } else {
        sanitized.to_string()
    }
}

fn is_reserved_attachment_basename(filename: &str) -> bool {
    let stem = filename
        .split('.')
        .next()
        .unwrap_or(filename)
        .trim_matches([' ', '.', '_', '-'])
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn attachment_storage_paths(
    worktree_path: &Path,
    _agent_project_root: &str,
    unique_token: &str,
    filename: &str,
) -> (PathBuf, String) {
    let sanitized = sanitize_file_attachment_name(filename);
    let file_name = format!("{unique_token}-{sanitized}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let agent_path = relative_path;
    (storage_path, agent_path)
}

fn validate_file_attachment_size(size: u64, limit: u64) -> Result<(), FileAttachmentError> {
    if size > limit {
        return Err(FileAttachmentError::TooLarge { size, limit });
    }
    Ok(())
}

pub(crate) fn prepare_file_attachment(
    worktree_path: &Path,
    agent_project_root: &str,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    file: &gwt::FileAttachment,
    unique_token: &str,
    limits: ContentLimits,
    upload_store: &AttachmentUploadStore,
) -> Result<PreparedFileAttachment, FileAttachmentError> {
    let _ = runtime_target;
    match file {
        gwt::FileAttachment::NativePath { path } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(FileAttachmentError::EmptyPath);
            }
            let source = PathBuf::from(path);
            let metadata = std::fs::metadata(&source)
                .map_err(|error| FileAttachmentError::ReadFailed(error.to_string()))?;
            if !metadata.is_file() {
                return Err(FileAttachmentError::NotAFile(path.to_string()));
            }
            let filename = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file");
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: None,
                source_path: Some(source),
                remove_source_after_save: false,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
        gwt::FileAttachment::Inline {
            filename,
            size,
            data_base64,
            ..
        } => {
            validate_file_attachment_size(*size, limits.binary_chunk_max_bytes)?;
            let bytes = base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                data_base64.trim(),
            )
            .map_err(|error| FileAttachmentError::InvalidBase64(error.to_string()))?;
            let actual = bytes.len() as u64;
            if actual != *size {
                return Err(FileAttachmentError::SizeMismatch {
                    declared: *size,
                    actual,
                });
            }
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: Some(bytes),
                source_path: None,
                remove_source_after_save: false,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
        gwt::FileAttachment::Uploaded {
            upload_id,
            filename,
            size,
            ..
        } => {
            let uploaded = upload_store
                .take(upload_id)
                .map_err(FileAttachmentError::ReadFailed)?
                .ok_or_else(|| FileAttachmentError::UploadedFileMissing(upload_id.clone()))?;
            if uploaded.size != *size {
                return Err(FileAttachmentError::SizeMismatch {
                    declared: *size,
                    actual: uploaded.size,
                });
            }
            let filename = if filename.trim().is_empty() {
                uploaded.filename.as_str()
            } else {
                filename.as_str()
            };
            let (storage_path, agent_path) =
                attachment_storage_paths(worktree_path, agent_project_root, unique_token, filename);
            Ok(PreparedFileAttachment {
                bytes: None,
                source_path: Some(uploaded.path),
                remove_source_after_save: true,
                storage_path: Some(storage_path),
                agent_path,
            })
        }
    }
}

fn save_file_attachment(file: &PreparedFileAttachment) -> Result<(), FileAttachmentError> {
    save_file_attachment_with_progress(file, |_bytes_done, _bytes_total| {})
}

fn save_file_attachment_with_progress(
    file: &PreparedFileAttachment,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), FileAttachmentError> {
    let Some(storage_path) = file.storage_path.as_ref() else {
        return Ok(());
    };
    if let Some(bytes) = file.bytes.as_ref() {
        return write_attachment_bytes_with_progress(storage_path, bytes, &mut on_progress)
            .map_err(FileAttachmentError::WriteFailed);
    }
    if let Some(source_path) = file.source_path.as_ref() {
        copy_attachment_file_with_progress(
            source_path,
            storage_path,
            file.remove_source_after_save,
            &mut on_progress,
        )
        .map_err(FileAttachmentError::WriteFailed)?;
    }
    Ok(())
}

fn quote_file_attachment_path(path: &str) -> String {
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

pub(crate) fn format_file_attachment_prompt(paths: &[String]) -> String {
    match paths {
        [] => String::new(),
        [path] => format!("File: {}", quote_file_attachment_path(path)),
        _ => format!(
            "Files: [{}]",
            paths
                .iter()
                .map(|path| quote_file_attachment_path(path))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn normalize_attachment_operation_id(operation_id: Option<String>) -> String {
    operation_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("attachment-{}", image_paste_unique_token()))
}

fn display_attachment_basename(filename: &str) -> String {
    filename
        .trim()
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .unwrap_or("file")
        .to_string()
}

fn display_name_for_file_attachment(file: &gwt::FileAttachment) -> String {
    match file {
        gwt::FileAttachment::NativePath { path } => display_attachment_basename(path),
        gwt::FileAttachment::Inline { filename, .. }
        | gwt::FileAttachment::Uploaded { filename, .. } => display_attachment_basename(filename),
    }
}

#[derive(Debug, Clone)]
struct AttachmentProgressUpdate {
    id: String,
    operation_id: String,
    phase: AttachmentProgressPhase,
    file_index: Option<usize>,
    file_count: usize,
    filename: Option<String>,
    bytes_done: Option<u64>,
    bytes_total: Option<u64>,
    message: Option<String>,
}

impl AttachmentProgressUpdate {
    fn new(
        id: impl Into<String>,
        operation_id: impl Into<String>,
        phase: AttachmentProgressPhase,
        file_count: usize,
    ) -> Self {
        Self {
            id: id.into(),
            operation_id: operation_id.into(),
            phase,
            file_index: None,
            file_count,
            filename: None,
            bytes_done: None,
            bytes_total: None,
            message: None,
        }
    }

    fn filename(mut self, filename: Option<String>) -> Self {
        self.filename = filename;
        self
    }

    fn file(mut self, index: usize, filename: String) -> Self {
        self.file_index = Some(index);
        self.filename = Some(filename);
        self
    }

    fn bytes(mut self, bytes_done: u64, bytes_total: Option<u64>) -> Self {
        self.bytes_done = Some(bytes_done);
        self.bytes_total = bytes_total;
        self
    }

    fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    fn outbound(self, client_id: ClientId) -> OutboundEvent {
        OutboundEvent::reply(
            client_id,
            BackendEvent::AttachmentProgress {
                id: self.id,
                operation_id: self.operation_id,
                phase: self.phase,
                file_index: self.file_index,
                file_count: self.file_count,
                filename: self.filename,
                bytes_done: self.bytes_done,
                bytes_total: self.bytes_total,
                message: self.message,
            },
        )
    }

    fn dispatch(self, proxy: &AppEventProxy, client_id: &ClientId) {
        proxy.send(UserEvent::Dispatch(vec![self.outbound(client_id.clone())]));
    }
}

struct UploadedImagePasteOperation {
    upload_id: String,
    mime_type: String,
    filename: Option<String>,
    size: u64,
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
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let _ = agent_project_root;
    let agent_path = relative_path;
    let prompt_text = format!("{IMAGE_PASTE_PROMPT_PREFIX}{agent_path}");

    Ok(ImagePasteFile {
        bytes: Some(bytes),
        source_path: None,
        remove_source_after_save: false,
        storage_path,
        agent_path,
        prompt_text,
    })
}

pub(crate) fn prepare_uploaded_image_paste_file(
    worktree_path: &Path,
    upload_store: &AttachmentUploadStore,
    upload_id: &str,
    mime_type: &str,
    filename: Option<&str>,
    declared_size: u64,
    unique_token: &str,
) -> Result<ImagePasteFile, ImagePasteError> {
    let extension = image_extension_for_mime(mime_type)
        .ok_or_else(|| ImagePasteError::UnsupportedMimeType(mime_type.to_string()))?;
    let uploaded = upload_store
        .take(upload_id)
        .map_err(ImagePasteError::WriteFailed)?
        .ok_or_else(|| {
            ImagePasteError::WriteFailed(format!("uploaded image missing: {upload_id}"))
        })?;
    if uploaded.size == 0 || declared_size == 0 {
        return Err(ImagePasteError::EmptyPayload);
    }
    let stem = sanitize_image_paste_stem(filename.or(Some(uploaded.filename.as_str())));
    let file_name = format!("{unique_token}-{stem}.{extension}");
    let storage_path = worktree_path
        .join(".gwt")
        .join("drop-files")
        .join(&file_name);
    let relative_path = format!("{FILE_ATTACHMENT_RELATIVE_DIR}/{file_name}");
    let agent_path = relative_path;
    let prompt_text = format!("{IMAGE_PASTE_PROMPT_PREFIX}{agent_path}");

    Ok(ImagePasteFile {
        bytes: None,
        source_path: Some(uploaded.path),
        remove_source_after_save: true,
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

fn write_attachment_bytes_with_progress(
    storage_path: &Path,
    bytes: &[u8],
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let Some(parent) = storage_path.parent() else {
        return Err("attachment path has no parent directory".to_string());
    };
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let total = bytes.len() as u64;
    on_progress(0, Some(total));
    std::fs::write(storage_path, bytes).map_err(|error| error.to_string())?;
    on_progress(total, Some(total));
    Ok(())
}

fn copy_attachment_file_with_progress(
    source_path: &Path,
    storage_path: &Path,
    remove_source_after_save: bool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let Some(parent) = storage_path.parent() else {
        return Err("attachment path has no parent directory".to_string());
    };
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let total = std::fs::metadata(source_path)
        .ok()
        .map(|metadata| metadata.len());
    on_progress(0, total);
    let mut source = std::fs::File::open(source_path).map_err(|error| error.to_string())?;
    let mut destination = std::fs::File::create(storage_path).map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read =
            std::io::Read::read(&mut source, &mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        if let Err(error) = destination.write_all(&buffer[..read]) {
            let _ = std::fs::remove_file(storage_path);
            return Err(error.to_string());
        }
        copied += read as u64;
        on_progress(copied, total);
    }
    destination.flush().map_err(|error| error.to_string())?;
    if remove_source_after_save {
        let _ = std::fs::remove_file(source_path);
    }
    Ok(())
}

fn save_image_paste_file(image: &ImagePasteFile) -> Result<(), ImagePasteError> {
    save_image_paste_file_with_progress(image, |_bytes_done, _bytes_total| {})
}

fn save_image_paste_file_with_progress(
    image: &ImagePasteFile,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), ImagePasteError> {
    if let Some(bytes) = image.bytes.as_ref() {
        return write_attachment_bytes_with_progress(&image.storage_path, bytes, &mut on_progress)
            .map_err(ImagePasteError::WriteFailed);
    }
    if let Some(source_path) = image.source_path.as_ref() {
        return copy_attachment_file_with_progress(
            source_path,
            &image.storage_path,
            image.remove_source_after_save,
            &mut on_progress,
        )
        .map_err(ImagePasteError::WriteFailed);
    }
    Err(ImagePasteError::EmptyPayload)
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

    /// Replace all cached sessions with a freshly disk-loaded set. Called from
    /// the off-thread branch load (#2995) so resume availability and resolution
    /// observe session TOMLs the hook CLI wrote out-of-process after launch,
    /// without ever blocking the main UI thread on disk I/O.
    fn replace_sessions(&mut self, sessions: Vec<gwt_agent::Session>) {
        self.sessions = sessions;
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
        BackendEvent::WindowCanvasState { workspace },
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

    events.push(OutboundEvent::reply(
        client_id,
        BackendEvent::LaunchWizardState {
            wizard: launch_wizard.map(Box::new),
        },
    ));

    if let Some(state) = pending_update {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::UpdateState(state),
        ));
    }

    // SPEC-2359 W-17 (FR-397): bulky terminal snapshots go last so a
    // reconnect replay delivers lightweight state (wizard, statuses, update)
    // before scrollback payloads, instead of burying it behind them.
    for (id, snapshot) in terminal_snapshots {
        events.push(OutboundEvent::reply(
            client_id,
            BackendEvent::TerminalSnapshot {
                id,
                data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
            },
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

/// SPEC-2359 Phase W-12 (FR-349): map the agent-session Work lifecycle state
/// to its snake_case wire string for [`gwt::ActiveWorkItemView::lifecycle_state`].
fn work_active_lifecycle_state_wire(
    state: gwt_core::workspace_projection::WorkActiveLifecycleState,
) -> &'static str {
    use gwt_core::workspace_projection::WorkActiveLifecycleState;

    match state {
        WorkActiveLifecycleState::Active => "active",
        WorkActiveLifecycleState::Paused => "paused",
        WorkActiveLifecycleState::Done => "done",
        WorkActiveLifecycleState::Discarded => "discarded",
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
    works: Vec<gwt::WorkspaceHistoryView>,
    cleanup_candidate: Option<gwt::ActiveWorkCleanupCandidateView>,
) -> gwt::ActiveWorkProjectionView {
    let project_root = projection.project_root.clone();
    let mut agents = projection
        .agents
        .iter()
        .filter(|agent| {
            agent.is_assigned() || workspace_agent_summary_work_id(&project_root, agent).is_some()
        })
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    agents.sort_by(|left, right| {
        active_work_agent_priority_rank(left)
            .cmp(&active_work_agent_priority_rank(right))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let active_agents = agents
        .iter()
        .filter(|agent| agent.status_category == "active")
        .count();
    let blocked_agents = agents
        .iter()
        .filter(|agent| agent.status_category == "blocked")
        .count();
    let agent_branch = agents.iter().find_map(|agent| agent.branch.clone());
    let agent_worktree = agents.iter().find_map(|agent| agent.worktree_path.clone());
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
    let mut unassigned_agents = projection
        .agents
        .iter()
        .filter(|agent| {
            agent.is_unassigned() && workspace_agent_summary_work_id(&project_root, agent).is_none()
        })
        .map(active_work_agent_view_from_summary)
        .collect::<Vec<_>>();
    unassigned_agents.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let active_works = active_work_items_from_projection(&projection, &agents, &works);
    let active_work_count = active_works.len();

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
        works,
        cleanup_candidate,
        active_work_count,
        active_works,
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
        title: format!("{} Work", tab.title),
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
        works: Vec::new(),
        cleanup_candidate: None,
        active_work_count: 0,
        active_works: Vec::new(),
        agents: Vec::new(),
        unassigned_agents: Vec::new(),
    }
}

fn workspace_agent_summary_work_id(
    project_root: &Path,
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> Option<String> {
    gwt_core::workspace_projection::canonical_work_id(
        project_root,
        agent.branch.as_deref(),
        agent.worktree_path.as_deref(),
    )
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): the canonical Work identity for an
/// active agent. `agent_session_id` is the primary key so that "1 agent
/// session : 1 Work" holds — two agents on the same branch but with distinct
/// `session_id`s resolve to distinct Work rows. Legacy agents that report an
/// empty `session_id` fall back to the historical branch/worktree-derived
/// identity, then `workspace_id`, then the provided `legacy_fallback`.
fn active_work_agent_work_id(
    project_root: &Path,
    agent: &gwt::ActiveWorkAgentView,
    legacy_fallback: Option<&str>,
) -> Option<String> {
    let session_id = agent.session_id.trim();
    if !session_id.is_empty() {
        return Some(format!("work-session-{session_id}"));
    }
    let worktree_path = agent.worktree_path.as_deref().map(Path::new);
    gwt_core::workspace_projection::canonical_work_id(
        project_root,
        agent.branch.as_deref(),
        worktree_path,
    )
    .or_else(|| {
        agent
            .workspace_id
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
    .or_else(|| legacy_fallback.map(str::to_string))
}

fn projection_matches_active_work(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    work_id: &str,
) -> bool {
    projection
        .git_details
        .as_ref()
        .and_then(|details| {
            gwt_core::workspace_projection::canonical_work_id(
                &projection.project_root,
                details.branch.as_deref(),
                details.worktree_path.as_deref(),
            )
        })
        .as_deref()
        == Some(work_id)
}

/// SPEC-2359 Phase W-12 Slice 2 (FR-348): with `agent_session_id` as the
/// primary Work identity, a session-derived `work_id` no longer matches the
/// branch-derived id computed from the projection's `git_details`. The current
/// projection's Work row is now identified by checking whether the group's
/// representative agent shares the projection's branch or worktree, so the
/// title / status_text / summary / PR selection driven by `is_current_projection`
/// keeps choosing the live projection values.
fn agent_matches_projection_git_details(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    agent: &gwt::ActiveWorkAgentView,
) -> bool {
    let Some(details) = projection.git_details.as_ref() else {
        return false;
    };
    let branch_matches = details
        .branch
        .as_deref()
        .map(normalize_branch_name)
        .zip(agent.branch.as_deref().map(normalize_branch_name))
        .is_some_and(|(left, right)| left == right);
    let worktree_matches = details
        .worktree_path
        .as_deref()
        .zip(agent.worktree_path.as_deref())
        .is_some_and(|(left, right)| left == Path::new(right));
    branch_matches || worktree_matches
}

fn find_active_work_history<'a>(
    work_id: &str,
    first_agent: Option<&gwt::ActiveWorkAgentView>,
    works: &'a [gwt::WorkspaceHistoryView],
) -> Option<&'a gwt::WorkspaceHistoryView> {
    works.iter().find(|item| item.id == work_id).or_else(|| {
        works.iter().find(|item| {
            item.execution_containers.iter().any(|container| {
                let branch_matches = first_agent
                    .and_then(|agent| agent.branch.as_deref())
                    .zip(container.branch.as_deref())
                    .is_some_and(|(left, right)| left == right);
                let worktree_matches = first_agent
                    .and_then(|agent| agent.worktree_path.as_deref())
                    .zip(container.worktree_path.as_deref())
                    .is_some_and(|(left, right)| Path::new(left) == Path::new(right));
                branch_matches || worktree_matches
            })
        })
    })
}

fn active_work_items_from_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    agents: &[gwt::ActiveWorkAgentView],
    works: &[gwt::WorkspaceHistoryView],
) -> Vec<gwt::ActiveWorkItemView> {
    let mut grouped: Vec<(String, Vec<gwt::ActiveWorkAgentView>)> = Vec::new();
    for agent in agents {
        let work_id =
            active_work_agent_work_id(&projection.project_root, agent, Some(&projection.id))
                .unwrap_or_else(|| projection.id.clone());
        if let Some((_, group_agents)) = grouped.iter_mut().find(|(id, _)| id == &work_id) {
            group_agents.push(agent.clone());
        } else {
            grouped.push((work_id, vec![agent.clone()]));
        }
    }

    let mut active_works = grouped
        .into_iter()
        .map(|(work_id, agents)| {
            let first_agent = agents.first();
            let history = find_active_work_history(&work_id, first_agent, works);
            let container = history.and_then(|item| item.execution_containers.first());
            let is_current_projection = work_id == projection.id
                || projection_matches_active_work(projection, &work_id)
                || first_agent
                    .is_some_and(|agent| agent_matches_projection_git_details(projection, agent));
            let active_agents = agents
                .iter()
                .filter(|agent| agent.status_category == "active")
                .count();
            let blocked_agents = agents
                .iter()
                .filter(|agent| agent.status_category == "blocked")
                .count();
            // FR-403: live rows sort by their freshest agent activity.
            let row_updated_at = agents
                .iter()
                .map(|agent| agent.updated_at.clone())
                .max()
                .unwrap_or_default();
            let status_category = if blocked_agents > 0 {
                "blocked".to_string()
            } else if active_agents > 0 {
                "active".to_string()
            } else if let Some(history) = history {
                history.status_category.clone()
            } else {
                workspace_status_category_wire(projection.effective_status_category()).to_string()
            };
            let status_text = if is_current_projection {
                projection.status_text.clone()
            } else {
                history
                    .and_then(|item| item.summary.clone().or_else(|| item.intent.clone()))
                    .unwrap_or_else(|| {
                        if blocked_agents > 0 {
                            format!("{blocked_agents} blocked agents")
                        } else if active_agents == 1 {
                            "1 active agent".to_string()
                        } else {
                            format!("{} active agents", agents.len())
                        }
                    })
            };
            gwt::ActiveWorkItemView {
                id: work_id.clone(),
                // SPEC-3075 FR-002/FR-004: the Work title is its *identity*
                // (purpose). `current_focus` is the agent's live "what now"
                // (status) and must never become the Work title — otherwise a
                // status line like "...execution mode..." leaks in as the
                // identity. `title_summary` is the agent-declared purpose, so it
                // stays as a fallback; `current_focus` is removed entirely.
                title: history
                    .map(|item| item.title.clone())
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| is_current_projection.then(|| projection.title.clone()))
                    .or_else(|| first_agent.and_then(|agent| agent.title_summary.clone()))
                    .unwrap_or(work_id),
                status_category,
                status_text,
                summary: history
                    .and_then(|item| item.summary.clone().or_else(|| item.intent.clone()))
                    .or_else(|| {
                        is_current_projection
                            .then(|| projection.summary.clone())
                            .flatten()
                    }),
                owner: history.and_then(|item| item.owner.clone()).or_else(|| {
                    is_current_projection
                        .then(|| projection.owner.clone())
                        .flatten()
                }),
                next_action: if is_current_projection {
                    projection.next_action.clone()
                } else {
                    None
                },
                active_agents,
                blocked_agents,
                branch: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.branch.clone())
                } else {
                    container
                        .and_then(|value| value.branch.clone())
                        .or_else(|| first_agent.and_then(|agent| agent.branch.clone()))
                },
                worktree_path: if is_current_projection {
                    projection.git_details.as_ref().and_then(|details| {
                        details
                            .worktree_path
                            .as_ref()
                            .map(|path| path.display().to_string())
                    })
                } else {
                    container
                        .and_then(|value| value.worktree_path.clone())
                        .or_else(|| first_agent.and_then(|agent| agent.worktree_path.clone()))
                },
                pr_number: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_number)
                } else {
                    container.and_then(|value| value.pr_number)
                },
                pr_url: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_url.clone())
                } else {
                    container.and_then(|value| value.pr_url.clone())
                },
                pr_state: if is_current_projection {
                    projection
                        .git_details
                        .as_ref()
                        .and_then(|details| details.pr_state.clone())
                } else {
                    container.and_then(|value| value.pr_state.clone())
                },
                board_refs: if is_current_projection {
                    projection.board_refs.clone()
                } else {
                    history
                        .map(|item| item.board_refs.clone())
                        .unwrap_or_default()
                },
                agents,
                // SPEC-2359 Phase W-12 (FR-349): active_work_items groups live
                // assigned agents, so the owning agent session is Running and
                // not user-closed → Active.
                lifecycle_state: work_active_lifecycle_state_wire(
                    gwt_core::workspace_projection::recompute_work_active_lifecycle(
                        gwt_core::workspace_projection::WorkAgentRuntime::Running,
                        None,
                    ),
                )
                .to_string(),
                closed_at: None,
                session_agent_total: 0,
                merged_into_base: false,
                workspace_key: None,
                remote_only: false,
                done_equivalent: false,
                updated_at: row_updated_at,
            }
        })
        .collect::<Vec<_>>();

    // SPEC-2359 Phase W-12 Slice 5a (FR-350): merge in Paused Work — items that
    // persist in the work history but have no live agent group. These are Works
    // whose owning agent stopped without an explicit user close, so they stay on
    // the Work surface as Paused until closed. Dedupe against the live rows by id
    // and by branch/worktree so a resumed (live again) Work surfaces once as
    // Active, and the launch-recorded history row (keyed by the projection id but
    // covered by a live session) never produces a phantom Paused duplicate.
    append_paused_work_items(&mut active_works, works);
    active_works
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): append Paused `active_works` rows for
/// retained Work-history items that have no live agent group. A history item is
/// Paused when it is incomplete (not Done) and is not already represented by a
/// live row (matched by Work id or by branch/worktree execution container). Done
/// items are skipped here — close/cleanup is handled in a later slice.
fn append_paused_work_items(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    works: &[gwt::WorkspaceHistoryView],
) {
    for work in works {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): terminal closes (Done and
        // Discarded) leave the active Work surface. Both are excluded so a
        // closed Work never re-appears as a Paused row.
        if work.status_category == "done" || work.status_category == "discarded" {
            continue;
        }
        if active_work_already_present(active_works, work) {
            continue;
        }
        let container = work.execution_containers.first();
        let branch = container.and_then(|value| value.branch.clone());
        let worktree_path = container.and_then(|value| value.worktree_path.clone());
        let title = Some(work.title.clone())
            .filter(|value| !value.trim().is_empty())
            .or_else(|| work.summary.clone())
            .or_else(|| work.intent.clone())
            .unwrap_or_else(|| work.id.clone());
        let status_text = work
            .summary
            .clone()
            .or_else(|| work.intent.clone())
            .unwrap_or_else(|| "Paused".to_string());
        active_works.push(gwt::ActiveWorkItemView {
            id: work.id.clone(),
            title,
            // Paused Work has no running agent; surface an idle runtime status.
            status_category: "idle".to_string(),
            status_text,
            summary: work.summary.clone().or_else(|| work.intent.clone()),
            owner: work.owner.clone(),
            next_action: None,
            active_agents: 0,
            blocked_agents: 0,
            branch,
            worktree_path,
            pr_number: container.and_then(|value| value.pr_number),
            pr_url: container.and_then(|value| value.pr_url.clone()),
            pr_state: container.and_then(|value| value.pr_state.clone()),
            board_refs: work.board_refs.clone(),
            // Carry the persisted Work's agents (each with its Session history)
            // so a Paused Workspace still renders Work → Session in the detail.
            agents: work
                .agents
                .iter()
                .map(paused_work_agent_view_from_history)
                .collect(),
            // No live agent session owns this Work and it is not user-closed →
            // WorkAgentRuntime::None resolves to Paused (FR-350).
            lifecycle_state: work_active_lifecycle_state_wire(
                gwt_core::workspace_projection::recompute_work_active_lifecycle(
                    gwt_core::workspace_projection::WorkAgentRuntime::None,
                    None,
                ),
            )
            .to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            // FR-403: paused/backfill rows carry the record's last update.
            updated_at: work.updated_at.clone(),
        });
    }
}

/// SPEC-2359 Phase W-12 Slice 5a (FR-350): a Work-history item is already
/// represented by an existing (live) `active_works` row when their ids match or
/// when they share a branch / worktree identity. Used to dedupe Paused rows so a
/// resumed Work and the launch-recorded history row do not duplicate the live
/// Active row.
fn active_work_already_present(
    active_works: &[gwt::ActiveWorkItemView],
    work: &gwt::WorkspaceHistoryView,
) -> bool {
    active_works.iter().any(|existing| {
        if existing.id == work.id {
            return true;
        }
        // A live Work synthesized without git_details carries no execution
        // container, so also dedupe by shared agent session id (the launch /
        // synthesized history row and the live row reference the same session).
        let session_matches = existing.agents.iter().any(|live_agent| {
            !live_agent.session_id.trim().is_empty()
                && work
                    .agents
                    .iter()
                    .any(|history_agent| history_agent.session_id == live_agent.session_id)
        });
        if session_matches {
            return true;
        }
        work.execution_containers.iter().any(|container| {
            let branch_matches = existing
                .branch
                .as_deref()
                .map(normalize_branch_name)
                .zip(container.branch.as_deref().map(normalize_branch_name))
                .is_some_and(|(left, right)| left == right);
            let worktree_matches = existing
                .worktree_path
                .as_deref()
                .zip(container.worktree_path.as_deref())
                .is_some_and(|(left, right)| Path::new(left) == Path::new(right));
            branch_matches || worktree_matches
        })
    })
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

/// Index agent sessions by their gwt session id (the Work / launch id) so the
/// view builder can attach each Work's Session history.
fn work_session_index(
    sessions: &[gwt_agent::Session],
) -> std::collections::HashMap<&str, &gwt_agent::Session> {
    sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect()
}

pub(crate) fn workspace_work_item_view_from_item(
    item: &gwt_core::workspace_projection::WorkItem,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
) -> gwt::WorkspaceHistoryView {
    gwt::WorkspaceHistoryView {
        id: item.id.clone(),
        title: item.title.clone(),
        intent: item.intent.clone(),
        summary: item.summary.clone(),
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a discarded Work surfaces as the
        // dedicated `"discarded"` status so the Work surface and the Paused
        // exclusion treat it as a terminal close distinct from Done.
        status_category: if item.discarded {
            "discarded".to_string()
        } else {
            workspace_status_category_wire(item.status_category).to_string()
        },
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
            .map(|agent| workspace_work_agent_view_from_ref(agent, session_index))
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
    agent: &gwt_core::workspace_projection::WorkAgentRef,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
) -> gwt::WorkspaceHistoryAgentView {
    // A Work's `session_id` is the gwt session id (the launch). It keys into the
    // persisted Session whose forward-only `session_history` is the Session list
    // (agent-tool conversation UUIDs) under this Work; the latest
    // `agent_session_id` marks the currently active Session.
    let sessions = session_index
        .get(agent.session_id.as_str())
        .map(|session| {
            let latest = session.agent_session_id.as_deref();
            // Render Sessions in stable chronological order (oldest first) so
            // clock skew or delayed persistence cannot scramble the timeline;
            // the append order alone is not guaranteed monotonic.
            let mut entries: Vec<_> = session.session_history.iter().collect();
            entries.sort_by_key(|entry| entry.started_at);
            if entries.is_empty() {
                // SPEC-2359 W-16 (FR-402 follow-up): `session_history` is newer
                // than most ledger TOMLs (zero coverage on long-lived machines),
                // but the latest conversation pointer still exists. Synthesize
                // it as the single Session row instead of "No session yet".
                return latest
                    .map(|conversation| {
                        vec![gwt::WorkspaceHistorySessionView {
                            agent_session_id: conversation.to_string(),
                            started_at: session
                                .updated_at
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            is_active: true,
                            resumable: session.is_resumable_conversation(conversation),
                        }]
                    })
                    .unwrap_or_default();
            }
            entries
                .into_iter()
                .map(|entry| gwt::WorkspaceHistorySessionView {
                    agent_session_id: entry.agent_session_id.clone(),
                    started_at: entry
                        .started_at
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    is_active: latest == Some(entry.agent_session_id.as_str()),
                    // A Session whose conversation handle is structurally
                    // unusable (empty / Codex placeholder) is history-only; the
                    // surface hides its Resume control.
                    resumable: session.is_resumable_conversation(&entry.agent_session_id),
                })
                .collect()
        })
        .unwrap_or_default();
    // Work records written without agent metadata (older record paths)
    // would render as an anonymous "Agent" group (user verification
    // 2026-06-12) — borrow identity from the ledger TOML when available.
    let ledger = session_index.get(agent.session_id.as_str());
    let display_name = agent
        .display_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            ledger
                .map(|session| session.display_name.clone())
                .filter(|name| !name.trim().is_empty())
        });
    let agent_id = agent
        .agent_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| ledger.map(|session| session.agent_id.command().to_string()));
    gwt::WorkspaceHistoryAgentView {
        session_id: agent.session_id.clone(),
        agent_id,
        display_name,
        updated_at: agent
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        sessions,
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
    event: &gwt_core::workspace_projection::WorkEvent,
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
    kind: gwt_core::workspace_projection::WorkEventKind,
) -> &'static str {
    use gwt_core::workspace_projection::WorkEventKind;

    match kind {
        WorkEventKind::Start => "start",
        WorkEventKind::Claim => "claim",
        WorkEventKind::Update => "update",
        WorkEventKind::Blocked => "blocked",
        WorkEventKind::Handoff => "handoff",
        WorkEventKind::Resume => "resume",
        WorkEventKind::Split => "split",
        WorkEventKind::Merge => "merge",
        WorkEventKind::Pr => "pr",
        WorkEventKind::Pause => "pause",
        WorkEventKind::Done => "done",
        WorkEventKind::Discard => "discard",
        WorkEventKind::Backfill => "backfill",
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

/// #3065: build the Workspace Resume context from the resumed branch's own
/// Work item. The repo-shared current projection (`current.json`) must NOT be
/// the source here: it carries the identity of whatever Work last wrote it,
/// and replaying that identity into a different Work's resume event is how
/// one Work's owner/title leaked into every other Workspace row. When no
/// Work item matches the container, the context is neutral — never the
/// shared identity.
fn workspace_resume_context_for_work_item(
    repo_path: &Path,
    branch: Option<&str>,
    worktree_path: &Path,
) -> WorkspaceResumeContext {
    let item = gwt_core::workspace_projection::load_workspace_work_items(repo_path)
        .ok()
        .flatten()
        .and_then(|projection| {
            gwt_core::workspace_projection::find_work_item_for_container(
                &projection,
                repo_path,
                branch,
                Some(worktree_path),
            )
            .cloned()
        });
    match item {
        Some(item) => WorkspaceResumeContext {
            title: non_empty_workspace_text(Some(&item.title)),
            owner: non_empty_workspace_text(item.owner.as_deref()),
            summary: non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        },
        None => WorkspaceResumeContext {
            title: None,
            owner: None,
            summary: None,
            next_action: None,
        },
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

/// SPEC-2359 W-17 (FR-398): dedup window for launches that are past window
/// registration but not yet live. Entries also clear on launch completion.
const INFLIGHT_LAUNCH_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Identity of a launch for in-flight dedup. Includes the agent and the
/// resume conversation so parallel restores of *different* Sessions on the
/// same Work (startup auto-resume) and multi-agent launches on one Work stay
/// allowed — only a re-request of the *same* launch dedupes. `None` when the
/// config carries neither a branch nor a working dir: such launches have no
/// stable Work identity and must never dedup against each other.
fn inflight_launch_key(tab_id: &str, config: &gwt_agent::LaunchConfig) -> Option<String> {
    let branch = config
        .branch
        .as_deref()
        .map(normalize_branch_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_default();
    let dir = config
        .working_dir
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if branch.is_empty() && dir.is_empty() {
        return None;
    }
    let agent = config.agent_id.command();
    let resume = config.resume_session_id.as_deref().unwrap_or_default();
    Some(format!(
        "{tab_id}\u{001f}{agent}\u{001f}{branch}\u{001f}{dir}\u{001f}{resume}"
    ))
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
        // Live projection summaries do not carry conversation history; Paused
        // Works fill this in from the persisted Session via
        // `paused_work_agent_view_from_history`.
        sessions: Vec::new(),
    }
}

/// Convert a persisted Work's agent (a launch, carrying its Session history) to
/// the active-surface agent view so Paused Workspaces render their Work →
/// Session list instead of an empty agent list.
/// SPEC-2359 Phase W-16 (FR-402): attach machine-local ledger sessions to
/// each Workspace (branch) row. Sessions whose TOML carries this project's
/// repo hash and the row's branch join the row's agents (deduped by gwt
/// session id, capped per [`crate::workspace_session_registry`]); the
/// uncapped count rides `session_agent_total` so the frontend can render
/// "+N more sessions".
fn attach_registry_sessions_to_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    agent_sessions: &[gwt_agent::Session],
    project_repo_hash: Option<gwt_core::repo_hash::RepoHash>,
    session_index: &std::collections::HashMap<&str, &gwt_agent::Session>,
) {
    let registry = crate::workspace_session_registry::branch_session_registry(
        agent_sessions,
        project_repo_hash.as_ref().map(|hash| hash.as_str()),
    );
    let cap = crate::workspace_session_registry::REGISTRY_SESSION_CAP;
    for work in active_works.iter_mut() {
        let existing: Vec<&str> = work
            .agents
            .iter()
            .map(|agent| agent.session_id.as_str())
            .collect();
        let (additions, extra_total) =
            crate::workspace_session_registry::registry_sessions_for_branch(
                &registry,
                work.branch.as_deref(),
                &existing,
                cap,
            );
        work.session_agent_total = (work.agents.len() + extra_total) as u32;
        for session in additions {
            let agent_ref = gwt_core::workspace_projection::WorkAgentRef {
                session_id: session.id.clone(),
                agent_id: Some(session.agent_id.command().to_string()),
                display_name: Some(session.display_name.clone()),
                updated_at: session.last_activity_at,
            };
            let history_view = workspace_work_agent_view_from_ref(&agent_ref, session_index);
            work.agents
                .push(paused_work_agent_view_from_history(&history_view));
        }
        // User verification 2026-06-12 (follow-up): ghost record agents —
        // ledger TOML gone, no identity recorded, no conversation — render
        // as a dead "Agent / No session yet" group whose Resume cannot work.
        // Drop them from the view; the Work row itself stays.
        {
            let before = work.agents.len();
            work.agents.retain(|agent| {
                !agent.display_name.trim().is_empty()
                    || !agent.agent_id.trim().is_empty()
                    || !agent.sessions.is_empty()
            });
            let dropped = (before - work.agents.len()) as u32;
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped);
        }
        // User verification 2026-06-12: a Resume creates a new gwt session for
        // the SAME agent conversation, which used to render as two Work rows
        // ("Agent" + "Claude Code") carrying one conversation id. Collapse
        // agents whose latest conversation matches — newest updated_at wins
        // and borrows the duplicate's display_name when its own is empty.
        {
            let mut sorted: Vec<gwt::ActiveWorkAgentView> = std::mem::take(&mut work.agents);
            sorted.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let mut seen_conversations: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            let mut kept: Vec<gwt::ActiveWorkAgentView> = Vec::with_capacity(sorted.len());
            let mut dropped = 0usize;
            for agent in sorted {
                let conversation = agent
                    .sessions
                    .iter()
                    .find(|session| session.is_active)
                    .or_else(|| agent.sessions.first())
                    .map(|session| session.agent_session_id.clone());
                match conversation {
                    Some(conversation) if !conversation.is_empty() => {
                        if let Some(&index) = seen_conversations.get(&conversation) {
                            if kept[index].display_name.trim().is_empty()
                                && !agent.display_name.trim().is_empty()
                            {
                                kept[index].display_name = agent.display_name.clone();
                            }
                            dropped += 1;
                        } else {
                            seen_conversations.insert(conversation, kept.len());
                            kept.push(agent);
                        }
                    }
                    _ => kept.push(agent),
                }
            }
            work.agents = kept;
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped as u32);
        }
        // User verification 2026-06-12 (screenshot): one group per historical
        // gwt session rendered e.g. five identical "Claude Code" groups. Per
        // agent identity only the latest history entry stays; live (active /
        // running) agents are never collapsed away — two running panes are
        // two real agents.
        {
            let mut sorted: Vec<gwt::ActiveWorkAgentView> = std::mem::take(&mut work.agents);
            sorted.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let mut seen_identities: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut kept: Vec<gwt::ActiveWorkAgentView> = Vec::with_capacity(sorted.len());
            let mut dropped = 0usize;
            for agent in sorted {
                let live = matches!(
                    agent.status_category.as_str(),
                    "active" | "running" | "blocked"
                );
                let identity = if !agent.display_name.trim().is_empty() {
                    agent.display_name.trim().to_lowercase()
                } else {
                    agent.agent_id.trim().to_lowercase()
                };
                if live || identity.is_empty() || seen_identities.insert(identity) {
                    kept.push(agent);
                } else {
                    dropped += 1;
                }
            }
            work.agents = kept;
            work.session_agent_total = work.session_agent_total.saturating_sub(dropped as u32);
        }
        // The cap applies to the row's TOTAL agents: a decomposed legacy row
        // can carry hundreds of record agents, and the workspace payload feeds
        // every connected client (unbounded fan-out amplifies the WebSocket
        // eviction storm). Keep the newest agents; the uncapped count already
        // rides `session_agent_total`. RFC3339 UTC strings sort lexically.
        if work.agents.len() > cap {
            work.agents
                .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            work.agents.truncate(cap);
        }
    }
    // SPEC-2359 Phase W-16 (FR-403): order the list by last update, newest
    // first — the row stamp or its freshest agent/ledger session, whichever
    // is newer. RFC3339 UTC strings compare lexically.
    let row_sort_key = |work: &gwt::ActiveWorkItemView| -> String {
        work.agents
            .iter()
            .map(|agent| agent.updated_at.clone())
            .chain(std::iter::once(work.updated_at.clone()))
            .max()
            .unwrap_or_default()
    };
    active_works.sort_by_key(|work| std::cmp::Reverse(row_sort_key(work)));
}

/// SPEC-2359 W16-2 (FR-389 / SC-259): assign every row its Workspace
/// grouping key (canonical branch identity → canonical worktree identity →
/// own id) and merge rows that share a key into ONE Workspace row. The
/// newest row is the representative; agents concatenate (the identity
/// collapse downstream dedups), numeric counts sum, and `merged_into_base`
/// ORs. Old branchless ids keep their own key, so legacy rows never vanish
/// or fuse.
fn assign_and_merge_workspace_groups(
    active_works: &mut Vec<gwt::ActiveWorkItemView>,
    project_root: &Path,
) {
    for work in active_works.iter_mut() {
        let branch = work
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let worktree = work.worktree_path.as_deref().map(std::path::Path::new);
        let key = gwt_core::workspace_projection::canonical_work_id(project_root, branch, None)
            .or_else(|| {
                gwt_core::workspace_projection::canonical_work_id(project_root, None, worktree)
            })
            .unwrap_or_else(|| work.id.clone());
        work.workspace_key = Some(key);
    }

    let mut merged: Vec<gwt::ActiveWorkItemView> = Vec::with_capacity(active_works.len());
    let mut index_by_key: HashMap<String, usize> = HashMap::new();
    for work in active_works.drain(..) {
        let key = work
            .workspace_key
            .clone()
            .unwrap_or_else(|| work.id.clone());
        match index_by_key.get(&key) {
            Some(&slot) => {
                let target = &mut merged[slot];
                let newer = work.updated_at > target.updated_at;
                let mut agents = std::mem::take(&mut target.agents);
                agents.extend(work.agents.iter().cloned());
                let active_agents = target.active_agents + work.active_agents;
                let blocked_agents = target.blocked_agents + work.blocked_agents;
                let session_agent_total = target.session_agent_total + work.session_agent_total;
                let merged_into_base = target.merged_into_base || work.merged_into_base;
                if newer {
                    let key = target.workspace_key.clone();
                    // SPEC-3075 FR-004: a session-derived row's title/owner is
                    // agent content (the live session), not the branch's
                    // identity. When a fresher session row merges into a
                    // branch-backed Work, take its fresher status but preserve
                    // the branch-backed identity so another agent's content
                    // never surfaces as this Work's title.
                    let target_branch_backed = !target.id.starts_with("work-session-");
                    let work_session_derived = work.id.starts_with("work-session-");
                    let preserved_identity = (target_branch_backed && work_session_derived)
                        .then(|| (target.title.clone(), target.owner.clone()));
                    *target = work;
                    target.workspace_key = key;
                    if let Some((title, owner)) = preserved_identity {
                        target.title = title;
                        if owner.is_some() {
                            target.owner = owner;
                        }
                    }
                }
                target.agents = agents;
                target.active_agents = active_agents;
                target.blocked_agents = blocked_agents;
                target.session_agent_total = session_agent_total;
                target.merged_into_base = merged_into_base;
                if target.branch.is_none() {
                    // keep any branch the group knows about
                    target.branch = merged_branch_fallback(&target.agents);
                }
            }
            None => {
                index_by_key.insert(key, merged.len());
                merged.push(work);
            }
        }
    }
    *active_works = merged;
}

fn merged_branch_fallback(agents: &[gwt::ActiveWorkAgentView]) -> Option<String> {
    agents.iter().find_map(|agent| agent.branch.clone())
}

/// SPEC-2359 W16-3 (FR-390): flag rows whose branch exists only as a fetched
/// remote ref — no recorded worktree path and no local worktree for the
/// branch. Display-only marking (FR-381/FR-390: rendering generates no
/// events); the existing Launch path materializes a worktree on demand.
fn mark_remote_only_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    local_branches: Option<&std::collections::HashSet<String>>,
) {
    for work in active_works.iter_mut() {
        let has_worktree = work
            .worktree_path
            .as_deref()
            .map(str::trim)
            .is_some_and(|path| !path.is_empty());
        if has_worktree {
            work.remote_only = false;
            continue;
        }
        let branch_local = work
            .branch
            .as_deref()
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
            .map(|branch| local_branches.is_some_and(|set| set.contains(&branch)));
        // Branchless rows are never "remote": there is nothing to fetch.
        work.remote_only = matches!(branch_local, Some(false));
    }
}

/// SPEC-2359 W-15 (FR-386): flag rows whose branch is merged into a base on
/// origin (background scan cache) or whose recorded PR state is merged — the
/// "safe to delete" signal. Display-only; no automatic close (US-61).
fn mark_merged_active_works(
    active_works: &mut [gwt::ActiveWorkItemView],
    merged_branches: Option<&HashMap<String, chrono::DateTime<chrono::Utc>>>,
) {
    for work in active_works.iter_mut() {
        let merge_reference = work
            .branch
            .as_deref()
            .map(crate::runtime_support::normalize_branch_name)
            .and_then(|branch| merged_branches.and_then(|map| map.get(&branch)))
            .copied();
        let by_pr = work
            .pr_state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
        work.merged_into_base = merge_reference.is_some() || by_pr;

        // SPEC-2359 W16-4 (FR-391): merged ∧ stale → derived Done-equivalent.
        // Membership rides the scan verdict ONLY (pr_state stays badge-only);
        // explicit terminal closes keep their own lifecycle; no event is ever
        // recorded from this classification (US-61).
        let terminal = matches!(work.lifecycle_state.as_str(), "done" | "discarded");
        let last_activity = work
            .agents
            .iter()
            .map(|agent| agent.updated_at.as_str())
            .chain(std::iter::once(work.updated_at.as_str()))
            .filter_map(|stamp| {
                chrono::DateTime::parse_from_rfc3339(stamp)
                    .ok()
                    .map(|value| value.with_timezone(&chrono::Utc))
            })
            .max();
        work.done_equivalent = !terminal
            && last_activity.is_some_and(|last| {
                gwt_core::workspace_projection::derive_merged_done_equivalent(
                    merge_reference.is_some(),
                    last,
                    merge_reference,
                )
            });
    }
}

fn paused_work_agent_view_from_history(
    agent: &gwt::WorkspaceHistoryAgentView,
) -> gwt::ActiveWorkAgentView {
    gwt::ActiveWorkAgentView {
        session_id: agent.session_id.clone(),
        window_id: None,
        agent_id: agent.agent_id.clone().unwrap_or_default(),
        display_name: agent.display_name.clone().unwrap_or_default(),
        affiliation_status: "assigned".to_string(),
        workspace_id: None,
        status_category: "idle".to_string(),
        current_focus: None,
        title_summary: None,
        branch: None,
        worktree_path: None,
        last_board_entry_id: None,
        last_board_entry_kind: None,
        coordination_scope: None,
        updated_at: agent.updated_at.clone(),
        sessions: agent.sessions.clone(),
    }
}

fn active_agent_session_matches_work(
    session: &ActiveAgentSession,
    normalized_branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> bool {
    let branch_matches = normalized_branch
        .is_some_and(|branch| normalize_branch_name(session.branch_name.trim()) == branch);
    let worktree_matches = worktree_path.is_some_and(|path| {
        same_worktree_path(&session.worktree_path, path) || session.worktree_path == path
    });
    branch_matches || worktree_matches
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
    live_session_ids: &std::collections::HashSet<String>,
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
        live_session_ids,
    )
}

fn save_resumed_workspace_projection(
    project_root: &Path,
    session: &ActiveAgentSession,
    base_branch: Option<&str>,
    linked_issue_number: Option<u64>,
    workspace_resume_context: &WorkspaceResumeContext,
    live_session_ids: &std::collections::HashSet<String>,
) -> Result<(), String> {
    save_workspace_launch_projection(
        project_root,
        session,
        base_branch,
        linked_issue_number,
        Some(workspace_resume_context),
        session.branch_name.starts_with("work/"),
        live_session_ids,
    )
}

#[derive(Debug, Clone)]
pub struct ProjectTabRuntime {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) project_root: PathBuf,
    pub(crate) kind: gwt::ProjectKind,
    pub(crate) workspace: WindowCanvasState,
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
/// Build the `(program, args)` that opens `url` in the OS default browser.
///
/// Windows deliberately uses `rundll32 url.dll,FileProtocolHandler <url>`
/// instead of `cmd /C start "" <url>`. `cmd.exe` re-parses its command line with
/// shell rules, so a URL's `&` (the query-string separator) is treated as a
/// command separator: the browser receives only the text up to the first `&`
/// and every later parameter is dropped. For OAuth authorize URLs that silently
/// strips `redirect_uri`, `scope`, and `state`, producing Slack's
/// "redirect_uri did not match" / "No scopes requested" errors. `rundll32`
/// (like `open` / `xdg-open`) receives the URL as a single CreateProcess
/// argument and hands the full string to the default protocol handler, so `&`
/// and `%` survive verbatim.
fn os_url_open_command(url: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open", vec![url.to_string()])
    } else if cfg!(target_os = "windows") {
        (
            "rundll32.exe",
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
        )
    } else {
        ("xdg-open", vec![url.to_string()])
    }
}

fn open_url_with_os_default(url: &str) -> Result<(), std::io::Error> {
    use std::process::Command;
    let (program, args) = os_url_open_command(url);
    let child = Command::new(program).args(&args).spawn()?;
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
    pub(crate) launch_error_terminal_details: HashMap<String, String>,
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
    /// SPEC-2359 W-17 (FR-398, Issue #3034): launches whose window is
    /// registered but whose agent session is not live yet, keyed by
    /// (tab, branch, working dir). A re-click in this window focuses the
    /// pending window instead of spawning a duplicate. Entries clear on
    /// launch completion/failure or after a TTL.
    pub(crate) inflight_launches: HashMap<String, (String, std::time::Instant)>,
    pub(crate) pending_auto_resume_sources: HashMap<String, String>,
    pub(crate) pending_startup_auto_resume_sessions: Vec<PendingStartupAutoResumeSession>,
    pub(crate) active_agent_sessions: HashMap<String, ActiveAgentSession>,
    /// SPEC-2359 W-15 (FR-386): per-project set of branches (canonical names)
    /// fully merged into a base on origin, filled by the background merge
    /// scan. Runtime-only; never persisted.
    /// SPEC-2359 W-15/W16-4 (FR-386/FR-391): merged branches per project →
    /// merge reference time (branch tip committer time proxy). Drives the
    /// "safe to delete" badge and the derived Done-equivalent classification.
    pub(crate) work_merged_branches:
        HashMap<PathBuf, HashMap<String, chrono::DateTime<chrono::Utc>>>,
    /// Incremental loader for the machine-local session ledger; keeps
    /// projection rebuilds from re-parsing thousands of unchanged TOMLs
    /// (window-close latency fix, 2026-06-11). RefCell: the runtime lives on
    /// the single event-loop thread and the projection builder takes `&self`.
    pub(crate) session_ledger_cache:
        std::cell::RefCell<crate::session_ledger_cache::SessionLedgerCache>,
    /// Same root fix for the home works.json (megabytes of Work items +
    /// events): cache hit clones instead of re-parsing per projection event.
    pub(crate) work_items_cache: std::cell::RefCell<gwt_core::workspace_projection::WorkItemsCache>,
    /// SPEC-2359 W-16 (FR-387): last work-events ingest per project — the
    /// 30s throttle for tab-change / post-launch triggers.
    pub(crate) last_work_events_ingest: std::cell::RefCell<HashMap<PathBuf, std::time::Instant>>,
    /// SPEC-2359 W16-3 (FR-390): normalized branch names that currently have
    /// a LOCAL worktree, per project — refreshed by the worktree reconcile.
    /// The view marks `remote_only` by cache lookup alone (no git spawn on
    /// the projection build path).
    pub(crate) local_worktree_branches:
        std::cell::RefCell<HashMap<PathBuf, std::collections::HashSet<String>>>,
    pub(crate) window_pty_statuses: HashMap<String, WindowProcessStatus>,
    pub(crate) window_hook_states: HashMap<String, WindowProcessStatus>,
    pub(crate) hook_forward_target: Option<HookForwardTarget>,
    pub(crate) issue_link_cache_dir: PathBuf,
    /// Cached update state so late-connecting WebView clients get the toast.
    pub(crate) pending_update: Option<gwt_core::update::UpdateState>,
    /// Shared PTY writer registry published to the WebSocket fast-path.
    pub(crate) pty_writers: PtyWriterRegistry,
    /// Browser-uploaded attachment temp files waiting to be staged under the
    /// active worktree.
    pub(crate) attachment_uploads: AttachmentUploadStore,
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
    /// SPEC-2970: notifies the background usage poller to refresh immediately
    /// (e.g. after the Claude opt-in toggle changes). `None` in unit tests and
    /// before `set_usage_refresh` is called during startup wiring.
    pub(crate) usage_refresh: Option<std::sync::Arc<tokio::sync::Notify>>,
}

impl ProjectTabRuntime {
    pub(crate) fn from_persisted(
        tab: gwt::PersistedSessionTabState,
        workspace: gwt::PersistedWindowCanvasState,
    ) -> Self {
        Self {
            id: tab.id,
            title: tab.title,
            project_root: tab.project_root,
            kind: tab.kind,
            workspace: WindowCanvasState::from_persisted(workspace),
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
        attachment_uploads: AttachmentUploadStore,
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
            launch_error_terminal_details: HashMap::new(),
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
            inflight_launches: HashMap::new(),
            pending_launch_feedback_contexts: HashMap::new(),
            pending_auto_resume_sources: HashMap::new(),
            pending_startup_auto_resume_sessions: Vec::new(),
            active_agent_sessions: HashMap::new(),
            work_merged_branches: HashMap::new(),
            session_ledger_cache: std::cell::RefCell::new(
                crate::session_ledger_cache::SessionLedgerCache::new(),
            ),
            work_items_cache: std::cell::RefCell::new(
                gwt_core::workspace_projection::WorkItemsCache::new(),
            ),
            last_work_events_ingest: std::cell::RefCell::new(HashMap::new()),
            local_worktree_branches: std::cell::RefCell::new(HashMap::new()),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            pending_update: None,
            pty_writers,
            attachment_uploads,
            persist_dispatcher,
            file_tree_worktree_roots: HashMap::new(),
            server_url: None,
            usage_refresh: None,
        };
        app.rebuild_window_lookup();
        app.seed_window_pty_statuses();
        app.seed_restored_window_details();
        Ok(app)
    }

    /// SPEC-2359 W-15 (FR-386): store the background merged-branch scan
    /// result and rebroadcast the Workspace projection so the "safe to
    /// delete" badge appears. Display-only; never records a close (US-61).
    pub(crate) fn apply_work_merge_status(
        &mut self,
        project_root: &Path,
        merged_branches: HashMap<String, chrono::DateTime<chrono::Utc>>,
    ) -> Vec<OutboundEvent> {
        self.work_merged_branches
            .insert(project_root.to_path_buf(), merged_branches);
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    /// SPEC-2359 W-15 (FR-386): scan the project's unclosed Workspace
    /// branches for merge-into-base off the UI thread (one `git cherry` per
    /// branch). Sends [`UserEvent::WorkMergeStatus`] when anything merged is
    /// found; silent otherwise so stub-proxy tests stay quiet.
    /// SPEC-2359 W-16 (FR-387): note an ingest attempt for `project_root`;
    /// returns false while the 30s throttle window is still open. Bootstrap
    /// and project-open callers pass `force` to bypass the window.
    pub(crate) fn note_work_events_ingest_attempt(&self, project_root: &Path, force: bool) -> bool {
        let now = std::time::Instant::now();
        let mut last = self.last_work_events_ingest.borrow_mut();
        if !force {
            if let Some(previous) = last.get(project_root) {
                if now.duration_since(*previous) < Duration::from_secs(30) {
                    return false;
                }
            }
        }
        last.insert(project_root.to_path_buf(), now);
        true
    }

    /// SPEC-2359 W-16 (FR-387): run the cross-machine work events ingest on a
    /// background thread, then hand control back to the event loop via
    /// [`UserEvent::WorkEventsIngested`] so the worktree reconcile runs in
    /// intake → reconcile order (plan decision 9).
    pub(crate) fn spawn_work_events_ingest(&self, project_root: PathBuf, force: bool) {
        if !self.note_work_events_ingest_attempt(&project_root, force) {
            return;
        }
        let proxy = self.proxy.clone();
        // Resolve the home-projection paths on the calling thread: HOME is
        // process-global and parallel unit tests scope it per test
        // (ScopedEnvVar, #3022) — a late resolution inside the worker would
        // race those scopes and write into another test's home.
        let work_items_path =
            gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&project_root);
        let state_path = gwt_core::paths::gwt_workspace_work_events_intake_state_path_for_repo_path(
            &project_root,
        );
        let projection_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        thread::spawn(move || {
            let summary = crate::work_events_ingest::ingest_project_work_events_paths(
                &project_root,
                &work_items_path,
                &state_path,
            );
            // #3065: detection-based repair for the resume owner bleed. Runs
            // after every ingest so re-ingested contaminated logs (from other
            // machines / refs) self-heal; converges to a no-op on clean data.
            let repaired = gwt_core::workspace_projection::repair_resume_owner_bleed_paths(
                &work_items_path,
                &projection_path,
                chrono::Utc::now(),
            )
            .map(|report| report.changed())
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "resume owner bleed repair failed");
                false
            });
            proxy.send(UserEvent::WorkEventsIngested {
                project_root,
                changed: summary.changed() || repaired,
            });
        });
    }

    /// Event-loop continuation of [`Self::spawn_work_events_ingest`]:
    /// reconcile worktrees after the intake, kick the merge scan, and
    /// rebroadcast the projection when the intake applied anything.
    pub(crate) fn handle_work_events_ingested(
        &mut self,
        project_root: PathBuf,
        changed: bool,
    ) -> Vec<OutboundEvent> {
        self.reconcile_workspace_worktrees(&project_root);
        self.spawn_work_merge_status_scan(project_root);
        if changed {
            self.active_work_projection_broadcast_for_active_tab()
                .into_iter()
                .collect()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn spawn_work_merge_status_scan(&self, project_root: PathBuf) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let Ok(projection) =
                gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(
                    &project_root,
                )
            else {
                return;
            };
            let mut branches: Vec<String> = Vec::new();
            for item in &projection.work_items {
                if item.is_terminal() {
                    continue;
                }
                for container in &item.execution_containers {
                    if let Some(branch) = container
                        .branch
                        .as_deref()
                        .map(crate::runtime_support::normalize_branch_name)
                    {
                        if !branch.is_empty() && !branches.contains(&branch) {
                            branches.push(branch);
                        }
                    }
                }
            }
            if branches.is_empty() {
                return;
            }
            let mut merged: Vec<String> = Vec::new();
            for branch in branches {
                if matches!(
                    gwt_git::branch::merged_base_target(&project_root, &branch),
                    Ok(Some(_))
                ) {
                    merged.push(branch);
                }
            }
            if merged.is_empty() {
                return;
            }
            // SPEC-2359 W16-4 (FR-391): one extra spawn resolves every tip
            // committer time — the merge-reference-time proxy for the derived
            // Done classification (plan decision 8).
            let tip_times =
                gwt_git::refs::branch_tip_committer_times(&project_root).unwrap_or_default();
            let merged_branches: HashMap<String, chrono::DateTime<chrono::Utc>> = merged
                .into_iter()
                .map(|branch| {
                    let unix = tip_times
                        .get(&branch)
                        .or_else(|| tip_times.get(&format!("origin/{branch}")))
                        .copied();
                    let reference = unix
                        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
                        .unwrap_or_else(chrono::Utc::now);
                    (branch, reference)
                })
                .collect();
            proxy.send(UserEvent::WorkMergeStatus {
                project_root,
                merged_branches,
            });
        });
    }

    /// SPEC-2359 Phase W-15 (FR-379/FR-380/FR-382): reconcile locally existing
    /// worktrees with the persisted Work records. Worktrees without a record
    /// are backfilled (event into the worktree's own `.gwt/work/events.jsonl`
    /// plus the home works projection) so the Workspace list shows the union
    /// of existing worktrees and unclosed records. Errors are logged and
    /// swallowed — reconciliation must never block startup or project open.
    pub(crate) fn reconcile_workspace_worktrees(&self, project_root: &Path) {
        let entries = match gwt::worktree_inventory::enumerate_worktrees(project_root, None) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(
                    "workspace worktree reconcile: enumerate failed for {}: {error}",
                    project_root.display()
                );
                return;
            }
        };
        // SPEC-2359 W16-3 (FR-390): refresh the local-worktree branch set the
        // remote_only view marking reads (cache lookup only — no git spawn at
        // view time).
        let local_branches: std::collections::HashSet<String> = entries
            .iter()
            .filter_map(|entry| entry.branch.as_deref())
            .map(crate::runtime_support::normalize_branch_name)
            .filter(|branch| !branch.is_empty())
            .collect();
        self.local_worktree_branches
            .borrow_mut()
            .insert(project_root.to_path_buf(), local_branches);
        let sources = gwt::worktree_inventory::worktree_reconcile_sources(&entries);
        if sources.is_empty() {
            return;
        }
        match gwt_core::workspace_projection::reconcile_worktree_work_items(
            project_root,
            &sources,
            chrono::Utc::now(),
        ) {
            Ok(0) => {}
            Ok(count) => tracing::info!(
                "workspace worktree reconcile: backfilled {count} worktree(s) for {}",
                project_root.display()
            ),
            Err(error) => tracing::warn!(
                "workspace worktree reconcile failed for {}: {error}",
                project_root.display()
            ),
        }
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
            // SPEC-2359 Phase W-16 (FR-393): decompose legacy mega-items
            // (pre-W-12 records keyed to one projection UUID fusing dozens of
            // branches) into canonical branch-keyed items so each branch row
            // shows its real title / sessions. Idempotent; must run before
            // the intake/reconcile chain so decomposed branches are not
            // redundantly backfilled.
            let _ = gwt_core::workspace_projection::decompose_legacy_multi_branch_work_items(
                &tab.project_root,
            );
            // SPEC-2359 W-16 (FR-387): cross-machine work events intake.
            // Supersedes the one-shot `rebuild_work_items_from_events_for_repo`
            // migration gate — the intake is a permanently-installed idempotent
            // consumer over the same (and more) sources. Runs on a background
            // thread; its completion event then runs the worktree reconcile
            // (intake → reconcile order) and the merge scan.
            self.spawn_work_events_ingest(tab.project_root.clone(), true);
            // SPEC-2359 Phase W-11 (US-58 / FR-346): one-shot, version-guarded
            // clear of legacy prompt-derived title_summary / current_focus so
            // existing broken titles ("あなたの目的は何ですか" etc.) heal via the
            // display fallback and agent re-authoring. Idempotent via
            // `agent_identity.migration.json`; never re-clears agent-authored
            // values written after the marker.
            let _ = gwt_core::workspace_projection::reset_legacy_agent_identity_for_repo(
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
            // Issue #2942: a persisted Stopped agent placeholder means the user
            // did not explicitly close the window (closing removes it from the
            // workspace). Such "still open" windows must restore regardless of
            // the session's status drift (e.g. idle-timeout -> Stopped) or age,
            // honoring "restore everything not explicitly closed". Sessions with
            // no placeholder are orphans (the workspace lost the window); keep
            // the conservative status / freshness gates so old, windowless
            // sessions are not resurrected at startup.
            // SPEC-2359 G: a Session whose worktree no longer exists on this
            // machine (moved machines, deleted repo, a path from another OS)
            // cannot be auto-resumed; skip here so a stale path never reaches an
            // async spawn that fails later. Applies to both placeholder and
            // orphan sessions (orphans previously skipped this check).
            if !session.worktree_path.exists() {
                continue;
            }
            let placeholder_tab = self.paused_placeholder_tab_for_session(&session.id);
            // Orphan sessions (workspace lost the window) keep the conservative
            // status / freshness gates so old, windowless sessions are not
            // resurrected; placeholder sessions restore regardless (Issue #2942).
            if placeholder_tab.is_none() {
                if !startup_auto_resume_window_was_open(&session) {
                    continue;
                }
                if !session.exact_auto_resume_candidate() {
                    continue;
                }
                if !startup_auto_resume_is_fresh(&session, now) {
                    continue;
                }
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
            let Some(tab_id) =
                placeholder_tab.or_else(|| self.auto_resume_tab_id_for_session(&session))
            else {
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
            let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                &session.worktree_path,
                Some(session.branch.as_str()),
                &session.worktree_path,
            ));
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
            let fallback_geometry =
                startup_auto_resume_window_geometry(index, total, bounds.clone());
            let mut spawned = self.spawn_restored_agent_session(
                &pending_session.tab_id,
                pending_session.session,
                pending_session.workspace_resume_context,
                fallback_geometry,
            );
            events.append(&mut spawned);
        }
        events
    }

    /// Spawn a single restored agent window from a persisted session, reusing
    /// the paused placeholder's geometry when present (Issue #2942). Shared by
    /// startup auto-resume and the Open Project restore path so both honor the
    /// "restore everything the user did not explicitly close" rule. Records the
    /// source session in `pending_auto_resume_sources` so the lifecycle handler
    /// retires the old session once the resumed window reports its own id.
    fn spawn_restored_agent_session(
        &mut self,
        tab_id: &str,
        session: gwt_agent::Session,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        fallback_geometry: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let config = launch_config_from_persisted_session(&session);
        let geometry = self
            .remove_stale_paused_agent_window(tab_id, &session.id)
            .unwrap_or(fallback_geometry);
        // Snapshot the window registry *after* the paused placeholder is
        // removed: the freshly spawned window may reuse the placeholder's id
        // (ids are assigned lowest-free), so a pre-removal snapshot would fail
        // to detect it and the source session would never be retired.
        let existing_windows = self
            .window_lookup
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        match self.spawn_agent_window_at_geometry(
            tab_id,
            config,
            geometry,
            workspace_resume_context,
        ) {
            Ok(events) => {
                if let Some(window_id) = self
                    .window_lookup
                    .keys()
                    .find(|window_id| !existing_windows.contains(*window_id))
                    .cloned()
                {
                    self.pending_auto_resume_sources
                        .insert(window_id, session.id);
                }
                events
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session.id,
                    error = %error,
                    "failed to spawn restored agent window"
                );
                Vec::new()
            }
        }
    }

    /// Restore every process window the user did not explicitly close in a
    /// freshly opened/restored project tab (Issue #2942). Closing a window
    /// removes it from the persisted workspace, so the persisted process
    /// windows are exactly the set to restart: agents resume via their native
    /// session id (or launch fresh when none exists), and non-agent process
    /// windows (e.g. Shell) launch fresh. Runs synchronously because each
    /// placeholder already carries its geometry, so no frontend canvas bounds
    /// round-trip is required. The startup `bootstrap` queue only covers tabs
    /// open at launch, so projects opened via Open Project / Reopen Recent were
    /// never restored before this path existed.
    fn restore_open_project_windows(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let windows = match self.tab(tab_id) {
            Some(tab) if tab.kind == gwt::ProjectKind::Git && !tab.migration_pending => tab
                .workspace
                .persisted()
                .windows
                .iter()
                .filter(|window| {
                    window.preset.requires_process()
                        && window.status == WindowProcessStatus::Stopped
                })
                .cloned()
                .collect::<Vec<_>>(),
            _ => return Vec::new(),
        };

        let mut events = Vec::new();
        for window in windows {
            let combined = combined_window_id(tab_id, &window.id);
            // A window with a live PTY/runtime is already running (e.g. when an
            // already-open project tab is re-selected); only paused placeholders
            // should be restarted. `window_lookup` is the registry of known
            // windows, not the set of running ones, so it must not gate here.
            if self.runtimes.contains_key(&combined) {
                continue;
            }
            if crate::runtime_support::window_is_agent_pane(&window) {
                let Some(session_id) = window.session_id.clone() else {
                    continue;
                };
                let path = self.sessions_dir.join(format!("{session_id}.toml"));
                let Ok(session) = gwt_agent::Session::load_and_migrate(&path) else {
                    continue;
                };
                if !session.worktree_path.exists() {
                    continue;
                }
                if self
                    .active_agent_sessions
                    .values()
                    .any(|active| active.session_id == session.id)
                {
                    continue;
                }
                let workspace_resume_context = Some(workspace_resume_context_for_work_item(
                    &session.worktree_path,
                    Some(session.branch.as_str()),
                    &session.worktree_path,
                ));
                let fallback_geometry = window.geometry.clone();
                let mut spawned = self.spawn_restored_agent_session(
                    tab_id,
                    session,
                    workspace_resume_context,
                    fallback_geometry,
                );
                events.append(&mut spawned);
            } else {
                events.extend(self.start_window(
                    tab_id,
                    &window.id,
                    window.preset,
                    window.geometry.clone(),
                ));
            }
        }
        events
    }

    /// Find the tab holding a persisted, paused (`Stopped`) agent placeholder
    /// window backed by `session_id`. Its presence proves the user did not
    /// explicitly close that window (Issue #2942), so the session must restore
    /// regardless of status drift or age.
    fn paused_placeholder_tab_for_session(&self, session_id: &str) -> Option<String> {
        self.tabs
            .iter()
            .filter(|tab| tab.kind == gwt::ProjectKind::Git && !tab.migration_pending)
            .find(|tab| {
                tab.workspace.persisted().windows.iter().any(|window| {
                    window.status == WindowProcessStatus::Stopped
                        && crate::runtime_support::window_is_agent_pane(window)
                        && window.session_id.as_deref() == Some(session_id)
                })
            })
            .map(|tab| tab.id.clone())
    }

    fn remove_stale_paused_agent_window(
        &mut self,
        tab_id: &str,
        session_id: &str,
    ) -> Option<WindowGeometry> {
        let tab = self.tab_mut(tab_id)?;
        let stale = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .find(|w| {
                w.preset == WindowPreset::Agent
                    && w.status == WindowProcessStatus::Stopped
                    && w.session_id.as_deref() == Some(session_id)
            })
            .map(|w| (w.id.clone(), w.geometry.clone()));
        let (raw_id, geometry) = stale?;
        tab.workspace.close_window(&raw_id);
        let combined = combined_window_id(tab_id, &raw_id);
        self.window_lookup.remove(&combined);
        self.window_details.remove(&combined);
        Some(geometry)
    }

    fn auto_resume_tab_id_for_session(&self, session: &gwt_agent::Session) -> Option<String> {
        if let Some(tab) = self.tabs.iter().find(|tab| {
            tab.kind == gwt::ProjectKind::Git
                && !tab.migration_pending
                && same_worktree_path(&tab.project_root, &session.worktree_path)
        }) {
            return Some(tab.id.clone());
        }

        // Issue #2942: a session's worktree belongs to the tab whose project
        // shares the same main worktree root (the gwt workspace home / bare
        // layout root). `repo_hash` / `project_scope_hash` differ between a
        // workspace-home project_root and its linked worktrees, so scope-hash
        // equality alone fails to associate worktree-backed agent sessions with
        // the parent tab and they never auto-resume on startup.
        if let Ok(session_root) = gwt_git::worktree::main_worktree_root(&session.worktree_path) {
            if let Some(tab) = self.tabs.iter().find(|tab| {
                tab.kind == gwt::ProjectKind::Git
                    && !tab.migration_pending
                    && same_worktree_path(&tab.main_worktree_root(), &session_root)
            }) {
                return Some(tab.id.clone());
            }
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

    /// SPEC-2970: wire the usage poller's refresh handle so frontend toggles
    /// can request an immediate re-poll.
    pub(crate) fn set_usage_refresh(&mut self, refresh: std::sync::Arc<tokio::sync::Notify>) {
        self.usage_refresh = Some(refresh);
    }

    /// SPEC-2970 FR-009/FR-013: persist the Claude account-usage opt-in and
    /// request an immediate refresh.
    fn set_claude_account_usage_enabled_events(&self, enabled: bool) -> Vec<OutboundEvent> {
        if let Err(error) = gwt_config::Settings::update_global(|settings| {
            settings.usage.claude_account_enabled = enabled;
            Ok(())
        }) {
            tracing::warn!(%error, "failed to persist Claude usage opt-in");
        }
        self.request_usage_refresh_events()
    }

    /// SPEC-2970 FR-022: nudge the background poller to refresh now.
    fn request_usage_refresh_events(&self) -> Vec<OutboundEvent> {
        if let Some(refresh) = &self.usage_refresh {
            refresh.notify_one();
        }
        Vec::new()
    }

    pub(crate) fn handle_frontend_event(
        &mut self,
        client_id: ClientId,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        log_frontend_user_action(&client_id, &event);
        match event {
            FrontendEvent::FrontendReady => {
                // SPEC-2970: kick an immediate usage poll on connect so the
                // status-bar pill populates right away instead of waiting for
                // the next 30s poller tick (otherwise a freshly loaded page
                // shows an empty usage cell).
                if let Some(refresh) = &self.usage_refresh {
                    refresh.notify_one();
                }
                self.frontend_sync_events(&client_id)
            }
            FrontendEvent::SetClaudeAccountUsageEnabled { enabled } => {
                self.set_claude_account_usage_enabled_events(enabled)
            }
            FrontendEvent::RefreshUsage => self.request_usage_refresh_events(),
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
            FrontendEvent::PaneSendInput { session_id, text } => {
                self.pane_send_input_events(client_id, &session_id, &text)
            }
            FrontendEvent::PasteImage {
                id,
                data_base64,
                mime_type,
                filename,
            } => self.paste_image_events(&id, &data_base64, &mime_type, filename.as_deref()),
            FrontendEvent::PasteImageUploaded {
                id,
                operation_id,
                upload_id,
                mime_type,
                filename,
                size,
            } => {
                if operation_id.is_some() {
                    self.paste_image_uploaded_operation_events(
                        client_id,
                        id,
                        operation_id,
                        UploadedImagePasteOperation {
                            upload_id,
                            mime_type,
                            filename,
                            size,
                        },
                    )
                } else {
                    self.paste_image_uploaded_events(
                        &id,
                        &upload_id,
                        &mime_type,
                        filename.as_deref(),
                        size,
                    )
                }
            }
            FrontendEvent::AttachFiles {
                id,
                operation_id,
                files,
            } => {
                if operation_id.is_some() {
                    self.attach_files_operation_events(client_id, id, operation_id, files)
                } else {
                    self.attach_files_events(&id, files)
                }
            }
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
                title,
                parent_id,
                topics,
                owners,
                targets,
                mentions,
                target_workspace,
                broadcast,
            } => self.post_board_entry_events(
                &client_id,
                BoardPostRequest {
                    id,
                    entry_kind,
                    body,
                    title,
                    parent_id,
                    topics,
                    owners,
                    targets,
                    mentions,
                    target_workspace,
                    broadcast,
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
            FrontendEvent::ResumeWorkspaceAgent {
                session_id,
                agent_session_id,
                bounds,
            } => {
                self.resume_workspace_agent_events(&client_id, session_id, agent_session_id, bounds)
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
            FrontendEvent::ApplyUpdateToVersion { version } => {
                self.apply_update_to_version_events(&client_id, version)
            }
            FrontendEvent::CloseWork {
                work_id,
                close_kind,
            } => self.close_work(&work_id, &close_kind),
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
            FrontendEvent::GetBoardAuthStatus => self.board_auth_status_events(client_id, None),
            FrontendEvent::BoardProviderSignIn { provider } => {
                self.board_provider_sign_in_events(client_id, &provider)
            }
            FrontendEvent::BoardProviderSignOut { provider } => {
                self.board_provider_sign_out_events(client_id, &provider)
            }
            FrontendEvent::UpdateBoardProviderConfig {
                provider,
                client_id: provider_client_id,
                default_channel,
                tenant_id,
                client_secret,
            } => self.board_provider_config_update_events(
                client_id,
                &provider,
                provider_client_id,
                default_channel,
                tenant_id,
                client_secret,
            ),
            FrontendEvent::UpdateBoardOauthPort { port } => {
                self.board_oauth_port_update_events(client_id, port)
            }
            FrontendEvent::UpdateSystemSettings {
                language,
                codex_trust_managed_hooks,
                board_provider,
            } => self.system_settings_update_events(
                client_id,
                language,
                codex_trust_managed_hooks,
                board_provider,
            ),
            FrontendEvent::GetAutostartStatus => self.autostart_status_events(client_id),
            FrontendEvent::UpdateAutostart { enabled } => {
                self.autostart_update_events(client_id, enabled)
            }
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
    ///
    /// SPEC #2780 v2 Amendment (FR-013): `current_version` is included so the
    /// frontend can label the Update / Downgrade / Current action button.
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
                current_version: env!("CARGO_PKG_VERSION").to_string(),
            }
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// SPEC #2780 v2 Amendment (FR-014): user clicked Update / Downgrade on
    /// a specific release in the Release Notes window. Resolves the platform
    /// asset for the requested tag on a worker thread (network), then routes
    /// through the existing `ApplyUpdateStart` pipeline so the standard
    /// update modal renders downloading → ready → restart.
    ///
    /// Codex review on PR #2917: the resolved state is also published as
    /// `UserEvent::UpdateAvailable` so `AppRuntime.pending_update` reflects
    /// the chosen release. Without this step, `ApplyUpdateLater` /
    /// `ApplyUpdateRestartNow` (which both gate on `self.pending_update`)
    /// would either no-op or fire against an unrelated latest-update state
    /// when the user selected a downgrade while `pending_update` was
    /// `UpToDate`.
    fn apply_update_to_version_events(
        &self,
        client_id: &str,
        version: String,
    ) -> Vec<OutboundEvent> {
        let proxy = self.proxy.clone();
        let client_id_owned = client_id.to_string();
        self.blocking_tasks.spawn(move || {
            let manager = gwt_core::update::UpdateManager::new();
            let current_exe = std::env::current_exe().ok();
            match manager.resolve_state_for_version(&version, current_exe.as_deref()) {
                Ok(state) => {
                    // Update `pending_update` first so Later / Restart now
                    // read the selected release. The frontend update-cta
                    // ignores the broadcast `UpdateState` here because its
                    // local status is already `applying` (the modal was
                    // opened by `beginUpdateDownloading` on click).
                    proxy.send(UserEvent::UpdateAvailable(state.clone()));
                    proxy.send(UserEvent::ApplyUpdateStart {
                        state,
                        client_id: client_id_owned,
                    });
                }
                Err(message) => {
                    proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                        client_id_owned,
                        update_apply_error_failed("Resolve release", &message),
                    )]));
                }
            }
        });
        vec![]
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

    /// SPEC-2963: reply with remote Board provider sign-in state plus the
    /// editable (non-secret) provider configuration for the settings UI.
    fn board_auth_status_events(
        &self,
        client_id: ClientId,
        message: Option<String>,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::board_auth_status_event(message),
        )]
    }

    /// SPEC-2963: persist remote Board provider configuration captured in the
    /// settings UI, then reply with the refreshed auth/config view. Non-secret
    /// fields go to `config.toml`; the client secret goes to the secure store.
    #[allow(clippy::too_many_arguments)]
    fn board_provider_config_update_events(
        &self,
        client_id: ClientId,
        provider: &str,
        provider_client_id: Option<String>,
        default_channel: Option<String>,
        tenant_id: Option<String>,
        client_secret: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(path) = gwt_config::Settings::global_config_path() else {
            return self.board_auth_status_events(
                client_id,
                Some("unable to resolve home directory (`~/.gwt/config.toml`)".to_string()),
            );
        };
        let message = match gwt::system_settings::write_board_provider_config(
            &path,
            provider,
            provider_client_id,
            default_channel,
            tenant_id,
            client_secret,
        ) {
            Ok(_) => Some(format!("Saved {provider} configuration.")),
            Err(error) => Some(format!("Failed to save configuration: {error}")),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963 FR-005: persist the fixed OAuth callback port, then reply with
    /// the refreshed auth/config view. The new port binds on the next launch.
    fn board_oauth_port_update_events(&self, client_id: ClientId, port: u16) -> Vec<OutboundEvent> {
        let Some(path) = gwt_config::Settings::global_config_path() else {
            return self.board_auth_status_events(
                client_id,
                Some("unable to resolve home directory (`~/.gwt/config.toml`)".to_string()),
            );
        };
        let message = match gwt::system_settings::write_oauth_redirect_port(&path, port) {
            Ok(saved) => Some(format!(
                "Saved OAuth callback port {saved}. Restart gwt and register \
                 http://127.0.0.1:{saved}/oauth/callback in the provider app."
            )),
            Err(error) => Some(format!("Failed to save OAuth port: {error}")),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963: begin OAuth sign-in for a remote Board provider by opening the
    /// browser to the authorize URL (redirect back to the embedded server).
    fn board_provider_sign_in_events(
        &self,
        client_id: ClientId,
        provider: &str,
    ) -> Vec<OutboundEvent> {
        let kind = match provider.trim().to_ascii_lowercase().as_str() {
            "slack" => gwt_config::BoardProviderKind::Slack,
            "teams" => gwt_config::BoardProviderKind::Teams,
            other => {
                return self.board_auth_status_events(
                    client_id,
                    Some(format!("Unknown provider '{other}'")),
                );
            }
        };
        // The OAuth redirect uses a fixed loopback callback port (from
        // settings.board.oauth_redirect_port), not the embedded server's
        // ephemeral URL, so sign-in works regardless of how the GUI server
        // bound. The dedicated callback listener is started at server boot.
        let settings = gwt_config::Settings::load().unwrap_or_default();
        let message = match gwt::board_remote::signin::begin_signin(kind, &settings) {
            Ok(authorize_url) => match open_url_with_os_default(&authorize_url) {
                Ok(()) => Some(format!(
                    "Opened the browser to sign in to {provider}. Complete it, then Refresh."
                )),
                Err(error) => Some(format!("Failed to open browser: {error}")),
            },
            Err(reason) => Some(reason),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963: clear stored credentials for a remote Board provider.
    fn board_provider_sign_out_events(
        &self,
        client_id: ClientId,
        provider: &str,
    ) -> Vec<OutboundEvent> {
        let key = match provider.trim().to_ascii_lowercase().as_str() {
            "slack" => "slack",
            "teams" => "teams",
            other => {
                return self.board_auth_status_events(
                    client_id,
                    Some(format!("Unknown provider '{other}'")),
                );
            }
        };
        let message = match gwt::board_remote::signin::sign_out(key) {
            Ok(()) => Some(format!("Signed out of {provider}.")),
            Err(error) => Some(format!("Failed to sign out: {error}")),
        };
        self.board_auth_status_events(client_id, message)
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
        board_provider: Option<String>,
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
            gwt::system_settings::update_event(
                &path,
                language,
                codex_trust_managed_hooks,
                board_provider,
            ),
        )]
    }

    fn autostart_status_events(&self, client_id: ClientId) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            autostart_status_event_from_result(
                gwt::cli::tray::autostart::AutostartManager::status(),
            ),
        )]
    }

    fn autostart_update_events(&self, client_id: ClientId, enabled: bool) -> Vec<OutboundEvent> {
        let result = if enabled {
            gwt::cli::tray::autostart::AutostartManager::install()
        } else {
            gwt::cli::tray::autostart::AutostartManager::uninstall()
        };
        let event = match result {
            Ok(()) => autostart_status_event_from_result(
                gwt::cli::tray::autostart::AutostartManager::status(),
            ),
            Err(error) => BackendEvent::AutostartError {
                message: error.to_string(),
            },
        };
        vec![OutboundEvent::reply(client_id, event)]
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
        let mut terminal_snapshots = self
            .runtimes
            .iter()
            .filter_map(|(id, runtime)| {
                // SPEC-1919 FR-001a / SPEC-2008 Phase 26.F: snapshot replay
                // must preserve the current formatted screen and enough
                // scrollback history for a fresh xterm.js instance to scroll
                // immediately after reconnect.
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.snapshot_bytes())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then_some((id.clone(), snapshot))
            })
            .collect::<Vec<_>>();
        let runtime_snapshot_ids = terminal_snapshots
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        for (id, detail) in &self.launch_error_terminal_details {
            if !runtime_snapshot_ids.contains(id)
                && self.window_status(id) == Some(WindowProcessStatus::Error)
            {
                terminal_snapshots.push((id.clone(), Self::launch_error_terminal_bytes(detail)));
            }
        }

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

    /// SPEC-2359 W-17 (FR-396): re-send full snapshots for panes whose
    /// streamed output was dropped under client queue pressure, restoring
    /// display consistency for the affected client only.
    pub(crate) fn client_pane_snapshot_repair_events(
        &self,
        client_id: &str,
        pane_ids: &[String],
    ) -> Vec<OutboundEvent> {
        pane_ids
            .iter()
            .filter_map(|id| {
                let runtime = self.runtimes.get(id)?;
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.snapshot_bytes())
                    .unwrap_or_default();
                (!snapshot.is_empty()).then(|| {
                    OutboundEvent::reply(
                        client_id,
                        BackendEvent::TerminalSnapshot {
                            id: id.clone(),
                            data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                        },
                    )
                })
            })
            .collect()
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
        let saved_projection =
            gwt_core::workspace_projection::load_workspace_projection(&tab.project_root)
                .ok()
                .flatten();
        // SPEC-2359 Phase W-15 (FR-379/FR-382): the Workspace list is the
        // union of existing worktrees and unclosed records, independent of
        // live agents and of whether the project was ever launched here. When
        // no projection has been saved yet (fresh home / never-launched
        // project) but Work records exist (e.g. worktree backfill), synthesize
        // a default projection so the records still surface.
        let loaded_projection = saved_projection.or_else(|| {
            self.work_items_cache
                .borrow_mut()
                .load_or_synthesize(&tab.project_root)
                .ok()
                .filter(|works| !works.work_items.is_empty())
                .map(|_| {
                    gwt_core::workspace_projection::WorkspaceProjection::default_for_project(
                        &tab.project_root,
                    )
                })
        });
        if let Some(projection) = loaded_projection {
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
            if had_saved_agents && !projection.has_current_agents() {
                projection.reset_idle_identity(&tab.title, updated_at);
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
            let agent_sessions = self
                .session_ledger_cache
                .borrow_mut()
                .load(&self.sessions_dir);
            let session_index = work_session_index(&agent_sessions);
            let workspaces = self
                .work_items_cache
                .borrow_mut()
                .load_or_synthesize(&tab.project_root)
                .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                    updated_at,
                    work_items: Vec::new(),
                })
                .work_items
                .iter()
                .map(|item| workspace_work_item_view_from_item(item, &session_index))
                .collect::<Vec<_>>();
            let mut view = active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                cleanup_candidate,
            );
            // SPEC-2359 W16-2 (FR-389): group Works sharing a canonical
            // branch into one Workspace row before the ledger attach, so the
            // attach / identity-collapse / cap run once per Workspace.
            assign_and_merge_workspace_groups(&mut view.active_works, &tab.project_root);
            // SPEC-2359 Phase W-16 (FR-402): attach the machine-local session
            // ledger to each Workspace (branch) row so sessions surface even
            // when works.json never recorded an agent for the branch.
            attach_registry_sessions_to_active_works(
                &mut view.active_works,
                &agent_sessions,
                gwt_core::repo_hash::detect_repo_hash(&tab.project_root),
                &session_index,
            );
            // SPEC-2359 W-15 (FR-386): "safe to delete" badge inputs — the
            // background merge-scan cache plus the recorded PR state.
            mark_merged_active_works(
                &mut view.active_works,
                self.work_merged_branches.get(&tab.project_root),
            );
            // SPEC-2359 W16-3 (FR-390): "Remote" rows — branch known only
            // from fetched refs, no local worktree (cache lookup only).
            mark_remote_only_active_works(
                &mut view.active_works,
                self.local_worktree_branches.borrow().get(&tab.project_root),
            );
            return Some(view);
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
        let active_works = vec![gwt::ActiveWorkItemView {
            id: tab_id.to_string(),
            title: format!("{} Work", tab.title),
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
            board_refs: Vec::new(),
            agents: agents.clone(),
            // SPEC-2359 Phase W-12 (FR-349): synthesized from live sessions, so
            // the owning agent is Running and not user-closed → Active.
            lifecycle_state: work_active_lifecycle_state_wire(
                gwt_core::workspace_projection::recompute_work_active_lifecycle(
                    gwt_core::workspace_projection::WorkAgentRuntime::Running,
                    None,
                ),
            )
            .to_string(),
            closed_at: None,
            session_agent_total: 0,
            merged_into_base: false,
            workspace_key: None,
            remote_only: false,
            done_equivalent: false,
            updated_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }];
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
            works: Vec::new(),
            cleanup_candidate: None,
            active_work_count: active_works.len(),
            active_works,
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
                // Issue #2942: restore the opened tab's process windows the
                // user did not explicitly close — resume agents (native session
                // id) and fresh-launch shells. The startup `bootstrap` queue
                // only covers tabs open at launch, so projects opened via this
                // path (Open Project / Reopen Recent) were never restored and
                // their agent panes stayed `Stopped`.
                if let Some(active_tab_id) = self.active_tab_id.clone() {
                    events.extend(self.restore_open_project_windows(&active_tab_id));
                }
                // SPEC-2359 W-16 (FR-387): run the cross-machine intake for
                // the opened project; its completion event reconciles the
                // worktrees (intake → reconcile order) and kicks the merge
                // scan, then rebroadcasts the projection.
                if let Some(project_root) = self
                    .active_tab_id
                    .as_ref()
                    .and_then(|id| self.tabs.iter().find(|tab| &tab.id == id))
                    .map(|tab| tab.project_root.clone())
                {
                    self.spawn_work_events_ingest(project_root, true);
                }
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
            workspace: WindowCanvasState::from_persisted({
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
        // SPEC-2359 W-16 (FR-387): tab changes piggyback the cross-machine
        // intake, throttled to once per 30s per project.
        if let Some(project_root) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.project_root.clone())
        {
            self.spawn_work_events_ingest(project_root, false);
        }
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

    /// SPEC-3050 FR-001/FR-002: inject one line of input into the pane bound
    /// to `session_id`. The event carries a session id instead of a window id,
    /// so a caller can only ever reach the pane of the session it presents;
    /// resolution + the live-runtime check both reply with an explicit
    /// `pane_send_result` (FR-005: no silent drop, unlike `terminal_input`).
    pub(crate) fn pane_send_input_events(
        &mut self,
        client_id: ClientId,
        session_id: &str,
        text: &str,
    ) -> Vec<OutboundEvent> {
        let target = self.tabs.iter().find_map(|tab| {
            tab.workspace
                .persisted()
                .windows
                .iter()
                .find(|window| window.session_id.as_deref() == Some(session_id))
                .map(|window| combined_window_id(&tab.id, &window.id))
        });
        let Some(window_id) = target else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: None,
                    error: Some(format!("no pane bound to session {session_id}")),
                },
            )];
        };

        let write_result = match self.runtimes.get(&window_id) {
            None => Err(format!("no live runtime for pane {window_id}")),
            Some(runtime) => runtime
                .pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| {
                    pane.write_input(text.as_bytes())
                        .map_err(|error| error.to_string())
                }),
        };

        match write_result {
            Ok(()) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: true,
                    window_id: Some(window_id),
                    error: None,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: Some(window_id),
                    error: Some(error),
                },
            )],
        }
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

    pub(crate) fn inject_attachment_prompt_events(
        &mut self,
        client_id: ClientId,
        window_id: String,
        operation_id: String,
        prompt: String,
        file_count: usize,
        filename: Option<String>,
    ) -> Vec<OutboundEvent> {
        let mut events = vec![AttachmentProgressUpdate::new(
            window_id.clone(),
            operation_id.clone(),
            AttachmentProgressPhase::Injecting,
            file_count,
        )
        .filename(filename.clone())
        .outbound(client_id.clone())];
        let terminal_events = self.terminal_input_events(&window_id, &prompt);
        if terminal_events.is_empty() {
            events.push(
                AttachmentProgressUpdate::new(
                    window_id,
                    operation_id,
                    AttachmentProgressPhase::Attached,
                    file_count,
                )
                .filename(filename)
                .outbound(client_id),
            );
        } else {
            events.extend(terminal_events);
            events.push(
                AttachmentProgressUpdate::new(
                    window_id,
                    operation_id,
                    AttachmentProgressPhase::Failed,
                    file_count,
                )
                .filename(filename)
                .message("failed to inject attachment prompt")
                .outbound(client_id),
            );
        }
        events
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

    fn paste_image_uploaded_operation_events(
        &mut self,
        client_id: ClientId,
        id: String,
        operation_id: Option<String>,
        upload: UploadedImagePasteOperation,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            tracing::debug!(window_id = %id, "uploaded image paste dropped: window not found");
            return Vec::new();
        };
        if self.tab(&address.tab_id).is_none() {
            tracing::debug!(window_id = %id, "uploaded image paste dropped: project tab not found");
            return Vec::new();
        }
        let Some(session) = self.active_agent_sessions.get(&id) else {
            tracing::debug!(
                window_id = %id,
                "uploaded image paste dropped: active agent session not found"
            );
            return Vec::new();
        };
        let operation_id = normalize_attachment_operation_id(operation_id);
        let display_filename = upload
            .filename
            .as_deref()
            .map(display_attachment_basename)
            .or_else(|| Some(display_attachment_basename("image")));
        let worktree_path = session.worktree_path.clone();
        let upload_store = self.attachment_uploads.clone();
        let proxy = self.proxy.clone();
        let spawner = self.blocking_tasks.clone();
        let worker_client_id = client_id.clone();
        let worker_window_id = id.clone();
        let worker_operation_id = operation_id.clone();
        let worker_filename = display_filename.clone();

        spawner.spawn(move || {
            let image = match prepare_uploaded_image_paste_file(
                &worktree_path,
                &upload_store,
                &upload.upload_id,
                &upload.mime_type,
                upload.filename.as_deref(),
                upload.size,
                &image_paste_unique_token(),
            ) {
                Ok(image) => image,
                Err(error) => {
                    AttachmentProgressUpdate::new(
                        worker_window_id.clone(),
                        worker_operation_id.clone(),
                        AttachmentProgressPhase::Failed,
                        1,
                    )
                    .file(
                        0,
                        worker_filename
                            .clone()
                            .unwrap_or_else(|| "image".to_string()),
                    )
                    .message(error.to_string())
                    .dispatch(&proxy, &worker_client_id);
                    return;
                }
            };
            let progress_filename = worker_filename
                .clone()
                .or_else(|| Some(display_attachment_basename(&image.agent_path)));
            if let Err(error) = save_image_paste_file_with_progress(&image, |bytes_done, total| {
                AttachmentProgressUpdate::new(
                    worker_window_id.clone(),
                    worker_operation_id.clone(),
                    AttachmentProgressPhase::Staging,
                    1,
                )
                .file(
                    0,
                    progress_filename
                        .clone()
                        .unwrap_or_else(|| "image".to_string()),
                )
                .bytes(bytes_done, total)
                .dispatch(&proxy, &worker_client_id);
            }) {
                AttachmentProgressUpdate::new(
                    worker_window_id.clone(),
                    worker_operation_id.clone(),
                    AttachmentProgressPhase::Failed,
                    1,
                )
                .filename(progress_filename)
                .message(error.to_string())
                .dispatch(&proxy, &worker_client_id);
                return;
            }
            proxy.send(UserEvent::AttachmentPromptReady {
                client_id: worker_client_id,
                window_id: worker_window_id,
                operation_id: worker_operation_id,
                prompt: image.prompt_text,
                file_count: 1,
                filename: progress_filename,
            });
        });

        vec![
            AttachmentProgressUpdate::new(id, operation_id, AttachmentProgressPhase::Queued, 1)
                .filename(display_filename)
                .outbound(client_id),
        ]
    }

    pub(crate) fn paste_image_uploaded_events(
        &mut self,
        id: &str,
        upload_id: &str,
        mime_type: &str,
        filename: Option<&str>,
        size: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            tracing::debug!(window_id = %id, "uploaded image paste dropped: window not found");
            return Vec::new();
        };
        if self.tab(&address.tab_id).is_none() {
            tracing::debug!(window_id = %id, "uploaded image paste dropped: project tab not found");
            return Vec::new();
        }
        let Some(session) = self.active_agent_sessions.get(id) else {
            tracing::debug!(
                window_id = %id,
                "uploaded image paste dropped: active agent session not found"
            );
            return Vec::new();
        };
        let worktree_path = session.worktree_path.clone();
        let runtime_target = session.runtime_target;

        let image = match prepare_uploaded_image_paste_file(
            &worktree_path,
            &self.attachment_uploads,
            upload_id,
            mime_type,
            filename,
            size,
            &image_paste_unique_token(),
        ) {
            Ok(image) => image,
            Err(error) => {
                tracing::debug!(
                    window_id = %id,
                    mime_type,
                    error = %error,
                    "uploaded image paste dropped"
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
            "saved uploaded pasted image"
        );
        self.terminal_input_events(id, &image.prompt_text)
    }

    fn attach_files_operation_events(
        &mut self,
        client_id: ClientId,
        id: String,
        operation_id: Option<String>,
        files: Vec<gwt::FileAttachment>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            tracing::debug!(window_id = %id, "file attachment dropped: window not found");
            return Vec::new();
        };
        if self.tab(&address.tab_id).is_none() {
            tracing::debug!(window_id = %id, "file attachment dropped: project tab not found");
            return Vec::new();
        }
        let Some(session) = self.active_agent_sessions.get(&id) else {
            tracing::debug!(
                window_id = %id,
                "file attachment dropped: active agent session not found"
            );
            return Vec::new();
        };
        if files.is_empty() {
            tracing::debug!(window_id = %id, "file attachment dropped: empty selection");
            return Vec::new();
        }

        let operation_id = normalize_attachment_operation_id(operation_id);
        let file_count = files.len();
        let display_filename =
            (file_count == 1).then(|| display_name_for_file_attachment(&files[0]));
        let worktree_path = session.worktree_path.clone();
        let agent_project_root = session.agent_project_root.clone();
        let runtime_target = session.runtime_target;
        let upload_store = self.attachment_uploads.clone();
        let limits = ContentLimits::default();
        let proxy = self.proxy.clone();
        let spawner = self.blocking_tasks.clone();
        let worker_client_id = client_id.clone();
        let worker_window_id = id.clone();
        let worker_operation_id = operation_id.clone();
        let worker_display_filename = display_filename.clone();

        spawner.spawn(move || {
            let mut agent_paths = Vec::with_capacity(files.len());
            for (index, file) in files.iter().enumerate() {
                let filename = display_name_for_file_attachment(file);
                let token = format!("{}-{index}", image_paste_unique_token());
                let prepared = match prepare_file_attachment(
                    &worktree_path,
                    &agent_project_root,
                    runtime_target,
                    file,
                    &token,
                    limits,
                    &upload_store,
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        AttachmentProgressUpdate::new(
                            worker_window_id.clone(),
                            worker_operation_id.clone(),
                            AttachmentProgressPhase::Failed,
                            file_count,
                        )
                        .file(index, filename)
                        .message(error.to_string())
                        .dispatch(&proxy, &worker_client_id);
                        return;
                    }
                };
                if let Err(error) =
                    save_file_attachment_with_progress(&prepared, |bytes_done, total| {
                        AttachmentProgressUpdate::new(
                            worker_window_id.clone(),
                            worker_operation_id.clone(),
                            AttachmentProgressPhase::Staging,
                            file_count,
                        )
                        .file(index, filename.clone())
                        .bytes(bytes_done, total)
                        .dispatch(&proxy, &worker_client_id);
                    })
                {
                    AttachmentProgressUpdate::new(
                        worker_window_id.clone(),
                        worker_operation_id.clone(),
                        AttachmentProgressPhase::Failed,
                        file_count,
                    )
                    .file(index, filename)
                    .message(error.to_string())
                    .dispatch(&proxy, &worker_client_id);
                    return;
                }
                agent_paths.push(prepared.agent_path);
            }

            let prompt = format_file_attachment_prompt(&agent_paths);
            if prompt.is_empty() {
                AttachmentProgressUpdate::new(
                    worker_window_id.clone(),
                    worker_operation_id.clone(),
                    AttachmentProgressPhase::Failed,
                    file_count,
                )
                .filename(worker_display_filename.clone())
                .message("no attachment prompt generated")
                .dispatch(&proxy, &worker_client_id);
                return;
            }
            proxy.send(UserEvent::AttachmentPromptReady {
                client_id: worker_client_id,
                window_id: worker_window_id,
                operation_id: worker_operation_id,
                prompt,
                file_count,
                filename: worker_display_filename,
            });
        });

        vec![AttachmentProgressUpdate::new(
            id,
            operation_id,
            AttachmentProgressPhase::Queued,
            file_count,
        )
        .filename(display_filename)
        .outbound(client_id)]
    }

    pub(crate) fn attach_files_events(
        &mut self,
        id: &str,
        files: Vec<gwt::FileAttachment>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            tracing::debug!(window_id = %id, "file attachment dropped: window not found");
            return Vec::new();
        };
        if self.tab(&address.tab_id).is_none() {
            tracing::debug!(window_id = %id, "file attachment dropped: project tab not found");
            return Vec::new();
        }
        let Some(session) = self.active_agent_sessions.get(id) else {
            tracing::debug!(
                window_id = %id,
                "file attachment dropped: active agent session not found"
            );
            return Vec::new();
        };
        if files.is_empty() {
            tracing::debug!(window_id = %id, "file attachment dropped: empty selection");
            return Vec::new();
        }
        let worktree_path = session.worktree_path.clone();
        let agent_project_root = session.agent_project_root.clone();
        let runtime_target = session.runtime_target;
        let limits = ContentLimits::default();

        let mut agent_paths = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let token = format!("{}-{index}", image_paste_unique_token());
            let prepared = match prepare_file_attachment(
                &worktree_path,
                &agent_project_root,
                runtime_target,
                file,
                &token,
                limits,
                &self.attachment_uploads,
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    tracing::debug!(
                        window_id = %id,
                        error = %error,
                        "file attachment dropped"
                    );
                    return Vec::new();
                }
            };
            if let Err(error) = save_file_attachment(&prepared) {
                return self.handle_runtime_status(
                    id.to_string(),
                    WindowProcessStatus::Error,
                    Some(error.to_string()),
                );
            }
            agent_paths.push(prepared.agent_path);
        }

        let prompt = format_file_attachment_prompt(&agent_paths);
        if prompt.is_empty() {
            return Vec::new();
        }
        tracing::debug!(
            window_id = %id,
            runtime_target = ?runtime_target,
            count = agent_paths.len(),
            "prepared file attachments"
        );
        self.terminal_input_events(id, &prompt)
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

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: format!("Window preset {:?} is not a Work surface", window.preset),
                },
            )];
        }

        spawn_branch_load_async(
            self.proxy.clone(),
            id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(&address.tab_id),
            // Pass the sessions dir so the async branch load reads resume
            // candidates fresh from disk instead of the stale in-memory cache
            // snapshot (#2995).
            self.sessions_dir.clone(),
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
            gwt::board_provider::load_snapshot(&project_root)
        } else {
            gwt::board_provider::load_snapshot_for_scope(&project_root, &scope)
        };
        match snapshot_result {
            Ok(snapshot) => {
                let mut entries = snapshot.board.entries;
                board::attach_board_body_html(&mut entries);
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardEntries {
                        id: id.to_string(),
                        entries,
                        has_more_before: snapshot.board.has_more_before,
                    },
                )]
            }
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
            gwt::board_provider::load_entries_before(&project_root, before_entry_id, limit)
        } else {
            gwt::board_provider::load_entries_before_for_scope(
                &project_root,
                before_entry_id,
                limit,
                &scope,
            )
        };
        match page_result {
            Ok(page) => {
                let mut entries = page.entries;
                board::attach_board_body_html(&mut entries);
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardHistoryPage {
                        id: id.to_string(),
                        entries,
                        has_more_before: page.has_more_before,
                    },
                )]
            }
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
        let Ok(snapshot) = gwt::board_provider::load_snapshot(project_root) else {
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
                    gwt::board_provider::load_snapshot_for_scope(&tab.project_root, &scope)
                        .map(|snapshot| snapshot.board)
                        .unwrap_or_else(|_| snapshot.board.clone())
                };
                let mut entries = board.entries;
                board::attach_board_body_html(&mut entries);
                events.push(OutboundEvent::broadcast(BackendEvent::BoardEntries {
                    id: window_id,
                    entries,
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
        // SPEC-2359 Phase W-11 (US-58 / FR-344): resolve the effective window
        // title with the display fallback chain — the agent-authored
        // `title_summary` first, then the linked Issue/SPEC title, then `None`
        // (which lets the frontend fall back to the neutral agent label). The
        // raw prompt is never written into a title, so it can never appear here.
        let issue_fallback_title = projection
            .linked_issues
            .first()
            .map(|issue| issue.number)
            .and_then(|number| {
                let cache_root =
                    gwt::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
                gwt::issue_cache::load_issue_title_from_cache(&cache_root, number)
            });

        let updates = projection
            .agents
            .iter()
            .filter_map(|agent| {
                let window_id = self.resolve_title_sync_window_id(agent, project_root)?;
                let title = agent
                    .title_summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| issue_fallback_title.clone());
                Some((window_id, title, agent.current_focus.clone()))
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
                .set_dynamic_title_with_detail(&address.raw_id, title, detail)
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
                // GUI interactive search: the watcher owns index builds.
                false,
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

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: format!("Window preset {:?} is not a Work surface", window.preset),
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
                                // SPEC-2009 FR-067: fresh load id from the shared
                                // sequence so the post-cleanup reload is never
                                // dropped as stale by the frontend.
                                load_id: gwt::next_branch_load_id(),
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

impl AppRuntime {
    fn latest_resumable_branch_session(
        &self,
        project_root: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        // Resolve from the in-memory cache so the Resume click never blocks the
        // main UI thread on disk I/O. Freshness is guaranteed by
        // [`apply_refreshed_launch_wizard_sessions`], which the off-thread
        // branch load dispatches before any Resume button is enabled (#2995).
        let normalized_branch_name = normalize_branch_name(branch_name);
        self.launch_wizard_cache
            .latest_resumable_branch_session(project_root, &normalized_branch_name)
    }

    /// Apply a freshly disk-loaded session set to the Launch Wizard cache.
    /// Dispatched from the off-thread branch load (#2995) so branch Resume
    /// availability and the subsequent cache-based resume resolution reflect
    /// session TOMLs the hook CLI wrote out-of-process after launch — without
    /// the main thread ever performing the session-directory scan.
    pub(crate) fn apply_refreshed_launch_wizard_sessions(
        &mut self,
        sessions: Vec<gwt_agent::Session>,
    ) {
        self.launch_wizard_cache.replace_sessions(sessions);
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
        self.inflight_launches
            .retain(|_, (pending_window_id, _)| pending_window_id != &window_id);
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
                // SPEC-2359 W-16 (FR-387): a launch fetches origin refs, so
                // piggyback the cross-machine intake (30s throttle keeps
                // launch bursts cheap).
                self.spawn_work_events_ingest(tab.project_root.clone(), false);
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
                if let Some(tab) = self.tab_mut(&tab_id) {
                    let _ = tab
                        .workspace
                        .set_session_id(&address.raw_id, Some(session_id_for_restore.clone()));
                }
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
                        let live_session_ids: std::collections::HashSet<String> = self
                            .active_agent_sessions
                            .values()
                            .map(|session| session.session_id.clone())
                            .collect();
                        let active_session = &self.active_agent_sessions[&window_id];
                        if let Some(context) = workspace_resume_context.as_ref() {
                            match save_resumed_workspace_projection(
                                &project_root,
                                active_session,
                                base_branch.as_deref(),
                                linked_issue_number,
                                context,
                                &live_session_ids,
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
                                &live_session_ids,
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
                        self.launch_error_terminal_details.remove(&window_id);
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
                        self.launch_error_terminal_details.remove(&window_id);
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

    pub(crate) fn live_agent_window_for_work(
        &self,
        tab_id: &str,
        branch: Option<&str>,
        worktree_path: Option<&Path>,
    ) -> Option<String> {
        let normalized_branch = branch
            .map(normalize_branch_name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.active_agent_sessions
            .iter()
            .find(|(window_id, session)| {
                session.tab_id == tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
                    && active_agent_session_matches_work(
                        session,
                        normalized_branch.as_deref(),
                        worktree_path,
                    )
            })
            .map(|(window_id, _)| window_id.clone())
    }

    pub(crate) fn focus_existing_live_work_agent_events(
        &mut self,
        window_id: &str,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let mut events = self.restore_window_events(window_id);
        events.extend(self.focus_window_events(window_id, bounds));
        if events.is_empty() {
            vec![self.workspace_state_broadcast()]
        } else {
            events
        }
    }

    fn spawn_agent_window_with_placement(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        placement: AgentWindowPlacement,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        if let Some(window_id) = self.live_agent_window_for_work(
            tab_id,
            config.branch.as_deref(),
            config.working_dir.as_deref(),
        ) {
            return Ok(
                self.focus_existing_live_work_agent_events(&window_id, Some(placement.bounds()))
            );
        }
        // SPEC-2359 W-17 (FR-398, Issue #3034): the live-window check above
        // only sees launches whose agent session is already live. A re-click
        // while the previous launch is still materializing (window registered,
        // session pending) must focus that pending window, not spawn a twin.
        let inflight_key = inflight_launch_key(tab_id, &config);
        {
            let window_lookup = &self.window_lookup;
            self.inflight_launches.retain(|_, (window_id, started)| {
                started.elapsed() < INFLIGHT_LAUNCH_TTL
                    && window_lookup.contains_key(window_id.as_str())
            });
        }
        if let Some(key) = inflight_key.as_deref() {
            if let Some((window_id, _)) = self.inflight_launches.get(key) {
                let window_id = window_id.clone();
                return Ok(self
                    .focus_existing_live_work_agent_events(&window_id, Some(placement.bounds())));
            }
        }
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
                    .add_window_with_title(WindowPreset::Agent, title, true, bounds)
            }
            AgentWindowPlacement::Exact(geometry) => tab
                .workspace
                .add_window_at_geometry_with_title(WindowPreset::Agent, title, true, geometry),
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
        if let Some(key) = inflight_key {
            self.inflight_launches
                .insert(key, (window_id.clone(), std::time::Instant::now()));
        }

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
        // SPEC-2014 FR-139..142 — while a Docker launch prepares (preflight,
        // compose ps/up incl. image build, exec probes), mirror docker-kind
        // Process Console lines into the agent terminal. Host launches keep
        // their immediate-PTY behavior untouched (FR-142).
        let docker_output_mirror =
            (config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker).then(|| {
                launch_output_mirror::DockerLaunchOutputMirror::start(
                    proxy.clone(),
                    window_id.clone(),
                )
            });
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
                message: "Configuring work...".to_string(),
            });
            let worktree_path = gwt_core::paths::normalize_windows_child_process_path(
                &config
                    .working_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(&project_root)),
            );
            if config.working_dir.is_some() {
                config.working_dir = Some(worktree_path.clone());
            }
            gwt_agent::LaunchEnvironment::from_active_profile(
                &profile_config_path,
                config.runtime_target,
            )?
            .with_project_root(&worktree_path)
            .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
            let codex_hook_discovery_mode = codex_hook_discovery_mode_for_launch_config(&config);
            refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
                &worktree_path,
                &config.agent_id,
                codex_hook_discovery_mode,
            )
            .map_err(|error| error.to_string())?;
            let codex_home = config.env_vars.get("CODEX_HOME").map(PathBuf::from);
            if let Some(report) = maybe_register_codex_managed_hook_trust_for_launch(
                &profile_config_path,
                &worktree_path,
                &config.agent_id,
                config.runtime_target,
                config.docker_service.as_deref(),
                codex_home.as_deref(),
                codex_hook_discovery_mode,
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

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
                let fallback_report = apply_host_package_runner_fallback_checked(&mut config)?;
                for message in fallback_report.messages {
                    proxy.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message,
                    });
                }
            }
            install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
            apply_windows_host_shell_wrapper(&mut config)?;

            let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());

            let agent_id = config.agent_id.clone();
            let mut session =
                gwt_agent::Session::new(&worktree_path, branch_name.clone(), agent_id.clone());
            session.project_state_root = Some(
                gwt_core::paths::normalize_windows_child_process_path(Path::new(&project_root)),
            );
            session.display_name = config.display_name.clone();
            session.tool_version = config.tool_version.clone();
            session.model = config.model.clone();
            session.reasoning_level = config.reasoning_level.clone();
            session.session_mode = config.session_mode;
            session.skip_permissions = config.skip_permissions;
            session.fast_mode = config.fast_mode;
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

        // Drop (= final drain + join) BEFORE dispatching the result so the
        // tail of the mirrored docker output lands in the terminal ahead of
        // the success transition or the `[gwt] Launch failed` summary —
        // otherwise the failure summary gets buried mid-stream.
        drop(docker_output_mirror);

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

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): handle a user-initiated Work close
    /// from the Work surface. `close_kind` is `"done"` or `"discarded"`.
    ///
    /// Behavior:
    /// - If the owning agent session (derived from `work_id`) is still live, the
    ///   close is blocked and the worktree is left untouched (FR-352). The
    ///   owning agent must be stopped first.
    /// - Otherwise (a Paused Work with no running agent), the worktree is removed
    ///   (worktree only — branch / PR are retained) and the terminal close is
    ///   recorded in the work history. A `done` close records a Done event; a
    ///   `discarded` close records a Discard event. Both remove the Work from the
    ///   active Work surface. Re-closing an already-closed Work is a noop.
    pub(crate) fn close_work(&mut self, work_id: &str, close_kind: &str) -> Vec<OutboundEvent> {
        let work_id = work_id.trim();
        if work_id.is_empty() {
            return Vec::new();
        }
        let close_kind = match close_kind.trim().to_ascii_lowercase().as_str() {
            "done" => gwt_core::workspace_projection::WorkCloseKind::Done,
            "discarded" => gwt_core::workspace_projection::WorkCloseKind::Discarded,
            other => {
                tracing::warn!(
                    work_id = %work_id,
                    close_kind = %other,
                    "ignoring Work close with unknown close_kind"
                );
                return Vec::new();
            }
        };

        let Some(project_root) = self.active_project_root().map(Path::to_path_buf) else {
            tracing::warn!(work_id = %work_id, "Work close has no active project tab");
            return Vec::new();
        };

        // The session id of an agent-session Work is encoded in the Work id
        // (`work-session-<session_id>`). A live agent owns the Work when any
        // active session matches that id.
        let session_id = work_id
            .strip_prefix("work-session-")
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let has_live_agent = session_id.is_some_and(|session_id| {
            self.active_agent_sessions
                .values()
                .any(|session| session.session_id == session_id)
        });

        // Resolve the worktree path from the retained work history so a Paused
        // Work can have its worktree removed without a live session.
        let worktree_path = self.resolve_work_worktree_path(&project_root, work_id);

        let decision =
            gwt_core::workspace_projection::decide_work_close(has_live_agent, worktree_path);

        match decision {
            gwt_core::workspace_projection::WorkCloseDecision::BlockedLiveAgent => {
                // FR-352: never clean up a Work while its agent session is live.
                tracing::warn!(
                    work_id = %work_id,
                    session_id = session_id.unwrap_or_default(),
                    "Work close blocked: owning agent session is still live; stop the agent before closing"
                );
                return Vec::new();
            }
            gwt_core::workspace_projection::WorkCloseDecision::CleanupWorktree {
                worktree_path,
            } => {
                self.remove_work_worktree_only(&project_root, &worktree_path);
            }
            gwt_core::workspace_projection::WorkCloseDecision::RecordOnly => {
                // No resolvable worktree path: record the close without any
                // filesystem side effect.
            }
        }

        // Record the terminal close in the work history. Idempotent against an
        // already-closed Work, so a duplicate close emits no new event.
        let now = chrono::Utc::now();
        let recorded = match close_kind {
            gwt_core::workspace_projection::WorkCloseKind::Done => {
                gwt_core::workspace_projection::emit_workspace_done_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
            gwt_core::workspace_projection::WorkCloseKind::Discarded => {
                gwt_core::workspace_projection::emit_workspace_discard_event_if_absent(
                    &project_root,
                    work_id,
                    now,
                )
            }
        };
        if let Err(error) = recorded {
            tracing::warn!(
                work_id = %work_id,
                error = %error,
                "failed to record Work terminal close event"
            );
        }

        // Broadcast the refreshed projection so the Work leaves the active
        // surface for every connected client.
        self.active_work_projection_broadcast_for_active_tab()
            .into_iter()
            .collect()
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): resolve the worktree path for
    /// `work_id` from the retained work history's execution containers. Returns
    /// `None` when the Work has no recorded worktree, in which case the close is
    /// recorded without filesystem cleanup.
    fn resolve_work_worktree_path(&self, project_root: &Path, work_id: &str) -> Option<PathBuf> {
        let projection = self
            .work_items_cache
            .borrow_mut()
            .load_or_synthesize(project_root)
            .ok()?;
        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == work_id)?;
        item.execution_containers
            .iter()
            .find_map(|container| container.worktree_path.clone())
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): remove the worktree at
    /// `worktree_path` (worktree only — the branch and any PR are retained). A
    /// missing or already-removed worktree is treated as success so the close
    /// stays robust; other failures are logged but do not abort recording the
    /// close.
    fn remove_work_worktree_only(&self, project_root: &Path, worktree_path: &Path) {
        let main_repo_path = match gwt_git::worktree::main_worktree_root(project_root) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    worktree_path = %worktree_path.display(),
                    error = %error,
                    "Work close could not resolve main worktree root; skipping worktree removal"
                );
                return;
            }
        };
        let manager = gwt_git::WorktreeManager::new(&main_repo_path);
        if let Err(error) = manager.remove_force(worktree_path) {
            tracing::warn!(
                worktree_path = %worktree_path.display(),
                error = %error,
                "Work close worktree removal failed; recording the close anyway"
            );
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
            // SPEC-2359 Phase W-12 Slice 5a (FR-350): persist a Paused marker
            // before clearing the agent from the live projection so the Work is
            // retained on the Work surface until the user explicitly closes it.
            self.persist_paused_work_for_stopped_session(&project_root, &session);
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

    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): record a Pause work event for a
    /// stopped agent session so the Work persists in the work history and keeps
    /// surfacing as Paused. The Work id is the session-derived canonical id
    /// (`work-session-<session_id>`) so a later resume groups the live agent onto
    /// the same row and dedupes the Paused entry away. Identity (title / branch /
    /// worktree / board refs) is recovered from the saved projection's matching
    /// agent and git details, falling back to the live session when unavailable.
    fn persist_paused_work_for_stopped_session(
        &self,
        project_root: &Path,
        session: &ActiveAgentSession,
    ) {
        let session_id = session.session_id.trim();
        if session_id.is_empty() {
            return;
        }
        let work_id = format!("work-session-{session_id}");
        let projection = gwt_core::workspace_projection::load_workspace_projection(project_root)
            .ok()
            .flatten();
        let agent_summary = projection.as_ref().and_then(|projection| {
            projection
                .agents
                .iter()
                .find(|agent| agent.session_id == session_id)
        });
        // #3065: owner / summary / the title fallback must come from the
        // session's own Work item (resolved by branch container inside the
        // background thread below), never from the repo-shared projection —
        // its identity belongs to whatever Work last wrote it.
        let agent_title = agent_summary
            .and_then(|agent| {
                agent
                    .title_summary
                    .clone()
                    .or_else(|| agent.current_focus.clone())
            })
            .filter(|value| !value.trim().is_empty());
        let board_refs = projection
            .as_ref()
            .map(|projection| projection.board_refs.clone())
            .unwrap_or_default();
        let branch = agent_summary
            .and_then(|agent| agent.branch.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.branch.clone())
            })
            .or_else(|| Some(session.branch_name.clone()))
            .filter(|value| !value.trim().is_empty());
        let worktree_path = agent_summary
            .and_then(|agent| agent.worktree_path.clone())
            .or_else(|| {
                projection
                    .as_ref()
                    .and_then(|projection| projection.git_details.as_ref())
                    .and_then(|details| details.worktree_path.clone())
            })
            .or_else(|| Some(session.worktree_path.clone()));
        let git_details = projection
            .as_ref()
            .and_then(|projection| projection.git_details.clone());
        let execution_container = (branch.is_some() || worktree_path.is_some()).then(|| {
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch,
                worktree_path,
                pr_number: git_details.as_ref().and_then(|details| details.pr_number),
                pr_url: git_details
                    .as_ref()
                    .and_then(|details| details.pr_url.clone()),
                pr_state: git_details
                    .as_ref()
                    .and_then(|details| details.pr_state.clone()),
            }
        });
        // Close-latency root fix (2026-06-12): the record loads + saves the
        // home works.json (megabytes once a project has hundreds of Works).
        // Doing that synchronously on the UI event loop made every agent
        // window × stall for seconds (sampled: serde to_vec_pretty dominating
        // the close handler). Inputs are gathered synchronously above from
        // the in-memory projection; the file IO runs on a background thread
        // and the workspace projection watcher broadcasts the refreshed rows
        // once the write lands.
        let project_root = project_root.to_path_buf();
        let session_id = session_id.to_string();
        let log_session_id = session.session_id.clone();
        let lookup_branch = execution_container
            .as_ref()
            .and_then(|container| container.branch.clone());
        let lookup_worktree = execution_container
            .as_ref()
            .and_then(|container| container.worktree_path.clone());
        let record = thread::spawn(move || {
            // #3065: resolve identity from the session's own Work item. The
            // works.json IO already happens on this background thread for the
            // record itself, so the lookup adds no UI-loop cost.
            let own_item = gwt_core::workspace_projection::load_workspace_work_items(&project_root)
                .ok()
                .flatten()
                .and_then(|works| {
                    gwt_core::workspace_projection::find_work_item_for_container(
                        &works,
                        &project_root,
                        lookup_branch.as_deref(),
                        lookup_worktree.as_deref(),
                    )
                    .map(|item| {
                        (
                            item.title.clone(),
                            item.summary.clone().or_else(|| item.intent.clone()),
                            item.owner.clone(),
                        )
                    })
                });
            let (item_title, summary, owner) = own_item.unwrap_or((String::new(), None, None));
            let title =
                agent_title.or_else(|| Some(item_title).filter(|value| !value.trim().is_empty()));
            if let Err(error) = gwt_core::workspace_projection::record_workspace_work_paused_event(
                &project_root,
                &work_id,
                title.as_deref(),
                summary.as_deref(),
                owner.as_deref(),
                &board_refs,
                execution_container,
                Some(&session_id),
                chrono::Utc::now(),
            ) {
                tracing::warn!(
                    error = %error,
                    project_root = %project_root.display(),
                    session_id = %log_session_id,
                    work_id = %work_id,
                    "failed to persist Paused Work for stopped Agent session"
                );
            }
        });
        // Unit tests assert the projection immediately after a stop, so the
        // write is joined for determinism there; production detaches it.
        #[cfg(test)]
        let _ = record.join();
        #[cfg(not(test))]
        drop(record);
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
        let threads = self.start_window_runtime_stop(window_id, mark_session_stopped);
        Self::join_runtime_stop_threads(threads);
    }

    fn start_window_runtime_stop(
        &mut self,
        window_id: &str,
        mark_session_stopped: bool,
    ) -> RuntimeStopThreads {
        if mark_session_stopped {
            self.mark_agent_session_stopped(window_id);
        }
        self.remove_window_state_tracking(window_id);
        self.deregister_pty_writer(window_id);
        let mut threads = RuntimeStopThreads {
            output_thread: None,
            status_thread: None,
        };
        if let Some(mut runtime) = self.runtimes.remove(window_id) {
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
            }
            threads.output_thread = runtime.output_thread.take();
            threads.status_thread = runtime.status_thread.take();
        }
        self.window_details.remove(window_id);
        threads
    }

    fn join_runtime_stop_threads(mut threads: RuntimeStopThreads) {
        if let Some(handle) = threads.output_thread.take() {
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
        if let Some(handle) = threads.status_thread.take() {
            let (tx, rx) = std_mpsc::channel();
            thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_millis(500));
        }
    }

    /// Stop every active window runtime. Called from the application shutdown
    /// paths so no PTY / agent process outlives the GUI.
    pub(crate) fn stop_all_runtimes(&mut self) {
        let ids: Vec<String> = self.runtimes.keys().cloned().collect();
        self.stop_runtimes_in_shutdown_order(ids);
    }

    fn stop_runtimes_in_shutdown_order(&mut self, ids: Vec<String>) {
        let mut threads = Vec::new();
        for id in ids {
            threads.push(self.start_window_runtime_stop(&id, false));
        }
        for runtime_threads in threads {
            Self::join_runtime_stop_threads(runtime_threads);
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
        OutboundEvent::broadcast(BackendEvent::WindowCanvasState {
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
            gwt::LaunchWizardAction::SetFastMode { .. } => "set_fast_mode",
            gwt::LaunchWizardAction::SetCodexFastMode { .. } => "set_codex_fast_mode",
            gwt::LaunchWizardAction::Submit => "submit",
            gwt::LaunchWizardAction::GotoStep { .. } => "goto_step",
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
        let terminal_output = Self::launch_error_terminal_output_event(window_id.clone(), &detail);
        if self.tracked_window_exists(&window_id) {
            self.launch_error_terminal_details
                .insert(window_id.clone(), detail.clone());
            let mut events =
                self.handle_runtime_status(window_id, WindowProcessStatus::Error, Some(detail));
            events.push(terminal_output);
            return events;
        }
        let mut events =
            Self::status_events(window_id, WindowProcessStatus::Error, Some(detail.clone()));
        events.push(terminal_output);
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

    fn launch_error_terminal_bytes(detail: &str) -> Vec<u8> {
        let mut message = String::from("\r\n[gwt] Launch failed before PTY started.\r\n");
        let detail = detail.trim();
        if !detail.is_empty() {
            message.push_str("[gwt] ");
            message.push_str(detail);
            message.push_str("\r\n");
        }
        message.into_bytes()
    }

    fn launch_error_terminal_output_event(window_id: String, detail: &str) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id: window_id,
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(Self::launch_error_terminal_bytes(detail)),
        })
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

fn codex_hook_discovery_mode_for_launch_config(
    config: &gwt_agent::LaunchConfig,
) -> gwt_skills::CodexHookDiscoveryMode {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return gwt_skills::CodexHookDiscoveryMode::WorkspaceHome;
    }
    if let Some(mode) =
        codex_hook_discovery_mode_from_selected_codex_version(config.tool_version.as_deref())
    {
        return mode;
    }
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return gwt_skills::CodexHookDiscoveryMode::Both;
    }
    detect_installed_codex_hook_discovery_mode(config)
        .unwrap_or(gwt_skills::CodexHookDiscoveryMode::Both)
}

fn codex_hook_discovery_mode_from_selected_codex_version(
    version: Option<&str>,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let version = version?.trim();
    if version.is_empty() || version == "installed" {
        return None;
    }
    if version == "latest" {
        return Some(gwt_skills::CodexHookDiscoveryMode::WorkspaceHome);
    }
    codex_hook_discovery_mode_from_semver(version)
}

fn codex_hook_discovery_mode_from_codex_version_output(
    output: &str,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    output
        .split_whitespace()
        .find_map(codex_hook_discovery_mode_from_semver)
}

fn codex_hook_discovery_mode_from_semver(raw: &str) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let token = raw
        .trim()
        .trim_start_matches('v')
        .trim_matches(|c| c == ',' || c == ';');
    let version = semver::Version::parse(token).ok()?;
    let boundary =
        semver::Version::parse("0.131.0-alpha.21").expect("valid Codex hook discovery boundary");
    Some(if version < boundary {
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal
    } else {
        gwt_skills::CodexHookDiscoveryMode::WorkspaceHome
    })
}

fn detect_installed_codex_hook_discovery_mode(
    config: &gwt_agent::LaunchConfig,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let mut command = std::process::Command::new(&config.command);
    command.arg("--version").envs(&config.env_vars);
    for key in &config.remove_env {
        command.env_remove(key);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push(' ');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    codex_hook_discovery_mode_from_codex_version_output(&text)
}

fn maybe_register_codex_managed_hook_trust_for_launch(
    profile_config_path: &Path,
    worktree_path: &Path,
    agent_id: &gwt_agent::AgentId,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    docker_service: Option<&str>,
    codex_home: Option<&Path>,
    codex_hook_discovery_mode: gwt_skills::CodexHookDiscoveryMode,
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
            let Some(codex_config_path) = codex_home
                .map(|home| home.join("config.toml"))
                .or_else(|| codex_config_path_for_profile_config(profile_config_path))
            else {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    "cannot derive Codex config path while preparing Codex hook trust; continuing launch"
                );
                return Ok(None);
            };
            match gwt_skills::register_codex_managed_hook_trust_for_mode(
                worktree_path,
                &codex_config_path,
                codex_hook_discovery_mode,
            ) {
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
                codex_hook_discovery_mode,
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
mod agent_launch_stage_tests;

#[cfg(test)]
mod tests;

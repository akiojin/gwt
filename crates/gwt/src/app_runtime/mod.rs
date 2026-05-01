use super::*;

#[derive(Clone)]
pub(crate) enum AppEventProxy {
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
pub(crate) enum BlockingTaskSpawner {
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

pub(crate) struct KnowledgeSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) selected_number: Option<u64>,
    pub(crate) list_scope: gwt::KnowledgeListScope,
}

pub(crate) struct KnowledgeLoadRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) request_id: Option<u64>,
    pub(crate) selected_number: Option<u64>,
    pub(crate) refresh: bool,
    pub(crate) list_scope: gwt::KnowledgeListScope,
}

struct KnowledgeRefreshTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    selected_number: Option<u64>,
    force: bool,
    list_scope: gwt::KnowledgeListScope,
}

struct KnowledgeSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    query: String,
    request_id: u64,
    selected_number: Option<u64>,
    list_scope: gwt::KnowledgeListScope,
}

pub(crate) struct WindowRuntime {
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
pub(crate) struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) remove_env: Vec<String>,
    pub(crate) cwd: Option<PathBuf>,
}

pub(crate) type AgentLaunchCompletion = (
    ProcessLaunch,
    String,
    String,
    String,
    PathBuf,
    gwt_agent::AgentId,
    Option<u64>,
);

pub(crate) type AgentLaunchResult = Result<AgentLaunchCompletion, String>;

mod board;
mod memory;
mod migration;
mod profile;
mod window;
mod wizard;
pub(crate) use board::BoardPostRequest;
use profile::ProfileSaveRequest;

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

#[derive(Debug, Clone)]
pub(crate) struct ActiveAgentSession {
    pub(crate) window_id: String,
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) branch_name: String,
    pub(crate) display_name: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) tab_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchWizardMemoryCache {
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

    fn previous_profile(&self, repo_path: &Path) -> Option<gwt::LaunchWizardPreviousProfile> {
        gwt::launch_wizard::previous_launch_profile_from_sessions(repo_path, &self.sessions)
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
pub(crate) enum DispatchTarget {
    Broadcast,
    Client(ClientId),
}

#[derive(Debug, Clone)]
pub(crate) struct OutboundEvent {
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

fn knowledge_error_event(
    id: impl Into<String>,
    kind: KnowledgeKind,
    message: impl Into<String>,
    request_id: Option<u64>,
    query: Option<String>,
    list_scope: Option<gwt::KnowledgeListScope>,
) -> BackendEvent {
    BackendEvent::KnowledgeError {
        id: id.into(),
        knowledge_kind: kind,
        request_id,
        query,
        list_scope,
        message: message.into(),
    }
}

fn knowledge_view_events(
    client_id: String,
    id: String,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    list_scope: gwt::KnowledgeListScope,
    view: gwt::KnowledgeBridgeView,
) -> Vec<OutboundEvent> {
    vec![
        OutboundEvent::reply(
            client_id.clone(),
            BackendEvent::KnowledgeEntries {
                id: id.clone(),
                knowledge_kind: kind,
                request_id,
                list_scope: Some(list_scope),
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
                list_scope: Some(list_scope),
                detail: view.detail,
            },
        ),
    ]
}

pub(crate) fn build_frontend_sync_events(
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

#[derive(Debug, Clone)]
pub(crate) struct ProjectTabRuntime {
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
pub(crate) struct WindowAddress {
    pub(crate) tab_id: String,
    pub(crate) raw_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchWizardSession {
    pub(crate) tab_id: String,
    pub(crate) wizard_id: String,
    pub(crate) wizard: LaunchWizardState,
}

#[derive(Debug, Clone)]
pub(crate) struct IssueLaunchWizardPrepared {
    pub(crate) client_id: ClientId,
    pub(crate) id: String,
    pub(crate) knowledge_kind: KnowledgeKind,
    pub(crate) tab_id: String,
    pub(crate) project_root: PathBuf,
    pub(crate) issue_number: u64,
    pub(crate) result: Result<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectOpenTarget {
    pub(crate) project_root: PathBuf,
    pub(crate) title: String,
    pub(crate) kind: gwt::ProjectKind,
    /// `true` when the resolved layout is a Normal Git checkout that gwt would
    /// like to migrate to its Nested Bare+Worktree convention (SPEC-1934 US-6).
    pub(crate) needs_migration: bool,
}

pub(crate) struct AppRuntime {
    pub(crate) tabs: Vec<ProjectTabRuntime>,
    pub(crate) active_tab_id: Option<String>,
    pub(crate) recent_projects: Vec<gwt::RecentProjectEntry>,
    pub(crate) profile_selections: HashMap<String, String>,
    pub(crate) profile_config_path: Option<PathBuf>,
    pub(crate) runtimes: HashMap<String, WindowRuntime>,
    pub(crate) window_details: HashMap<String, String>,
    pub(crate) window_lookup: HashMap<String, WindowAddress>,
    pub(crate) session_state_path: PathBuf,
    pub(crate) log_dir: PathBuf,
    pub(crate) proxy: AppEventProxy,
    pub(crate) blocking_tasks: BlockingTaskSpawner,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) launch_wizard_cache: LaunchWizardMemoryCache,
    pub(crate) launch_wizard: Option<LaunchWizardSession>,
    pub(crate) active_agent_sessions: HashMap<String, ActiveAgentSession>,
    pub(crate) window_pty_statuses: HashMap<String, WindowProcessStatus>,
    pub(crate) window_hook_states: HashMap<String, WindowProcessStatus>,
    pub(crate) hook_forward_target: Option<HookForwardTarget>,
    pub(crate) issue_link_cache_dir: PathBuf,
    /// Cached update state so late-connecting WebView clients get the toast.
    pub(crate) pending_update: Option<gwt_core::update::UpdateState>,
    /// Shared PTY writer registry published to the WebSocket fast-path.
    pub(crate) pty_writers: PtyWriterRegistry,
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

        let mut app = Self {
            tabs,
            active_tab_id,
            recent_projects: dedupe_recent_projects(persisted.recent_projects),
            profile_selections: HashMap::new(),
            profile_config_path: None,
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path,
            log_dir,
            proxy: AppEventProxy::new(proxy),
            blocking_tasks,
            sessions_dir,
            launch_wizard_cache,
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            pending_update: None,
            pty_writers,
        };
        app.rebuild_window_lookup();
        app.seed_window_pty_statuses();
        app.seed_restored_window_details();
        Ok(app)
    }

    pub(crate) fn bootstrap(&mut self) {
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

    pub(crate) fn set_hook_forward_target(&mut self, target: HookForwardTarget) {
        self.hook_forward_target = Some(target);
    }

    pub(crate) fn handle_frontend_event(
        &mut self,
        client_id: ClientId,
        event: FrontendEvent,
    ) -> Vec<OutboundEvent> {
        match event {
            FrontendEvent::FrontendReady => self.frontend_sync_events(&client_id),
            FrontendEvent::OpenProjectDialog => self.open_project_dialog_events(),
            FrontendEvent::ReopenRecentProject { path } => {
                self.open_project_path_events(PathBuf::from(path))
            }
            FrontendEvent::SelectProjectTab { tab_id } => self.select_project_tab_events(&tab_id),
            FrontendEvent::CloseProjectTab { tab_id } => self.close_project_tab_events(&tab_id),
            FrontendEvent::CreateWindow { preset, bounds } => {
                self.create_window_events(preset, bounds)
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
            FrontendEvent::ListWindows => {
                vec![OutboundEvent::reply(client_id, self.list_windows_event())]
            }
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry,
                cols,
                rows,
            } => self.update_window_geometry_events(&id, geometry, cols, rows),
            FrontendEvent::CloseWindow { id } => self.close_window_events(&id),
            FrontendEvent::TerminalInput { id, data } => self.terminal_input_events(&id, &data),
            FrontendEvent::LoadFileTree { id, path } => {
                let path = path.unwrap_or_default();
                vec![OutboundEvent::reply(
                    client_id,
                    self.load_file_tree_event(&id, &path),
                )]
            }
            FrontendEvent::LoadBranches { id } => self.load_branches_events(&client_id, &id),
            FrontendEvent::LoadBoard { id } => self.load_board_events(&client_id, &id),
            FrontendEvent::LoadProfile { id } => self.load_profile_events(&client_id, &id),
            FrontendEvent::LoadMemo { id } => self.load_memo_events(&client_id, &id),
            FrontendEvent::LoadLogs { id } => self.load_logs_events(&client_id, &id),
            FrontendEvent::LoadKnowledgeBridge {
                id,
                knowledge_kind,
                request_id,
                selected_number,
                refresh,
                list_scope,
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number,
                    refresh,
                    list_scope: list_scope.unwrap_or(gwt::KnowledgeListScope::Open),
                },
            ),
            FrontendEvent::SearchKnowledgeBridge {
                id,
                knowledge_kind,
                query,
                request_id,
                selected_number,
                list_scope,
            } => self.search_knowledge_bridge_events(
                &client_id,
                KnowledgeSearchRequest {
                    id: &id,
                    kind: knowledge_kind,
                    query: &query,
                    request_id,
                    selected_number,
                    list_scope: list_scope.unwrap_or(gwt::KnowledgeListScope::Open),
                },
            ),
            FrontendEvent::SelectKnowledgeBridgeEntry {
                id,
                knowledge_kind,
                request_id,
                number,
                list_scope,
            } => self.load_knowledge_bridge_events(
                &client_id,
                KnowledgeLoadRequest {
                    id: &id,
                    kind: knowledge_kind,
                    request_id,
                    selected_number: Some(number),
                    refresh: false,
                    list_scope: list_scope.unwrap_or(gwt::KnowledgeListScope::Open),
                },
            ),
            FrontendEvent::RunBranchCleanup {
                id,
                branches,
                delete_remote,
            } => self.run_branch_cleanup_events(&client_id, &id, &branches, delete_remote),
            FrontendEvent::PostBoardEntry {
                id,
                entry_kind,
                body,
                parent_id,
                topics,
                owners,
                targets,
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
                },
            ),
            FrontendEvent::CreateMemoNote {
                id,
                title,
                body,
                pinned,
            } => self.create_memo_note_events(&client_id, &id, title, body, pinned),
            FrontendEvent::UpdateMemoNote {
                id,
                note_id,
                title,
                body,
                pinned,
            } => self.update_memo_note_events(&client_id, &id, &note_id, title, body, pinned),
            FrontendEvent::DeleteMemoNote { id, note_id } => {
                self.delete_memo_note_events(&client_id, &id, &note_id)
            }
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
            FrontendEvent::OpenLaunchWizard {
                id,
                branch_name,
                linked_issue_number,
            } => self.open_launch_wizard(&id, &branch_name, linked_issue_number),
            FrontendEvent::LaunchWizardAction { action, bounds } => {
                self.handle_launch_wizard_action(action, bounds)
            }
            FrontendEvent::ApplyUpdate => {
                std::thread::spawn(apply_update_and_exit);
                vec![]
            }
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
            FrontendEvent::StartMigration { tab_id } => self.start_migration_events(&tab_id),
            FrontendEvent::SkipMigration { tab_id } => self.skip_migration_events(&tab_id),
            FrontendEvent::QuitMigration { tab_id } => self.quit_migration_events(&tab_id),
        }
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
                let snapshot = runtime
                    .pane
                    .lock()
                    .map(|pane| pane.screen().contents().into_bytes())
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
        // SPEC-1934 US-6.1: surface pending migrations to a newly-connected
        // frontend during state hydration so the modal opens without waiting
        // for another roundtrip.
        events.extend(self.migration_detected_replies(client_id));
        events
    }

    pub(crate) fn open_project_dialog_events(&mut self) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        self.open_project_path_events(path)
    }

    pub(crate) fn open_project_path_events(&mut self, path: PathBuf) -> Vec<OutboundEvent> {
        match self.open_project_path(path) {
            Ok(wizard_closed) => {
                let mut events = vec![self.workspace_state_broadcast()];
                if wizard_closed {
                    events.push(self.launch_wizard_state_broadcast(None));
                }
                // SPEC-1934 US-6.1: when a tab was opened on a Normal Git
                // layout, surface the confirmation modal alongside the regular
                // workspace broadcast.
                events.extend(self.migration_detected_broadcasts());
                events
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                message: error,
            })],
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

    /// SPEC-1934 US-6.1 broadcast variant: used by `open_project_path_events`
    /// to inform every connected frontend that a tab needs migration.
    pub(crate) fn migration_detected_broadcasts(&self) -> Vec<OutboundEvent> {
        self.tabs
            .iter()
            .filter(|tab| tab.migration_pending)
            .map(|tab| OutboundEvent::broadcast(self.migration_detected_event_for(tab)))
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

    /// SPEC-1934 FR-019: user accepted the migration confirmation modal.
    pub(crate) fn remember_recent_project(&mut self, target: &ProjectOpenTarget) {
        self.recent_projects
            .retain(|entry| !same_worktree_path(&entry.path, &target.project_root));
        self.recent_projects.insert(
            0,
            gwt::RecentProjectEntry {
                path: target.project_root.clone(),
                title: target.title.clone(),
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

    pub(crate) fn load_file_tree_event(&self, id: &str, path: &str) -> BackendEvent {
        let Some(address) = self.window_lookup.get(id) else {
            return BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: "Window not found".to_string(),
            };
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: "Window not found".to_string(),
            };
        };

        if window.preset != WindowPreset::FileTree {
            return BackendEvent::FileTreeError {
                id: id.to_string(),
                path: path.to_string(),
                message: "Window is not a file tree".to_string(),
            };
        }

        let relative_path = if path.is_empty() {
            None
        } else {
            Some(Path::new(path))
        };

        match list_directory_entries(&tab.project_root, relative_path) {
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
            client_id.to_string(),
            id.to_string(),
            tab.project_root.clone(),
            self.active_session_branches_for_tab(&address.tab_id),
        );
        Vec::new()
    }

    pub(crate) fn load_board_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
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

        match gwt_core::coordination::load_snapshot(&tab.project_root) {
            Ok(snapshot) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardEntries {
                    id: id.to_string(),
                    entries: snapshot.board.entries,
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
            Ok(entries) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::LogEntries {
                    id: id.to_string(),
                    entries,
                },
            )],
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
        &self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Ok(snapshot) = gwt_core::coordination::load_snapshot(project_root) else {
            return Vec::new();
        };

        let mut events = Vec::new();
        for tab in &self.tabs {
            if !same_worktree_path(&tab.project_root, project_root) {
                continue;
            }
            for window in &tab.workspace.persisted().windows {
                if window.preset != WindowPreset::Board {
                    continue;
                }
                events.push(OutboundEvent::broadcast(BackendEvent::BoardEntries {
                    id: combined_window_id(&tab.id, &window.id),
                    entries: snapshot.board.entries.clone(),
                }));
            }
        }
        events
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
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    request.request_id,
                    None,
                    Some(request.list_scope),
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
                    request.request_id,
                    None,
                    Some(request.list_scope),
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
                    request.request_id,
                    None,
                    Some(request.list_scope),
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
                    request.request_id,
                    None,
                    Some(request.list_scope),
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
                list_scope: request.list_scope,
            });
            return Vec::new();
        }

        match load_knowledge_bridge(
            &tab.project_root,
            kind,
            request.selected_number,
            false,
            request.list_scope,
        ) {
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
                        list_scope: request.list_scope,
                    });
                }
                knowledge_view_events(
                    client_id.to_string(),
                    id.to_string(),
                    kind,
                    request.request_id,
                    request.list_scope,
                    view,
                )
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    error,
                    request.request_id,
                    None,
                    Some(request.list_scope),
                ),
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
            list_scope,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let refreshed = match gwt::refresh_knowledge_bridge_cache(&project_root, force) {
                Ok(refreshed) => refreshed,
                Err(error) => {
                    if force {
                        proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                            client_id,
                            knowledge_error_event(
                                id,
                                kind,
                                error,
                                request_id,
                                None,
                                Some(list_scope),
                            ),
                        )]));
                    }
                    return;
                }
            };
            if !force && !refreshed {
                return;
            }
            let event = match gwt::load_knowledge_bridge(
                &project_root,
                kind,
                selected_number,
                false,
                list_scope,
            ) {
                Ok(view) => {
                    knowledge_view_events(client_id, id, kind, request_id, list_scope, view)
                }
                Err(error) => vec![OutboundEvent::reply(
                    client_id,
                    knowledge_error_event(id, kind, error, request_id, None, Some(list_scope)),
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
                    Some(request.list_scope),
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
                    Some(request.list_scope),
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
                    Some(request.list_scope),
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
                    Some(request.list_scope),
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
            list_scope: request.list_scope,
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
            list_scope,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = match gwt::search_knowledge_bridge(
                &project_root,
                kind,
                &query,
                selected_number,
                list_scope,
            ) {
                Ok(view) => BackendEvent::KnowledgeSearchResults {
                    id: id.clone(),
                    knowledge_kind: kind,
                    query: query.clone(),
                    request_id,
                    list_scope: Some(list_scope),
                    entries: view.entries,
                    selected_number: view.selected_number,
                    empty_message: view.empty_message,
                    refresh_enabled: view.refresh_enabled,
                },
                Err(error) => knowledge_error_event(
                    id,
                    kind,
                    error,
                    Some(request_id),
                    Some(query),
                    Some(list_scope),
                ),
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    pub(crate) fn run_branch_cleanup_events(
        &self,
        client_id: &str,
        id: &str,
        branches: &[String],
        delete_remote: bool,
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
            delete_remote,
        );
        Vec::new()
    }
}

fn load_log_entries_from_dir(log_dir: &Path) -> Result<Vec<gwt_core::logging::LogEvent>, String> {
    let path = gwt_core::logging::current_log_file(log_dir);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to open log file {}: {error}",
                path.display()
            ))
        }
    };
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();
    for (index, line) in std::io::BufRead::lines(reader).enumerate() {
        let line = line.map_err(|error| {
            format!(
                "Failed to read log line {} from {}: {error}",
                index + 1,
                path.display()
            )
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry =
            serde_json::from_str::<gwt_core::logging::LogEvent>(trimmed).map_err(|error| {
                format!(
                    "Failed to parse log line {} from {}: {error}",
                    index + 1,
                    path.display()
                )
            })?;
        entries.push(entry);
    }
    Ok(entries)
}

fn spawn_branch_cleanup_async(
    proxy: AppEventProxy,
    client_id: ClientId,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branches: Vec<String>,
    delete_remote: bool,
) {
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let results = cleanup_selected_branches(
                        &project_root,
                        &entries,
                        &branches,
                        delete_remote,
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

impl AppRuntime {
    pub(crate) fn open_issue_launch_wizard_events(
        &mut self,
        client_id: &str,
        id: &str,
        issue_number: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Window not found",
                    None,
                    None,
                    None,
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Project tab not found",
                    None,
                    None,
                    None,
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Window not found",
                    None,
                    None,
                    None,
                ),
            )];
        };
        let Some(kind) = knowledge_kind_for_preset(window.preset) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    KnowledgeKind::Issue,
                    "Window is not a knowledge bridge",
                    None,
                    None,
                    None,
                ),
            )];
        };

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id = id.to_string();
        let active_session_branches = self.active_session_branches_for_tab(&address.tab_id);
        thread::spawn(move || {
            let result =
                list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
                    .map_err(|error| error.to_string())
                    .and_then(|entries| {
                        preferred_issue_launch_branch(&entries)
                            .ok_or_else(|| "No local branch is available for launch".to_string())
                    });
            proxy.send(UserEvent::IssueLaunchWizardPrepared(
                IssueLaunchWizardPrepared {
                    client_id,
                    id,
                    knowledge_kind: kind,
                    tab_id,
                    project_root,
                    issue_number,
                    result,
                },
            ));
        });
        Vec::new()
    }

    pub(crate) fn handle_issue_launch_wizard_prepared(
        &mut self,
        prepared: IssueLaunchWizardPrepared,
    ) -> Vec<OutboundEvent> {
        let IssueLaunchWizardPrepared {
            client_id,
            id,
            knowledge_kind,
            tab_id,
            project_root,
            issue_number,
            result,
        } = prepared;
        if self.tab(&tab_id).is_none() {
            return vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(
                    id,
                    knowledge_kind,
                    "Project tab not found",
                    None,
                    None,
                    None,
                ),
            )];
        }

        match result {
            Ok(branch_name) => match self.open_launch_wizard_for_branch(
                &tab_id,
                &project_root,
                &branch_name,
                Some(issue_number),
            ) {
                Ok(()) => vec![self.launch_wizard_state_outbound()],
                Err(error) => vec![OutboundEvent::reply(
                    &client_id,
                    knowledge_error_event(id, knowledge_kind, error, None, None, None),
                )],
            },
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                knowledge_error_event(id, knowledge_kind, error, None, None, None),
            )],
        }
    }

    pub(crate) fn handle_launch_wizard_action(
        &mut self,
        action: gwt::LaunchWizardAction,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(mut session) = self.launch_wizard.take() else {
            return Vec::new();
        };
        let action_stage = Self::launch_wizard_action_error_stage(&action);
        let action_label = Self::launch_wizard_action_label(&action);
        let requested_agent_id = match &action {
            gwt::LaunchWizardAction::SetAgent { agent_id } => Some(agent_id.clone()),
            _ => None,
        };
        session.wizard.apply(action);
        if let Some(error) = session.wizard.error.as_deref() {
            Self::log_launch_wizard_error(
                &session,
                action_stage,
                action_label,
                requested_agent_id.as_deref(),
                error,
            );
        }

        match session.wizard.completion.take() {
            Some(LaunchWizardCompletion::Cancelled) => {
                vec![self.launch_wizard_state_broadcast(None)]
            }
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                let Some(tab) = self.tab_mut(&address.tab_id) else {
                    let error = "Project tab not found".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                if !tab.workspace.focus_window(&address.raw_id, None) {
                    let error = "The selected session window is no longer available".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "focus_window",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                }
                self.active_tab_id = Some(address.tab_id);
                let _ = self.persist();
                vec![
                    self.workspace_state_broadcast(),
                    self.launch_wizard_state_broadcast(None),
                ]
            }
            Some(LaunchWizardCompletion::Launch(config)) => {
                let Some(bounds) = bounds else {
                    let error = "Viewport bounds are required to launch a window".to_string();
                    Self::log_launch_wizard_error(
                        &session,
                        "launch_bounds",
                        action_label,
                        requested_agent_id.as_deref(),
                        &error,
                    );
                    session.wizard.error = Some(error);
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                match *config {
                    LaunchWizardLaunchRequest::Agent(config) => {
                        match self.spawn_agent_window(&session.tab_id, *config, bounds) {
                            Ok(mut events) => {
                                events.push(self.launch_wizard_state_broadcast(None));
                                events
                            }
                            Err(error) => {
                                Self::log_launch_wizard_error(
                                    &session,
                                    "spawn_agent_window",
                                    action_label,
                                    requested_agent_id.as_deref(),
                                    &error,
                                );
                                session.wizard.error = Some(error);
                                self.launch_wizard = Some(session);
                                vec![self.launch_wizard_state_outbound()]
                            }
                        }
                    }
                    LaunchWizardLaunchRequest::Shell(config) => {
                        match self.spawn_wizard_shell_window(&session.tab_id, *config, bounds) {
                            Ok(mut events) => {
                                events.push(self.launch_wizard_state_broadcast(None));
                                events
                            }
                            Err(error) => {
                                Self::log_launch_wizard_error(
                                    &session,
                                    "spawn_shell_window",
                                    action_label,
                                    requested_agent_id.as_deref(),
                                    &error,
                                );
                                session.wizard.error = Some(error);
                                self.launch_wizard = Some(session);
                                vec![self.launch_wizard_state_outbound()]
                            }
                        }
                    }
                }
            }
            None => {
                self.launch_wizard = Some(session);
                vec![self.launch_wizard_state_outbound()]
            }
        }
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
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
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

    pub(crate) fn handle_runtime_output(
        &mut self,
        id: String,
        data: Vec<u8>,
    ) -> Vec<OutboundEvent> {
        if !self.window_lookup.contains_key(&id) {
            return Vec::new();
        }
        vec![OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })]
    }

    pub(crate) fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(_address) = self.window_lookup.get(&id).cloned() else {
            self.remove_window_state_tracking(&id);
            self.deregister_pty_writer(&id);
            self.runtimes.remove(&id);
            self.window_details.remove(&id);
            return Vec::new();
        };

        self.window_pty_statuses.insert(id.clone(), status);
        let composed_status = self.recompute_window_state(&id).unwrap_or(status);
        let should_auto_close =
            should_auto_close_agent_window(&self.active_agent_sessions, &id, &composed_status);
        match detail.as_ref() {
            Some(detail) if !detail.is_empty() => {
                self.window_details.insert(id.clone(), detail.clone());
            }
            _ => {
                self.window_details.remove(&id);
            }
        }
        if should_auto_close {
            self.stop_window_runtime(&id);
            self.remove_window_state_tracking(&id);
            if !close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &id,
            ) {
                return Vec::new();
            }
            let _ = self.persist();
            return vec![self.workspace_state_broadcast()];
        }
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Stopped
        ) {
            self.runtimes.remove(&id);
            self.remove_window_state_tracking(&id);
            self.mark_agent_session_stopped(&id);
        }
        let _ = self.persist();

        let mut events = vec![self.workspace_state_broadcast()];
        events.extend(Self::status_events(id, composed_status, detail));
        events
    }

    pub(crate) fn handle_runtime_hook_event(
        &mut self,
        event: gwt::RuntimeHookEvent,
    ) -> Vec<OutboundEvent> {
        let mut events = vec![OutboundEvent::broadcast(BackendEvent::RuntimeHookEvent {
            event: event.clone(),
        })];
        let Some(window_id) = self.active_window_for_runtime_event(&event) else {
            return events;
        };
        let Some(hook_state) = gwt::window_state::runtime_hook_window_state(&event) else {
            return events;
        };
        self.window_hook_states
            .insert(window_id.clone(), hook_state);
        let Some(composed_state) = self.recompute_window_state(&window_id) else {
            return events;
        };
        let should_auto_close = should_auto_close_agent_window(
            &self.active_agent_sessions,
            &window_id,
            &composed_state,
        );
        let detail = self.window_details.get(&window_id).cloned();
        if should_auto_close {
            self.stop_window_runtime(&window_id);
            self.remove_window_state_tracking(&window_id);
            if close_window_from_workspace(
                &mut self.tabs,
                &mut self.window_lookup,
                &mut self.window_details,
                &window_id,
            ) {
                let _ = self.persist();
                events.push(self.workspace_state_broadcast());
            }
            return events;
        }
        let _ = self.persist();
        events.push(self.workspace_state_broadcast());
        events.extend(Self::status_events(window_id, composed_state, detail));
        events
    }

    pub(crate) fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: AgentLaunchResult,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
            )) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return self.launch_error_events(window_id, "Window not found".to_string());
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self
                        .launch_error_events(window_id, "Project tab not found".to_string());
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(window_id, "Window not found".to_string());
                };
                let geometry = window.geometry.clone();

                self.active_agent_sessions.insert(
                    window_id.clone(),
                    ActiveAgentSession {
                        window_id: window_id.clone(),
                        session_id,
                        agent_id: agent_id.to_string(),
                        branch_name,
                        display_name,
                        worktree_path: worktree_path.clone(),
                        tab_id: address.tab_id,
                    },
                );
                self.refresh_launch_wizard_session_cache(&window_id);

                match self.spawn_process_window(&window_id, geometry, process_launch) {
                    Ok(()) => {
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
                        let _ = self.persist();
                        let mut events = vec![self.workspace_state_broadcast()];
                        events.extend(Self::status_events(
                            window_id,
                            WindowProcessStatus::Running,
                            None,
                        ));
                        events
                    }
                    Err(error) => self.launch_error_events(window_id, error),
                }
            }
            Err(error) => self.launch_error_events(window_id, error),
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
                    return self.launch_error_events(window_id, "Window not found".to_string());
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return self
                        .launch_error_events(window_id, "Project tab not found".to_string());
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return self.launch_error_events(window_id, "Window not found".to_string());
                };
                let geometry = window.geometry.clone();

                match self.spawn_process_window(&window_id, geometry, process_launch) {
                    Ok(()) => {
                        let mut events = vec![self.workspace_state_broadcast()];
                        events.extend(Self::status_events(
                            window_id,
                            WindowProcessStatus::Running,
                            None,
                        ));
                        events
                    }
                    Err(error) => self.launch_error_events(window_id, error),
                }
            }
            Err(error) => self.launch_error_events(window_id, error),
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

        match self.spawn_process_window(
            &window_id,
            geometry,
            ProcessLaunch {
                command: launch.command,
                args: launch.args,
                env,
                remove_env,
                cwd: Some(project_root),
            },
        ) {
            Ok(()) => Self::status_events(window_id, WindowProcessStatus::Running, None),
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                Self::status_events(window_id, WindowProcessStatus::Error, Some(error))
            }
        }
    }

    pub(crate) fn spawn_process_window(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
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

        let output_thread = self.spawn_output_thread(id.to_string(), pane.clone());
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
    ) -> Result<Vec<OutboundEvent>, String> {
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root = tab.project_root.display().to_string();
        let title = format!(
            "{} · {}",
            config.display_name,
            config.branch.as_ref().unwrap_or(&"workspace".to_string())
        );
        let window =
            tab.workspace
                .add_window_with_title(WindowPreset::Agent, title.clone(), false, bounds);
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        let mut events = vec![self.workspace_state_broadcast()];
        events.extend(Self::status_events(
            window_id.clone(),
            WindowProcessStatus::Running,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let hook_forward_target = self.hook_forward_target.clone();
        let profile_config_path = self.profile_config_path()?;

        thread::spawn(move || {
            Self::spawn_agent_window_async(
                proxy,
                sessions_dir,
                project_root,
                window_id,
                config,
                profile_config_path,
                hook_forward_target,
            )
        });

        Ok(events)
    }

    pub(crate) fn spawn_wizard_shell_window(
        &mut self,
        tab_id: &str,
        config: ShellLaunchConfig,
        bounds: WindowGeometry,
    ) -> Result<Vec<OutboundEvent>, String> {
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let project_root = tab.project_root.display().to_string();
        let title = format!(
            "{} · {}",
            config.display_name,
            config.branch.as_ref().unwrap_or(&"workspace".to_string())
        );
        let window = tab
            .workspace
            .add_window_with_title(WindowPreset::Shell, title, false, bounds);
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        self.window_pty_statuses
            .insert(window_id.clone(), WindowProcessStatus::Running);
        self.window_hook_states.remove(&window_id);

        let mut events = vec![self.workspace_state_broadcast()];
        events.extend(Self::status_events(
            window_id.clone(),
            WindowProcessStatus::Running,
            Some("Launching...".to_string()),
        ));

        let proxy = self.proxy.clone();
        let profile_config_path = self.profile_config_path()?;
        thread::spawn(move || {
            Self::spawn_wizard_shell_window_async(
                proxy,
                project_root,
                window_id,
                config,
                profile_config_path,
            )
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
            refresh_managed_gwt_assets_for_worktree(&worktree_path)
                .map_err(|error| error.to_string())?;

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

    pub(crate) fn spawn_wizard_shell_window_async(
        proxy: AppEventProxy,
        project_root: String,
        window_id: String,
        mut config: ShellLaunchConfig,
        profile_config_path: PathBuf,
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_shell_launch_worktree(Path::new(&project_root), &mut config)?;
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

            if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                proxy.send(UserEvent::LaunchProgress {
                    window_id: window_id.clone(),
                    message: "Starting Docker service...".to_string(),
                });
            }

            build_shell_process_launch(Path::new(&project_root), &mut config)
        })();

        proxy.send(UserEvent::ShellLaunchComplete { window_id, result });
    }

    pub(crate) fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            return;
        };
        let _ = gwt_agent::persist_session_status(
            &self.sessions_dir,
            &session.session_id,
            gwt_agent::AgentStatus::Stopped,
        );
        self.launch_wizard_cache.mark_stopped(&session.session_id);
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
        self.mark_agent_session_stopped(window_id);
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
            self.stop_window_runtime(&id);
        }
    }

    pub(crate) fn spawn_output_thread(&self, id: String, pane: Arc<Mutex<Pane>>) -> JoinHandle<()> {
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
        app_state_view_from_parts(
            &self.tabs,
            self.active_tab_id.as_deref(),
            &self.recent_projects,
        )
    }

    pub(crate) fn workspace_state_broadcast(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::WorkspaceState {
            workspace: self.app_state_view(),
        })
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
        Some(gwt::window_state::compose_window_state(
            pty_state, preset, hook_state,
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
        let composed = gwt::window_state::compose_window_state(pty_state, preset, hook_state);
        let address = self.window_lookup.get(window_id)?.clone();
        if let Some(tab) = self.tab_mut(&address.tab_id) {
            let _ = tab.workspace.set_status(&address.raw_id, composed);
        }
        Some(composed)
    }

    fn remove_window_state_tracking(&mut self, window_id: &str) {
        self.window_pty_statuses.remove(window_id);
        self.window_hook_states.remove(window_id);
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

    fn launch_error_events(&mut self, window_id: String, detail: String) -> Vec<OutboundEvent> {
        self.log_window_launch_error("launch_complete", &window_id, &detail);
        if self.tracked_window_exists(&window_id) {
            return self.handle_runtime_status(window_id, WindowProcessStatus::Error, Some(detail));
        }
        Self::status_events(window_id, WindowProcessStatus::Error, Some(detail))
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

    pub(crate) fn persist(&self) -> std::io::Result<()> {
        save_session_state(
            &self.session_state_path,
            &gwt::PersistedSessionState {
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
        )?;

        for tab in &self.tabs {
            save_workspace_state(
                &workspace_state_path(&tab.project_root),
                &tab.workspace.persistable_state(),
            )?;
        }

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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::{mpsc, Arc, Mutex, RwLock},
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use base64::Engine;
    use gwt::{
        empty_workspace_state, load_restored_workspace_state, load_session_state, BackendEvent,
        BranchCleanupInfo, BranchListEntry, BranchScope, FrontendEvent, LaunchWizardAction,
        LaunchWizardContext, LaunchWizardState, ProfileEnvEntryView, ProjectKind, WindowGeometry,
        WindowPreset, WindowProcessStatus, WorkspaceState,
    };
    use gwt_config::{Profile, Settings};
    use gwt_core::{
        coordination::{load_snapshot, post_entry, AuthorKind, BoardEntry, BoardEntryKind},
        logging::{current_log_file, LogEvent, LogLevel},
        notes::{
            create_note as create_memo_note, load_snapshot as load_memo_snapshot, MemoNoteDraft,
        },
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
        dispatch_agent_launch_success, ActiveAgentSession, AgentLaunchCompletion, AppEventProxy,
        AppRuntime, BlockingTaskSpawner, DispatchTarget, KnowledgeLoadRequest,
        KnowledgeRefreshTask, KnowledgeSearchRequest, LaunchWizardMemoryCache, LaunchWizardSession,
        OutboundEvent, ProcessLaunch, ProjectTabRuntime, UserEvent, WindowRuntime,
    };
    use crate::{combined_window_id, same_worktree_path, PtyWriterRegistry};

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
        static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
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
        let python = home
            .join(".gwt")
            .join("runtime")
            .join("chroma-venv")
            .join("bin")
            .join("python3");
        fs::create_dir_all(python.parent().expect("fake python parent"))
            .expect("create fake python dir");
        fs::write(
            &python,
            r#"#!/bin/sh
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
"#,
        )
        .expect("write fake python");
        #[cfg(unix)]
        {
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

    fn prepend_fake_gh_to_path(fake_gh: &Path) -> ScopedEnvVar {
        let parent = fake_gh.parent().expect("fake gh parent");
        let mut paths = vec![parent.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        let joined = std::env::join_paths(paths).expect("join PATH");
        ScopedEnvVar::set("PATH", joined)
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
            z_index: 1,
            status,
            minimized: false,
            maximized: false,
            pre_maximize_geometry: None,
            persist: true,
            agent_id: None,
            agent_color: None,
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
        }
    }

    fn sample_active_agent_session(tab_id: &str, window_id: &str) -> ActiveAgentSession {
        ActiveAgentSession {
            window_id: window_id.to_string(),
            session_id: "session-1".to_string(),
            agent_id: "codex".to_string(),
            branch_name: "feature/test".to_string(),
            display_name: "Codex".to_string(),
            worktree_path: PathBuf::from("E:/gwt/test-repo"),
            tab_id: tab_id.to_string(),
        }
    }

    fn runtime_hook_state(status: &str, session_id: &str) -> gwt::RuntimeHookEvent {
        gwt::RuntimeHookEvent {
            kind: gwt::RuntimeHookEventKind::RuntimeState,
            source_event: Some("Stop".to_string()),
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
        let mut runtime = AppRuntime {
            tabs,
            active_tab_id: active_tab_id.map(str::to_owned),
            recent_projects: Vec::new(),
            profile_selections: HashMap::new(),
            profile_config_path: Some(temp_root.join("profile-config.toml")),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path: temp_root.join("session-state.json"),
            log_dir,
            proxy,
            blocking_tasks: BlockingTaskSpawner::thread(),
            sessions_dir,
            launch_wizard_cache,
            launch_wizard: None,
            active_agent_sessions: HashMap::<String, ActiveAgentSession>::new(),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_cache_dir(),
            pending_update: None,
            pty_writers,
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
            |_project_root| gwt::ProjectIndexStatusView {
                state: gwt::ProjectIndexStatusState::Ready,
                detail: "test bootstrap complete".to_string(),
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
        );

        dispatch_agent_launch_success(
            proxy,
            "tab-1::agent-1".to_string(),
            completion,
            |proxy, project_root| {
                proxy.send(UserEvent::ProjectIndexStatus {
                    project_root: project_root.display().to_string(),
                    status: gwt::ProjectIndexStatusView {
                        state: gwt::ProjectIndexStatusState::Ready,
                        detail: "ready".to_string(),
                    },
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
                    },
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: None,
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                },
                Vec::new(),
            ),
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
                    },
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: Some(project_root.to_path_buf()),
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                },
                Vec::new(),
                Vec::new(),
            ),
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

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(matches!(events[1].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[1].event,
            BackendEvent::LaunchWizardState { wizard: None }
        ));
    }

    #[test]
    fn app_runtime_runtime_status_broadcasts_workspace_before_terminal_status() {
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

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
        ));
        assert!(matches!(events[1].target, DispatchTarget::Broadcast));
        assert!(matches!(
            &events[1].event,
            BackendEvent::WindowState { window_id: id, state }
                if id == &window_id && *state == WindowProcessStatus::Error
        ));
        assert!(matches!(events[2].target, DispatchTarget::Broadcast));
        assert!(matches!(
            &events[2].event,
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
            .open_launch_wizard_for_branch("tab-1", &repo, "feature/demo", None)
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
    fn app_runtime_open_launch_wizard_uses_branch_worktree_for_docker_context() {
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
            .open_launch_wizard_for_branch("tab-1", &repo, "feature/docker", None)
            .expect("open launch wizard");

        let wizard = &runtime.launch_wizard.as_ref().expect("wizard").wizard;
        assert!(wizard
            .context
            .worktree_path
            .as_ref()
            .is_some_and(|path| same_worktree_path(path, &branch_worktree)));
        assert!(same_worktree_path(
            &wizard.context.quick_start_root,
            &branch_worktree
        ));
        let view = wizard.view();
        assert!(view.show_runtime_target);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
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
        let temp = tempdir().expect("tempdir");
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
                )
                .len(),
            1
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
    fn app_runtime_load_board_replies_with_repo_scoped_snapshot() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardEntries { id, entries },
            }] if client_id == "client-1"
                && id == &window_id
                && entries.len() == 1
                && entries[0].body == "Need review"
        ));
    }

    #[test]
    fn app_runtime_load_knowledge_bridge_replies_with_cache_backed_issue_and_spec_views() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                list_scope: gwt::KnowledgeListScope::Open,
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
                list_scope: gwt::KnowledgeListScope::Open,
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
                list_scope: gwt::KnowledgeListScope::Open,
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
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                list_scope: Some(gwt::KnowledgeListScope::Open),
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
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                list_scope: Some(gwt::KnowledgeListScope::Open),
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
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                list_scope: Some(gwt::KnowledgeListScope::Closed),
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
                                    list_scope,
                                    message,
                                    ..
                                } if id == &window_id
                                    && *knowledge_kind == gwt::KnowledgeKind::Issue
                                    && *request_id == Some(32)
                                    && *list_scope == Some(gwt::KnowledgeListScope::Closed)
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
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _gh_lock = fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            project_root: repo.clone(),
            kind: gwt::KnowledgeKind::Issue,
            request_id: Some(33),
            selected_number: None,
            force: false,
            list_scope: gwt::KnowledgeListScope::Open,
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
            list_scope: gwt::KnowledgeListScope::Open,
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
                list_scope: gwt::KnowledgeListScope::Open,
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
    fn app_runtime_load_memo_replies_with_repo_scoped_snapshot() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        create_memo_note(
            &repo,
            MemoNoteDraft::new("Pinned note", "Verify repo-scoped storage", true),
        )
        .expect("seed memo snapshot");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "memo-1",
            repo,
            WindowPreset::Memo,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "memo-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::LoadMemo {
                id: window_id.clone(),
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::MemoNotes {
                    id,
                    notes,
                    selected_note_id,
                },
            }] if client_id == "client-1"
                && id == &window_id
                && notes.len() == 1
                && notes[0].title == "Pinned note"
                && selected_note_id.is_none()
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
        let entry =
            LogEvent::new(LogLevel::Warn, "pty", "runtime stalled").with_detail("retrying read");
        fs::write(
            &log_path,
            format!(
                "{}\n",
                serde_json::to_string(&entry).expect("serialize log event")
            ),
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
                && matches!(entries[0].severity, LogLevel::Warn)
        ));
    }

    #[test]
    fn app_runtime_create_memo_note_broadcasts_repo_scoped_snapshot_to_memo_windows() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut persisted = empty_workspace_state();
        persisted.windows.push(sample_window(
            "memo-1",
            WindowPreset::Memo,
            WindowProcessStatus::Ready,
        ));
        persisted.windows.push(sample_window(
            "memo-2",
            WindowPreset::Memo,
            WindowProcessStatus::Ready,
        ));
        persisted.next_z_index = 3;
        let tab = ProjectTabRuntime {
            id: "tab-1".to_string(),
            title: "Repo".to_string(),
            project_root: repo.clone(),
            kind: ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
            migration_pending: false,
        };
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let current_window_id = combined_window_id("tab-1", "memo-1");
        let sibling_window_id = combined_window_id("tab-1", "memo-2");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::CreateMemoNote {
                id: current_window_id.clone(),
                title: "".to_string(),
                body: "".to_string(),
                pinned: false,
            },
        );

        assert_eq!(events.len(), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MemoNotes {
                    id,
                    notes,
                    selected_note_id: Some(selected_note_id),
                },
            } if id == &current_window_id
                && notes.len() == 1
                && selected_note_id == &notes[0].id
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MemoNotes {
                    id,
                    notes,
                    selected_note_id: None,
                },
            } if id == &sibling_window_id && notes.len() == 1
        )));

        let snapshot = load_memo_snapshot(&repo).expect("load memo snapshot");
        assert_eq!(snapshot.notes.len(), 1);
    }

    #[test]
    fn app_runtime_update_memo_note_persists_repo_scoped_edits() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let created = create_memo_note(&repo, MemoNoteDraft::new("Draft", "Initial note", false))
            .expect("seed memo snapshot");
        let tab = sample_project_tab_with_window_at(
            "tab-1",
            "memo-1",
            repo.clone(),
            WindowPreset::Memo,
            WindowProcessStatus::Ready,
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = combined_window_id("tab-1", "memo-1");

        let events = runtime.handle_frontend_event(
            "client-1".to_string(),
            FrontendEvent::UpdateMemoNote {
                id: window_id.clone(),
                note_id: created.id.clone(),
                title: "Pinned note".to_string(),
                body: "Updated note".to_string(),
                pinned: true,
            },
        );

        assert!(events.iter().any(|event| matches!(
            event,
            OutboundEvent {
                target: DispatchTarget::Broadcast,
                event: BackendEvent::MemoNotes {
                    id,
                    notes,
                    selected_note_id: Some(selected_note_id),
                },
            } if id == &window_id
                && selected_note_id == &created.id
                && notes.iter().any(|note|
                    note.id == created.id
                        && note.title == "Pinned note"
                        && note.body == "Updated note"
                        && note.pinned
                )
        )));

        let snapshot = load_memo_snapshot(&repo).expect("load memo snapshot");
        assert!(snapshot.notes.iter().any(|note| note.id == created.id
            && note.title == "Pinned note"
            && note.body == "Updated note"
            && note.pinned));
    }

    #[test]
    fn app_runtime_post_board_entry_persists_reply_topics_and_owners() {
        let _env_lock = env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            },
        );

        assert!(matches!(
            &events[..],
            [OutboundEvent {
                target: DispatchTarget::Client(client_id),
                event: BackendEvent::BoardEntries { id, entries },
            }] if client_id == "client-1"
                && id == &window_id
                && entries.iter().any(|entry|
                    entry.body == "I will take the next slice"
                    && entry.parent_id.as_deref() == Some(parent.id.as_str())
                    && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
                    && entry.related_owners == vec!["2018".to_string()]
                )
        ));

        let snapshot = load_snapshot(&repo).expect("load board snapshot");
        assert!(snapshot.board.entries.iter().any(|entry| entry.body
            == "I will take the next slice"
            && entry.parent_id.as_deref() == Some(parent.id.as_str())
            && entry.related_topics == vec!["coordination".to_string(), "phase-1b".to_string()]
            && entry.related_owners == vec!["2018".to_string()]));
    }

    #[test]
    fn app_runtime_board_projection_change_broadcasts_to_matching_board_windows_only() {
        let temp = tempdir().expect("tempdir");
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
        };
        let other_tab = sample_project_tab_with_window_at(
            "tab-2",
            "board-3",
            other_repo,
            WindowPreset::Board,
            WindowProcessStatus::Ready,
        );
        let runtime = sample_runtime(temp.path(), vec![matching_tab, other_tab], Some("tab-1"));

        let events = runtime.handle_board_projection_changed_events(&repo);

        assert_eq!(events.len(), 2);
        for expected_id in [
            combined_window_id("tab-1", "board-1"),
            combined_window_id("tab-1", "board-2"),
        ] {
            assert!(events.iter().any(|event| matches!(
                event,
                OutboundEvent {
                    target: DispatchTarget::Broadcast,
                    event: BackendEvent::BoardEntries { id, entries },
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
        }
    }

    #[test]
    fn migration_detected_broadcasts_only_for_pending_tabs() {
        let temp = tempdir().expect("tempdir");
        let repo_a = temp.path().join("repo-a");
        let repo_b = temp.path().join("repo-b");
        fs::create_dir_all(&repo_a).expect("repo-a");
        fs::create_dir_all(&repo_b).expect("repo-b");

        let pending = migration_pending_tab("tab-1", repo_a.clone());
        let mut clean = sample_project_tab("tab-2", "Other", repo_b.clone(), ProjectKind::Git, &[]);
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

        let tab = migration_pending_tab("tab-1", project.clone());
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
}

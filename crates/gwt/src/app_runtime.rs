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

pub(crate) struct WindowRuntime {
    pane: Arc<Mutex<Pane>>,
    /// Handle to the background reader thread that forwards PTY output.
    /// Taken and joined during `stop_window_runtime` so the reader releases
    /// its Arc clone of `pane` before the runtime is fully torn down.
    output_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessLaunch {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct BoardPostRequest {
    pub(crate) id: String,
    pub(crate) entry_kind: gwt_core::coordination::BoardEntryKind,
    pub(crate) body: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) topics: Vec<String>,
    pub(crate) owners: Vec<String>,
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
}

pub(crate) struct AppRuntime {
    pub(crate) tabs: Vec<ProjectTabRuntime>,
    pub(crate) active_tab_id: Option<String>,
    pub(crate) recent_projects: Vec<gwt::RecentProjectEntry>,
    pub(crate) runtimes: HashMap<String, WindowRuntime>,
    pub(crate) window_details: HashMap<String, String>,
    pub(crate) window_lookup: HashMap<String, WindowAddress>,
    pub(crate) session_state_path: PathBuf,
    pub(crate) proxy: AppEventProxy,
    pub(crate) blocking_tasks: BlockingTaskSpawner,
    pub(crate) sessions_dir: PathBuf,
    pub(crate) launch_wizard: Option<LaunchWizardSession>,
    pub(crate) active_agent_sessions: HashMap<String, ActiveAgentSession>,
    pub(crate) hook_forward_target: Option<HookForwardTarget>,
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

        let mut app = Self {
            tabs,
            active_tab_id,
            recent_projects: dedupe_recent_projects(persisted.recent_projects),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path,
            proxy: AppEventProxy::new(proxy),
            blocking_tasks,
            sessions_dir,
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
            hook_forward_target: None,
            pending_update: None,
            pty_writers,
        };
        app.rebuild_window_lookup();
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
            FrontendEvent::LoadKnowledgeBridge {
                id,
                knowledge_kind,
                selected_number,
                refresh,
            } => self.load_knowledge_bridge_events(
                &client_id,
                &id,
                knowledge_kind,
                selected_number,
                refresh,
            ),
            FrontendEvent::SelectKnowledgeBridgeEntry {
                id,
                knowledge_kind,
                number,
            } => self.load_knowledge_bridge_events(
                &client_id,
                &id,
                knowledge_kind,
                Some(number),
                false,
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
            } => self.post_board_entry_events(
                &client_id,
                BoardPostRequest {
                    id,
                    entry_kind,
                    body,
                    parent_id,
                    topics,
                    owners,
                },
            ),
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
            FrontendEvent::AddCustomAgentFromPreset { input } => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::add_from_preset_event(input),
            )],
            FrontendEvent::UpdateCustomAgent { agent } => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::update_event(*agent),
            )],
            FrontendEvent::DeleteCustomAgent { agent_id } => vec![OutboundEvent::reply(
                client_id,
                gwt::custom_agents_dispatch::delete_event(agent_id),
            )],
            FrontendEvent::TestBackendConnection { base_url, api_key } => {
                self.spawn_backend_connection_probe(client_id, base_url, api_key);
                Vec::new()
            }
        }
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

        build_frontend_sync_events(
            client_id,
            self.app_state_view(),
            terminal_statuses,
            terminal_snapshots,
            self.launch_wizard
                .as_ref()
                .map(|wizard| wizard.wizard.view()),
            self.pending_update.clone(),
        )
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
        });
        self.active_tab_id = Some(tab_id);
        self.remember_recent_project(&target);
        let wizard_closed = self.clear_launch_wizard().is_some();
        self.persist().map_err(|error| error.to_string())?;
        Ok(wizard_closed)
    }

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
            self.window_lookup.remove(&window_id);
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

    pub(crate) fn create_window_events(
        &mut self,
        preset: WindowPreset,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.add_window(preset, bounds)
        };
        self.register_window(&tab_id, &window.id);
        let runtime_event = self.start_window(&tab_id, &window.id, window.preset, window.geometry);
        let _ = self.persist();
        let mut events = vec![self.workspace_state_broadcast()];
        if let Some(event) = runtime_event {
            events.push(OutboundEvent::broadcast(event));
        }
        events
    }

    pub(crate) fn focus_window_events(
        &mut self,
        id: &str,
        bounds: Option<WindowGeometry>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(tab) = self.tab_mut(&address.tab_id) else {
            return Vec::new();
        };
        if !tab.workspace.focus_window(&address.raw_id, bounds) {
            return Vec::new();
        }
        self.active_tab_id = Some(address.tab_id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn cycle_focus_events(
        &mut self,
        direction: gwt::FocusCycleDirection,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.active_tab_mut() else {
            return Vec::new();
        };
        if tab.workspace.cycle_focus(direction, bounds).is_none() {
            return Vec::new();
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn update_viewport_events(
        &mut self,
        viewport: gwt::CanvasViewport,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.active_tab_mut() else {
            return Vec::new();
        };
        tab.workspace.update_viewport(viewport);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn arrange_windows_events(
        &mut self,
        mode: gwt::ArrangeMode,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let arranged = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.arrange_windows(mode, bounds)
        };
        if !arranged {
            return Vec::new();
        }
        if let Some(tab) = self.tab(&tab_id) {
            for window in tab.workspace.persisted().windows.iter() {
                self.resize_runtime_to_window(&combined_window_id(&tab_id, &window.id));
            }
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn maximize_window_events(
        &mut self,
        id: &str,
        bounds: WindowGeometry,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.maximize_window(&address.raw_id, bounds)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn minimize_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.minimize_window(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn restore_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.restore_window(&address.raw_id)
        };
        if !updated {
            return Vec::new();
        }
        let _ = self.set_active_tab(address.tab_id);
        self.resize_runtime_to_window(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn update_window_geometry_events(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let updated = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.update_geometry(&address.raw_id, geometry)
        };
        if !updated {
            return Vec::new();
        }
        if let Some(runtime) = self.runtimes.get(id) {
            if let Ok(mut pane) = runtime.pane.lock() {
                let _ = pane.resize(cols.max(20), rows.max(6));
            }
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn close_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        self.stop_window_runtime(id);
        if !close_window_from_workspace(
            &mut self.tabs,
            &mut self.window_lookup,
            &mut self.window_details,
            id,
        ) {
            return Vec::new();
        }
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    pub(crate) fn list_windows_event(&self) -> BackendEvent {
        let windows = self
            .active_tab_id
            .as_ref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| workspace_view_for_tab(tab).windows)
            .unwrap_or_default();
        BackendEvent::WindowList { windows }
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

    pub(crate) fn post_board_entry_events(
        &self,
        client_id: &str,
        request: BoardPostRequest,
    ) -> Vec<OutboundEvent> {
        let BoardPostRequest {
            id,
            entry_kind,
            body,
            parent_id,
            topics,
            owners,
        } = request;

        let Some(address) = self.window_lookup.get(&id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }

        let trimmed_body = body.trim();
        if trimmed_body.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Board entry body is required".to_string(),
                },
            )];
        }

        let parent_id = parent_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let topics = sanitize_board_list(&topics);
        let owners = sanitize_board_list(&owners);

        let snapshot = match gwt_core::coordination::load_snapshot(&tab.project_root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: error.to_string(),
                    },
                )];
            }
        };
        if let Some(parent_id) = parent_id.as_deref() {
            if !snapshot
                .board
                .entries
                .iter()
                .any(|entry| entry.id == parent_id)
            {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: "Reply target was not found".to_string(),
                    },
                )];
            }
        }

        let entry = gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::User,
            "You",
            entry_kind,
            trimmed_body,
            None,
            parent_id,
            topics,
            owners,
        );
        match gwt_core::coordination::post_entry(&tab.project_root, entry) {
            Ok(snapshot) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardEntries {
                    id,
                    entries: snapshot.board.entries,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn load_knowledge_bridge_events(
        &self,
        client_id: &str,
        id: &str,
        kind: KnowledgeKind,
        selected_number: Option<u64>,
        refresh: bool,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: kind,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: kind,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: kind,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: kind,
                    message: "Window is not a knowledge bridge".to_string(),
                },
            )];
        }

        match load_knowledge_bridge(&tab.project_root, kind, selected_number, refresh) {
            Ok(view) => vec![
                OutboundEvent::reply(
                    client_id,
                    BackendEvent::KnowledgeEntries {
                        id: id.to_string(),
                        knowledge_kind: kind,
                        entries: view.entries,
                        selected_number: view.selected_number,
                        empty_message: view.empty_message,
                        refresh_enabled: view.refresh_enabled,
                    },
                ),
                OutboundEvent::reply(
                    client_id,
                    BackendEvent::KnowledgeDetail {
                        id: id.to_string(),
                        knowledge_kind: kind,
                        detail: view.detail,
                    },
                ),
            ],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: kind,
                    message: error,
                },
            )],
        }
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

fn sanitize_board_list(values: &[String]) -> Vec<String> {
    let mut sanitized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || sanitized.iter().any(|item| item == trimmed) {
            continue;
        }
        sanitized.push(trimmed.to_string());
    }
    sanitized
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
    pub(crate) fn open_launch_wizard(
        &mut self,
        id: &str,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            })];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            })];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            })];
        };

        if window.preset != WindowPreset::Branches {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window is not a branches list".to_string(),
            })];
        }

        let project_root = tab.project_root.clone();
        let tab_id = address.tab_id.clone();
        match self.open_launch_wizard_for_branch(
            &tab_id,
            &project_root,
            branch_name,
            linked_issue_number,
        ) {
            Ok(()) => vec![self.launch_wizard_state_outbound()],
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: error,
            })],
        }
    }

    pub(crate) fn open_launch_wizard_for_branch(
        &mut self,
        tab_id: &str,
        project_root: &Path,
        branch_name: &str,
        linked_issue_number: Option<u64>,
    ) -> Result<(), String> {
        let normalized_branch_name = normalize_branch_name(branch_name);
        let live_sessions = self.live_sessions_for_branch(tab_id, &normalized_branch_name);
        let wizard_id = Uuid::new_v4().to_string();
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: wizard_id.clone(),
            wizard: LaunchWizardState::open_loading(
                LaunchWizardContext {
                    selected_branch: synthetic_branch_entry(branch_name),
                    normalized_branch_name,
                    worktree_path: None,
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions,
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number,
                },
                Vec::new(),
            ),
        });

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let project_root = project_root.to_path_buf();
        let branch_name = branch_name.to_string();
        let active_session_branches = self.active_session_branches_for_tab(tab_id);
        thread::spawn(move || {
            let result = resolve_launch_wizard_hydration(
                &project_root,
                &branch_name,
                &active_session_branches,
                &sessions_dir,
            );
            proxy.send(UserEvent::LaunchWizardHydrated { wizard_id, result });
        });

        Ok(())
    }

    pub(crate) fn open_issue_launch_wizard_events(
        &mut self,
        client_id: &str,
        id: &str,
        issue_number: u64,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(kind) = knowledge_kind_for_preset(window.preset) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::KnowledgeError {
                    id: id.to_string(),
                    knowledge_kind: KnowledgeKind::Issue,
                    message: "Window is not a knowledge bridge".to_string(),
                },
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

    pub(crate) fn handle_launch_wizard_hydrated(
        &mut self,
        wizard_id: String,
        result: Result<LaunchWizardHydration, String>,
    ) -> Vec<OutboundEvent> {
        let Some(session) = self.launch_wizard.as_mut() else {
            return Vec::new();
        };
        if session.wizard_id != wizard_id {
            return Vec::new();
        }

        match result {
            Ok(hydration) => session.wizard.apply_hydration(hydration),
            Err(error) => session.wizard.set_hydration_error(error),
        }

        vec![self.launch_wizard_state_outbound()]
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
                BackendEvent::KnowledgeError {
                    id,
                    knowledge_kind,
                    message: "Project tab not found".to_string(),
                },
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
                    BackendEvent::KnowledgeError {
                        id,
                        knowledge_kind,
                        message: error,
                    },
                )],
            },
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::KnowledgeError {
                    id,
                    knowledge_kind,
                    message: error,
                },
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
        session.wizard.apply(action);

        match session.wizard.completion.take() {
            Some(LaunchWizardCompletion::Cancelled) => {
                vec![self.launch_wizard_state_broadcast(None)]
            }
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    session.wizard.error =
                        Some("The selected session window is no longer available".to_string());
                    self.launch_wizard = Some(session);
                    return vec![self.launch_wizard_state_outbound()];
                };
                let Some(tab) = self.tab_mut(&address.tab_id) else {
                    return Vec::new();
                };
                if !tab.workspace.focus_window(&address.raw_id, None) {
                    session.wizard.error =
                        Some("The selected session window is no longer available".to_string());
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
            Some(LaunchWizardCompletion::Launch(config)) => match *config {
                LaunchWizardLaunchRequest::Agent(config) => {
                    match self.spawn_agent_window(&session.tab_id, *config, bounds) {
                        Ok(mut events) => {
                            events.push(self.launch_wizard_state_broadcast(None));
                            events
                        }
                        Err(error) => {
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
                            session.wizard.error = Some(error);
                            self.launch_wizard = Some(session);
                            vec![self.launch_wizard_state_outbound()]
                        }
                    }
                }
            },
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
        let should_auto_close =
            should_auto_close_agent_window(&self.active_agent_sessions, &id, &status);
        let Some(address) = self.window_lookup.get(&id).cloned() else {
            self.deregister_pty_writer(&id);
            self.runtimes.remove(&id);
            self.window_details.remove(&id);
            return Vec::new();
        };

        if let Some(tab) = self.tab_mut(&address.tab_id) {
            let _ = tab.workspace.set_status(&address.raw_id, status.clone());
        }
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
            WindowProcessStatus::Error | WindowProcessStatus::Exited
        ) {
            self.runtimes.remove(&id);
            self.mark_agent_session_stopped(&id);
        }
        let _ = self.persist();

        vec![
            self.workspace_state_broadcast(),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus { id, status, detail }),
        ]
    }

    pub(crate) fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: Result<
            (
                ProcessLaunch,
                String,
                String,
                String,
                PathBuf,
                gwt_agent::AgentId,
            ),
            String,
        >,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok((
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
            )) => {
                let Some(address) = self.window_lookup.get(&window_id).cloned() else {
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Window not found".to_string()),
                    })];
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Project tab not found".to_string()),
                    })];
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Window not found".to_string()),
                    })];
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
                        worktree_path,
                        tab_id: address.tab_id,
                    },
                );

                let _ = self.persist();

                match self.spawn_process_window(&window_id, geometry, process_launch) {
                    Ok(event) => vec![
                        self.workspace_state_broadcast(),
                        OutboundEvent::broadcast(event),
                    ],
                    Err(error) => vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    })],
                }
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id,
                status: WindowProcessStatus::Error,
                detail: Some(error),
            })],
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
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Window not found".to_string()),
                    })];
                };
                let Some(tab) = self.tab(&address.tab_id) else {
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Project tab not found".to_string()),
                    })];
                };
                let Some(window) = tab.workspace.window(&address.raw_id) else {
                    return vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some("Window not found".to_string()),
                    })];
                };
                let geometry = window.geometry.clone();

                match self.spawn_process_window(&window_id, geometry, process_launch) {
                    Ok(event) => vec![
                        self.workspace_state_broadcast(),
                        OutboundEvent::broadcast(event),
                    ],
                    Err(error) => vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                        id: window_id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    })],
                }
            }
            Err(error) => vec![OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id,
                status: WindowProcessStatus::Error,
                detail: Some(error),
            })],
        }
    }

    pub(crate) fn start_window(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Option<BackendEvent> {
        self.register_window(tab_id, raw_id);
        let window_id = combined_window_id(tab_id, raw_id);
        if !preset.requires_process() {
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Ready);
            return None;
        }

        let project_root = self
            .tab(tab_id)
            .map(|tab| tab.project_root.clone())
            .unwrap_or_else(|| PathBuf::from("."));

        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), error.to_string());
                return Some(BackendEvent::TerminalStatus {
                    id: window_id,
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        let launch = match resolve_launch_spec_with_fallback(preset, &shell) {
            Ok(launch) => launch,
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), error.to_string());
                return Some(BackendEvent::TerminalStatus {
                    id: window_id,
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        match self.spawn_process_window(
            &window_id,
            geometry,
            ProcessLaunch {
                command: launch.command,
                args: launch.args,
                env: spawn_env(),
                cwd: Some(project_root),
            },
        ) {
            Ok(event) => Some(event),
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                Some(BackendEvent::TerminalStatus {
                    id: window_id,
                    status: WindowProcessStatus::Error,
                    detail: Some(error),
                })
            }
        }
    }

    pub(crate) fn spawn_process_window(
        &mut self,
        id: &str,
        geometry: WindowGeometry,
        launch: ProcessLaunch,
    ) -> Result<BackendEvent, String> {
        let (cols, rows) = geometry_to_pty_size(&geometry);
        let pane = Pane::new(
            id.to_string(),
            launch.command,
            launch.args,
            cols,
            rows,
            launch.env,
            launch.cwd,
        )
        .map_err(|error| error.to_string())?;
        let pane = Arc::new(Mutex::new(pane));

        let output_thread = self.spawn_output_thread(id.to_string(), pane.clone());
        if let Some(address) = self.window_lookup.get(id).cloned() {
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
            },
        );
        Ok(BackendEvent::TerminalStatus {
            id: id.to_string(),
            status: WindowProcessStatus::Running,
            detail: None,
        })
    }

    pub(crate) fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        bounds: Option<WindowGeometry>,
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
        let default_bounds = WindowGeometry {
            x: 100.0,
            y: 40.0,
            width: 1000.0,
            height: 760.0,
        };
        let window = tab.workspace.add_window_with_title(
            WindowPreset::Agent,
            title.clone(),
            false,
            bounds.unwrap_or(default_bounds),
        );
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        let events = vec![
            self.workspace_state_broadcast(),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id.clone(),
                status: WindowProcessStatus::Starting,
                detail: None,
            }),
        ];

        let proxy = self.proxy.clone();
        let sessions_dir = self.sessions_dir.clone();
        let hook_forward_target = self.hook_forward_target.clone();

        thread::spawn(move || {
            Self::spawn_agent_window_async(
                proxy,
                sessions_dir,
                project_root,
                window_id,
                config,
                hook_forward_target,
            )
        });

        Ok(events)
    }

    pub(crate) fn spawn_wizard_shell_window(
        &mut self,
        tab_id: &str,
        config: ShellLaunchConfig,
        bounds: Option<WindowGeometry>,
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
        let default_bounds = WindowGeometry {
            x: 100.0,
            y: 40.0,
            width: 1000.0,
            height: 760.0,
        };
        let window = tab.workspace.add_window_with_title(
            WindowPreset::Shell,
            title,
            false,
            bounds.unwrap_or(default_bounds),
        );
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);

        let events = vec![
            self.workspace_state_broadcast(),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus {
                id: window_id.clone(),
                status: WindowProcessStatus::Starting,
                detail: None,
            }),
        ];

        let proxy = self.proxy.clone();
        thread::spawn(move || {
            Self::spawn_wizard_shell_window_async(proxy, project_root, window_id, config)
        });

        Ok(events)
    }

    pub(crate) fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        mut config: gwt_agent::LaunchConfig,
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
            refresh_managed_gwt_assets_for_worktree(&worktree_path)
                .map_err(|error| error.to_string())?;
            if let Err(error) = gwt::index_worker::bootstrap_project_index_for_path(&worktree_path)
            {
                tracing::warn!(
                    worktree = %worktree_path.display(),
                    error = %error,
                    "project index bootstrap skipped during worktree prepare"
                );
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
            session.skip_permissions = config.skip_permissions;
            session.codex_fast_mode = config.codex_fast_mode;
            session.runtime_target = config.runtime_target;
            session.docker_service = config.docker_service.clone();
            session.docker_lifecycle_intent = config.docker_lifecycle_intent;
            session.linked_issue_number = config.linked_issue_number;
            session.launch_command = config.command.clone();
            session.launch_args = config.args.clone();
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
                cwd: config.working_dir.clone(),
            };

            Ok((
                process_launch,
                session_id,
                branch_name,
                config.display_name,
                worktree_path,
                agent_id,
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
            )) => {
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    result: Ok((
                        process_launch,
                        session_id,
                        branch_name,
                        display_name,
                        worktree_path,
                        agent_id,
                    )),
                });
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
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });
            resolve_shell_launch_worktree(Path::new(&project_root), &mut config)?;

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
                Ok(PaneStatus::Running) | Ok(PaneStatus::Completed(0)) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Exited,
                        detail: Some("Process exited".to_string()),
                    });
                }
                Ok(PaneStatus::Completed(code)) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(format!("Process exited with status {code}")),
                    });
                }
                Ok(PaneStatus::Error(message)) => {
                    proxy.send(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(message),
                    });
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

    pub(crate) fn launch_wizard_state_outbound(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: self
                .launch_wizard
                .as_ref()
                .map(|wizard| Box::new(wizard.wizard.view())),
        })
    }

    pub(crate) fn launch_wizard_state_broadcast(
        &self,
        wizard: Option<gwt::LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: wizard.map(Box::new),
        })
    }

    pub(crate) fn window_status(&self, window_id: &str) -> Option<WindowProcessStatus> {
        let address = self.window_lookup.get(window_id)?;
        let tab = self.tab(&address.tab_id)?;
        let window = tab.workspace.window(&address.raw_id)?;
        Some(window.status.clone())
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

    pub(crate) fn clear_launch_wizard(&mut self) -> Option<LaunchWizardSession> {
        self.launch_wizard.take()
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

    pub(crate) fn seed_restored_window_details(&mut self) {
        for tab in &self.tabs {
            for window in &tab.workspace.persisted().windows {
                if window.preset.requires_process() && window.status == WindowProcessStatus::Exited
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex, RwLock},
    };

    use tempfile::tempdir;

    use base64::Engine;
    use gwt::{
        empty_workspace_state, load_restored_workspace_state, load_session_state, BackendEvent,
        BranchCleanupInfo, BranchListEntry, BranchScope, FrontendEvent, LaunchWizardContext,
        LaunchWizardState, ProjectKind, WindowGeometry, WindowPreset, WindowProcessStatus,
        WorkspaceState,
    };
    use gwt_core::coordination::{
        load_snapshot, post_entry, AuthorKind, BoardEntry, BoardEntryKind,
    };
    use gwt_terminal::Pane;

    use super::{
        ActiveAgentSession, AppEventProxy, AppRuntime, BlockingTaskSpawner, DispatchTarget,
        LaunchWizardSession, OutboundEvent, ProjectTabRuntime, WindowRuntime,
    };
    use crate::{combined_window_id, PtyWriterRegistry};

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
        }
    }

    fn sample_runtime(
        temp_root: &Path,
        tabs: Vec<ProjectTabRuntime>,
        active_tab_id: Option<&str>,
    ) -> AppRuntime {
        let (proxy, _events) = AppEventProxy::stub();
        let sessions_dir = temp_root.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let pty_writers: PtyWriterRegistry = Arc::new(RwLock::new(HashMap::new()));
        let mut runtime = AppRuntime {
            tabs,
            active_tab_id: active_tab_id.map(str::to_owned),
            recent_projects: Vec::new(),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path: temp_root.join("session-state.json"),
            proxy,
            blocking_tasks: BlockingTaskSpawner::thread(),
            sessions_dir,
            launch_wizard: None,
            active_agent_sessions: HashMap::<String, ActiveAgentSession>::new(),
            hook_forward_target: None,
            pending_update: None,
            pty_writers,
        };
        runtime.rebuild_window_lookup();
        runtime
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
                vec!["/C".to_string(), "exit 0".to_string()],
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

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0].target, DispatchTarget::Broadcast));
        assert!(matches!(
            events[0].event,
            BackendEvent::WorkspaceState { .. }
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

        let event = runtime
            .start_window("tab-1", &window.id, window.preset, window.geometry.clone())
            .expect("process launch event");

        assert!(matches!(
            event,
            BackendEvent::TerminalStatus {
                ref id,
                status: WindowProcessStatus::Running,
                detail: None,
            } if id == &window_id
        ));
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
        let temp = tempdir().expect("tempdir");
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
    fn app_runtime_post_board_entry_persists_reply_topics_and_owners() {
        let temp = tempdir().expect("tempdir");
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
}

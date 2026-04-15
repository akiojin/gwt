use std::{
    collections::HashMap,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use gwt::{
    default_wizard_version_cache_path, detect_shell_program, list_branch_entries,
    list_directory_entries, load_restored_workspace_state, load_session_state,
    migrate_legacy_workspace_state, refresh_managed_gwt_assets_for_worktree, resolve_launch_spec,
    save_session_state, save_workspace_state, workspace_state_path, BackendEvent,
    DockerWizardContext, FrontendEvent, LaunchWizardCompletion, LaunchWizardContext,
    LaunchWizardState, LiveSessionEntry, WindowGeometry, WindowPreset, WindowProcessStatus,
    WorkspaceState, APP_NAME,
};
use gwt_terminal::{Pane, PaneStatus};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use tokio::{
    net::TcpListener,
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use uuid::Uuid;
use wry::WebViewBuilder;

type ClientId = String;
const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";

#[derive(Debug, Clone)]
enum UserEvent {
    Frontend {
        client_id: ClientId,
        event: FrontendEvent,
    },
    RuntimeOutput {
        id: String,
        data: Vec<u8>,
    },
    RuntimeStatus {
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    #[cfg(target_os = "macos")]
    MenuEvent(muda::MenuEvent),
}

struct WindowRuntime {
    pane: Arc<Mutex<Pane>>,
}

#[derive(Debug, Clone)]
struct ProcessLaunch {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ActiveAgentSession {
    window_id: String,
    session_id: String,
    branch_name: String,
    display_name: String,
    worktree_path: PathBuf,
    tab_id: String,
}

#[derive(Debug, Clone)]
enum DispatchTarget {
    Broadcast,
    Client(ClientId),
}

#[derive(Debug, Clone)]
struct OutboundEvent {
    target: DispatchTarget,
    event: BackendEvent,
}

impl OutboundEvent {
    fn broadcast(event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Broadcast,
            event,
        }
    }

    fn reply(client_id: impl Into<ClientId>, event: BackendEvent) -> Self {
        Self {
            target: DispatchTarget::Client(client_id.into()),
            event,
        }
    }
}

#[derive(Debug, Clone)]
struct ProjectTabRuntime {
    id: String,
    title: String,
    project_root: PathBuf,
    kind: gwt::ProjectKind,
    workspace: WorkspaceState,
}

#[derive(Debug, Clone)]
struct WindowAddress {
    tab_id: String,
    raw_id: String,
}

#[derive(Debug, Clone)]
struct LaunchWizardSession {
    tab_id: String,
    wizard: LaunchWizardState,
}

#[derive(Debug, Clone)]
struct ProjectOpenTarget {
    project_root: PathBuf,
    title: String,
    kind: gwt::ProjectKind,
}

struct AppRuntime {
    tabs: Vec<ProjectTabRuntime>,
    active_tab_id: Option<String>,
    recent_projects: Vec<gwt::RecentProjectEntry>,
    runtimes: HashMap<String, WindowRuntime>,
    window_details: HashMap<String, String>,
    window_lookup: HashMap<String, WindowAddress>,
    session_state_path: PathBuf,
    proxy: EventLoopProxy<UserEvent>,
    sessions_dir: PathBuf,
    launch_wizard: Option<LaunchWizardSession>,
    active_agent_sessions: HashMap<String, ActiveAgentSession>,
}

impl ProjectTabRuntime {
    fn from_persisted(
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
    fn new(proxy: EventLoopProxy<UserEvent>) -> std::io::Result<Self> {
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
            proxy,
            sessions_dir,
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
        };
        app.rebuild_window_lookup();
        app.seed_restored_window_details();
        Ok(app)
    }

    fn bootstrap(&mut self) {
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

    fn handle_frontend_event(
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
            FrontendEvent::CreateWindow { preset } => self.create_window_events(preset),
            FrontendEvent::FocusWindow { id } => self.focus_window_events(&id),
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
            FrontendEvent::LoadBranches { id } => {
                vec![OutboundEvent::reply(
                    client_id,
                    self.load_branches_event(&id),
                )]
            }
            FrontendEvent::OpenLaunchWizard { id, branch_name } => {
                self.open_launch_wizard(&id, &branch_name)
            }
            FrontendEvent::LaunchWizardAction { action } => {
                self.handle_launch_wizard_action(action)
            }
        }
    }

    fn frontend_sync_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let mut events = vec![OutboundEvent::reply(
            client_id,
            BackendEvent::WorkspaceState {
                workspace: self.app_state_view(),
            },
        )];

        for (id, detail) in &self.window_details {
            let Some(status) = self.window_status(id) else {
                continue;
            };
            events.push(OutboundEvent::reply(
                client_id,
                BackendEvent::TerminalStatus {
                    id: id.clone(),
                    status,
                    detail: Some(detail.clone()),
                },
            ));
        }

        for (id, runtime) in &self.runtimes {
            let snapshot = runtime
                .pane
                .lock()
                .map(|pane| pane.screen().contents().into_bytes())
                .unwrap_or_default();
            if snapshot.is_empty() {
                continue;
            }
            events.push(OutboundEvent::reply(
                client_id,
                BackendEvent::TerminalSnapshot {
                    id: id.clone(),
                    data_base64: base64::engine::general_purpose::STANDARD.encode(snapshot),
                },
            ));
        }

        if let Some(wizard) = self.launch_wizard.as_ref() {
            events.push(OutboundEvent::reply(
                client_id,
                BackendEvent::LaunchWizardState {
                    wizard: Some(Box::new(wizard.wizard.view())),
                },
            ));
        }

        events
    }

    fn open_project_dialog_events(&mut self) -> Vec<OutboundEvent> {
        let selected = rfd::FileDialog::new().pick_folder();
        let Some(path) = selected else {
            return Vec::new();
        };
        self.open_project_path_events(path)
    }

    fn open_project_path_events(&mut self, path: PathBuf) -> Vec<OutboundEvent> {
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

    fn open_project_path(&mut self, path: PathBuf) -> Result<bool, String> {
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

    fn remember_recent_project(&mut self, target: &ProjectOpenTarget) {
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

    fn select_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
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

    fn close_project_tab_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
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

    fn create_window_events(&mut self, preset: WindowPreset) -> Vec<OutboundEvent> {
        let Some(tab_id) = self.active_tab_id.clone() else {
            return Vec::new();
        };
        let window = {
            let Some(tab) = self.tab_mut(&tab_id) else {
                return Vec::new();
            };
            tab.workspace.add_window(preset)
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

    fn focus_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        let Some(tab) = self.tab_mut(&address.tab_id) else {
            return Vec::new();
        };
        if !tab.workspace.focus_window(&address.raw_id) {
            return Vec::new();
        }
        self.active_tab_id = Some(address.tab_id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    fn cycle_focus_events(
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

    fn update_viewport_events(&mut self, viewport: gwt::CanvasViewport) -> Vec<OutboundEvent> {
        let Some(tab) = self.active_tab_mut() else {
            return Vec::new();
        };
        tab.workspace.update_viewport(viewport);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    fn arrange_windows_events(
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

    fn maximize_window_events(&mut self, id: &str, bounds: WindowGeometry) -> Vec<OutboundEvent> {
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

    fn minimize_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
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

    fn restore_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
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

    fn update_window_geometry_events(
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

    fn close_window_events(&mut self, id: &str) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id).cloned() else {
            return Vec::new();
        };
        self.stop_window_runtime(id);
        let closed = {
            let Some(tab) = self.tab_mut(&address.tab_id) else {
                return Vec::new();
            };
            tab.workspace.close_window(&address.raw_id)
        };
        if !closed {
            return Vec::new();
        }
        self.window_lookup.remove(id);
        let _ = self.persist();
        vec![self.workspace_state_broadcast()]
    }

    fn list_windows_event(&self) -> BackendEvent {
        let windows = self
            .active_tab_id
            .as_ref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| self.workspace_view(tab).windows)
            .unwrap_or_default();
        BackendEvent::WindowList { windows }
    }

    fn terminal_input_events(&mut self, id: &str, data: &str) -> Vec<OutboundEvent> {
        let Some(runtime) = self.runtimes.get(id) else {
            return Vec::new();
        };
        match runtime
            .pane
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|pane| {
                pane.write_input(data.as_bytes())
                    .map_err(|error| error.to_string())
            }) {
            Ok(()) => Vec::new(),
            Err(error) => {
                self.handle_runtime_status(id.to_string(), WindowProcessStatus::Error, Some(error))
            }
        }
    }

    fn load_file_tree_event(&self, id: &str, path: &str) -> BackendEvent {
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

    fn load_branches_event(&self, id: &str) -> BackendEvent {
        let Some(address) = self.window_lookup.get(id) else {
            return BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return BackendEvent::BranchError {
                id: id.to_string(),
                message: "Project tab not found".to_string(),
            };
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window not found".to_string(),
            };
        };

        if window.preset != WindowPreset::Branches {
            return BackendEvent::BranchError {
                id: id.to_string(),
                message: "Window is not a branches list".to_string(),
            };
        }

        match list_branch_entries(&tab.project_root) {
            Ok(entries) => BackendEvent::BranchEntries {
                id: id.to_string(),
                entries,
            },
            Err(error) => BackendEvent::BranchError {
                id: id.to_string(),
                message: error.to_string(),
            },
        }
    }

    fn open_launch_wizard(&mut self, id: &str, branch_name: &str) -> Vec<OutboundEvent> {
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

        let entries = match list_branch_entries(&tab.project_root) {
            Ok(entries) => entries,
            Err(error) => {
                return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                    id: id.to_string(),
                    message: error.to_string(),
                })];
            }
        };

        let Some(selected_branch) = entries.into_iter().find(|entry| entry.name == branch_name)
        else {
            return vec![OutboundEvent::broadcast(BackendEvent::BranchError {
                id: id.to_string(),
                message: format!("Branch not found: {branch_name}"),
            })];
        };

        let normalized_branch_name = normalize_branch_name(&selected_branch.name);
        let worktree_path = branch_worktree_path(&tab.project_root, &normalized_branch_name);
        let quick_start_root = worktree_path
            .clone()
            .unwrap_or_else(|| tab.project_root.clone());
        let live_sessions = self.live_sessions_for_branch(&address.tab_id, &normalized_branch_name);
        let (docker_context, docker_service_status) =
            detect_wizard_docker_context_and_status(&quick_start_root);
        self.launch_wizard = Some(LaunchWizardSession {
            tab_id: address.tab_id,
            wizard: LaunchWizardState::open(
                LaunchWizardContext {
                    selected_branch,
                    normalized_branch_name,
                    worktree_path,
                    quick_start_root,
                    live_sessions,
                    docker_context,
                    docker_service_status,
                },
                &self.sessions_dir,
                &default_wizard_version_cache_path(),
            ),
        });

        vec![self.launch_wizard_state_outbound()]
    }

    fn handle_launch_wizard_action(
        &mut self,
        action: gwt::LaunchWizardAction,
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
                if !tab.workspace.focus_window(&address.raw_id) {
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
            Some(LaunchWizardCompletion::Launch(config)) => {
                match self.spawn_agent_window(&session.tab_id, *config) {
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
            None => {
                self.launch_wizard = Some(session);
                vec![self.launch_wizard_state_outbound()]
            }
        }
    }

    fn live_sessions_for_branch(&self, tab_id: &str, branch_name: &str) -> Vec<LiveSessionEntry> {
        let mut entries = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id && session.branch_name == branch_name)
            .map(|session| LiveSessionEntry {
                session_id: session.session_id.clone(),
                window_id: session.window_id.clone(),
                kind: "agent".to_string(),
                name: session.display_name.clone(),
                detail: Some(session.worktree_path.display().to_string()),
                active: true,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        entries
    }

    fn handle_runtime_output(&mut self, id: String, data: Vec<u8>) -> Vec<OutboundEvent> {
        if !self.window_lookup.contains_key(&id) {
            return Vec::new();
        }
        vec![OutboundEvent::broadcast(BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        })]
    }

    fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(&id).cloned() else {
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

    fn start_window(
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

    fn spawn_process_window(
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

        self.spawn_output_thread(id.to_string(), pane.clone());
        if let Some(address) = self.window_lookup.get(id).cloned() {
            self.set_window_status(
                &address.tab_id,
                &address.raw_id,
                WindowProcessStatus::Running,
            );
        }
        self.window_details.remove(id);
        self.runtimes.insert(id.to_string(), WindowRuntime { pane });
        Ok(BackendEvent::TerminalStatus {
            id: id.to_string(),
            status: WindowProcessStatus::Running,
            detail: None,
        })
    }

    fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        mut config: gwt_agent::LaunchConfig,
    ) -> Result<Vec<OutboundEvent>, String> {
        let project_root = self
            .tab(tab_id)
            .map(|tab| tab.project_root.clone())
            .ok_or_else(|| "Project tab not found".to_string())?;
        resolve_launch_worktree(&project_root, &mut config)?;
        apply_docker_runtime_to_launch_config(&project_root, &mut config)?;

        let worktree_path = config
            .working_dir
            .clone()
            .unwrap_or_else(|| project_root.clone());
        refresh_managed_gwt_assets_for_worktree(&worktree_path)
            .map_err(|error| error.to_string())?;
        let branch_name = config
            .branch
            .clone()
            .unwrap_or_else(|| "workspace".to_string());

        let mut session =
            gwt_agent::Session::new(&worktree_path, branch_name.clone(), config.agent_id.clone());
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

        let runtime_path = gwt_agent::runtime_state_path(&self.sessions_dir, &session.id);
        config.env_vars.insert(
            gwt_agent::GWT_SESSION_ID_ENV.to_string(),
            session.id.clone(),
        );
        config.env_vars.insert(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
            runtime_path.display().to_string(),
        );
        if let Ok(gwt_bin) = std::env::current_exe() {
            config.env_vars.insert(
                gwt_agent::session::GWT_BIN_PATH_ENV.to_string(),
                gwt_bin.display().to_string(),
            );
        }
        config
            .env_vars
            .entry("COLORTERM".to_string())
            .or_insert_with(|| "truecolor".to_string());

        let title = format!("{} · {}", config.display_name, branch_name);
        let window = {
            let Some(tab) = self.tab_mut(tab_id) else {
                return Err("Project tab not found".to_string());
            };
            tab.workspace
                .add_window_with_title(WindowPreset::Agent, title, false)
        };
        self.register_window(tab_id, &window.id);
        let window_id = combined_window_id(tab_id, &window.id);
        let runtime_event = match self.spawn_process_window(
            &window_id,
            window.geometry.clone(),
            ProcessLaunch {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env_vars.clone(),
                cwd: config.working_dir.clone(),
            },
        ) {
            Ok(event) => event,
            Err(error) => {
                if let Some(tab) = self.tab_mut(tab_id) {
                    let _ = tab.workspace.close_window(&window.id);
                }
                self.window_lookup.remove(&window_id);
                return Err(error);
            }
        };

        session
            .save(&self.sessions_dir)
            .map_err(|error| error.to_string())?;
        gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Running)
            .save(&runtime_path)
            .map_err(|error| error.to_string())?;

        self.active_agent_sessions.insert(
            window_id.clone(),
            ActiveAgentSession {
                window_id: window_id.clone(),
                session_id: session.id.clone(),
                branch_name,
                display_name: config.display_name.clone(),
                worktree_path,
                tab_id: tab_id.to_string(),
            },
        );

        let _ = self.persist();
        Ok(vec![
            self.workspace_state_broadcast(),
            OutboundEvent::broadcast(runtime_event),
        ])
    }

    fn mark_agent_session_stopped(&mut self, window_id: &str) {
        let Some(session) = self.active_agent_sessions.remove(window_id) else {
            return;
        };
        let _ = gwt_agent::persist_session_status(
            &self.sessions_dir,
            &session.session_id,
            gwt_agent::AgentStatus::Stopped,
        );
    }

    fn stop_window_runtime(&mut self, window_id: &str) {
        self.mark_agent_session_stopped(window_id);
        if let Some(runtime) = self.runtimes.remove(window_id) {
            if let Ok(pane) = runtime.pane.lock() {
                let _ = pane.kill();
            }
        }
        self.window_details.remove(window_id);
    }

    fn spawn_output_thread(&self, id: String, pane: Arc<Mutex<Pane>>) {
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let reader = match pane
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|pane| pane.reader().map_err(|error| error.to_string()))
            {
                Ok(reader) => reader,
                Err(error) => {
                    let _ = proxy.send_event(UserEvent::RuntimeStatus {
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
                        if let Ok(mut pane) = pane.lock() {
                            pane.process_bytes(&chunk);
                        }
                        let _ = proxy.send_event(UserEvent::RuntimeOutput {
                            id: id.clone(),
                            data: chunk,
                        });
                    }
                    Err(error) => {
                        let _ = proxy.send_event(UserEvent::RuntimeStatus {
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
                Ok(PaneStatus::Running) | Ok(PaneStatus::Completed(_)) => {
                    let _ = proxy.send_event(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Exited,
                        detail: Some("Process exited".to_string()),
                    });
                }
                Ok(PaneStatus::Error(message)) => {
                    let _ = proxy.send_event(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(message),
                    });
                }
                Err(error) => {
                    let _ = proxy.send_event(UserEvent::RuntimeStatus {
                        id,
                        status: WindowProcessStatus::Error,
                        detail: Some(error),
                    });
                }
            }
        });
    }

    fn app_state_view(&self) -> gwt::AppStateView {
        gwt::AppStateView {
            tabs: self
                .tabs
                .iter()
                .map(|tab| gwt::ProjectTabView {
                    id: tab.id.clone(),
                    title: tab.title.clone(),
                    project_root: tab.project_root.display().to_string(),
                    kind: tab.kind,
                    workspace: self.workspace_view(tab),
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

    fn workspace_view(&self, tab: &ProjectTabRuntime) -> gwt::WorkspaceView {
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

    fn workspace_state_broadcast(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::WorkspaceState {
            workspace: self.app_state_view(),
        })
    }

    fn launch_wizard_state_outbound(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: self
                .launch_wizard
                .as_ref()
                .map(|wizard| Box::new(wizard.wizard.view())),
        })
    }

    fn launch_wizard_state_broadcast(
        &self,
        wizard: Option<gwt::LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: wizard.map(Box::new),
        })
    }

    fn window_status(&self, window_id: &str) -> Option<WindowProcessStatus> {
        let address = self.window_lookup.get(window_id)?;
        let tab = self.tab(&address.tab_id)?;
        let window = tab.workspace.window(&address.raw_id)?;
        Some(window.status.clone())
    }

    fn register_window(&mut self, tab_id: &str, raw_id: &str) {
        self.window_lookup.insert(
            combined_window_id(tab_id, raw_id),
            WindowAddress {
                tab_id: tab_id.to_string(),
                raw_id: raw_id.to_string(),
            },
        );
    }

    fn set_window_status(&mut self, tab_id: &str, raw_id: &str, status: WindowProcessStatus) {
        if let Some(tab) = self.tab_mut(tab_id) {
            let _ = tab.workspace.set_status(raw_id, status);
        }
    }

    fn resize_runtime_to_window(&self, window_id: &str) {
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

    fn tab(&self, tab_id: &str) -> Option<&ProjectTabRuntime> {
        self.tabs.iter().find(|tab| tab.id == tab_id)
    }

    fn tab_mut(&mut self, tab_id: &str) -> Option<&mut ProjectTabRuntime> {
        self.tabs.iter_mut().find(|tab| tab.id == tab_id)
    }

    fn active_tab_mut(&mut self) -> Option<&mut ProjectTabRuntime> {
        let active_tab_id = self.active_tab_id.clone()?;
        self.tab_mut(&active_tab_id)
    }

    fn set_active_tab(&mut self, tab_id: String) -> bool {
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

    fn clear_launch_wizard(&mut self) -> Option<LaunchWizardSession> {
        self.launch_wizard.take()
    }

    fn rebuild_window_lookup(&mut self) {
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

    fn seed_restored_window_details(&mut self) {
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

    fn persist(&self) -> std::io::Result<()> {
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

fn combined_window_id(tab_id: &str, raw_id: &str) -> String {
    format!("{tab_id}::{raw_id}")
}

fn should_auto_start_restored_window(window: &gwt::PersistedWindowState) -> bool {
    window.preset.requires_process()
        && matches!(
            window.status,
            WindowProcessStatus::Starting | WindowProcessStatus::Running
        )
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use gwt::{PersistedWindowState, WindowGeometry, WindowPreset, WindowProcessStatus};
    use tempfile::tempdir;

    use super::{resolve_project_target, should_auto_start_restored_window};

    fn sample_window(preset: WindowPreset, status: WindowProcessStatus) -> PersistedWindowState {
        PersistedWindowState {
            id: "sample-1".to_string(),
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

    #[test]
    fn restored_process_window_is_not_auto_started_when_exited() {
        assert!(!should_auto_start_restored_window(&sample_window(
            WindowPreset::Claude,
            WindowProcessStatus::Exited,
        )));
    }

    #[test]
    fn restored_process_window_is_auto_started_only_when_running_or_starting() {
        assert!(should_auto_start_restored_window(&sample_window(
            WindowPreset::Shell,
            WindowProcessStatus::Running,
        )));
        assert!(should_auto_start_restored_window(&sample_window(
            WindowPreset::Shell,
            WindowProcessStatus::Starting,
        )));
        assert!(!should_auto_start_restored_window(&sample_window(
            WindowPreset::Branches,
            WindowProcessStatus::Ready,
        )));
    }

    #[test]
    fn resolve_project_target_uses_selected_directory_name_for_git_subdir_title() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("demo-repo");
        fs::create_dir_all(repo.join("apps/frontend")).expect("create repo dirs");
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .arg(&repo)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");

        let selected = repo.join("apps/frontend");
        let target = resolve_project_target(&selected).expect("project target");

        assert_eq!(target.title, "frontend");
        assert_eq!(target.kind, gwt::ProjectKind::Git);
        assert_eq!(
            target.project_root,
            dunce::canonicalize(&repo).expect("canonical repo root"),
        );
    }
}

fn normalize_active_tab_id(
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

fn dedupe_recent_projects(entries: Vec<gwt::RecentProjectEntry>) -> Vec<gwt::RecentProjectEntry> {
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

fn fallback_project_target(path: PathBuf) -> ProjectOpenTarget {
    ProjectOpenTarget {
        title: gwt::project_title_from_path(&path),
        project_root: path,
        kind: gwt::ProjectKind::NonRepo,
    }
}

fn resolve_project_target(path: &Path) -> Result<ProjectOpenTarget, String> {
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

#[derive(Clone, Default)]
struct ClientHub {
    clients: Arc<Mutex<HashMap<ClientId, mpsc::UnboundedSender<String>>>>,
}

impl ClientHub {
    fn register(&self, client_id: ClientId) -> mpsc::UnboundedReceiver<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.clients
            .lock()
            .expect("client hub lock")
            .insert(client_id, tx);
        rx
    }

    fn unregister(&self, client_id: &str) {
        self.clients
            .lock()
            .expect("client hub lock")
            .remove(client_id);
    }

    fn dispatch(&self, events: Vec<OutboundEvent>) {
        let clients = self.clients.lock().expect("client hub lock");
        for outbound in events {
            let payload = serde_json::to_string(&outbound.event).expect("backend event json");
            match outbound.target {
                DispatchTarget::Broadcast => {
                    for sender in clients.values() {
                        let _ = sender.send(payload.clone());
                    }
                }
                DispatchTarget::Client(client_id) => {
                    if let Some(sender) = clients.get(&client_id) {
                        let _ = sender.send(payload);
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct ServerState {
    proxy: EventLoopProxy<UserEvent>,
    clients: ClientHub,
}

struct EmbeddedServer {
    url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl EmbeddedServer {
    fn start(
        runtime: &Runtime,
        proxy: EventLoopProxy<UserEvent>,
        clients: ClientHub,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(("127.0.0.1", 0)))?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let app = Router::new()
            .route("/", get(index_handler))
            .route("/healthz", get(health_handler))
            .route("/ws", get(websocket_handler))
            .with_state(ServerState { proxy, clients });

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded server error: {error}");
            }
        });

        Ok(Self {
            url: format!("http://127.0.0.1:{}/", addr.port()),
            shutdown_tx: Some(shutdown_tx),
        })
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn index_handler() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

async fn health_handler() -> &'static str {
    "ok"
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| client_session(socket, state))
}

async fn client_session(socket: WebSocket, state: ServerState) {
    let client_id = Uuid::new_v4().to_string();
    let mut outbound = state.clients.register(client_id.clone());
    let (mut sender, mut receiver) = socket.split();

    loop {
        tokio::select! {
            maybe_payload = outbound.recv() => {
                let Some(payload) = maybe_payload else {
                    break;
                };
                if sender.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            maybe_message = receiver.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<FrontendEvent>(text.as_ref()) {
                            Ok(event) => {
                                let _ = state.proxy.send_event(UserEvent::Frontend {
                                    client_id: client_id.clone(),
                                    event,
                                });
                            }
                            Err(error) => {
                                eprintln!("invalid frontend message: {error}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        eprintln!("websocket error: {error}");
                        break;
                    }
                }
            }
        }
    }

    state.clients.unregister(&client_id);
}

fn normalize_branch_name(branch_name: &str) -> String {
    if let Some(name) = branch_name.strip_prefix("refs/remotes/") {
        return name.strip_prefix("origin/").unwrap_or(name).to_string();
    }
    if let Some(name) = branch_name.strip_prefix("origin/") {
        return name.to_string();
    }
    branch_name.to_string()
}

fn branch_worktree_path(repo_path: &Path, branch_name: &str) -> Option<PathBuf> {
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

fn detect_wizard_docker_context_and_status(
    project_root: &Path,
) -> (
    Option<DockerWizardContext>,
    gwt_docker::ComposeServiceStatus,
) {
    let files = gwt_docker::detect_docker_files(project_root);
    let Some(compose_file) = docker_compose_file_for_launch(project_root, &files)
        .ok()
        .flatten()
    else {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    };

    let Ok(services) = gwt_docker::parse_compose_file(&compose_file) else {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    };
    if services.is_empty() {
        return (None, gwt_docker::ComposeServiceStatus::NotFound);
    }

    let suggested_service = docker_devcontainer_defaults(project_root, &files)
        .and_then(|defaults| defaults.service)
        .or_else(|| services.first().map(|service| service.name.clone()));
    let status = suggested_service
        .as_deref()
        .map(|service| {
            gwt_docker::compose_service_status(&compose_file, service)
                .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound)
        })
        .unwrap_or(gwt_docker::ComposeServiceStatus::NotFound);

    (
        Some(DockerWizardContext {
            services: services.into_iter().map(|service| service.name).collect(),
            suggested_service,
        }),
        status,
    )
}

fn resolve_launch_worktree(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    let Some(branch_name) = config.branch.clone() else {
        return Ok(());
    };
    if config.working_dir.is_some() {
        return Ok(());
    }

    let current_branch = current_git_branch(repo_path);
    if current_branch.is_err() && config.base_branch.is_none() {
        return Ok(());
    }
    if current_branch
        .as_ref()
        .is_ok_and(|current| current == &branch_name)
    {
        config.working_dir = Some(repo_path.to_path_buf());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            repo_path.display().to_string(),
        );
        return Ok(());
    }

    let main_repo_path =
        gwt_git::worktree::main_worktree_root(repo_path).map_err(|err| err.to_string())?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let worktrees = manager.list().map_err(|err| err.to_string())?;
    if let Some(existing_worktree) = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name.as_str()))
        .map(|worktree| worktree.path.clone())
    {
        config.working_dir = Some(existing_worktree.clone());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            existing_worktree.display().to_string(),
        );
        return Ok(());
    }

    let base_branch = config
        .base_branch
        .clone()
        .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string());
    let remote_base_ref = origin_remote_ref(&base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);

    manager
        .fetch_origin()
        .map_err(|err| format!("failed to fetch origin: {err}"))?;

    if !manager
        .remote_branch_exists(&remote_base_ref)
        .map_err(|err| format!("failed to verify remote base branch {remote_base_ref}: {err}"))?
    {
        return Err(format!(
            "remote base branch does not exist: {remote_base_ref}"
        ));
    }

    if !manager
        .remote_branch_exists(&remote_branch_ref)
        .map_err(|err| format!("failed to verify remote branch {remote_branch_ref}: {err}"))?
    {
        manager
            .create_remote_branch_from_base(&remote_base_ref, &branch_name)
            .map_err(|err| {
                format!(
                    "failed to create remote branch {remote_branch_ref} from {remote_base_ref}: {err}"
                )
            })?;
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to refresh origin refs after push: {err}"))?;
    }

    let preferred_worktree_path =
        gwt_git::worktree::sibling_worktree_path(&main_repo_path, &branch_name);
    let worktree_path = first_available_worktree_path(&preferred_worktree_path, &worktrees)
        .ok_or_else(|| {
            format!("failed to resolve available worktree path for branch {branch_name}")
        })?;
    if local_branch_exists(&main_repo_path, &branch_name)? {
        manager
            .create(&branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    } else {
        manager
            .create_from_remote(&remote_branch_ref, &branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    }

    config.working_dir = Some(worktree_path.clone());
    config.env_vars.insert(
        "GWT_PROJECT_ROOT".to_string(),
        worktree_path.display().to_string(),
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct DockerLaunchPlan {
    compose_file: PathBuf,
    service: String,
    container_cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerExecProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerPackageRunnerCandidate {
    executable: &'static str,
    base_args: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct DevContainerLaunchDefaults {
    service: Option<String>,
    workspace_folder: Option<String>,
    compose_file: Option<PathBuf>,
}

fn apply_docker_runtime_to_launch_config(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        return Ok(());
    }

    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    ensure_docker_launch_service_ready(&launch, config.docker_lifecycle_intent)?;
    ensure_docker_gwt_binary_setup(&worktree)?;
    maybe_inject_docker_sandbox_env(&launch, config)?;
    let runtime_program = resolve_docker_exec_program(&launch, config)?;

    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        launch.compose_file.display().to_string(),
        "exec".to_string(),
        "-w".to_string(),
        launch.container_cwd.clone(),
    ];
    args.extend(docker_compose_exec_env_args(&config.env_vars));
    args.push(launch.service.clone());
    args.push(runtime_program.executable);
    args.extend(runtime_program.args);

    config.command = docker_binary_for_launch();
    config.args = args;
    config
        .env_vars
        .insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    // Override GWT_BIN_PATH for Docker containers to use the mounted binary
    config.env_vars.insert(
        gwt_agent::session::GWT_BIN_PATH_ENV.to_string(),
        "/usr/local/bin/gwt".to_string(),
    );
    config.docker_service = Some(launch.service);
    Ok(())
}

fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
    if !gwt_docker::docker_available() {
        return Err("Docker is not installed or not available on PATH".to_string());
    }
    if !gwt_docker::compose_available() {
        return Err("docker compose is not available".to_string());
    }
    if !gwt_docker::daemon_running() {
        return Err("Docker daemon is not running".to_string());
    }
    Ok(())
}

fn ensure_docker_gwt_binary_setup(repo_path: &Path) -> Result<(), String> {
    use std::{fs, path::PathBuf};

    let home = if cfg!(windows) {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .map(PathBuf::from)
    .map_err(|_| "Could not determine home directory".to_string())?;

    let gwt_bin_dir = home.join(".gwt").join("bin");
    let gwt_linux = gwt_bin_dir.join("gwt-linux");

    // Check if Linux gwt binary exists
    if !gwt_linux.exists() {
        // TODO: Download Linux gwt binary from GitHub Release
        // For now, create a note for the user
        let override_path = repo_path.join("docker-compose.override.yml");
        if !override_path.exists() {
            eprintln!(
                "Note: Linux gwt binary not found at ~/.gwt/bin/gwt-linux\n\
                 This is required for Docker agent support.\n\
                 You can either:\n\
                 1. Download gwt-linux-x86_64 from GitHub Releases and place it at ~/.gwt/bin/gwt-linux\n\
                 2. Run 'gwt setup docker' to set up Docker integration automatically"
            );
        }
    }

    // Ensure docker-compose.override.yml exists with gwt volume mount
    let override_path = repo_path.join("docker-compose.override.yml");
    if !override_path.exists() {
        let override_content = "# Auto-generated docker-compose override for gwt binary mounting\n\
                                version: '3.8'\n\
                                services:\n\
                                  # Add your service name and uncomment the volumes section\n\
                                  # <service>:\n\
                                  #   volumes:\n\
                                  #     - ~/.gwt/bin/gwt-linux:/usr/local/bin/gwt:ro\n";
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to create docker-compose.override.yml: {err}\n\
                 Manually create {} with gwt volume mount",
                override_path.display()
            )
        })?;
    }

    Ok(())
}

fn maybe_inject_docker_sandbox_env(
    launch: &DockerLaunchPlan,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if cfg!(windows)
        || !matches!(config.agent_id, gwt_agent::AgentId::ClaudeCode)
        || !config.skip_permissions
    {
        return Ok(());
    }

    let is_root = gwt_docker::compose_service_user_is_root(&launch.compose_file, &launch.service)
        .map_err(|err| {
        format!(
            "Failed to determine Docker user for service '{}': {err}",
            launch.service
        )
    })?;
    if is_root {
        config
            .env_vars
            .insert("IS_SANDBOX".to_string(), "1".to_string());
    }
    Ok(())
}

fn docker_compose_exec_env_args(env_vars: &HashMap<String, String>) -> Vec<String> {
    let mut keys = env_vars.keys().collect::<Vec<_>>();
    keys.sort();

    let mut args = Vec::new();
    for key in keys {
        let key = key.trim();
        if key.is_empty() || !is_valid_docker_env_key(key) {
            continue;
        }
        let value = env_vars.get(key).map(String::as_str).unwrap_or_default();
        args.push("-e".to_string());
        args.push(format!("{key}={value}"));
    }
    args
}

fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn resolve_docker_exec_program(
    launch: &DockerLaunchPlan,
    config: &gwt_agent::LaunchConfig,
) -> Result<DockerExecProgram, String> {
    let Some(version_spec) = docker_package_version_spec(config) else {
        ensure_docker_launch_command_ready(launch, &config.command)?;
        return Ok(DockerExecProgram {
            executable: config.command.clone(),
            args: config.args.clone(),
        });
    };
    resolve_docker_package_runner(launch, config, &version_spec)
}

fn docker_package_version_spec(config: &gwt_agent::LaunchConfig) -> Option<String> {
    let package = config.agent_id.package_name()?;
    let version = config.tool_version.as_deref()?;
    if version == "installed" || version.is_empty() {
        return None;
    }
    Some(if version == "latest" {
        format!("{package}@latest")
    } else {
        format!("{package}@{version}")
    })
}

fn resolve_docker_package_runner(
    launch: &DockerLaunchPlan,
    config: &gwt_agent::LaunchConfig,
    version_spec: &str,
) -> Result<DockerExecProgram, String> {
    let agent_args = strip_docker_package_runner_args(&config.args, version_spec);
    let candidates = vec![
        DockerPackageRunnerCandidate {
            executable: "bunx",
            base_args: vec![version_spec.to_string()],
        },
        DockerPackageRunnerCandidate {
            executable: "npx",
            base_args: vec!["--yes".to_string(), version_spec.to_string()],
        },
    ];

    for candidate in candidates {
        let output = gwt_docker::compose_service_exec_capture(
            &launch.compose_file,
            &launch.service,
            Some(&launch.container_cwd),
            &candidate.probe_args(),
        )
        .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(candidate.into_exec_program(agent_args.clone()));
        }
    }

    Err(format!(
        "Selected Docker runtime cannot launch {version_spec} in service '{}'",
        launch.service
    ))
}

fn strip_docker_package_runner_args(args: &[String], version_spec: &str) -> Vec<String> {
    if args.first().is_some_and(|first| first == "--yes")
        && args.get(1).is_some_and(|arg| arg == version_spec)
    {
        return args[2..].to_vec();
    }
    if args.first().is_some_and(|arg| arg == version_spec) {
        return args[1..].to_vec();
    }
    args.to_vec()
}

fn ensure_docker_launch_command_ready(
    launch: &DockerLaunchPlan,
    command: &str,
) -> Result<(), String> {
    let available =
        gwt_docker::compose_service_has_command(&launch.compose_file, &launch.service, command)
            .map_err(|err| err.to_string())?;
    if available {
        Ok(())
    } else {
        Err(format!(
            "Command '{command}' is not available in Docker service '{}'",
            launch.service
        ))
    }
}

impl DockerPackageRunnerCandidate {
    fn probe_args(&self) -> Vec<String> {
        let mut args = vec![self.executable.to_string()];
        args.extend(self.base_args.clone());
        args.push("--version".to_string());
        args
    }

    fn into_exec_program(self, mut agent_args: Vec<String>) -> DockerExecProgram {
        let mut args = self.base_args;
        args.append(&mut agent_args);
        DockerExecProgram {
            executable: self.executable.to_string(),
            args,
        }
    }
}

fn ensure_docker_launch_service_ready(
    launch: &DockerLaunchPlan,
    intent: gwt_agent::DockerLifecycleIntent,
) -> Result<(), String> {
    let status = gwt_docker::compose_service_status(&launch.compose_file, &launch.service)
        .map_err(|err| err.to_string())?;
    match normalize_docker_launch_action(intent, status) {
        DockerLaunchServiceAction::Connect => Ok(()),
        DockerLaunchServiceAction::Start => {
            gwt_docker::compose_up(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())?;
            Ok(())
        }
        DockerLaunchServiceAction::Restart => {
            gwt_docker::compose_restart(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())
        }
        DockerLaunchServiceAction::Recreate => {
            gwt_docker::compose_up_force_recreate(&launch.compose_file, &launch.service)
                .map_err(|err| err.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerLaunchServiceAction {
    Connect,
    Start,
    Restart,
    Recreate,
}

fn normalize_docker_launch_action(
    intent: gwt_agent::DockerLifecycleIntent,
    status: gwt_docker::ComposeServiceStatus,
) -> DockerLaunchServiceAction {
    use gwt_agent::DockerLifecycleIntent;
    use gwt_docker::ComposeServiceStatus;

    match intent {
        DockerLifecycleIntent::Recreate => DockerLaunchServiceAction::Recreate,
        DockerLifecycleIntent::Restart if status == ComposeServiceStatus::Running => {
            DockerLaunchServiceAction::Restart
        }
        DockerLifecycleIntent::Connect
        | DockerLifecycleIntent::Start
        | DockerLifecycleIntent::Restart
        | DockerLifecycleIntent::CreateAndStart => match status {
            ComposeServiceStatus::Running => DockerLaunchServiceAction::Connect,
            ComposeServiceStatus::Stopped
            | ComposeServiceStatus::Exited
            | ComposeServiceStatus::NotFound => DockerLaunchServiceAction::Start,
        },
    }
}

fn resolve_docker_launch_plan(
    worktree: &Path,
    selected_service: Option<&str>,
) -> Result<DockerLaunchPlan, String> {
    let files = gwt_docker::detect_docker_files(worktree);
    let compose_file = docker_compose_file_for_launch(worktree, &files)?.ok_or_else(|| {
        "Docker launch requires a docker-compose.yml or devcontainer compose target".to_string()
    })?;
    let services = gwt_docker::parse_compose_file(&compose_file).map_err(|err| err.to_string())?;
    if services.is_empty() {
        return Err("Docker launch requires at least one compose service".to_string());
    }

    let devcontainer_defaults = docker_devcontainer_defaults(worktree, &files);
    let service_name = selected_service
        .map(str::to_string)
        .or_else(|| {
            devcontainer_defaults
                .as_ref()
                .and_then(|defaults| defaults.service.clone())
        })
        .or_else(|| {
            if services.len() == 1 {
                services.first().map(|service| service.name.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "Multiple Docker services detected; select a Docker service in Launch Agent Wizard"
                .to_string()
        })?;

    let service = services
        .iter()
        .find(|service| service.name == service_name)
        .ok_or_else(|| {
            format!("Selected Docker service was not found in compose file: {service_name}")
        })?;

    let container_cwd = devcontainer_defaults
        .as_ref()
        .and_then(|defaults| defaults.workspace_folder.clone())
        .or_else(|| service.working_dir.clone())
        .or_else(|| compose_workspace_mount_target(worktree, service))
        .ok_or_else(|| {
            format!(
                "Docker service {} is missing working_dir/workspaceFolder and no project-root volume mount was detected",
                service.name
            )
        })?;

    Ok(DockerLaunchPlan {
        compose_file,
        service: service.name.clone(),
        container_cwd,
    })
}

fn docker_binary_for_launch() -> String {
    std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

fn docker_compose_file_for_launch(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Result<Option<PathBuf>, String> {
    Ok(docker_devcontainer_defaults(project_root, files)
        .and_then(|defaults| defaults.compose_file)
        .or_else(|| files.compose_file.clone()))
}

fn docker_devcontainer_defaults(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Option<DevContainerLaunchDefaults> {
    let devcontainer_dir = files.devcontainer_dir.as_ref()?;
    let path = devcontainer_dir.join("devcontainer.json");
    if !path.is_file() {
        return None;
    }

    let config = gwt_docker::DevContainerConfig::load(&path).ok()?;
    let compose_file = config
        .docker_compose_file
        .as_ref()
        .and_then(|value| {
            value
                .to_vec()
                .into_iter()
                .map(|candidate| devcontainer_dir.join(candidate))
                .find(|path| path.is_file())
        })
        .or_else(|| files.compose_file.clone())
        .or_else(|| {
            let fallback = project_root.join("docker-compose.yml");
            fallback.is_file().then_some(fallback)
        });

    Some(DevContainerLaunchDefaults {
        service: config.service,
        workspace_folder: config.workspace_folder,
        compose_file,
    })
}

fn compose_workspace_mount_target(
    project_root: &Path,
    service: &gwt_docker::ComposeService,
) -> Option<String> {
    service
        .volumes
        .iter()
        .find(|mount| mount_source_matches_project_root(&mount.source, project_root))
        .map(|mount| mount.target.clone())
}

fn mount_source_matches_project_root(source: &str, project_root: &Path) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim_end_matches("/.");

    if matches!(normalized, "." | "$PWD" | "${PWD}") {
        return true;
    }

    let source_path = Path::new(normalized);
    source_path.is_absolute() && same_worktree_path(source_path, project_root)
}

fn first_available_worktree_path(
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

fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

fn worktree_path_is_occupied(path: &Path, worktrees: &[gwt_git::WorktreeInfo]) -> bool {
    worktrees
        .iter()
        .any(|worktree| same_worktree_path(&worktree.path, path))
}

fn same_worktree_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn origin_remote_ref(branch_name: &str) -> String {
    if let Some(ref_name) = branch_name.strip_prefix("refs/remotes/") {
        ref_name.to_string()
    } else if branch_name.starts_with("origin/") {
        branch_name.to_string()
    } else {
        format!("origin/{branch_name}")
    }
}

fn current_git_branch(repo_path: &Path) -> Result<String, String> {
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

fn local_branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
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

fn resolve_launch_spec_with_fallback(
    preset: WindowPreset,
    shell: &gwt::ShellProgram,
) -> Result<gwt::LaunchSpec, gwt::PresetResolveError> {
    let _ = shell;
    resolve_launch_spec(preset)
}

fn spawn_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env
}

fn geometry_to_pty_size(geometry: &WindowGeometry) -> (u16, u16) {
    let cols = ((geometry.width.max(420.0) - 26.0) / 8.4).floor() as u16;
    let rows = ((geometry.height.max(260.0) - 58.0) / 18.0).floor() as u16;
    (cols.max(20), rows.max(6))
}

fn run_cli(argv: &[String]) -> io::Result<()> {
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
        let mut env = match gwt::cli::DefaultCliEnv::new(&owner, &repo, repo_path) {
            Ok(env) => env,
            Err(error) => {
                eprintln!(
                    "gwt {}: {error}",
                    argv.get(1).map(String::as_str).unwrap_or("issue")
                );
                std::process::exit(1);
            }
        };
        std::process::exit(gwt::cli::dispatch(&mut env, argv));
    }

    let mut env = match gwt::cli::DefaultCliEnv::new_for_hooks() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("gwt hook: {error}");
            std::process::exit(1);
        }
    };
    std::process::exit(gwt::cli::dispatch(&mut env, argv));
}

fn resolve_repo_coordinates() -> Option<(String, String)> {
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

fn parse_github_remote_url(url: &str) -> Option<(String, String)> {
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

fn main() -> wry::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if gwt::cli::should_dispatch_cli(&argv) {
        if let Err(error) = run_cli(&argv) {
            eprintln!("gwt CLI dispatch failed: {error}");
            std::process::exit(1);
        }
    }

    let runtime = Runtime::new().expect("tokio runtime");
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    let menu_proxy = proxy.clone();
    #[cfg(target_os = "macos")]
    muda::MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_proxy.send_event(UserEvent::MenuEvent(event));
    }));
    #[cfg(not(target_os = "macos"))]
    let clients = ClientHub::default();
    #[cfg(target_os = "macos")]
    let clients = ClientHub::default();
    let mut app = AppRuntime::new(proxy.clone()).expect("app runtime");
    app.bootstrap();

    let mut server =
        EmbeddedServer::start(&runtime, proxy.clone(), clients.clone()).expect("embedded server");
    eprintln!("gwt browser URL: {}", server.url());

    let window = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 920.0))
        .build(&event_loop)
        .expect("window");
    #[cfg(target_os = "macos")]
    let native_menu = {
        let native_menu = gwt::MacosNativeMenu::new();
        native_menu.init_for_app();
        native_menu
    };

    let builder = WebViewBuilder::new().with_url(server.url());

    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        builder.build_gtk(vbox)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _ = &webview;
        let _ = &runtime;
        #[cfg(target_os = "macos")]
        let _ = &native_menu;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                server.shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Frontend { client_id, event }) => {
                let events = app.handle_frontend_event(client_id, event);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::RuntimeOutput { id, data }) => {
                let events = app.handle_runtime_output(id, data);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::RuntimeStatus { id, status, detail }) => {
                let events = app.handle_runtime_status(id, status, detail);
                clients.dispatch(events);
            }
            #[cfg(target_os = "macos")]
            Event::UserEvent(UserEvent::MenuEvent(event)) => {
                use gwt::NativeMenuCommand;
                if let Some(command) = gwt::native_menu_command_for_id(event.id.as_ref()) {
                    match command {
                        NativeMenuCommand::OpenProject => {
                            let events = app.open_project_dialog_events();
                            clients.dispatch(events);
                        }
                        NativeMenuCommand::ReloadWebView => {
                            if let Err(error) = webview.reload() {
                                eprintln!("webview reload failed: {error}");
                            }
                        }
                    }
                }
            }
            Event::LoopDestroyed => {
                server.shutdown();
            }
            _ => {}
        }
    });
}

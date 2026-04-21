use std::{
    collections::HashMap,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicU64, mpsc as std_mpsc, Arc, Mutex, RwLock},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::repo_browser::{preferred_issue_launch_branch, spawn_branch_load_async};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use gwt::{
    build_builtin_agent_options, cleanup_selected_branches, default_wizard_version_cache_path,
    detect_shell_program, list_branch_entries_with_active_sessions, list_directory_entries,
    load_knowledge_bridge, load_restored_workspace_state, load_session_state,
    migrate_legacy_workspace_state, refresh_managed_gwt_assets_for_worktree, resolve_launch_spec,
    save_session_state, save_workspace_state, workspace_state_path, BackendEvent,
    BranchEntriesPhase, BranchListEntry, DockerWizardContext, FrontendEvent, HookForwardTarget,
    KnowledgeKind, LaunchWizardCompletion, LaunchWizardContext, LaunchWizardHydration,
    LaunchWizardLaunchRequest, LaunchWizardState, LiveSessionEntry, RuntimeHookEvent,
    ShellLaunchConfig, WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState, APP_NAME,
};
use gwt_terminal::{Pane, PaneStatus, PtyHandle};
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

mod custom_agents_controller;
mod embedded_web;
mod repo_browser;

type ClientId = String;
const DOCKER_GWT_BIN_PATH: &str = "/usr/local/bin/gwt";
const DOCKER_GWTD_BIN_PATH: &str = "/usr/local/bin/gwtd";
const DOCKER_HOST_GWT_BIN_NAME: &str = "gwt-linux";
const DOCKER_HOST_GWTD_BIN_NAME: &str = "gwtd-linux";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerBundleMounts {
    host_gwt: PathBuf,
    host_gwtd: PathBuf,
}

/// Shared lock-free PTY writer registry used by the WebSocket fast-path.
///
/// The WS receiver task (tokio async) looks up the `Arc<PtyHandle>` by window
/// id and calls `write_input` directly, bypassing the tao event loop and the
/// surrounding `Mutex<Pane>` guard. This eliminates the two main contention
/// sources for intermittent key drops under heavy output bursts
/// (bugfix/input-key): (a) FIFO queue behind many `RuntimeOutput` events on
/// the single-threaded tao main loop, and (b) pane mutex held by the reader
/// thread while parsing vt100 chunks. Reads are hot (every keystroke), writes
/// are rare (pane create/destroy), so `RwLock` is the natural fit.
type PtyWriterRegistry = Arc<RwLock<HashMap<String, Arc<PtyHandle>>>>;

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
    LaunchProgress {
        window_id: String,
        message: String,
    },
    LaunchComplete {
        window_id: String,
        result: Result<AgentLaunchReady, String>,
    },
    ShellLaunchComplete {
        window_id: String,
        result: Result<ProcessLaunch, String>,
    },
    LaunchWizardHydrated {
        wizard_id: String,
        result: Result<LaunchWizardHydration, String>,
    },
    IssueLaunchWizardPrepared(IssueLaunchWizardPrepared),
    Dispatch(Vec<OutboundEvent>),
    UpdateAvailable(gwt_core::update::UpdateState),
    #[cfg(target_os = "macos")]
    MenuEvent(muda::MenuEvent),
}

#[derive(Clone)]
enum AppEventProxy {
    Real(EventLoopProxy<UserEvent>),
    #[cfg(test)]
    Stub(Arc<Mutex<Vec<UserEvent>>>),
}

impl AppEventProxy {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self::Real(proxy)
    }

    fn send(&self, event: UserEvent) {
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
    fn stub() -> (Self, Arc<Mutex<Vec<UserEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (Self::Stub(events.clone()), events)
    }
}

#[derive(Clone)]
enum BlockingTaskSpawner {
    Tokio(tokio::runtime::Handle),
    #[cfg(test)]
    Thread,
}

impl BlockingTaskSpawner {
    fn tokio(handle: tokio::runtime::Handle) -> Self {
        Self::Tokio(handle)
    }

    #[cfg(test)]
    fn thread() -> Self {
        Self::Thread
    }

    fn spawn<F>(&self, task: F)
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

struct WindowRuntime {
    pane: Arc<Mutex<Pane>>,
    /// Handle to the background reader thread that forwards PTY output.
    /// Taken and joined during `stop_window_runtime` so the reader releases
    /// its Arc clone of `pane` before the runtime is fully torn down.
    output_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct ProcessLaunch {
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct AgentLaunchReady {
    process_launch: ProcessLaunch,
    session_id: String,
    branch_name: String,
    display_name: String,
    worktree_path: PathBuf,
    agent_id: gwt_agent::AgentId,
    linked_issue_number: Option<u64>,
}

#[derive(Debug, Clone)]
struct ActiveAgentSession {
    window_id: String,
    session_id: String,
    agent_id: String,
    branch_name: String,
    display_name: String,
    worktree_path: PathBuf,
    tab_id: String,
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct IssueBranchLinkStore {
    #[serde(default)]
    branches: HashMap<String, u64>,
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

fn build_frontend_sync_events(
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
    wizard_id: String,
    wizard: LaunchWizardState,
}

#[derive(Debug, Clone)]
struct IssueLaunchWizardPrepared {
    client_id: ClientId,
    id: String,
    knowledge_kind: KnowledgeKind,
    tab_id: String,
    project_root: PathBuf,
    issue_number: u64,
    result: Result<String, String>,
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
    proxy: AppEventProxy,
    custom_agents: custom_agents_controller::CustomAgentsController,
    sessions_dir: PathBuf,
    launch_wizard: Option<LaunchWizardSession>,
    active_agent_sessions: HashMap<String, ActiveAgentSession>,
    hook_forward_target: Option<HookForwardTarget>,
    issue_link_cache_dir: PathBuf,
    /// Cached update state so late-connecting WebView clients get the toast.
    pending_update: Option<gwt_core::update::UpdateState>,
    /// Shared PTY writer registry published to the WebSocket fast-path.
    pty_writers: PtyWriterRegistry,
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
    fn new(
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

        let proxy = AppEventProxy::new(proxy);
        let custom_agents =
            custom_agents_controller::CustomAgentsController::new(proxy.clone(), blocking_tasks);

        let mut app = Self {
            tabs,
            active_tab_id,
            recent_projects: dedupe_recent_projects(persisted.recent_projects),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path,
            proxy,
            custom_agents,
            sessions_dir,
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            pending_update: None,
            pty_writers,
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

    fn set_hook_forward_target(&mut self, target: HookForwardTarget) {
        self.hook_forward_target = Some(target);
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
            custom_agents_event @ (FrontendEvent::ListCustomAgents
            | FrontendEvent::ListCustomAgentPresets
            | FrontendEvent::AddCustomAgentFromPreset { .. }
            | FrontendEvent::UpdateCustomAgent { .. }
            | FrontendEvent::DeleteCustomAgent { .. }
            | FrontendEvent::TestBackendConnection { .. }) => self
                .custom_agents
                .handle_event(client_id, custom_agents_event),
            FrontendEvent::ListProfiles {
                id,
                selected_profile,
            } => vec![OutboundEvent::reply(
                client_id,
                gwt::profiles_dispatch::list_event(id, selected_profile),
            )],
            FrontendEvent::SwitchProfile { id, profile_name } => {
                vec![OutboundEvent::broadcast(
                    gwt::profiles_dispatch::switch_event(id, profile_name),
                )]
            }
            FrontendEvent::AddProfile {
                id,
                name,
                description,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::add_profile_event(id, name, description),
            )],
            FrontendEvent::UpdateProfile {
                id,
                current_name,
                name,
                description,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::update_profile_event(id, current_name, name, description),
            )],
            FrontendEvent::DeleteProfile { id, profile_name } => {
                vec![OutboundEvent::broadcast(
                    gwt::profiles_dispatch::delete_profile_event(id, profile_name),
                )]
            }
            FrontendEvent::SetProfileEnvVar {
                id,
                profile_name,
                key,
                value,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::set_env_var_event(id, profile_name, key, value),
            )],
            FrontendEvent::UpdateProfileEnvVar {
                id,
                profile_name,
                current_key,
                key,
                value,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::update_env_var_event(
                    id,
                    profile_name,
                    current_key,
                    key,
                    value,
                ),
            )],
            FrontendEvent::DeleteProfileEnvVar {
                id,
                profile_name,
                key,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::delete_env_var_event(id, profile_name, key),
            )],
            FrontendEvent::AddDisabledEnv {
                id,
                profile_name,
                key,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::add_disabled_env_event(id, profile_name, key),
            )],
            FrontendEvent::UpdateDisabledEnv {
                id,
                profile_name,
                current_key,
                key,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::update_disabled_env_event(
                    id,
                    profile_name,
                    current_key,
                    key,
                ),
            )],
            FrontendEvent::DeleteDisabledEnv {
                id,
                profile_name,
                key,
            } => vec![OutboundEvent::broadcast(
                gwt::profiles_dispatch::delete_disabled_env_event(id, profile_name, key),
            )],
        }
    }

    fn frontend_sync_events(&self, client_id: &str) -> Vec<OutboundEvent> {
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

    fn create_window_events(
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

    fn focus_window_events(
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

    fn list_windows_event(&self) -> BackendEvent {
        let windows = self
            .active_tab_id
            .as_ref()
            .and_then(|tab_id| self.tab(tab_id))
            .map(|tab| workspace_view_for_tab(tab).windows)
            .unwrap_or_default();
        BackendEvent::WindowList { windows }
    }

    fn terminal_input_events(&mut self, id: &str, data: &str) -> Vec<OutboundEvent> {
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

    fn load_branches_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
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

    fn load_knowledge_bridge_events(
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

    fn run_branch_cleanup_events(
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
    fn open_launch_wizard(
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

    fn open_launch_wizard_for_branch(
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

    fn open_issue_launch_wizard_events(
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

    fn handle_launch_wizard_hydrated(
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

    fn handle_issue_launch_wizard_prepared(
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

    fn handle_launch_wizard_action(
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

    fn live_sessions_for_branch(&self, tab_id: &str, branch_name: &str) -> Vec<LiveSessionEntry> {
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

    fn active_session_branches_for_tab(&self, tab_id: &str) -> std::collections::HashSet<String> {
        self.active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.branch_name.clone())
            .collect()
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

    fn handle_launch_complete(
        &mut self,
        window_id: String,
        result: Result<AgentLaunchReady, String>,
    ) -> Vec<OutboundEvent> {
        match result {
            Ok(AgentLaunchReady {
                process_launch,
                session_id,
                branch_name,
                display_name,
                worktree_path,
                agent_id,
                linked_issue_number,
            }) => {
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
                    Ok(event) => {
                        self.active_agent_sessions.insert(
                            window_id.clone(),
                            ActiveAgentSession {
                                window_id: window_id.clone(),
                                session_id,
                                agent_id: agent_id.to_string(),
                                branch_name: branch_name.clone(),
                                display_name,
                                worktree_path: worktree_path.clone(),
                                tab_id: address.tab_id,
                            },
                        );
                        let linkage_result = match linked_issue_number {
                            Some(issue_number) => record_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &branch_name,
                                issue_number,
                                &self.issue_link_cache_dir,
                            ),
                            None => clear_issue_branch_link_with_cache_dir(
                                &worktree_path,
                                &branch_name,
                                &self.issue_link_cache_dir,
                            ),
                        };
                        if let Err(error) = linkage_result {
                            tracing::warn!(
                                worktree = %worktree_path.display(),
                                branch = %branch_name,
                                ?linked_issue_number,
                                error = %error,
                                "issue branch linkage update skipped after agent launch"
                            );
                        }
                        let _ = self.persist();
                        vec![
                            self.workspace_state_broadcast(),
                            OutboundEvent::broadcast(event),
                        ]
                    }
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

    fn handle_shell_launch_complete(
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

    fn spawn_agent_window(
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

    fn spawn_wizard_shell_window(
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

    fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        config: gwt_agent::LaunchConfig,
        hook_forward_target: Option<HookForwardTarget>,
    ) {
        let result = (|| {
            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Preparing worktree...".to_string(),
            });

            proxy.send(UserEvent::LaunchProgress {
                window_id: window_id.clone(),
                message: "Starting Docker service...".to_string(),
            });
            let proxy_for_refresh = proxy.clone();
            let prepared = gwt_agent::prepare_agent_launch(
                Path::new(&project_root),
                &sessions_dir,
                config,
                hook_forward_target.map(|target| gwt_agent::HookForwardEnv {
                    url: target.url,
                    token: target.token,
                }),
                |worktree_path| {
                    proxy_for_refresh.send(UserEvent::LaunchProgress {
                        window_id: window_id.clone(),
                        message: "Configuring workspace...".to_string(),
                    });
                    refresh_managed_gwt_assets_for_worktree(worktree_path)
                        .map_err(|error| error.to_string())
                },
            )?;

            if let Err(error) =
                gwt::index_worker::bootstrap_project_index_for_path(&prepared.worktree_path)
            {
                tracing::warn!(
                    worktree = %prepared.worktree_path.display(),
                    error = %error,
                    "project index bootstrap skipped during worktree prepare"
                );
            }

            if prepared.used_host_package_runner_fallback {
                proxy.send(UserEvent::LaunchProgress {
                    window_id: window_id.clone(),
                    message: "bunx unavailable, switching to npx...".to_string(),
                });
            }

            Ok(AgentLaunchReady {
                process_launch: ProcessLaunch {
                    command: prepared.process_launch.command,
                    args: prepared.process_launch.args,
                    env: prepared.process_launch.env,
                    cwd: prepared.process_launch.cwd,
                },
                session_id: prepared.session.id.clone(),
                branch_name: prepared.session.branch.clone(),
                display_name: prepared.session.display_name.clone(),
                worktree_path: prepared.worktree_path,
                agent_id: prepared.session.agent_id.clone(),
                linked_issue_number: prepared.session.linked_issue_number,
            })
        })();

        match result {
            Ok(launch) => {
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    result: Ok(launch),
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

    fn spawn_wizard_shell_window_async(
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

    fn register_pty_writer(&self, id: &str, pane: &Arc<Mutex<Pane>>) {
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

    fn deregister_pty_writer(&self, id: &str) {
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

    fn stop_window_runtime(&mut self, window_id: &str) {
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
    fn stop_all_runtimes(&mut self) {
        let ids: Vec<String> = self.runtimes.keys().cloned().collect();
        for id in ids {
            self.stop_window_runtime(&id);
        }
    }

    fn spawn_output_thread(&self, id: String, pane: Arc<Mutex<Pane>>) -> JoinHandle<()> {
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

    fn app_state_view(&self) -> gwt::AppStateView {
        app_state_view_from_parts(
            &self.tabs,
            self.active_tab_id.as_deref(),
            &self.recent_projects,
        )
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

fn should_auto_close_agent_window(
    active_agent_sessions: &HashMap<String, ActiveAgentSession>,
    window_id: &str,
    status: &WindowProcessStatus,
) -> bool {
    matches!(status, WindowProcessStatus::Exited) && active_agent_sessions.contains_key(window_id)
}

fn close_window_from_workspace(
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

fn should_auto_start_restored_window(window: &gwt::PersistedWindowState) -> bool {
    window.preset.requires_process()
        && matches!(
            window.status,
            WindowProcessStatus::Starting | WindowProcessStatus::Running
        )
}

fn current_app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn workspace_view_for_tab(tab: &ProjectTabRuntime) -> gwt::WorkspaceView {
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

fn app_state_view_from_parts(
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex, RwLock},
        time::Duration,
    };

    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
    use base64::Engine;
    use tempfile::tempdir;

    use gwt::{
        empty_workspace_state, AgentOption, ArrangeMode, BackendEvent, BranchCleanupInfo,
        BranchListEntry, BranchScope, CanvasViewport, FocusCycleDirection, KnowledgeKind,
        LaunchWizardAction, LaunchWizardContext, LaunchWizardState, PersistedWindowState,
        ProjectKind, QuickStartEntry, QuickStartLaunchMode, RuntimeHookEvent, RuntimeHookEventKind,
        ShellLaunchConfig, WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState,
    };
    use gwt_agent::{AgentId, AgentLaunchBuilder, DockerLifecycleIntent, LaunchRuntimeTarget};
    use gwt_core::update::UpdateState;
    use gwt_terminal::PaneStatus;

    use super::{
        app_state_view_from_parts, apply_host_package_runner_fallback_with_probe,
        broadcast_runtime_hook_event, build_frontend_sync_events, build_shell_process_launch,
        close_window_from_workspace, combined_window_id, current_git_branch,
        docker_bundle_mounts_for_home, docker_bundle_override_content, hook_forward_authorized,
        install_launch_gwt_bin_env_with_lookup, knowledge_kind_for_preset,
        record_issue_branch_link_with_cache_dir, resolve_project_target,
        should_auto_close_agent_window, should_auto_start_restored_window, ActiveAgentSession,
        AgentLaunchReady, AppEventProxy, AppRuntime, BlockingTaskSpawner, ClientHub,
        DispatchTarget, LaunchWizardSession, OutboundEvent, ProcessLaunch, ProjectTabRuntime,
        UserEvent, WindowAddress,
    };

    fn canvas_bounds() -> WindowGeometry {
        WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 1400.0,
            height: 900.0,
        }
    }

    fn init_git_repo(path: &Path) {
        fs::create_dir_all(path).expect("create repo dir");
        let init = Command::new("git")
            .args(["init", "-q"])
            .arg(path)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed");

        for args in [
            vec!["config", "user.name", "Codex Test"],
            vec!["config", "user.email", "codex@example.com"],
            vec!["commit", "--allow-empty", "-qm", "init"],
            vec!["branch", "feature/demo"],
        ] {
            let status = Command::new("git")
                .args(&args)
                .current_dir(path)
                .status()
                .expect("git command");
            assert!(status.success(), "git {:?} failed", args);
        }
    }

    fn init_git_clone_with_origin(path: &Path) -> PathBuf {
        let root = path.parent().expect("repo parent");
        let seed = root.join("seed");
        let origin = root.join("origin.git");

        fs::create_dir_all(&seed).expect("create seed dir");
        let status = Command::new("git")
            .args(["init", "-q", "-b", "develop"])
            .arg(&seed)
            .status()
            .expect("git init seed");
        assert!(status.success(), "git init seed failed");

        for args in [
            vec!["config", "user.name", "Codex Test"],
            vec!["config", "user.email", "codex@example.com"],
        ] {
            let status = Command::new("git")
                .args(&args)
                .current_dir(&seed)
                .status()
                .expect("git seed config");
            assert!(status.success(), "git {:?} failed", args);
        }

        fs::write(seed.join("README.md"), "seed\n").expect("write seed readme");
        for args in [vec!["add", "README.md"], vec!["commit", "-qm", "init"]] {
            let status = Command::new("git")
                .args(&args)
                .current_dir(&seed)
                .status()
                .expect("git seed commit");
            assert!(status.success(), "git {:?} failed", args);
        }

        let status = Command::new("git")
            .args(["clone", "--bare"])
            .arg(&seed)
            .arg(&origin)
            .status()
            .expect("git clone --bare");
        assert!(status.success(), "git clone --bare failed");

        let status = Command::new("git")
            .args(["clone"])
            .arg(&origin)
            .arg(path)
            .status()
            .expect("git clone repo");
        assert!(status.success(), "git clone repo failed");

        for args in [
            vec!["config", "user.name", "Codex Test"],
            vec!["config", "user.email", "codex@example.com"],
        ] {
            let status = Command::new("git")
                .args(&args)
                .current_dir(path)
                .status()
                .expect("git repo config");
            assert!(status.success(), "git {:?} failed", args);
        }

        origin
    }

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
    fn runtime_hook_event_broadcast_reaches_all_registered_clients() {
        let clients = ClientHub::default();
        let mut native = clients.register("native".to_string());
        let mut browser = clients.register("browser".to_string());

        broadcast_runtime_hook_event(
            &clients,
            RuntimeHookEvent {
                kind: RuntimeHookEventKind::RuntimeState,
                source_event: Some("PreToolUse".to_string()),
                gwt_session_id: Some("session-1".to_string()),
                agent_session_id: Some("agent-1".to_string()),
                project_root: Some("E:/gwt/test-repo".to_string()),
                branch: Some("feature/runtime".to_string()),
                status: Some("Running".to_string()),
                tool_name: Some("Bash".to_string()),
                message: None,
                occurred_at: "2026-04-20T00:00:00Z".to_string(),
            },
        );

        let native_payload = native.try_recv().expect("native payload");
        let browser_payload = browser.try_recv().expect("browser payload");
        assert_eq!(native_payload, browser_payload);
        assert!(native_payload.contains("\"kind\":\"runtime_hook_event\""));
        assert!(native_payload.contains("\"source_event\":\"PreToolUse\""));
    }

    fn drain_client_payloads(
        receiver: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> Vec<String> {
        let mut payloads = Vec::new();
        while let Ok(payload) = receiver.try_recv() {
            payloads.push(payload);
        }
        payloads
    }

    #[test]
    fn frontend_sync_events_reply_only_to_connecting_client() {
        let tabs = vec![sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        )];
        let workspace = app_state_view_from_parts(&tabs, Some("tab-1"), &[]);
        let snapshot = b"hello from terminal".to_vec();
        let expected_snapshot =
            base64::engine::general_purpose::STANDARD.encode(snapshot.as_slice());

        let events = build_frontend_sync_events(
            "browser-1",
            workspace,
            vec![(
                "tab-1::shell-1".to_string(),
                WindowProcessStatus::Ready,
                "Shell ready".to_string(),
            )],
            vec![("tab-1::shell-1".to_string(), snapshot)],
            None,
            Some(UpdateState::UpToDate { checked_at: None }),
        );

        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|event| {
            matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "browser-1")
        }));
        assert!(matches!(
            &events[0].event,
            gwt::BackendEvent::WorkspaceState { .. }
        ));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            gwt::BackendEvent::TerminalStatus { id, status, detail }
                if id == "tab-1::shell-1"
                    && *status == WindowProcessStatus::Ready
                    && detail.as_deref() == Some("Shell ready")
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            gwt::BackendEvent::TerminalSnapshot { id, data_base64 }
                if id == "tab-1::shell-1" && data_base64 == &expected_snapshot
        )));
        assert!(events.iter().any(|event| matches!(
            &event.event,
            gwt::BackendEvent::UpdateState(UpdateState::UpToDate { checked_at: None })
        )));
    }

    #[test]
    fn client_hub_dispatch_keeps_frontend_sync_events_client_scoped() {
        let clients = ClientHub::default();
        let mut primary = clients.register("primary".to_string());
        let mut secondary = clients.register("secondary".to_string());
        let tabs = vec![sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        )];
        let workspace = app_state_view_from_parts(&tabs, Some("tab-1"), &[]);
        let mut events =
            build_frontend_sync_events("primary", workspace, Vec::new(), Vec::new(), None, None);
        events.push(OutboundEvent::broadcast(
            gwt::BackendEvent::ProjectOpenError {
                message: "shared".to_string(),
            },
        ));

        clients.dispatch(events);

        let primary_payloads = drain_client_payloads(&mut primary);
        let secondary_payloads = drain_client_payloads(&mut secondary);

        assert_eq!(primary_payloads.len(), 2);
        assert_eq!(secondary_payloads.len(), 1);
        assert!(primary_payloads
            .iter()
            .any(|payload| payload.contains("\"kind\":\"workspace_state\"")));
        assert!(primary_payloads
            .iter()
            .any(|payload| payload.contains("\"kind\":\"project_open_error\"")));
        assert!(secondary_payloads
            .iter()
            .all(|payload| payload.contains("\"kind\":\"project_open_error\"")));
        assert!(!secondary_payloads
            .iter()
            .any(|payload| payload.contains("\"kind\":\"workspace_state\"")));
    }

    #[test]
    fn hook_forward_authorized_accepts_matching_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );

        assert!(hook_forward_authorized(&headers, "secret-token"));
        assert!(!hook_forward_authorized(&headers, "other-token"));
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

    fn sample_project_tab_with_window(
        tab_id: &str,
        raw_window_id: &str,
        preset: WindowPreset,
        status: WindowProcessStatus,
    ) -> ProjectTabRuntime {
        let mut persisted = empty_workspace_state();
        let mut window = sample_window(preset, status);
        window.id = raw_window_id.to_string();
        persisted.windows.push(window);
        persisted.next_z_index = 2;
        ProjectTabRuntime {
            id: tab_id.to_string(),
            title: "Repo".to_string(),
            project_root: PathBuf::from("E:/gwt/test-repo"),
            kind: gwt::ProjectKind::Git,
            workspace: WorkspaceState::from_persisted(persisted),
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
        sample_runtime_with_events(temp_root, tabs, active_tab_id).0
    }

    fn sample_runtime_with_events(
        temp_root: &Path,
        tabs: Vec<ProjectTabRuntime>,
        active_tab_id: Option<&str>,
    ) -> (AppRuntime, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, events) = AppEventProxy::stub();
        let sessions_dir = temp_root.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let mut runtime = AppRuntime {
            tabs,
            active_tab_id: active_tab_id.map(str::to_owned),
            recent_projects: Vec::new(),
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            window_lookup: HashMap::new(),
            session_state_path: temp_root.join("session-state.json"),
            custom_agents: super::custom_agents_controller::CustomAgentsController::new(
                proxy.clone(),
                BlockingTaskSpawner::thread(),
            ),
            proxy,
            sessions_dir,
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: temp_root.join("cache"),
            pending_update: None,
            pty_writers: Arc::new(RwLock::new(HashMap::new())),
        };
        runtime.rebuild_window_lookup();
        (runtime, events)
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
    fn issue_branch_linkage_store_records_and_merges_launch_links() {
        let temp = tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let repo = temp.path().join("repo");
        let _origin = init_git_clone_with_origin(&repo);

        record_issue_branch_link_with_cache_dir(&repo, "feature/old", 7, &cache_root)
            .expect("record old link");
        record_issue_branch_link_with_cache_dir(&repo, "feature/demo", 42, &cache_root)
            .expect("record launch link");

        let repo_hash = gwt::index_worker::detect_repo_hash(&repo).expect("repo hash");
        let path = cache_root
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()));
        let raw = fs::read_to_string(path).expect("read issue links");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse issue links");

        assert_eq!(value["branches"]["feature/old"], 7);
        assert_eq!(value["branches"]["feature/demo"], 42);
    }

    fn issue_branch_link_path(repo_path: &Path, cache_root: &Path) -> PathBuf {
        let repo_hash = gwt::index_worker::detect_repo_hash(repo_path).expect("repo hash");
        cache_root
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()))
    }

    fn successful_test_process_launch(cwd: &Path) -> ProcessLaunch {
        #[cfg(windows)]
        let (command, args) = (
            "cmd".to_string(),
            vec!["/C".to_string(), "exit".to_string(), "0".to_string()],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "sh".to_string(),
            vec!["-c".to_string(), "exit 0".to_string()],
        );

        ProcessLaunch {
            command,
            args,
            env: HashMap::new(),
            cwd: Some(cwd.to_path_buf()),
        }
    }

    fn missing_test_process_launch(cwd: &Path) -> ProcessLaunch {
        ProcessLaunch {
            command: "__gwt_missing_command_for_issue_link_test__".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: Some(cwd.to_path_buf()),
        }
    }

    fn agent_launch_ready(
        process_launch: ProcessLaunch,
        session_id: &str,
        branch_name: &str,
        worktree_path: PathBuf,
        linked_issue_number: Option<u64>,
    ) -> AgentLaunchReady {
        AgentLaunchReady {
            process_launch,
            session_id: session_id.to_string(),
            branch_name: branch_name.to_string(),
            display_name: "Codex".to_string(),
            worktree_path,
            agent_id: AgentId::Codex,
            linked_issue_number,
        }
    }

    #[test]
    fn agent_launch_completion_records_issue_link_only_after_spawn_success() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let _origin = init_git_clone_with_origin(&repo);
        let cache_root = temp.path().join("cache");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Claude, WindowPreset::Claude],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.issue_link_cache_dir = cache_root.clone();
        let link_path = issue_branch_link_path(&repo, &cache_root);
        let failing_window_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 0);
        let successful_window_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 1);

        let failed = runtime.handle_launch_complete(
            failing_window_id,
            Ok(agent_launch_ready(
                missing_test_process_launch(&repo),
                "session-failed",
                "feature/demo",
                repo.clone(),
                Some(42),
            )),
        );
        assert!(matches!(
            failed[0].event,
            BackendEvent::TerminalStatus { ref status, .. }
                if *status == WindowProcessStatus::Error
        ));
        assert!(
            !link_path.exists(),
            "failed process spawn must not persist issue linkage"
        );

        let launched = runtime.handle_launch_complete(
            successful_window_id,
            Ok(agent_launch_ready(
                successful_test_process_launch(&repo),
                "session-ok",
                "feature/demo",
                repo.clone(),
                Some(42),
            )),
        );
        assert!(launched.iter().any(|event| matches!(
            event.event,
            BackendEvent::TerminalStatus {
                status: WindowProcessStatus::Running,
                ..
            }
        )));
        let raw = fs::read_to_string(link_path).expect("read issue link store");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse issue link store");
        assert_eq!(value["branches"]["feature/demo"], 42);
    }

    #[test]
    fn agent_launch_completion_clears_stale_issue_link_after_unlinked_success() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let _origin = init_git_clone_with_origin(&repo);
        let cache_root = temp.path().join("cache");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::Git,
            &[WindowPreset::Claude],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        runtime.issue_link_cache_dir = cache_root.clone();
        record_issue_branch_link_with_cache_dir(&repo, "feature/demo", 42, &cache_root)
            .expect("seed stale issue link");
        let link_path = issue_branch_link_path(&repo, &cache_root);
        let window_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 0);

        let events = runtime.handle_launch_complete(
            window_id,
            Ok(agent_launch_ready(
                successful_test_process_launch(&repo),
                "session-unlinked",
                "feature/demo",
                repo.clone(),
                None,
            )),
        );

        assert!(events.iter().any(|event| matches!(
            event.event,
            BackendEvent::TerminalStatus {
                status: WindowProcessStatus::Running,
                ..
            }
        )));
        let raw = fs::read_to_string(link_path).expect("read issue link store");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("parse issue link store");
        let branches = value["branches"].as_object().expect("branches object");
        assert!(
            !branches.contains_key("feature/demo"),
            "unlinked launch success must clear stale issue linkage"
        );
    }

    fn sample_branch_entry(name: &str) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: BranchScope::Local,
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: BranchCleanupInfo::default(),
        }
    }

    fn sample_wizard_agent_options() -> Vec<AgentOption> {
        vec![AgentOption {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            available: true,
            installed_version: Some("0.110.0".to_string()),
            versions: vec!["0.110.0".to_string()],
        }]
    }

    fn sample_wizard_quick_start_entry(live_window_id: Option<&str>) -> QuickStartEntry {
        QuickStartEntry {
            session_id: "gwt-session-1".to_string(),
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("gpt-5.4".to_string()),
            reasoning: Some("high".to_string()),
            version: Some("0.110.0".to_string()),
            resume_session_id: Some("resume-1".to_string()),
            live_window_id: live_window_id.map(str::to_string),
            skip_permissions: true,
            codex_fast_mode: true,
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
        }
    }

    fn sample_focus_launch_wizard_session(
        tab_id: &str,
        project_root: &Path,
        live_window_id: Option<&str>,
    ) -> LaunchWizardSession {
        LaunchWizardSession {
            tab_id: tab_id.to_string(),
            wizard_id: "wizard-focus".to_string(),
            wizard: LaunchWizardState::open_with(
                LaunchWizardContext {
                    selected_branch: sample_branch_entry("feature/demo"),
                    normalized_branch_name: "feature/demo".to_string(),
                    worktree_path: Some(project_root.to_path_buf()),
                    quick_start_root: project_root.to_path_buf(),
                    live_sessions: Vec::new(),
                    docker_context: None,
                    docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                    linked_issue_number: Some(42),
                },
                sample_wizard_agent_options(),
                vec![sample_wizard_quick_start_entry(live_window_id)],
            ),
        }
    }

    fn window_id_for_preset(
        runtime: &AppRuntime,
        tab_id: &str,
        preset: WindowPreset,
        ordinal: usize,
    ) -> String {
        let tab = runtime.tab(tab_id).expect("tab");
        let raw_id = tab
            .workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == preset)
            .nth(ordinal)
            .map(|window| window.id.clone())
            .expect("window");
        combined_window_id(tab_id, &raw_id)
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

    #[test]
    fn frontend_sync_events_replay_status_wizard_and_pending_update() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[WindowPreset::FileTree],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let window_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::FileTree, 0);
        runtime
            .window_details
            .insert(window_id.clone(), "Paused".to_string());
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        runtime.pending_update = Some(gwt_core::update::UpdateState::UpToDate { checked_at: None });

        let events = runtime.frontend_sync_events("client-1");

        assert!(matches!(
            events.first(),
            Some(event)
                if matches!(&event.target, DispatchTarget::Client(client_id) if client_id == "client-1")
                    && matches!(event.event, BackendEvent::WorkspaceState { .. })
        ));
        assert!(events.iter().any(|event| {
            matches!(
                &event.event,
                BackendEvent::TerminalStatus { id, status, detail }
                    if id == &window_id
                        && *status == WindowProcessStatus::Ready
                        && detail.as_deref() == Some("Paused")
            )
        }));
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
    fn open_project_path_reuses_existing_tab_and_adds_new_tab() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let other = temp.path().join("other");
        let scratch = temp.path().join("scratch");
        init_git_repo(&repo);
        fs::create_dir_all(&other).expect("create other");
        fs::create_dir_all(&scratch).expect("create scratch");
        let tabs = vec![
            sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::Git,
                &[WindowPreset::Branches],
            ),
            sample_project_tab("tab-2", "Other", other.clone(), ProjectKind::NonRepo, &[]),
        ];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-2"));
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-2", &other));

        let existing = runtime
            .open_project_path(repo.clone())
            .expect("open existing project");
        let new_active = runtime.active_tab_id.clone().expect("active tab");

        assert!(existing);
        assert_eq!(new_active, "tab-1");
        assert!(runtime.launch_wizard.is_none());
        assert_eq!(runtime.recent_projects[0].path, repo);

        let added = runtime
            .open_project_path(scratch.clone())
            .expect("open new project");

        assert!(!added);
        assert_eq!(runtime.tabs.len(), 3);
        assert_eq!(runtime.recent_projects[0].path, scratch);
        assert!(runtime
            .active_tab_id
            .as_deref()
            .is_some_and(|tab_id| tab_id != "tab-1" && tab_id != "tab-2"));
    }

    #[test]
    fn select_and_close_project_tabs_emit_workspace_and_wizard_updates() {
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
                other.clone(),
                ProjectKind::NonRepo,
                &[WindowPreset::FileTree],
            ),
        ];
        let mut runtime = sample_runtime(temp.path(), tabs, Some("tab-1"));
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));

        let select_events = runtime.select_project_tab_events("tab-2");

        assert_eq!(select_events.len(), 2);
        assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-2"));
        assert!(runtime.launch_wizard.is_none());
        assert!(matches!(
            select_events[1].event,
            BackendEvent::LaunchWizardState { wizard: None }
        ));

        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-2", &other));
        let close_events = runtime.close_project_tab_events("tab-2");

        assert_eq!(close_events.len(), 2);
        assert_eq!(runtime.tabs.len(), 1);
        assert_eq!(runtime.active_tab_id.as_deref(), Some("tab-1"));
        assert!(runtime.launch_wizard.is_none());
        assert!(runtime
            .window_lookup
            .keys()
            .all(|id| id.starts_with("tab-1::")));
    }

    #[test]
    fn window_management_events_cover_canvas_operations() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab("tab-1", "Repo", repo, ProjectKind::NonRepo, &[]);
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let bounds = canvas_bounds();

        assert_eq!(
            runtime
                .create_window_events(WindowPreset::Branches, bounds.clone())
                .len(),
            1
        );
        assert_eq!(
            runtime
                .create_window_events(WindowPreset::FileTree, bounds.clone())
                .len(),
            1
        );

        let branches_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Branches, 0);
        let file_tree_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::FileTree, 0);
        let file_tree_raw_id = runtime
            .window_lookup
            .get(&file_tree_id)
            .expect("file tree lookup")
            .raw_id
            .clone();

        assert_eq!(
            runtime.window_status(&branches_id),
            Some(WindowProcessStatus::Ready)
        );
        assert_eq!(
            runtime
                .focus_window_events(&branches_id, Some(bounds.clone()))
                .len(),
            1
        );
        assert_eq!(
            runtime
                .cycle_focus_events(FocusCycleDirection::Forward, bounds.clone())
                .len(),
            1
        );
        assert_eq!(
            runtime
                .update_viewport_events(CanvasViewport {
                    x: 10.0,
                    y: 20.0,
                    zoom: 1.2,
                })
                .len(),
            1
        );
        assert_eq!(
            runtime
                .arrange_windows_events(ArrangeMode::Tile, bounds.clone())
                .len(),
            1
        );
        assert_eq!(
            runtime
                .maximize_window_events(&file_tree_id, bounds.clone())
                .len(),
            1
        );
        assert!(
            runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .window(&file_tree_raw_id)
                .expect("window")
                .maximized
        );
        assert_eq!(runtime.minimize_window_events(&file_tree_id).len(), 1);
        assert!(
            runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .window(&file_tree_raw_id)
                .expect("window")
                .minimized
        );
        assert_eq!(runtime.restore_window_events(&file_tree_id).len(), 1);
        assert!(
            !runtime
                .tab("tab-1")
                .expect("tab")
                .workspace
                .window(&file_tree_raw_id)
                .expect("window")
                .minimized
        );

        let geometry = WindowGeometry {
            x: 30.0,
            y: 40.0,
            width: 500.0,
            height: 320.0,
        };
        assert_eq!(
            runtime
                .update_window_geometry_events(&file_tree_id, geometry.clone(), 10, 1)
                .len(),
            1
        );
        let updated_window = runtime
            .tab("tab-1")
            .expect("tab")
            .workspace
            .window(&file_tree_raw_id)
            .expect("window");
        assert_eq!(updated_window.geometry, geometry);

        match runtime.list_windows_event() {
            BackendEvent::WindowList { windows } => assert_eq!(windows.len(), 2),
            other => panic!("expected window list, got {other:?}"),
        }

        assert_eq!(runtime.close_window_events(&file_tree_id).len(), 1);
        assert!(!runtime.window_lookup.contains_key(&file_tree_id));
    }

    #[test]
    fn loaders_and_wizard_entrypoints_cover_success_and_error_paths() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        fs::create_dir_all(repo.join("src")).expect("create src");
        fs::write(repo.join("README.md"), "hello").expect("write readme");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo,
            ProjectKind::Git,
            &[
                WindowPreset::Branches,
                WindowPreset::FileTree,
                WindowPreset::Issue,
            ],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let branches_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Branches, 0);
        let file_tree_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::FileTree, 0);

        assert!(matches!(
            runtime.load_file_tree_event("missing", ""),
            BackendEvent::FileTreeError { ref message, .. } if message == "Window not found"
        ));
        assert!(matches!(
            runtime.load_file_tree_event(&branches_id, ""),
            BackendEvent::FileTreeError { ref message, .. } if message == "Window is not a file tree"
        ));
        assert!(matches!(
            runtime.load_file_tree_event(&file_tree_id, ""),
            BackendEvent::FileTreeEntries { ref entries, .. } if !entries.is_empty()
        ));

        let missing_branches = runtime.load_branches_events("client-1", "missing");
        assert_eq!(missing_branches.len(), 1);
        assert!(matches!(
            missing_branches[0].event,
            BackendEvent::BranchError { ref message, .. } if message == "Window not found"
        ));
        let wrong_window_branches = runtime.load_branches_events("client-1", &file_tree_id);
        assert_eq!(wrong_window_branches.len(), 1);
        assert!(matches!(
            wrong_window_branches[0].event,
            BackendEvent::BranchError { ref message, .. } if message == "Window is not a branches list"
        ));
        assert!(runtime
            .load_branches_events("client-1", &branches_id)
            .is_empty());

        let knowledge_missing = runtime.load_knowledge_bridge_events(
            "client-1",
            "missing",
            KnowledgeKind::Issue,
            None,
            false,
        );
        assert_eq!(knowledge_missing.len(), 1);
        assert!(matches!(
            knowledge_missing[0].event,
            BackendEvent::KnowledgeError { ref message, .. } if message == "Window not found"
        ));

        let knowledge_wrong = runtime.load_knowledge_bridge_events(
            "client-1",
            &branches_id,
            KnowledgeKind::Issue,
            None,
            false,
        );
        assert_eq!(knowledge_wrong.len(), 1);
        assert!(matches!(
            knowledge_wrong[0].event,
            BackendEvent::KnowledgeError { ref message, .. }
                if message == "Window is not a knowledge bridge"
        ));

        let cleanup_missing = runtime.run_branch_cleanup_events("client-1", "missing", &[], false);
        assert_eq!(cleanup_missing.len(), 1);
        assert!(matches!(
            cleanup_missing[0].event,
            BackendEvent::BranchError { ref message, .. } if message == "Window not found"
        ));

        let cleanup_wrong =
            runtime.run_branch_cleanup_events("client-1", &file_tree_id, &[], false);
        assert_eq!(cleanup_wrong.len(), 1);
        assert!(matches!(
            cleanup_wrong[0].event,
            BackendEvent::BranchError { ref message, .. }
                if message == "Window is not a branches list"
        ));

        let wizard_missing = runtime.open_launch_wizard("missing", "feature/demo", None);
        assert_eq!(wizard_missing.len(), 1);
        assert!(matches!(
            wizard_missing[0].event,
            BackendEvent::BranchError { ref message, .. } if message == "Window not found"
        ));

        let wizard_wrong = runtime.open_launch_wizard(&file_tree_id, "feature/demo", None);
        assert_eq!(wizard_wrong.len(), 1);
        assert!(matches!(
            wizard_wrong[0].event,
            BackendEvent::BranchError { ref message, .. }
                if message == "Window is not a branches list"
        ));

        let issue_missing = runtime.open_issue_launch_wizard_events("client-1", "missing", 7);
        assert_eq!(issue_missing.len(), 1);
        assert!(matches!(
            issue_missing[0].event,
            BackendEvent::KnowledgeError { ref message, .. } if message == "Window not found"
        ));

        let issue_wrong = runtime.open_issue_launch_wizard_events("client-1", &file_tree_id, 7);
        assert_eq!(issue_wrong.len(), 1);
        assert!(matches!(
            issue_wrong[0].event,
            BackendEvent::KnowledgeError { ref message, .. }
                if message == "Window is not a knowledge bridge"
        ));
    }

    #[test]
    fn runtime_status_helpers_cover_sessions_auto_close_and_launch_errors() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let tab = sample_project_tab(
            "tab-1",
            "Repo",
            repo.clone(),
            ProjectKind::NonRepo,
            &[
                WindowPreset::Claude,
                WindowPreset::Claude,
                WindowPreset::Shell,
            ],
        );
        let mut runtime = sample_runtime(temp.path(), vec![tab], Some("tab-1"));
        let claude_one_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 0);
        let claude_two_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 1);
        let shell_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Shell, 0);
        runtime.active_agent_sessions.insert(
            claude_one_id.clone(),
            ActiveAgentSession {
                display_name: "Beta".to_string(),
                branch_name: "feature/demo".to_string(),
                ..sample_active_agent_session("tab-1", &claude_one_id)
            },
        );
        runtime.active_agent_sessions.insert(
            claude_two_id.clone(),
            ActiveAgentSession {
                display_name: "Alpha".to_string(),
                branch_name: "feature/demo".to_string(),
                session_id: "session-2".to_string(),
                ..sample_active_agent_session("tab-1", &claude_two_id)
            },
        );

        let live_sessions = runtime.live_sessions_for_branch("tab-1", "feature/demo");
        assert_eq!(live_sessions.len(), 2);
        assert_eq!(live_sessions[0].name, "Alpha");
        assert_eq!(live_sessions[1].name, "Beta");
        assert!(runtime
            .active_session_branches_for_tab("tab-1")
            .contains("feature/demo"));

        assert!(runtime
            .handle_runtime_output("missing".to_string(), b"noop".to_vec())
            .is_empty());
        let output_events = runtime.handle_runtime_output(shell_id.clone(), b"hello".to_vec());
        assert!(matches!(
            output_events[0].event,
            BackendEvent::TerminalOutput { ref id, ref data_base64 }
                if id == &shell_id && data_base64 == "aGVsbG8="
        ));

        let error_events = runtime.handle_runtime_status(
            claude_one_id.clone(),
            WindowProcessStatus::Error,
            Some("boom".to_string()),
        );
        assert_eq!(error_events.len(), 2);
        assert!(!runtime.active_agent_sessions.contains_key(&claude_one_id));
        assert_eq!(
            runtime
                .window_details
                .get(&claude_one_id)
                .map(String::as_str),
            Some("boom")
        );
        assert!(matches!(
            error_events[1].event,
            BackendEvent::TerminalStatus { ref status, ref detail, .. }
                if *status == WindowProcessStatus::Error
                    && detail.as_deref() == Some("boom")
        ));

        let close_events = runtime.handle_runtime_status(
            claude_two_id.clone(),
            WindowProcessStatus::Exited,
            Some("Process exited".to_string()),
        );
        assert_eq!(close_events.len(), 1);
        assert!(!runtime.active_agent_sessions.contains_key(&claude_two_id));
        assert!(!runtime.window_lookup.contains_key(&claude_two_id));

        let failed_launch = runtime.handle_launch_complete(
            "tab-1::missing".to_string(),
            Err("launch failed".to_string()),
        );
        assert!(matches!(
            failed_launch[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("launch failed")
        ));

        let missing_window_launch = runtime.handle_launch_complete(
            "tab-1::missing".to_string(),
            Ok(agent_launch_ready(
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-3",
                "feature/demo",
                repo.clone(),
                None,
            )),
        );
        assert!(matches!(
            missing_window_launch[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Window not found")
        ));

        let shell_launch = runtime.handle_shell_launch_complete(
            "tab-1::missing".to_string(),
            Ok(ProcessLaunch {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            }),
        );
        assert!(matches!(
            shell_launch[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Window not found")
        ));
    }

    #[test]
    fn app_runtime_window_helpers_cover_lookup_status_and_seeded_details() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut runtime = sample_runtime(
            temp.path(),
            vec![sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::NonRepo,
                &[WindowPreset::Claude],
            )],
            Some("tab-1"),
        );
        let window_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 0);
        let raw_id = runtime
            .window_lookup
            .get(&window_id)
            .expect("window lookup")
            .raw_id
            .clone();
        runtime.set_window_status("tab-1", &raw_id, WindowProcessStatus::Exited);
        runtime.seed_restored_window_details();

        assert_eq!(
            runtime.window_status(&window_id),
            Some(WindowProcessStatus::Exited)
        );
        assert!(runtime
            .window_details
            .get(&window_id)
            .is_some_and(|detail| detail.contains("Restored window is paused")));

        runtime.window_lookup.clear();
        runtime.register_window("tab-1", &raw_id);
        assert!(runtime.window_lookup.contains_key(&window_id));
        runtime.window_lookup.clear();
        runtime.rebuild_window_lookup();
        assert!(runtime.window_lookup.contains_key(&window_id));

        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        assert!(runtime.clear_launch_wizard().is_some());
        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        assert!(!runtime.set_active_tab("tab-1".to_string()));
    }

    #[test]
    fn async_main_helpers_emit_proxy_events_without_gui_runtime() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_git_repo(&repo);
        let default_branch = current_git_branch(&repo).expect("current branch");
        let status = Command::new("git")
            .args(["checkout", "-qb", "feature/prune-me"])
            .current_dir(&repo)
            .status()
            .expect("create branch");
        assert!(status.success(), "create branch failed");
        let status = Command::new("git")
            .args(["checkout", default_branch.as_str()])
            .current_dir(&repo)
            .status()
            .expect("checkout default branch");
        assert!(status.success(), "checkout default branch failed");

        let (mut runtime, events) = sample_runtime_with_events(
            temp.path(),
            vec![sample_project_tab(
                "tab-1",
                "Repo",
                repo,
                ProjectKind::Git,
                &[WindowPreset::Branches, WindowPreset::Issue],
            )],
            Some("tab-1"),
        );
        let branches_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Branches, 0);
        let issue_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Issue, 0);

        let cleanup_events = runtime.run_branch_cleanup_events(
            "client-1",
            &branches_id,
            &[String::from("feature/prune-me")],
            false,
        );
        assert!(cleanup_events.is_empty());
        wait_for_recorded_event("branch cleanup dispatch", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| matches!(
                            outbound.event,
                            BackendEvent::BranchCleanupResult { .. }
                        ))
                )
            })
        });

        let wizard_events = runtime.open_launch_wizard(&branches_id, "feature/demo", Some(42));
        assert_eq!(wizard_events.len(), 1);
        assert!(matches!(
            wizard_events[0].event,
            BackendEvent::LaunchWizardState { wizard: Some(_) }
        ));
        wait_for_recorded_event("launch wizard hydration", &events, |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardHydrated { .. }))
        });

        let issue_events = runtime.open_issue_launch_wizard_events("client-1", &issue_id, 42);
        assert!(issue_events.is_empty());
        wait_for_recorded_event("issue launch preparation", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::IssueLaunchWizardPrepared(prepared)
                        if prepared.id == issue_id
                            && prepared.client_id == "client-1"
                            && prepared.issue_number == 42
                            && prepared.result.is_ok()
                )
            })
        });
    }

    #[test]
    fn frontend_event_dispatch_routes_canvas_knowledge_and_async_paths() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let scratch = temp.path().join("scratch");
        init_git_repo(&repo);
        fs::create_dir_all(repo.join("src")).expect("create src");
        fs::create_dir_all(&scratch).expect("create scratch");
        fs::write(repo.join("README.md"), "hello").expect("write readme");

        let (mut runtime, events) = sample_runtime_with_events(
            temp.path(),
            vec![sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::Git,
                &[
                    WindowPreset::Branches,
                    WindowPreset::FileTree,
                    WindowPreset::Issue,
                ],
            )],
            Some("tab-1"),
        );
        let bounds = canvas_bounds();
        let branches_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Branches, 0);
        let file_tree_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::FileTree, 0);
        let issue_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Issue, 0);

        assert!(!runtime
            .handle_frontend_event("client-1".to_string(), gwt::FrontendEvent::FrontendReady)
            .is_empty());
        assert!(!runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::ReopenRecentProject {
                    path: scratch.display().to_string(),
                },
            )
            .is_empty());
        assert!(!runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::SelectProjectTab {
                    tab_id: "tab-1".to_string(),
                },
            )
            .is_empty());
        assert!(runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::CloseProjectTab {
                    tab_id: "missing".to_string(),
                },
            )
            .is_empty());

        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::CreateWindow {
                        preset: WindowPreset::Settings,
                        bounds: bounds.clone(),
                    },
                )
                .len(),
            1
        );
        let settings_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Settings, 0);

        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::FocusWindow {
                        id: branches_id.clone(),
                        bounds: Some(bounds.clone()),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::CycleFocus {
                        direction: FocusCycleDirection::Forward,
                        bounds: bounds.clone(),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::UpdateViewport {
                        viewport: CanvasViewport {
                            x: 5.0,
                            y: 10.0,
                            zoom: 1.1,
                        },
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::ArrangeWindows {
                        mode: ArrangeMode::Tile,
                        bounds: bounds.clone(),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::MaximizeWindow {
                        id: file_tree_id.clone(),
                        bounds: bounds.clone(),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::MinimizeWindow {
                        id: file_tree_id.clone(),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::RestoreWindow {
                        id: file_tree_id.clone(),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event("client-1".to_string(), gwt::FrontendEvent::ListWindows,)
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::UpdateWindowGeometry {
                        id: file_tree_id.clone(),
                        geometry: WindowGeometry {
                            x: 20.0,
                            y: 30.0,
                            width: 480.0,
                            height: 300.0,
                        },
                        cols: 80,
                        rows: 24,
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::CloseWindow { id: settings_id },
                )
                .len(),
            1
        );
        assert!(runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::TerminalInput {
                    id: "missing".to_string(),
                    data: "noop".to_string(),
                },
            )
            .is_empty());
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::LoadFileTree {
                        id: file_tree_id.clone(),
                        path: Some("src".to_string()),
                    },
                )
                .len(),
            1
        );
        assert_eq!(
            runtime
                .handle_frontend_event(
                    "client-1".to_string(),
                    gwt::FrontendEvent::LoadBranches {
                        id: branches_id.clone(),
                    },
                )
                .len(),
            0
        );
        assert!(!runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::LoadKnowledgeBridge {
                    id: issue_id.clone(),
                    knowledge_kind: KnowledgeKind::Issue,
                    selected_number: None,
                    refresh: false,
                },
            )
            .is_empty());
        assert!(!runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::SelectKnowledgeBridgeEntry {
                    id: issue_id.clone(),
                    knowledge_kind: KnowledgeKind::Issue,
                    number: 42,
                },
            )
            .is_empty());

        let cleanup_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            gwt::FrontendEvent::RunBranchCleanup {
                id: branches_id.clone(),
                branches: vec!["feature/missing".to_string()],
                delete_remote: false,
            },
        );
        assert!(cleanup_events.is_empty());
        wait_for_recorded_event("branch cleanup dispatch", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::Dispatch(dispatched)
                        if dispatched.iter().any(|outbound| matches!(
                            outbound.event,
                            BackendEvent::BranchCleanupResult { .. }
                        ))
                )
            })
        });

        assert!(runtime
            .handle_frontend_event(
                "client-1".to_string(),
                gwt::FrontendEvent::LaunchWizardAction {
                    action: LaunchWizardAction::Cancel,
                    bounds: None,
                },
            )
            .is_empty());

        let wizard_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            gwt::FrontendEvent::OpenLaunchWizard {
                id: branches_id,
                branch_name: "feature/demo".to_string(),
                linked_issue_number: Some(42),
            },
        );
        assert_eq!(wizard_events.len(), 1);
        wait_for_recorded_event("launch wizard hydration", &events, |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardHydrated { .. }))
        });

        let issue_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            gwt::FrontendEvent::OpenIssueLaunchWizard {
                id: issue_id.clone(),
                issue_number: 42,
            },
        );
        assert!(issue_events.is_empty());
        wait_for_recorded_event("issue launch preparation", &events, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    UserEvent::IssueLaunchWizardPrepared(prepared)
                        if prepared.id == issue_id
                            && prepared.client_id == "client-1"
                            && prepared.issue_number == 42
                )
            })
        });
    }

    #[test]
    fn test_backend_connection_replies_through_async_dispatch() {
        let temp = tempdir().expect("tempdir");
        let (mut runtime, events) = sample_runtime_with_events(temp.path(), Vec::new(), None);

        let immediate_events = runtime.handle_frontend_event(
            "client-1".to_string(),
            gwt::FrontendEvent::TestBackendConnection {
                base_url: "ws://not-http".to_string(),
                api_key: "secret".to_string(),
            },
        );

        assert!(
            immediate_events.is_empty(),
            "blocking probe must not reply on the frontend event loop"
        );
        wait_for_recorded_event("backend connection dispatch", &events, |events| {
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
                                BackendEvent::CustomAgentError {
                                    code: gwt::CustomAgentErrorCode::Probe,
                                    ..
                                }
                            )
                        })
                )
            })
        });
    }

    #[test]
    fn custom_agents_controller_dispatches_preset_list_reply() {
        let (proxy, _events) = AppEventProxy::stub();
        let controller = super::custom_agents_controller::CustomAgentsController::new(
            proxy,
            BlockingTaskSpawner::thread(),
        );

        let outbound = controller.handle_event(
            "client-1".to_string(),
            gwt::FrontendEvent::ListCustomAgentPresets,
        );

        assert_eq!(outbound.len(), 1);
        match &outbound[0].target {
            DispatchTarget::Client(client_id) => assert_eq!(client_id, "client-1"),
            other => panic!("expected client reply, got {other:?}"),
        }
        match &outbound[0].event {
            BackendEvent::CustomAgentPresetList { presets } => assert!(!presets.is_empty()),
            other => panic!("expected CustomAgentPresetList, got {other:?}"),
        }
    }

    #[test]
    fn wizard_handler_helpers_cover_hydration_preparation_focus_and_error_paths() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let (mut runtime, events) = sample_runtime_with_events(
            temp.path(),
            vec![sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::NonRepo,
                &[WindowPreset::Issue, WindowPreset::Claude],
            )],
            Some("tab-1"),
        );
        let claude_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Claude, 0);

        assert!(runtime
            .handle_launch_wizard_hydrated("wizard-1".to_string(), Err("missing".to_string()))
            .is_empty());

        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        assert!(runtime
            .handle_launch_wizard_hydrated("other".to_string(), Err("skip".to_string()))
            .is_empty());

        let hydration_error = runtime.handle_launch_wizard_hydrated(
            "wizard-1".to_string(),
            Err("hydrate failed".to_string()),
        );
        assert_eq!(hydration_error.len(), 1);
        assert_eq!(
            runtime
                .launch_wizard
                .as_ref()
                .unwrap()
                .wizard
                .hydration_error
                .as_deref(),
            Some("hydrate failed")
        );

        let hydration_ok = runtime.handle_launch_wizard_hydrated(
            "wizard-1".to_string(),
            Ok(gwt::LaunchWizardHydration {
                selected_branch: Some(sample_branch_entry("feature/demo")),
                normalized_branch_name: "feature/demo".to_string(),
                worktree_path: Some(repo.clone()),
                quick_start_root: repo.clone(),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
                agent_options: sample_wizard_agent_options(),
                quick_start_entries: vec![sample_wizard_quick_start_entry(None)],
            }),
        );
        assert_eq!(hydration_ok.len(), 1);
        assert!(!runtime.launch_wizard.as_ref().unwrap().wizard.is_hydrating);

        let missing_tab =
            runtime.handle_issue_launch_wizard_prepared(super::IssueLaunchWizardPrepared {
                client_id: "client-1".to_string(),
                id: "issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                tab_id: "missing".to_string(),
                project_root: repo.clone(),
                issue_number: 7,
                result: Ok("feature/demo".to_string()),
            });
        assert!(matches!(
            missing_tab[0].event,
            BackendEvent::KnowledgeError { ref message, .. }
                if message == "Project tab not found"
        ));

        let prepared_error =
            runtime.handle_issue_launch_wizard_prepared(super::IssueLaunchWizardPrepared {
                client_id: "client-1".to_string(),
                id: "issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                tab_id: "tab-1".to_string(),
                project_root: repo.clone(),
                issue_number: 7,
                result: Err("No local branch is available for launch".to_string()),
            });
        assert!(matches!(
            prepared_error[0].event,
            BackendEvent::KnowledgeError { ref message, .. }
                if message == "No local branch is available for launch"
        ));

        let prepared_ok =
            runtime.handle_issue_launch_wizard_prepared(super::IssueLaunchWizardPrepared {
                client_id: "client-1".to_string(),
                id: "issue-1".to_string(),
                knowledge_kind: KnowledgeKind::Issue,
                tab_id: "tab-1".to_string(),
                project_root: repo.clone(),
                issue_number: 7,
                result: Ok("feature/demo".to_string()),
            });
        assert_eq!(prepared_ok.len(), 1);
        assert!(matches!(
            prepared_ok[0].event,
            BackendEvent::LaunchWizardState { wizard: Some(_) }
        ));
        wait_for_recorded_event("prepared launch hydration", &events, |events| {
            events
                .iter()
                .any(|event| matches!(event, UserEvent::LaunchWizardHydrated { .. }))
        });

        runtime.launch_wizard = None;
        assert!(runtime
            .handle_launch_wizard_action(LaunchWizardAction::Cancel, None)
            .is_empty());

        runtime.launch_wizard = Some(sample_focus_launch_wizard_session(
            "tab-1",
            &repo,
            Some("missing"),
        ));
        let missing_focus = runtime.handle_launch_wizard_action(
            LaunchWizardAction::ApplyQuickStart {
                index: 0,
                mode: QuickStartLaunchMode::Resume,
            },
            None,
        );
        assert!(!missing_focus.is_empty());

        runtime.launch_wizard = Some(sample_focus_launch_wizard_session(
            "tab-1",
            &repo,
            Some(&claude_id),
        ));
        let focus_events = runtime.handle_launch_wizard_action(
            LaunchWizardAction::ApplyQuickStart {
                index: 0,
                mode: QuickStartLaunchMode::Resume,
            },
            None,
        );
        assert!(focus_events.len() >= 2);
        assert!(runtime.launch_wizard.is_none());

        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        let cancel_events = runtime.handle_launch_wizard_action(LaunchWizardAction::Cancel, None);
        assert_eq!(cancel_events.len(), 1);
        assert!(matches!(
            cancel_events[0].event,
            BackendEvent::LaunchWizardState { wizard: None }
        ));

        runtime.launch_wizard = Some(sample_launch_wizard_session("tab-1", &repo));
        let update_events = runtime.handle_launch_wizard_action(
            LaunchWizardAction::SetLinkedIssue { issue_number: 99 },
            None,
        );
        assert_eq!(update_events.len(), 1);
        assert_eq!(
            runtime
                .launch_wizard
                .as_ref()
                .unwrap()
                .wizard
                .linked_issue_number,
            Some(99)
        );
    }

    #[test]
    fn launch_completion_and_project_target_error_paths_are_reported() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let mut runtime = sample_runtime(
            temp.path(),
            vec![sample_project_tab(
                "tab-1",
                "Repo",
                repo.clone(),
                ProjectKind::NonRepo,
                &[WindowPreset::Claude, WindowPreset::Shell],
            )],
            Some("tab-1"),
        );
        let shell_id = window_id_for_preset(&runtime, "tab-1", WindowPreset::Shell, 0);
        runtime
            .window_details
            .insert(shell_id.clone(), "old detail".to_string());

        let status_events =
            runtime.handle_runtime_status(shell_id.clone(), WindowProcessStatus::Error, None);
        assert_eq!(status_events.len(), 2);
        assert!(!runtime.window_details.contains_key(&shell_id));
        assert!(matches!(
            status_events[1].event,
            BackendEvent::TerminalStatus { ref detail, .. } if detail.is_none()
        ));

        let project_missing_id = "tab-1::ghost-project".to_string();
        runtime.window_lookup.insert(
            project_missing_id.clone(),
            WindowAddress {
                tab_id: "missing".to_string(),
                raw_id: "ghost".to_string(),
            },
        );
        let project_missing = runtime.handle_launch_complete(
            project_missing_id.clone(),
            Ok(agent_launch_ready(
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-1",
                "feature/demo",
                repo.clone(),
                None,
            )),
        );
        assert!(matches!(
            project_missing[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Project tab not found")
        ));

        let raw_missing_id = "tab-1::ghost-window".to_string();
        runtime.window_lookup.insert(
            raw_missing_id.clone(),
            WindowAddress {
                tab_id: "tab-1".to_string(),
                raw_id: "ghost".to_string(),
            },
        );
        let raw_missing = runtime.handle_launch_complete(
            raw_missing_id.clone(),
            Ok(agent_launch_ready(
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-2",
                "feature/demo",
                repo.clone(),
                None,
            )),
        );
        assert!(matches!(
            raw_missing[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Window not found")
        ));

        let shell_project_missing = runtime.handle_shell_launch_complete(
            project_missing_id,
            Ok(ProcessLaunch {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            }),
        );
        assert!(matches!(
            shell_project_missing[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Project tab not found")
        ));

        let shell_raw_missing = runtime.handle_shell_launch_complete(
            raw_missing_id,
            Ok(ProcessLaunch {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            }),
        );
        assert!(matches!(
            shell_raw_missing[0].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Window not found")
        ));

        let file = temp.path().join("not-a-dir.txt");
        fs::write(&file, "hello").expect("write file");
        let file_err = resolve_project_target(&file).expect_err("file target must fail");
        assert!(file_err.contains("selected project is not a directory"));

        let missing_dir = temp.path().join("missing");
        let missing_err = resolve_project_target(&missing_dir).expect_err("missing path must fail");
        assert!(missing_err.contains("failed to open project"));

        let bare = temp.path().join("bare.git");
        let status = Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .status()
            .expect("git init --bare");
        assert!(status.success(), "git init --bare failed");
        let target = resolve_project_target(&bare).expect("bare repo target");
        assert_eq!(target.kind, ProjectKind::Bare);
        assert_eq!(target.project_root, dunce::canonicalize(&bare).unwrap());
    }

    #[test]
    fn exited_active_agent_window_is_marked_for_auto_close() {
        let window_id = "tab-1::claude-1";
        let sessions = HashMap::from([(
            window_id.to_string(),
            sample_active_agent_session("tab-1", window_id),
        )]);

        assert!(should_auto_close_agent_window(
            &sessions,
            window_id,
            &WindowProcessStatus::Exited,
        ));
        assert!(!should_auto_close_agent_window(
            &sessions,
            window_id,
            &WindowProcessStatus::Error,
        ));
    }

    #[test]
    fn non_agent_window_is_not_marked_for_auto_close() {
        assert!(!should_auto_close_agent_window(
            &HashMap::new(),
            "tab-1::shell-1",
            &WindowProcessStatus::Exited,
        ));
    }

    #[test]
    fn failed_completed_pane_status_is_not_auto_close_eligible() {
        let status = match PaneStatus::Completed(1) {
            PaneStatus::Completed(0) => WindowProcessStatus::Exited,
            PaneStatus::Completed(_) | PaneStatus::Error(_) => WindowProcessStatus::Error,
            PaneStatus::Running => WindowProcessStatus::Exited,
        };

        let window_id = "tab-1::claude-1";
        let sessions = HashMap::from([(
            window_id.to_string(),
            sample_active_agent_session("tab-1", window_id),
        )]);

        assert_eq!(status, WindowProcessStatus::Error);
        assert!(!should_auto_close_agent_window(
            &sessions, window_id, &status
        ));
    }

    #[test]
    fn close_window_from_workspace_removes_window_lookup_and_details() {
        let tab_id = "tab-1";
        let raw_window_id = "claude-1";
        let window_id = combined_window_id(tab_id, raw_window_id);
        let mut tabs = vec![sample_project_tab_with_window(
            tab_id,
            raw_window_id,
            WindowPreset::Claude,
            WindowProcessStatus::Exited,
        )];
        let mut window_lookup = HashMap::from([(
            window_id.clone(),
            WindowAddress {
                tab_id: tab_id.to_string(),
                raw_id: raw_window_id.to_string(),
            },
        )]);
        let mut window_details = HashMap::from([(window_id.clone(), "Process exited".to_string())]);

        assert!(close_window_from_workspace(
            &mut tabs,
            &mut window_lookup,
            &mut window_details,
            &window_id,
        ));
        assert!(tabs[0].workspace.window(raw_window_id).is_none());
        assert!(!window_lookup.contains_key(&window_id));
        assert!(!window_details.contains_key(&window_id));
    }

    #[test]
    fn app_state_view_includes_current_app_version() {
        let tabs = vec![sample_project_tab_with_window(
            "tab-1",
            "shell-1",
            WindowPreset::Shell,
            WindowProcessStatus::Ready,
        )];
        let view = app_state_view_from_parts(&tabs, Some("tab-1"), &[]);

        assert_eq!(view.app_version, env!("CARGO_PKG_VERSION"));
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
    fn sample_versioned_launch_config() -> gwt_agent::LaunchConfig {
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .working_dir("E:/gwt/develop")
            .version("latest")
            .build();
        config.command = "bunx".to_string();
        config.args = vec![
            "@anthropic-ai/claude-code@latest".to_string(),
            "--print".to_string(),
        ];
        config.env_vars = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        config.working_dir = Some(PathBuf::from("E:/gwt/develop"));
        config.runtime_target = LaunchRuntimeTarget::Host;
        config.docker_lifecycle_intent = DockerLifecycleIntent::Connect;
        config
    }

    #[test]
    fn host_package_runner_fallback_switches_bunx_to_npx_when_probe_fails() {
        let mut config = sample_versioned_launch_config();

        let changed = apply_host_package_runner_fallback_with_probe(
            &mut config,
            "npx".to_string(),
            |command, args, _env, cwd| {
                assert_eq!(command, "bunx");
                assert_eq!(
                    args,
                    vec![
                        "@anthropic-ai/claude-code@latest".to_string(),
                        "--version".to_string(),
                    ]
                );
                assert_eq!(cwd, Some(PathBuf::from("E:/gwt/develop")));
                false
            },
        );

        assert!(changed, "expected bunx failure to switch to npx");
        assert_eq!(config.command, "npx");
        assert_eq!(
            config.args,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ]
        );
    }

    #[test]
    fn host_package_runner_fallback_keeps_bunx_when_probe_succeeds() {
        let mut config = sample_versioned_launch_config();
        let original_command = config.command.clone();
        let original_args = config.args.clone();

        let changed = apply_host_package_runner_fallback_with_probe(
            &mut config,
            "npx".to_string(),
            |_command, _args, _env, _cwd| true,
        );

        assert!(!changed, "successful bunx probe should keep bunx");
        assert_eq!(config.command, original_command);
        assert_eq!(config.args, original_args);
    }

    #[test]
    fn host_package_runner_fallback_ignores_direct_installed_command() {
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .working_dir("E:/gwt/develop")
            .version("installed")
            .build();
        let original_command = config.command.clone();
        let original_args = config.args.clone();

        let changed = apply_host_package_runner_fallback_with_probe(
            &mut config,
            "npx".to_string(),
            |_command, _args, _env, _cwd| {
                panic!("installed command should not probe bunx");
            },
        );

        assert!(!changed);
        assert_eq!(config.command, original_command);
        assert_eq!(config.args, original_args);
    }

    #[test]
    fn build_shell_process_launch_for_host_uses_worktree_env() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        fs::create_dir_all(&worktree).expect("create worktree");
        let mut config = ShellLaunchConfig {
            working_dir: Some(worktree.clone()),
            branch: Some("feature/gui".to_string()),
            base_branch: None,
            display_name: "Shell".to_string(),
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            env_vars: HashMap::from([("EXTRA_FLAG".to_string(), "1".to_string())]),
        };

        let launch = build_shell_process_launch(&worktree, &mut config).expect("shell launch");

        assert!(!launch.command.is_empty());
        assert_eq!(launch.cwd.as_deref(), Some(worktree.as_path()));
        assert_eq!(launch.env.get("EXTRA_FLAG").map(String::as_str), Some("1"));
        assert_eq!(
            launch.env.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some(worktree.display().to_string().as_str())
        );
        assert_eq!(
            config.env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some(worktree.display().to_string().as_str())
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_prefers_public_gwt_binary_for_host_sessions() {
        let current_exe = PathBuf::from(
            r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
        );
        let stable = PathBuf::from(r"C:\Users\Example\.bun\bin\gwt.exe");
        let mut env = HashMap::new();

        install_launch_gwt_bin_env_with_lookup(
            &mut env,
            LaunchRuntimeTarget::Host,
            &current_exe,
            |command| {
                assert_eq!(command, "gwt");
                Some(stable.clone())
            },
        )
        .expect("install GWT_BIN_PATH");

        assert_eq!(
            env.get(gwt_agent::GWT_BIN_PATH_ENV).map(String::as_str),
            Some(stable.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn docker_bundle_override_content_mounts_front_door_and_daemon() {
        let home = PathBuf::from("/home/example");
        let bundle = docker_bundle_mounts_for_home(&home);
        let content = docker_bundle_override_content("app", &bundle);

        assert!(content.contains("/home/example/.gwt/bin/gwt-linux:/usr/local/bin/gwt:ro"));
        assert!(content.contains("/home/example/.gwt/bin/gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!content.contains("gwtd-linux:/usr/local/bin/gwt:ro"));
    }

    #[test]
    fn issue_and_spec_presets_route_to_knowledge_bridge_kind() {
        assert_eq!(
            knowledge_kind_for_preset(WindowPreset::Issue),
            Some(KnowledgeKind::Issue)
        );
        assert_eq!(
            knowledge_kind_for_preset(WindowPreset::Spec),
            Some(KnowledgeKind::Spec)
        );
        assert_eq!(
            knowledge_kind_for_preset(WindowPreset::Pr),
            Some(KnowledgeKind::Pr)
        );
        assert_eq!(knowledge_kind_for_preset(WindowPreset::Branches), None);
    }
    #[test]
    fn preferred_issue_launch_branch_prefers_develop_then_head_then_first_local() {
        let entries = vec![
            BranchListEntry {
                name: "feature/demo".to_string(),
                scope: BranchScope::Local,
                is_head: true,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
                cleanup_ready: true,
                cleanup: BranchCleanupInfo::default(),
            },
            BranchListEntry {
                name: "develop".to_string(),
                scope: BranchScope::Local,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
                cleanup_ready: true,
                cleanup: BranchCleanupInfo::default(),
            },
        ];
        assert_eq!(
            super::preferred_issue_launch_branch(&entries),
            Some("develop".to_string())
        );

        let head_only = vec![BranchListEntry {
            name: "feature/demo".to_string(),
            scope: BranchScope::Local,
            is_head: true,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: BranchCleanupInfo::default(),
        }];
        assert_eq!(
            super::preferred_issue_launch_branch(&head_only),
            Some("feature/demo".to_string())
        );
    }

    #[test]
    fn normalize_active_tab_id_prefers_existing_selection_or_first_tab() {
        let tabs = vec![
            sample_project_tab_with_window(
                "tab-1",
                "shell-1",
                WindowPreset::Shell,
                WindowProcessStatus::Ready,
            ),
            sample_project_tab_with_window(
                "tab-2",
                "claude-1",
                WindowPreset::Claude,
                WindowProcessStatus::Running,
            ),
        ];

        assert_eq!(
            super::normalize_active_tab_id(&tabs, None),
            Some("tab-1".to_string())
        );
        assert_eq!(
            super::normalize_active_tab_id(&tabs, Some("tab-2".to_string())),
            Some("tab-2".to_string())
        );
        assert_eq!(
            super::normalize_active_tab_id(&tabs, Some("missing".to_string())),
            Some("tab-1".to_string())
        );
        assert_eq!(super::normalize_active_tab_id(&[], None), None);
    }

    #[test]
    fn recent_project_and_path_helpers_dedupe_and_fallback() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("repo");
        fs::create_dir_all(&project).expect("project dir");
        let project_dot = project.join(".");
        let entries = vec![
            gwt::RecentProjectEntry {
                path: project.clone(),
                title: "repo".to_string(),
                kind: gwt::ProjectKind::Git,
            },
            gwt::RecentProjectEntry {
                path: project_dot.clone(),
                title: "repo-dot".to_string(),
                kind: gwt::ProjectKind::Git,
            },
        ];

        let deduped = super::dedupe_recent_projects(entries);
        assert_eq!(deduped.len(), 1);
        assert!(super::same_worktree_path(&project, &project_dot));

        let fallback = super::fallback_project_target(project.clone());
        assert_eq!(fallback.project_root, project);
        assert_eq!(fallback.kind, gwt::ProjectKind::NonRepo);
        assert_eq!(fallback.title, "repo");
    }

    #[test]
    fn client_hub_dispatches_broadcast_and_targeted_messages() {
        let hub = super::ClientHub::default();
        let mut client_one = hub.register("client-1".to_string());
        let mut client_two = hub.register("client-2".to_string());

        hub.dispatch(vec![
            super::OutboundEvent::broadcast(gwt::BackendEvent::ProjectOpenError {
                message: "broadcast".to_string(),
            }),
            super::OutboundEvent::reply(
                "client-2",
                gwt::BackendEvent::ProjectOpenError {
                    message: "targeted".to_string(),
                },
            ),
        ]);

        let first = client_one.try_recv().expect("broadcast for client one");
        assert!(first.contains("broadcast"));
        assert!(matches!(
            client_one.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let second = client_two.try_recv().expect("broadcast for client two");
        let third = client_two.try_recv().expect("targeted for client two");
        assert!(second.contains("broadcast"));
        assert!(third.contains("targeted"));

        hub.unregister("client-1");
        hub.dispatch(vec![super::OutboundEvent::broadcast(
            gwt::BackendEvent::ProjectOpenError {
                message: "after-unregister".to_string(),
            },
        )]);
        assert!(matches!(
            client_one.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
                | Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(client_two
            .try_recv()
            .expect("client two should still receive messages")
            .contains("after-unregister"));
    }

    #[test]
    fn branch_package_runner_and_env_helpers_cover_common_cases() {
        assert_eq!(
            super::normalize_branch_name("refs/remotes/origin/feature/gui"),
            "feature/gui"
        );
        assert_eq!(super::normalize_branch_name("origin/develop"), "develop");
        assert_eq!(super::origin_remote_ref("develop"), "origin/develop");
        assert_eq!(
            super::origin_remote_ref("refs/remotes/origin/feature/gui"),
            "origin/feature/gui"
        );

        let branch = super::synthetic_branch_entry("feature/gui");
        assert_eq!(branch.name, "feature/gui");
        assert_eq!(branch.scope, BranchScope::Local);
        assert!(!branch.is_head);

        let config = sample_versioned_launch_config();
        assert_eq!(
            super::package_runner_version_spec(&config),
            Some("@anthropic-ai/claude-code@latest".to_string())
        );
        assert_eq!(
            super::strip_package_runner_args(
                &[
                    "--yes".to_string(),
                    "@anthropic-ai/claude-code@latest".to_string(),
                    "--print".to_string(),
                ],
                "@anthropic-ai/claude-code@latest",
            ),
            vec!["--print".to_string()]
        );
        assert!(super::command_matches_runner(
            "C:/Users/test/bunx.cmd",
            "bunx"
        ));
        assert!(!super::command_matches_runner(
            "C:/Users/test/node.exe",
            "bunx"
        ));
        assert!(super::is_valid_docker_env_key("GOOD_NAME"));
        assert!(!super::is_valid_docker_env_key("9BAD"));
        assert_eq!(
            super::docker_compose_exec_env_args(&HashMap::from([
                ("Z_VAR".to_string(), "last".to_string()),
                ("BAD-NAME".to_string(), "ignored".to_string()),
                ("A_VAR".to_string(), "first".to_string()),
            ])),
            vec![
                "-e".to_string(),
                "A_VAR=first".to_string(),
                "-e".to_string(),
                "Z_VAR=last".to_string(),
            ]
        );
        assert_eq!(
            super::normalize_docker_launch_action(
                DockerLifecycleIntent::Restart,
                gwt_docker::ComposeServiceStatus::Running,
            ),
            super::DockerLaunchServiceAction::Restart
        );
        assert_eq!(
            super::normalize_docker_launch_action(
                DockerLifecycleIntent::Connect,
                gwt_docker::ComposeServiceStatus::Stopped,
            ),
            super::DockerLaunchServiceAction::Start
        );
    }

    #[test]
    fn docker_defaults_and_mount_helpers_prefer_devcontainer_settings() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        let devcontainer_dir = project_root.join(".devcontainer");
        fs::create_dir_all(&devcontainer_dir).expect("devcontainer dir");
        let compose_file = project_root.join("docker-compose.yml");
        fs::write(&compose_file, "services:\n  app:\n    image: alpine:3.20\n").expect("compose");
        fs::write(
            devcontainer_dir.join("devcontainer.json"),
            r#"{
  "dockerComposeFile": ["missing.yml", "../docker-compose.yml"],
  "service": "app",
  "workspaceFolder": "/workspaces/repo"
}"#,
        )
        .expect("devcontainer config");

        let files = gwt_docker::DockerFiles {
            dockerfile: None,
            compose_file: Some(compose_file.clone()),
            devcontainer_dir: Some(devcontainer_dir.clone()),
        };

        let defaults =
            super::docker_devcontainer_defaults(&project_root, &files).expect("defaults");
        assert_eq!(defaults.service.as_deref(), Some("app"));
        assert_eq!(
            defaults.workspace_folder.as_deref(),
            Some("/workspaces/repo")
        );
        assert!(super::same_worktree_path(
            defaults
                .compose_file
                .as_deref()
                .expect("compose file from defaults"),
            &compose_file,
        ));
        assert!(super::same_worktree_path(
            super::docker_compose_file_for_launch(&project_root, &files)
                .unwrap()
                .as_deref()
                .expect("compose file for launch"),
            &compose_file,
        ));

        let service = gwt_docker::ComposeService {
            name: "app".to_string(),
            image: Some("alpine:3.20".to_string()),
            ports: Vec::new(),
            depends_on: Vec::new(),
            working_dir: None,
            volumes: vec![gwt_docker::compose::ComposeVolumeMount {
                source: project_root.display().to_string(),
                target: "/workspaces/repo".to_string(),
                mode: None,
            }],
        };
        assert_eq!(
            super::compose_workspace_mount_target(&project_root, &service),
            Some("/workspaces/repo".to_string())
        );
        assert!(super::mount_source_matches_project_root(".", &project_root));
        assert!(super::mount_source_matches_project_root(
            &project_root.display().to_string(),
            &project_root,
        ));
    }

    #[test]
    fn worktree_git_and_misc_helpers_cover_repo_paths_and_defaults() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("repo dir");
        let init = Command::new("git")
            .args(["init", "-q", "-b", "develop"])
            .current_dir(&repo)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed");
        let config_name = Command::new("git")
            .args(["config", "user.name", "Codex"])
            .current_dir(&repo)
            .status()
            .expect("git config user.name");
        assert!(config_name.success(), "git config user.name failed");
        let config_email = Command::new("git")
            .args(["config", "user.email", "codex@example.com"])
            .current_dir(&repo)
            .status()
            .expect("git config user.email");
        assert!(config_email.success(), "git config user.email failed");
        fs::write(repo.join("README.md"), "repo\n").expect("write readme");
        let add = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .status()
            .expect("git add");
        assert!(add.success(), "git add failed");
        let commit = Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(&repo)
            .status()
            .expect("git commit");
        assert!(commit.success(), "git commit failed");
        let branch = Command::new("git")
            .args(["branch", "feature/demo"])
            .current_dir(&repo)
            .status()
            .expect("git branch");
        assert!(branch.success(), "git branch failed");

        assert_eq!(
            super::branch_worktree_path(&repo, "develop"),
            Some(repo.clone())
        );
        assert!(super::local_branch_exists(&repo, "feature/demo").unwrap());
        assert!(!super::local_branch_exists(&repo, "feature/missing").unwrap());

        let preferred = temp.path().join("feature-demo");
        let worktrees = vec![gwt_git::WorktreeInfo {
            path: preferred.clone(),
            branch: Some("feature/demo".to_string()),
            locked: false,
            prunable: false,
        }];
        assert_eq!(
            super::suffixed_worktree_path(&preferred, 2),
            Some(temp.path().join("feature-demo-2"))
        );
        assert_eq!(
            super::first_available_worktree_path(&preferred, &worktrees),
            Some(temp.path().join("feature-demo-2"))
        );
        assert!(super::worktree_path_is_occupied(&preferred, &worktrees));
        assert!(super::same_worktree_path(&repo, &repo.join(".")));

        let env = super::spawn_env();
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
        assert_eq!(
            super::geometry_to_pty_size(&WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            }),
            (46, 11)
        );
        assert_eq!(
            super::parse_github_remote_url("git@github.com:akiojin/gwt.git"),
            Some(("akiojin".to_string(), "gwt".to_string()))
        );
        assert_eq!(
            super::parse_github_remote_url("https://github.com/akiojin/gwt/"),
            Some(("akiojin".to_string(), "gwt".to_string()))
        );
        assert_eq!(
            super::parse_github_remote_url("ssh://example.com/akiojin/gwt"),
            None
        );

        let health = tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(super::health_handler());
        assert_eq!(health, "ok");
    }

    #[test]
    fn resolve_launch_worktree_helpers_cover_short_circuits_existing_and_remote_creation() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        init_git_clone_with_origin(&repo);

        let mut working_dir = None;
        let mut env_vars = HashMap::new();
        super::resolve_launch_worktree_request(
            temp.path(),
            None,
            None,
            &mut working_dir,
            &mut env_vars,
        )
        .expect("branchless launch");
        assert!(working_dir.is_none());
        assert!(env_vars.is_empty());

        let scratch = temp.path().join("scratch");
        fs::create_dir_all(&scratch).expect("create scratch");
        super::resolve_launch_worktree_request(
            &scratch,
            Some("feature/demo"),
            None,
            &mut working_dir,
            &mut env_vars,
        )
        .expect("non repo short circuit");
        assert!(working_dir.is_none());

        let mut current_dir = None;
        let mut current_env = HashMap::new();
        super::resolve_launch_worktree_request(
            &repo,
            Some("develop"),
            None,
            &mut current_dir,
            &mut current_env,
        )
        .expect("current branch worktree");
        assert_eq!(current_dir.as_deref(), Some(repo.as_path()));
        assert!(current_env
            .get("GWT_PROJECT_ROOT")
            .is_some_and(|value| super::same_worktree_path(Path::new(value), &repo)));

        let preset = temp.path().join("preset");
        let mut preset_dir = Some(preset.clone());
        let mut preset_env = HashMap::new();
        super::resolve_launch_worktree_request(
            &repo,
            Some("feature/ignored"),
            Some("develop"),
            &mut preset_dir,
            &mut preset_env,
        )
        .expect("preselected working dir");
        assert_eq!(preset_dir.as_deref(), Some(preset.as_path()));
        assert!(preset_env.is_empty());

        let status = Command::new("git")
            .args(["branch", "feature/existing"])
            .current_dir(&repo)
            .status()
            .expect("create feature branch");
        assert!(status.success(), "create feature branch failed");
        let existing_worktree = temp.path().join("feature-existing");
        let status = Command::new("git")
            .args(["worktree", "add"])
            .arg(&existing_worktree)
            .arg("feature/existing")
            .current_dir(&repo)
            .status()
            .expect("git worktree add");
        assert!(status.success(), "git worktree add failed");

        let mut existing_dir = None;
        let mut existing_env = HashMap::new();
        super::resolve_launch_worktree_request(
            &repo,
            Some("feature/existing"),
            Some("develop"),
            &mut existing_dir,
            &mut existing_env,
        )
        .expect("existing worktree");
        assert_eq!(existing_dir.as_deref(), Some(existing_worktree.as_path()));
        assert!(existing_env
            .get("GWT_PROJECT_ROOT")
            .is_some_and(|value| super::same_worktree_path(Path::new(value), &existing_worktree)));

        let err = super::resolve_launch_worktree_request(
            &repo,
            Some("feature/missing-base"),
            Some("release"),
            &mut None,
            &mut HashMap::new(),
        )
        .expect_err("missing base branch");
        assert!(err.contains("remote base branch does not exist"));

        let mut created_dir = None;
        let mut created_env = HashMap::new();
        super::resolve_launch_worktree_request(
            &repo,
            Some("feature/created"),
            Some("develop"),
            &mut created_dir,
            &mut created_env,
        )
        .expect("remote-backed worktree");
        let created_dir = created_dir.expect("created worktree dir");
        assert!(created_dir.exists());
        assert!(created_env
            .get("GWT_PROJECT_ROOT")
            .is_some_and(|value| super::same_worktree_path(Path::new(value), &created_dir)));

        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&created_dir)
            .output()
            .expect("current branch in created worktree");
        assert!(output.status.success(), "git branch --show-current failed");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "feature/created"
        );

        let mut launch_config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .branch("feature/existing")
            .base_branch("develop")
            .build();
        launch_config.working_dir = None;
        launch_config.env_vars = HashMap::new();
        super::resolve_launch_worktree(&repo, &mut launch_config).expect("agent launch wrapper");
        assert_eq!(
            launch_config.working_dir.as_deref(),
            Some(existing_worktree.as_path())
        );

        let mut shell_config = ShellLaunchConfig {
            working_dir: None,
            branch: Some("feature/existing".to_string()),
            base_branch: Some("develop".to_string()),
            display_name: "Shell".to_string(),
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            env_vars: HashMap::new(),
        };
        super::resolve_shell_launch_worktree(&repo, &mut shell_config)
            .expect("shell launch wrapper");
        assert_eq!(
            shell_config.working_dir.as_deref(),
            Some(existing_worktree.as_path())
        );
    }

    #[test]
    fn docker_launch_plan_and_helper_logic_cover_defaults_and_errors() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let devcontainer_dir = project.join(".devcontainer");
        fs::create_dir_all(&devcontainer_dir).expect("create devcontainer dir");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    volumes:\n      - .:/workspace/app\n  worker:\n    image: alpine:3.19\n    working_dir: /srv/worker\n",
        )
        .expect("write compose file");
        fs::write(
            devcontainer_dir.join("devcontainer.json"),
            r#"{
  "dockerComposeFile": "docker-compose.yml",
  "service": "app",
  "workspaceFolder": "/workspace/dev"
}"#,
        )
        .expect("write devcontainer");

        let plan = super::resolve_docker_launch_plan(&project, None).expect("launch plan");
        assert_eq!(plan.service, "app");
        assert_eq!(plan.container_cwd, "/workspace/dev");
        assert_eq!(plan.compose_file, project.join("docker-compose.yml"));

        let (context, status) = super::detect_wizard_docker_context_and_status(&project);
        let context = context.expect("docker context");
        assert!(context.services.contains(&"app".to_string()));
        assert_eq!(context.suggested_service.as_deref(), Some("app"));
        assert_eq!(status, gwt_docker::ComposeServiceStatus::NotFound);

        let multi = temp.path().join("multi");
        fs::create_dir_all(&multi).expect("create multi project");
        fs::write(
            multi.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n  worker:\n    image: alpine:3.19\n",
        )
        .expect("write multi compose");
        let multi_err = super::resolve_docker_launch_plan(&multi, None).expect_err("multi service");
        assert!(multi_err.contains("Multiple Docker services detected"));

        let invalid_service = super::resolve_docker_launch_plan(&project, Some("missing"))
            .expect_err("missing docker service");
        assert!(invalid_service.contains("Selected Docker service was not found"));

        let no_cwd = temp.path().join("no-cwd");
        fs::create_dir_all(&no_cwd).expect("create no-cwd project");
        fs::write(
            no_cwd.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n",
        )
        .expect("write no-cwd compose");
        let no_cwd_err =
            super::resolve_docker_launch_plan(&no_cwd, Some("app")).expect_err("no cwd");
        assert!(no_cwd_err.contains("missing working_dir/workspaceFolder"));

        let missing_compose =
            super::resolve_docker_launch_plan(temp.path(), None).expect_err("missing compose");
        assert!(missing_compose.contains("docker-compose.yml"));

        assert_eq!(
            super::normalize_docker_launch_action(
                DockerLifecycleIntent::Restart,
                gwt_docker::ComposeServiceStatus::Running,
            ),
            super::DockerLaunchServiceAction::Restart
        );
        assert_eq!(
            super::normalize_docker_launch_action(
                DockerLifecycleIntent::CreateAndStart,
                gwt_docker::ComposeServiceStatus::Exited,
            ),
            super::DockerLaunchServiceAction::Start
        );
        assert_eq!(
            super::normalize_docker_launch_action(
                DockerLifecycleIntent::Recreate,
                gwt_docker::ComposeServiceStatus::Stopped,
            ),
            super::DockerLaunchServiceAction::Recreate
        );

        assert_eq!(super::origin_remote_ref("develop"), "origin/develop");
        assert_eq!(
            super::origin_remote_ref("refs/remotes/origin/main"),
            "origin/main"
        );
        assert!(super::command_matches_runner("C:/tools/bunx.cmd", "bunx"));
        assert!(!super::command_matches_runner("C:/tools/node.exe", "bunx"));

        let version_spec = super::package_runner_version_spec(&sample_versioned_launch_config())
            .expect("version spec");
        assert_eq!(version_spec, "@anthropic-ai/claude-code@latest");
        assert_eq!(
            super::strip_package_runner_args(
                &[
                    "--yes".to_string(),
                    version_spec.clone(),
                    "--print".to_string(),
                ],
                &version_spec,
            ),
            vec!["--print".to_string()]
        );
        assert_eq!(
            super::strip_package_runner_args(
                &[version_spec.clone(), "--print".to_string()],
                &version_spec,
            ),
            vec!["--print".to_string()]
        );
        assert_eq!(
            super::strip_package_runner_args(&["--print".to_string()], &version_spec),
            vec!["--print".to_string()]
        );

        let old_docker_bin = std::env::var_os("GWT_DOCKER_BIN");
        std::env::set_var("GWT_DOCKER_BIN", "podman");
        assert_eq!(super::docker_binary_for_launch(), "podman");
        match old_docker_bin {
            Some(value) => std::env::set_var("GWT_DOCKER_BIN", value),
            None => std::env::remove_var("GWT_DOCKER_BIN"),
        }
    }

    #[test]
    fn branch_selection_and_mount_helpers_cover_preferred_paths() {
        assert_eq!(
            super::normalize_branch_name("refs/remotes/origin/feature/coverage"),
            "feature/coverage"
        );
        assert_eq!(super::normalize_branch_name("origin/main"), "main");
        assert_eq!(
            super::normalize_branch_name("feature/coverage"),
            "feature/coverage"
        );

        let mut head = sample_branch_entry("feature/current");
        head.is_head = true;
        let entries = vec![sample_branch_entry("main"), head.clone()];
        assert_eq!(
            super::preferred_issue_launch_branch(&entries).as_deref(),
            Some("main")
        );
        assert_eq!(
            super::preferred_issue_launch_branch(&[head.clone()]).as_deref(),
            Some("feature/current")
        );
        assert!(super::preferred_issue_launch_branch(&[]).is_none());

        assert_eq!(
            super::knowledge_kind_for_preset(WindowPreset::Issue),
            Some(KnowledgeKind::Issue)
        );
        assert_eq!(
            super::knowledge_kind_for_preset(WindowPreset::Spec),
            Some(KnowledgeKind::Spec)
        );
        assert_eq!(
            super::knowledge_kind_for_preset(WindowPreset::Pr),
            Some(KnowledgeKind::Pr)
        );
        assert_eq!(super::knowledge_kind_for_preset(WindowPreset::Shell), None);

        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).expect("create project root");
        assert!(super::mount_source_matches_project_root(".", &project_root));
        assert!(super::mount_source_matches_project_root(
            "$PWD",
            &project_root
        ));
        assert!(super::mount_source_matches_project_root(
            &project_root.display().to_string(),
            &project_root,
        ));
        assert!(!super::mount_source_matches_project_root(
            "/tmp/somewhere-else",
            &project_root,
        ));

        let service = gwt_docker::ComposeService {
            name: "app".to_string(),
            image: Some("alpine:3.19".to_string()),
            ports: Vec::new(),
            depends_on: Vec::new(),
            working_dir: Some("/workspace".to_string()),
            volumes: vec![gwt_docker::compose::ComposeVolumeMount {
                source: ".".to_string(),
                target: "/workspace".to_string(),
                mode: None,
            }],
        };
        assert_eq!(
            super::compose_workspace_mount_target(&project_root, &service).as_deref(),
            Some("/workspace")
        );

        let preferred = temp.path().join("feature");
        fs::create_dir_all(&preferred).expect("create preferred worktree path");
        let occupied = vec![gwt_git::WorktreeInfo {
            path: temp.path().join("feature-2"),
            branch: Some("feature/other".to_string()),
            locked: false,
            prunable: false,
        }];
        assert_eq!(
            super::suffixed_worktree_path(&preferred, 3).unwrap(),
            temp.path().join("feature-3")
        );
        assert_eq!(
            super::first_available_worktree_path(&preferred, &occupied).unwrap(),
            temp.path().join("feature-3")
        );
        assert!(super::worktree_path_is_occupied(
            &temp.path().join("feature-2"),
            &occupied,
        ));
        assert!(super::same_worktree_path(&project_root, &project_root));
    }

    #[test]
    fn git_and_cli_metadata_helpers_cover_parsing_geometry_and_repo_state() {
        let env = super::spawn_env();
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));

        let geometry = WindowGeometry {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        assert_eq!(super::geometry_to_pty_size(&geometry), (46, 11));

        assert_eq!(
            super::parse_github_remote_url("git@github.com:akiojin/gwt.git"),
            Some(("akiojin".to_string(), "gwt".to_string()))
        );
        assert_eq!(
            super::parse_github_remote_url("https://github.com/akiojin/gwt/"),
            Some(("akiojin".to_string(), "gwt".to_string()))
        );
        assert_eq!(
            super::parse_github_remote_url("https://example.com/repo"),
            None
        );

        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repo)
            .output()
            .expect("git init");
        assert!(init.status.success(), "git init failed");
        let remote = Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/akiojin/gwt.git",
            ])
            .current_dir(&repo)
            .output()
            .expect("git remote");
        assert!(remote.status.success(), "git remote add failed");
        let branch = Command::new("git")
            .args(["checkout", "-b", "feature/coverage"])
            .current_dir(&repo)
            .output()
            .expect("git checkout");
        assert!(branch.status.success(), "git checkout failed");
        let config_name = Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&repo)
            .output()
            .expect("git config user.name");
        assert!(config_name.status.success(), "git config user.name failed");
        let config_email = Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&repo)
            .output()
            .expect("git config user.email");
        assert!(
            config_email.status.success(),
            "git config user.email failed"
        );
        fs::write(repo.join("README.md"), "hello\n").expect("write README");
        let add = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add failed");
        let commit = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo)
            .output()
            .expect("git commit");
        assert!(commit.status.success(), "git commit failed");

        assert_eq!(super::origin_remote_ref("main"), "origin/main");
        assert_eq!(
            super::current_git_branch(&repo).as_deref(),
            Ok("feature/coverage")
        );
        assert_eq!(
            super::local_branch_exists(&repo, "feature/coverage"),
            Ok(true)
        );
        assert_eq!(super::local_branch_exists(&repo, "missing"), Ok(false));
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
            .unwrap_or_else(|p| p.into_inner())
            .insert(client_id, tx);
        rx
    }

    fn unregister(&self, client_id: &str) {
        self.clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(client_id);
    }

    fn dispatch(&self, events: Vec<OutboundEvent>) {
        let clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());
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
    hook_forward_token: String,
    /// Shared PTY writer registry for the `terminal_input` fast-path.
    pty_writers: PtyWriterRegistry,
}

struct EmbeddedServer {
    url: String,
    hook_forward_token: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl EmbeddedServer {
    fn start(
        runtime: &Runtime,
        proxy: EventLoopProxy<UserEvent>,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(("127.0.0.1", 0)))?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let hook_forward_token = Uuid::new_v4().to_string();

        let app = Router::new()
            .route("/", get(embedded_web::index_handler))
            .route("/healthz", get(health_handler))
            .route("/internal/hook-live", post(hook_live_handler))
            .route("/ws", get(websocket_handler))
            .with_state(ServerState {
                proxy,
                clients,
                hook_forward_token: hook_forward_token.clone(),
                pty_writers,
            });

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
            hook_forward_token,
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

    fn hook_forward_target(&self) -> HookForwardTarget {
        HookForwardTarget {
            url: format!("{}internal/hook-live", self.url),
            token: self.hook_forward_token.clone(),
        }
    }
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

async fn hook_live_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(event): Json<RuntimeHookEvent>,
) -> StatusCode {
    if !hook_forward_authorized(&headers, &state.hook_forward_token) {
        return StatusCode::UNAUTHORIZED;
    }

    broadcast_runtime_hook_event(&state.clients, event);
    StatusCode::NO_CONTENT
}

async fn client_session(socket: WebSocket, state: ServerState) {
    let client_id = Uuid::new_v4().to_string();
    let mut outbound = state.clients.register(client_id.clone());
    let (mut sender, mut receiver) = socket.split();

    // Per-session counter that tags each inbound `terminal_input` in order so
    // we can diff layer counts (frontend → WS → event loop → PTY writer) when
    // diagnosing intermittent key-input drops (bugfix/input-key).
    let input_seq = Arc::new(AtomicU64::new(0));

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
                        let text_len = text.len();
                        match serde_json::from_str::<FrontendEvent>(text.as_ref()) {
                            Ok(event) => {
                                handle_frontend_message(
                                    &state,
                                    &client_id,
                                    &input_seq,
                                    text_len,
                                    event,
                                );
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

/// Dispatch a parsed `FrontendEvent` from the WebSocket receiver task.
///
/// `TerminalInput` takes the fast-path: the pane's PTY handle is looked up in
/// the shared registry and written to directly, bypassing the single-threaded
/// tao event loop. Other events still flow through `UserEvent::Frontend` so
/// they can mutate `AppRuntime` on the main thread.
///
/// If the fast-path fails (unknown window, PTY write error, or poisoned
/// registry lock), the input is forwarded to the proxy so the existing
/// error-reporting path in `terminal_input_events` still runs.
fn handle_frontend_message(
    state: &ServerState,
    client_id: &str,
    input_seq: &AtomicU64,
    text_len: usize,
    event: FrontendEvent,
) {
    // Fast-path only applies to TerminalInput. For every other variant, just
    // forward to the main-thread event loop as before.
    let (id, data) = match event {
        FrontendEvent::TerminalInput { id, data } => (id, data),
        other => {
            let _ = state.proxy.send_event(UserEvent::Frontend {
                client_id: client_id.to_string(),
                event: other,
            });
            return;
        }
    };

    let seq = input_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let data_len = data.len();
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_recv",
        client_id = %client_id,
        seq,
        window_id = %id,
        data_len,
        text_len,
        "terminal_input received over WebSocket"
    );

    let pty_handle = match state.pty_writers.read() {
        Ok(guard) => guard.get(&id).cloned(),
        Err(error) => {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "fast_path_lock_poisoned",
                client_id = %client_id,
                seq,
                window_id = %id,
                error = %error,
                "pty_writers read lock poisoned; falling back to event loop"
            );
            None
        }
    };

    if let Some(pty) = pty_handle {
        let write_started = Instant::now();
        match pty.write_input(data.as_bytes()) {
            Ok(()) => {
                tracing::debug!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    data_len,
                    write_us = write_started.elapsed().as_micros() as u64,
                    "terminal_input written to PTY via WS fast-path"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write_err",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    data_len,
                    error = %error,
                    "fast-path PTY write failed; forwarding to event loop for error handling"
                );
                // fall through to proxy path so `terminal_input_events` can
                // route the error through `handle_runtime_status`.
            }
        }
    } else {
        tracing::debug!(
            target: "gwt_input_trace",
            stage = "fast_path_miss",
            client_id = %client_id,
            seq,
            window_id = %id,
            data_len,
            "pty_writers registry miss; falling back to event loop"
        );
    }

    let send_result = state.proxy.send_event(UserEvent::Frontend {
        client_id: client_id.to_string(),
        event: FrontendEvent::TerminalInput {
            id: id.clone(),
            data,
        },
    });
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_dispatch",
        client_id = %client_id,
        seq,
        window_id = %id,
        data_len,
        ok = send_result.is_ok(),
        "terminal_input forwarded to event loop proxy (fallback)"
    );
}

fn hook_forward_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected_token)
}

fn broadcast_runtime_hook_event(clients: &ClientHub, event: RuntimeHookEvent) {
    clients.dispatch(vec![OutboundEvent::broadcast(
        BackendEvent::RuntimeHookEvent { event },
    )]);
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

fn synthetic_branch_entry(branch_name: &str) -> BranchListEntry {
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

fn resolve_launch_wizard_hydration(
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

fn knowledge_kind_for_preset(preset: WindowPreset) -> Option<KnowledgeKind> {
    match preset {
        WindowPreset::Issue => Some(KnowledgeKind::Issue),
        WindowPreset::Spec => Some(KnowledgeKind::Spec),
        WindowPreset::Pr => Some(KnowledgeKind::Pr),
        _ => None,
    }
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

fn resolve_launch_worktree_request(
    repo_path: &Path,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), String> {
    gwt_agent::resolve_launch_worktree_request(
        repo_path,
        branch_name,
        base_branch,
        working_dir,
        env_vars,
    )
}

#[cfg(test)]
fn resolve_launch_worktree(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    gwt_agent::resolve_launch_worktree(repo_path, config)
}

fn resolve_shell_launch_worktree(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<(), String> {
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        config.base_branch.as_deref(),
        &mut config.working_dir,
        &mut config.env_vars,
    )
}

#[derive(Debug, Clone)]
struct DockerLaunchPlan {
    compose_file: PathBuf,
    service: String,
    container_cwd: String,
}

#[derive(Debug, Clone, Default)]
struct DevContainerLaunchDefaults {
    service: Option<String>,
    workspace_folder: Option<String>,
    compose_file: Option<PathBuf>,
}

fn build_shell_process_launch(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<ProcessLaunch, String> {
    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let mut env = spawn_env();
    env.extend(config.env_vars.clone());

    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        let shell = detect_shell_program().map_err(|error| error.to_string())?;
        env.entry("GWT_PROJECT_ROOT".to_string())
            .or_insert_with(|| worktree.display().to_string());
        install_launch_gwt_bin_env(&mut env, gwt_agent::LaunchRuntimeTarget::Host)?;
        config.env_vars = env.clone();
        return Ok(ProcessLaunch {
            command: shell.command,
            args: shell.args,
            env,
            cwd: Some(worktree),
        });
    }

    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    ensure_docker_launch_service_ready(&launch, config.docker_lifecycle_intent)?;
    ensure_docker_gwt_binary_setup(&worktree, &launch.service)?;
    let shell_command = resolve_docker_shell_command(&launch)?;
    env.insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    install_launch_gwt_bin_env(&mut env, gwt_agent::LaunchRuntimeTarget::Docker)?;
    config.docker_service = Some(launch.service.clone());
    config.env_vars = env.clone();

    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        launch.compose_file.display().to_string(),
        "exec".to_string(),
        "-w".to_string(),
        launch.container_cwd.clone(),
    ];
    args.extend(docker_compose_exec_env_args(&env));
    args.push(launch.service);
    args.push(shell_command);

    Ok(ProcessLaunch {
        command: docker_binary_for_launch(),
        args,
        env,
        cwd: Some(worktree),
    })
}

#[cfg(test)]
fn apply_host_package_runner_fallback_with_probe<F>(
    config: &mut gwt_agent::LaunchConfig,
    fallback_executable: String,
    probe: F,
) -> bool
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, Option<PathBuf>) -> bool,
{
    gwt_agent::apply_host_package_runner_fallback_with_probe(config, fallback_executable, probe)
}
#[cfg(test)]
fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
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

fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> Result<(), String> {
    gwt_agent::install_launch_gwt_bin_env(env_vars, runtime_target)
}

#[cfg(test)]
fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    gwt_agent::install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, current_exe, lookup)
}

fn resolve_user_home_dir() -> Result<PathBuf, String> {
    let home = if cfg!(windows) {
        std::env::var("USERPROFILE")
    } else {
        std::env::var("HOME")
    }
    .map(PathBuf::from)
    .map_err(|_| "Could not determine home directory".to_string())?;
    Ok(home)
}

fn docker_bundle_mounts_for_home(home: &Path) -> DockerBundleMounts {
    let gwt_bin_dir = home.join(".gwt").join("bin");
    DockerBundleMounts {
        host_gwt: gwt_bin_dir.join(DOCKER_HOST_GWT_BIN_NAME),
        host_gwtd: gwt_bin_dir.join(DOCKER_HOST_GWTD_BIN_NAME),
    }
}

fn docker_compose_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn docker_bundle_override_content(service: &str, bundle: &DockerBundleMounts) -> String {
    let host_gwt = docker_compose_mount_path(&bundle.host_gwt);
    let host_gwtd = docker_compose_mount_path(&bundle.host_gwtd);
    format!(
        "# Auto-generated docker-compose override for gwt bundle mounting\n\
         version: '3.8'\n\
         services:\n\
           {service}:\n\
             volumes:\n\
               - \"{host_gwt}:{DOCKER_GWT_BIN_PATH}:ro\"\n\
               - \"{host_gwtd}:{DOCKER_GWTD_BIN_PATH}:ro\"\n"
    )
}

fn ensure_docker_gwt_binary_setup(repo_path: &Path, service: &str) -> Result<(), String> {
    use std::fs;

    let home = resolve_user_home_dir()?;
    let bundle = docker_bundle_mounts_for_home(&home);

    if !bundle.host_gwt.exists() || !bundle.host_gwtd.exists() {
        let override_path = repo_path.join("docker-compose.override.yml");
        if !override_path.exists() {
            eprintln!(
                "Note: Linux gwt bundle not found at {} and {}\n\
                 This is required for Docker agent support.\n\
                 You can either:\n\
                 1. Download the Linux release bundle and place the extracted binaries at these paths\n\
                 2. Run 'gwt setup docker' to set up Docker integration automatically"
                ,
                bundle.host_gwt.display(),
                bundle.host_gwtd.display()
            );
        }
    }

    let override_path = repo_path.join("docker-compose.override.yml");
    if !override_path.exists() {
        let override_content = docker_bundle_override_content(service, &bundle);
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to create docker-compose.override.yml: {err}\n\
                 Manually create {} with gwt/gwtd bundle mounts",
                override_path.display()
            )
        })?;
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
#[cfg(test)]
fn package_runner_version_spec(config: &gwt_agent::LaunchConfig) -> Option<String> {
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
#[cfg(test)]
fn strip_package_runner_args(args: &[String], version_spec: &str) -> Vec<String> {
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

fn resolve_docker_shell_command(launch: &DockerLaunchPlan) -> Result<String, String> {
    for candidate in ["bash", "sh"] {
        let available = gwt_docker::compose_service_has_command(
            &launch.compose_file,
            &launch.service,
            candidate,
        )
        .map_err(|err| err.to_string())?;
        if available {
            return Ok(candidate.to_string());
        }
    }

    Err(format!(
        "Selected Docker runtime has no interactive shell in service '{}'",
        launch.service
    ))
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

#[cfg(test)]
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

#[cfg(test)]
fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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
        let mut env = gwt::cli::DefaultCliEnv::new(&owner, &repo, repo_path);
        std::process::exit(gwt::cli::dispatch(&mut env, argv));
    }

    let mut env = gwt::cli::DefaultCliEnv::new_for_hooks();
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

    // Install the tracing subscriber so that `tracing::debug!/info!` lands in
    // `~/.gwt/logs/gwt.log`. The returned guard must outlive the event loop;
    // we bind it to `_log_handles` and keep it until `main` returns.
    //
    // Diagnostic trace for intermittent key-input drop (bugfix/input-key) is
    // emitted at `debug` level under `target: "gwt_input_trace"`. Enable with
    // `RUST_LOG=gwt_input_trace=debug`.
    let _log_handles = gwt_core::logging::init(gwt_core::logging::LoggingConfig::new(
        gwt_core::paths::gwt_logs_dir(),
    ))
    .map_err(|error| {
        eprintln!("gwt logging init failed: {error}");
    })
    .ok();

    if let Ok(project_root) = std::env::current_dir() {
        if let Err(error) = gwt::cli::prepare_daemon_front_door_for_path(&project_root) {
            eprintln!("gwt daemon bootstrap: {error}");
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
    let pty_writers: PtyWriterRegistry = Arc::new(RwLock::new(HashMap::new()));
    let mut app = AppRuntime::new(
        proxy.clone(),
        pty_writers.clone(),
        BlockingTaskSpawner::tokio(runtime.handle().clone()),
    )
    .expect("app runtime");
    app.bootstrap();

    let mut server = EmbeddedServer::start(
        &runtime,
        proxy.clone(),
        clients.clone(),
        pty_writers.clone(),
    )
    .expect("embedded server");
    app.set_hook_forward_target(server.hook_forward_target());
    eprintln!("gwt browser URL: {}", server.url());

    // Startup update check (T-031): runs in background; broadcasts UpdateState::Available if a
    // newer release is found. Silent on failure and in CI environments.
    {
        let clients = clients.clone();
        let update_proxy = proxy.clone();
        runtime.spawn(async move {
            if gwt_core::update::is_ci() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
            let current_exe = std::env::current_exe().ok();
            for attempt in 0..3u32 {
                if attempt > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
                }
                let exe = current_exe.clone();
                let state = match tokio::task::spawn_blocking(move || {
                    gwt_core::update::UpdateManager::new()
                        .check_for_executable(false, exe.as_deref())
                })
                .await
                {
                    Ok(s) => s,
                    Err(_) => break,
                };
                match &state {
                    gwt_core::update::UpdateState::Available {
                        asset_url: Some(_), ..
                    } => {
                        // Notify main thread to cache the state for reconnecting clients.
                        let _ = update_proxy.send_event(UserEvent::UpdateAvailable(state.clone()));
                        clients.dispatch(vec![OutboundEvent::broadcast(
                            BackendEvent::UpdateState(state),
                        )]);
                        return;
                    }
                    gwt_core::update::UpdateState::Available {
                        asset_url: None, ..
                    }
                    | gwt_core::update::UpdateState::UpToDate { .. } => return,
                    gwt_core::update::UpdateState::Failed { .. } => {
                        // retry on next iteration
                    }
                }
            }
        });
    }

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
                // Kill every PTY / agent before the event loop exits so no
                // child process outlives the window.
                app.stop_all_runtimes();
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
            Event::UserEvent(UserEvent::LaunchProgress { window_id, message }) => {
                clients.dispatch(vec![OutboundEvent::broadcast(
                    BackendEvent::LaunchProgress {
                        id: window_id,
                        message,
                    },
                )]);
            }
            Event::UserEvent(UserEvent::LaunchComplete { window_id, result }) => {
                let events = app.handle_launch_complete(window_id, result);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::ShellLaunchComplete { window_id, result }) => {
                let events = app.handle_shell_launch_complete(window_id, result);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::LaunchWizardHydrated { wizard_id, result }) => {
                let events = app.handle_launch_wizard_hydrated(wizard_id, result);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::IssueLaunchWizardPrepared(prepared)) => {
                let events = app.handle_issue_launch_wizard_prepared(prepared);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::Dispatch(events)) => {
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::UpdateAvailable(state)) => {
                app.pending_update = Some(state);
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
                // Belt-and-suspenders: if the event loop is torn down via a
                // path other than CloseRequested, still release PTY children.
                app.stop_all_runtimes();
                server.shutdown();
            }
            _ => {}
        }
    });
}

/// Download and apply a pending update, then exit.
///
/// Called from a background thread so the GUI remains responsive during download.
/// On success, this function calls `std::process::exit(0)` and never returns.
/// On any failure, it returns silently.
fn apply_update_and_exit() {
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let mgr = gwt_core::update::UpdateManager::new();
    // Use force=false to read from the TTL cache rather than forcing a new network round-trip.
    // The startup check already confirmed the update; a second network call here could flip the
    // result to Failed if connectivity was lost between discovery and user confirmation.
    let state = mgr.check_for_executable(false, Some(&current_exe));
    let (latest, asset_url) = match state {
        gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(asset_url),
            ..
        } => (latest, asset_url),
        _ => return,
    };
    let payload = match mgr.prepare_update(&latest, &asset_url) {
        Ok(p) => p,
        Err(_) => return,
    };
    let args_file = match &payload {
        gwt_core::update::PreparedPayload::PortableBinary { path }
        | gwt_core::update::PreparedPayload::Installer { path, .. } => {
            path.parent().map(|d| d.join("restart-args.json"))
        }
    };
    let Some(args_file) = args_file else {
        return;
    };
    let restart_args: Vec<String> = std::env::args().skip(1).collect();
    if mgr
        .write_restart_args_file(&args_file, restart_args)
        .is_err()
    {
        return;
    }
    let helper_exe = if cfg!(windows) {
        match mgr.make_helper_copy(&current_exe, &latest) {
            Ok(p) => p,
            Err(_) => return,
        }
    } else {
        current_exe.clone()
    };
    let old_pid = std::process::id();
    let result = match payload {
        gwt_core::update::PreparedPayload::PortableBinary { path } => {
            mgr.spawn_internal_apply_update(&helper_exe, old_pid, &current_exe, &path, &args_file)
        }
        gwt_core::update::PreparedPayload::Installer { path, kind } => mgr
            .spawn_internal_run_installer(
                &helper_exe,
                old_pid,
                &current_exe,
                &path,
                kind,
                &args_file,
            ),
    };
    if result.is_ok() {
        std::process::exit(0);
    }
}

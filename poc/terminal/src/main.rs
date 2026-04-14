use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
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
use gwt_terminal::{Pane, PaneStatus};
use poc_terminal::{
    detect_shell_program, list_branch_entries, list_directory_entries, load_workspace_state,
    resolve_launch_spec, save_workspace_state, workspace_state_path, BackendEvent, FrontendEvent,
    WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState,
};
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
}

struct WindowRuntime {
    pane: Arc<Mutex<Pane>>,
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

struct AppRuntime {
    workspace: WorkspaceState,
    runtimes: HashMap<String, WindowRuntime>,
    window_details: HashMap<String, String>,
    state_path: PathBuf,
    proxy: EventLoopProxy<UserEvent>,
    workdir: PathBuf,
}

impl AppRuntime {
    fn new(proxy: EventLoopProxy<UserEvent>) -> std::io::Result<Self> {
        let state_path = workspace_state_path();
        let workspace = WorkspaceState::from_persisted(load_workspace_state(&state_path)?);
        let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(Self {
            workspace,
            runtimes: HashMap::new(),
            window_details: HashMap::new(),
            state_path,
            proxy,
            workdir,
        })
    }

    fn bootstrap(&mut self) {
        let windows = self.workspace.persisted().windows.clone();
        for window in windows {
            let _ = self.start_window(&window.id, window.preset, window.geometry.clone());
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
            FrontendEvent::CreateWindow { preset } => {
                let window = self.workspace.add_window(preset);
                let runtime_event =
                    self.start_window(&window.id, window.preset, window.geometry.clone());
                let _ = self.persist();
                let mut events = vec![OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                })];
                if let Some(event) = runtime_event {
                    events.push(OutboundEvent::broadcast(event));
                }
                events
            }
            FrontendEvent::FocusWindow { id } => {
                if !self.workspace.focus_window(&id) {
                    return Vec::new();
                }
                let _ = self.persist();
                vec![OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                })]
            }
            FrontendEvent::UpdateViewport { viewport } => {
                self.workspace.update_viewport(viewport);
                let _ = self.persist();
                vec![OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                })]
            }
            FrontendEvent::UpdateWindowGeometry {
                id,
                geometry,
                cols,
                rows,
            } => {
                if !self.workspace.update_geometry(&id, geometry) {
                    return Vec::new();
                }
                if let Some(runtime) = self.runtimes.get(&id) {
                    if let Ok(mut pane) = runtime.pane.lock() {
                        let _ = pane.resize(cols.max(20), rows.max(6));
                    }
                }
                let _ = self.persist();
                vec![OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                })]
            }
            FrontendEvent::CloseWindow { id } => {
                if let Some(runtime) = self.runtimes.remove(&id) {
                    if let Ok(pane) = runtime.pane.lock() {
                        let _ = pane.kill();
                    }
                }
                self.window_details.remove(&id);
                if !self.workspace.close_window(&id) {
                    return Vec::new();
                }
                let _ = self.persist();
                vec![OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                })]
            }
            FrontendEvent::TerminalInput { id, data } => {
                let Some(runtime) = self.runtimes.get(&id) else {
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
                        self.handle_runtime_status(id, WindowProcessStatus::Error, Some(error))
                    }
                }
            }
            FrontendEvent::LoadFileTree { id, path } => {
                let path = path.unwrap_or_default();
                let event = self.load_file_tree_event(&id, &path);
                vec![OutboundEvent::reply(client_id, event)]
            }
            FrontendEvent::LoadBranches { id } => {
                let event = self.load_branches_event(&id);
                vec![OutboundEvent::reply(client_id, event)]
            }
        }
    }

    fn frontend_sync_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let mut events = vec![OutboundEvent::reply(
            client_id,
            BackendEvent::WorkspaceState {
                workspace: self.workspace.persisted().clone(),
            },
        )];

        for (id, detail) in &self.window_details {
            let Some(window) = self.workspace.window(id) else {
                continue;
            };
            events.push(OutboundEvent::reply(
                client_id,
                BackendEvent::TerminalStatus {
                    id: id.clone(),
                    status: window.status.clone(),
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

        events
    }

    fn load_file_tree_event(&self, id: &str, path: &str) -> BackendEvent {
        let Some(window) = self.workspace.window(id) else {
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

        match list_directory_entries(&self.workdir, relative_path) {
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
        let Some(window) = self.workspace.window(id) else {
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

        match list_branch_entries(&self.workdir) {
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

    fn handle_runtime_output(&mut self, id: String, data: Vec<u8>) -> Vec<OutboundEvent> {
        if self.workspace.window(&id).is_none() {
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
        if self.workspace.window(&id).is_none() {
            self.runtimes.remove(&id);
            self.window_details.remove(&id);
            return Vec::new();
        }

        self.workspace.set_status(&id, status.clone());
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
        }
        let _ = self.persist();

        vec![
            OutboundEvent::broadcast(BackendEvent::WorkspaceState {
                workspace: self.workspace.persisted().clone(),
            }),
            OutboundEvent::broadcast(BackendEvent::TerminalStatus { id, status, detail }),
        ]
    }

    fn start_window(
        &mut self,
        id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Option<BackendEvent> {
        if !preset.requires_process() {
            self.workspace.set_status(id, WindowProcessStatus::Ready);
            return None;
        }

        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                self.workspace.set_status(id, WindowProcessStatus::Error);
                self.window_details
                    .insert(id.to_string(), error.to_string());
                return Some(BackendEvent::TerminalStatus {
                    id: id.to_string(),
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        let launch = match resolve_launch_spec_with_fallback(preset, &shell) {
            Ok(launch) => launch,
            Err(error) => {
                self.workspace.set_status(id, WindowProcessStatus::Error);
                self.window_details
                    .insert(id.to_string(), error.to_string());
                return Some(BackendEvent::TerminalStatus {
                    id: id.to_string(),
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        let (cols, rows) = geometry_to_pty_size(&geometry);
        let pane = match Pane::new(
            id.to_string(),
            launch.command,
            launch.args,
            cols,
            rows,
            spawn_env(),
            Some(self.workdir.clone()),
        ) {
            Ok(pane) => Arc::new(Mutex::new(pane)),
            Err(error) => {
                self.workspace.set_status(id, WindowProcessStatus::Error);
                self.window_details
                    .insert(id.to_string(), error.to_string());
                return Some(BackendEvent::TerminalStatus {
                    id: id.to_string(),
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        self.spawn_output_thread(id.to_string(), pane.clone());
        self.workspace.set_status(id, WindowProcessStatus::Running);
        self.window_details.remove(id);
        self.runtimes.insert(id.to_string(), WindowRuntime { pane });
        Some(BackendEvent::TerminalStatus {
            id: id.to_string(),
            status: WindowProcessStatus::Running,
            detail: None,
        })
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

    fn persist(&self) -> std::io::Result<()> {
        save_workspace_state(&self.state_path, self.workspace.persisted())
    }
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

fn resolve_launch_spec_with_fallback(
    preset: WindowPreset,
    shell: &poc_terminal::ShellProgram,
) -> Result<poc_terminal::LaunchSpec, poc_terminal::PresetResolveError> {
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

fn main() -> wry::Result<()> {
    let runtime = Runtime::new().expect("tokio runtime");
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let clients = ClientHub::default();
    let mut app = AppRuntime::new(proxy.clone()).expect("app runtime");
    app.bootstrap();

    let mut server =
        EmbeddedServer::start(&runtime, proxy.clone(), clients.clone()).expect("embedded server");
    eprintln!("poc-terminal browser URL: {}", server.url());

    let window = WindowBuilder::new()
        .with_title("gwt terminal poc")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 920.0))
        .build(&event_loop)
        .expect("window");

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
            _ => {}
        }
    });
}

use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use base64::Engine;
use poc_terminal::{
    detect_shell_program, load_workspace_state, resolve_launch_spec, save_workspace_state,
    workspace_state_path, CanvasViewport, PersistedWorkspaceState, WindowGeometry, WindowPreset,
    WindowProcessStatus, WorkspaceState,
};
use serde::{Deserialize, Serialize};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

use gwt_terminal::{Pane, PaneStatus};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FrontendEvent {
    FrontendReady,
    CreateWindow {
        preset: WindowPreset,
    },
    FocusWindow {
        id: String,
    },
    UpdateViewport {
        viewport: CanvasViewport,
    },
    UpdateWindowGeometry {
        id: String,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
    },
    CloseWindow {
        id: String,
    },
    TerminalInput {
        id: String,
        data: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BackendEvent {
    WorkspaceState {
        workspace: PersistedWorkspaceState,
    },
    TerminalOutput {
        id: String,
        data_base64: String,
    },
    TerminalStatus {
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone)]
enum UserEvent {
    Frontend(FrontendEvent),
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

struct AppRuntime {
    workspace: WorkspaceState,
    runtimes: HashMap<String, WindowRuntime>,
    pending_events: Vec<BackendEvent>,
    state_path: PathBuf,
    proxy: EventLoopProxy<UserEvent>,
    frontend_ready: bool,
}

impl AppRuntime {
    fn new(proxy: EventLoopProxy<UserEvent>) -> std::io::Result<Self> {
        let state_path = workspace_state_path();
        let workspace = WorkspaceState::from_persisted(load_workspace_state(&state_path)?);
        Ok(Self {
            workspace,
            runtimes: HashMap::new(),
            pending_events: Vec::new(),
            state_path,
            proxy,
            frontend_ready: false,
        })
    }

    fn bootstrap(&mut self) {
        let windows = self.workspace.persisted().windows.clone();
        for window in windows {
            if let Some(event) =
                self.start_window(&window.id, window.preset, window.geometry.clone())
            {
                self.pending_events.push(event);
            }
        }
        let _ = self.persist();
    }

    fn handle_frontend_event(&mut self, event: FrontendEvent) -> Vec<BackendEvent> {
        match event {
            FrontendEvent::FrontendReady => {
                self.frontend_ready = true;
                let mut events = vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }];
                events.append(&mut self.pending_events);
                events
            }
            FrontendEvent::CreateWindow { preset } => {
                let window = self.workspace.add_window(preset);
                let runtime_event =
                    self.start_window(&window.id, window.preset, window.geometry.clone());
                let _ = self.persist();
                let mut events = vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }];
                if let Some(event) = runtime_event {
                    events.push(event);
                }
                events
            }
            FrontendEvent::FocusWindow { id } => {
                if !self.workspace.focus_window(&id) {
                    return Vec::new();
                }
                let _ = self.persist();
                vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }]
            }
            FrontendEvent::UpdateViewport { viewport } => {
                self.workspace.update_viewport(viewport);
                let _ = self.persist();
                vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }]
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
                vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }]
            }
            FrontendEvent::CloseWindow { id } => {
                if let Some(runtime) = self.runtimes.remove(&id) {
                    if let Ok(pane) = runtime.pane.lock() {
                        let _ = pane.kill();
                    }
                }
                if !self.workspace.close_window(&id) {
                    return Vec::new();
                }
                let _ = self.persist();
                vec![BackendEvent::WorkspaceState {
                    workspace: self.workspace.persisted().clone(),
                }]
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
        }
    }

    fn handle_runtime_output(&mut self, id: String, data: Vec<u8>) -> Vec<BackendEvent> {
        vec![BackendEvent::TerminalOutput {
            id,
            data_base64: base64::engine::general_purpose::STANDARD.encode(data),
        }]
    }

    fn handle_runtime_status(
        &mut self,
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    ) -> Vec<BackendEvent> {
        self.workspace.set_status(&id, status.clone());
        if matches!(
            status,
            WindowProcessStatus::Error | WindowProcessStatus::Exited
        ) {
            self.runtimes.remove(&id);
        }
        let _ = self.persist();

        vec![
            BackendEvent::WorkspaceState {
                workspace: self.workspace.persisted().clone(),
            },
            BackendEvent::TerminalStatus { id, status, detail },
        ]
    }

    fn start_window(
        &mut self,
        id: &str,
        preset: WindowPreset,
        geometry: WindowGeometry,
    ) -> Option<BackendEvent> {
        let shell = match detect_shell_program() {
            Ok(shell) => shell,
            Err(error) => {
                self.workspace.set_status(id, WindowProcessStatus::Error);
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
            std::env::current_dir().ok(),
        ) {
            Ok(pane) => Arc::new(Mutex::new(pane)),
            Err(error) => {
                self.workspace.set_status(id, WindowProcessStatus::Error);
                return Some(BackendEvent::TerminalStatus {
                    id: id.to_string(),
                    status: WindowProcessStatus::Error,
                    detail: Some(error.to_string()),
                });
            }
        };

        self.spawn_output_thread(id.to_string(), pane.clone());
        self.workspace.set_status(id, WindowProcessStatus::Running);
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

fn flush_events(webview: &wry::WebView, app: &mut AppRuntime, events: Vec<BackendEvent>) {
    if app.frontend_ready {
        for event in events {
            let _ = dispatch_event(webview, &event);
        }
    } else {
        app.pending_events.extend(events);
    }
}

fn dispatch_event(webview: &wry::WebView, event: &BackendEvent) -> wry::Result<()> {
    let payload = serde_json::to_string(event).expect("backend event json");
    webview.evaluate_script(&format!("window.__POC__?.receive({payload});"))
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut app = AppRuntime::new(proxy).expect("app runtime");
    app.bootstrap();

    let window = WindowBuilder::new()
        .with_title("gwt terminal poc")
        .with_inner_size(tao::dpi::LogicalSize::new(1440.0, 920.0))
        .build(&event_loop)
        .expect("window");

    let builder = WebViewBuilder::new()
        .with_html(include_str!("../web/index.html"))
        .with_ipc_handler({
            let proxy = app.proxy.clone();
            move |request: wry::http::Request<String>| match serde_json::from_str::<FrontendEvent>(
                request.body(),
            ) {
                Ok(event) => {
                    let _ = proxy.send_event(UserEvent::Frontend(event));
                }
                Err(error) => {
                    eprintln!("invalid frontend message: {error}");
                }
            }
        });

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

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UserEvent::Frontend(event)) => {
                let events = app.handle_frontend_event(event);
                flush_events(&webview, &mut app, events);
            }
            Event::UserEvent(UserEvent::RuntimeOutput { id, data }) => {
                let events = app.handle_runtime_output(id, data);
                flush_events(&webview, &mut app, events);
            }
            Event::UserEvent(UserEvent::RuntimeStatus { id, status, detail }) => {
                let events = app.handle_runtime_status(id, status, detail);
                flush_events(&webview, &mut app, events);
            }
            _ => {}
        }
    });
}

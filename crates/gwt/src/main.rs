use std::{
    collections::HashMap,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc as std_mpsc, Arc, Mutex, RwLock},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::repo_browser::{preferred_issue_launch_branch, spawn_branch_load_async};
use base64::Engine;
use gwt::{
    cleanup_selected_branches, default_wizard_version_cache_path, detect_shell_program,
    list_branch_entries_with_active_sessions, list_directory_entries, load_agent_options,
    load_knowledge_bridge, load_restored_workspace_state, load_session_state,
    migrate_legacy_workspace_state, refresh_managed_gwt_assets_for_worktree, resolve_launch_spec,
    save_session_state, save_workspace_state, workspace_state_path, BackendEvent,
    BranchEntriesPhase, BranchListEntry, DockerWizardContext, FrontendEvent, HookForwardTarget,
    KnowledgeKind, LaunchWizardCompletion, LaunchWizardContext, LaunchWizardHydration,
    LaunchWizardLaunchRequest, LaunchWizardState, LiveSessionEntry, ShellLaunchConfig,
    WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState, APP_NAME,
};
use gwt_terminal::{Pane, PaneStatus, PtyHandle};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};
use tokio::runtime::Runtime;
use uuid::Uuid;
use wry::WebViewBuilder;

mod app_runtime;
mod docker_launch;
mod embedded_server;
mod embedded_web;
mod launch_runtime;
mod repo_browser;
mod runtime_support;
mod update_front_door;

#[cfg(test)]
pub(crate) use app_runtime::{build_frontend_sync_events, LaunchWizardSession};
pub(crate) use app_runtime::{
    ActiveAgentSession, AgentLaunchResult, AppEventProxy, AppRuntime, BlockingTaskSpawner,
    DispatchTarget, IssueLaunchWizardPrepared, OutboundEvent, ProcessLaunch, ProjectOpenTarget,
    ProjectTabRuntime, WindowAddress,
};
pub(crate) use docker_launch::{
    apply_docker_runtime_to_launch_config, detect_wizard_docker_context_and_status,
    docker_binary_for_launch, docker_compose_exec_env_args, ensure_docker_gwt_binary_setup,
    ensure_docker_launch_service_ready, finalize_docker_agent_launch_config,
    package_runner_version_spec, resolve_docker_launch_plan, resolve_docker_shell_command,
    strip_package_runner_args, PackageRunnerProgram,
};
#[cfg(test)]
pub(crate) use docker_launch::{
    compose_workspace_mount_target, docker_bundle_mounts_for_home, docker_bundle_override_content,
    docker_compose_file_for_launch, docker_devcontainer_defaults, is_valid_docker_env_key,
    mount_source_matches_project_root, normalize_docker_launch_action, DockerLaunchServiceAction,
};
#[cfg(test)]
use embedded_server::{broadcast_runtime_hook_event, health_handler, hook_forward_authorized};
use embedded_server::{ClientHub, EmbeddedServer};
pub(crate) use launch_runtime::{
    apply_host_package_runner_fallback, apply_windows_host_shell_wrapper,
    build_shell_process_launch, ensure_docker_launch_runtime_ready, install_launch_gwt_bin_env,
    resolve_launch_worktree, resolve_shell_launch_worktree,
};
#[cfg(test)]
pub(crate) use launch_runtime::{
    apply_host_package_runner_fallback_with_probe, command_matches_runner,
    install_launch_gwt_bin_env_with_lookup, probe_host_package_runner_with_timeout,
    resolve_launch_worktree_request,
};
pub(crate) use runtime_support::{
    app_state_view_from_parts, close_window_from_workspace, combined_window_id, current_git_branch,
    dedupe_recent_projects, fallback_project_target, first_available_worktree_path,
    front_door_route, geometry_to_pty_size, knowledge_kind_for_preset, local_branch_exists,
    normalize_active_tab_id, normalize_branch_name, origin_remote_ref,
    resolve_launch_spec_with_fallback, resolve_launch_wizard_hydration, resolve_project_target,
    run_cli, same_worktree_path, should_auto_close_agent_window, should_auto_start_restored_window,
    spawn_env, synthetic_branch_entry, workspace_view_for_tab,
};
#[cfg(test)]
pub(crate) use runtime_support::{
    branch_worktree_path, parse_github_remote_url, suffixed_worktree_path,
    worktree_path_is_occupied,
};
pub(crate) use update_front_door::{apply_update_and_exit, spawn_startup_update_check};
#[cfg(test)]
pub(crate) use update_front_door::{classify_startup_update_state, StartupUpdateAction};

type ClientId = String;
const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";
const DOCKER_GWTD_BIN_PATH: &str = "/usr/local/bin/gwtd";
const DOCKER_HOST_GWT_BIN_NAME: &str = "gwt-linux";
const DOCKER_HOST_GWTD_BIN_NAME: &str = "gwtd-linux";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuiFrontDoorLaunchSurface<'a> {
    browser_url: &'a str,
    webview_url: &'a str,
}

fn gui_front_door_launch_surface(server_url: &str) -> GuiFrontDoorLaunchSurface<'_> {
    GuiFrontDoorLaunchSurface {
        browser_url: server_url,
        webview_url: server_url,
    }
}

fn logging_dir_for_startup_path(startup_path: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_logs_dir_for_project_path(startup_path)
}

fn broadcast_log_entry(clients: &ClientHub, entry: gwt_core::logging::LogEvent) {
    clients.dispatch(vec![OutboundEvent::broadcast(
        BackendEvent::LogEntryAppended { entry },
    )]);
}

fn spawn_project_index_status_check(runtime: &Runtime, proxy: EventLoopProxy<UserEvent>) {
    let project_root = std::env::current_dir().ok();
    drop(runtime.spawn(async move {
        let status = match project_root {
            Some(path) => tokio::task::spawn_blocking(move || {
                gwt::index_worker::project_index_status_for_path(&path)
            })
            .await
            .unwrap_or_else(|err| gwt::ProjectIndexStatusView {
                state: "error".to_string(),
                detail: format!("Project index status task failed: {err}"),
            }),
            None => gwt::ProjectIndexStatusView {
                state: "skipped".to_string(),
                detail: "No current directory".to_string(),
            },
        };
        let _ = proxy.send_event(UserEvent::ProjectIndexStatus { status });
    }));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DockerBundleMounts {
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
    LogEntry {
        entry: gwt_core::logging::LogEvent,
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
    RuntimeHook(gwt::RuntimeHookEvent),
    LaunchProgress {
        window_id: String,
        message: String,
    },
    ProjectIndexStatus {
        status: gwt::ProjectIndexStatusView,
    },
    LaunchComplete {
        window_id: String,
        result: AgentLaunchResult,
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, Mutex, RwLock},
        time::{Duration, Instant},
    };

    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
    use base64::Engine;
    use chrono::Utc;
    use tempfile::tempdir;

    use gwt::{
        empty_workspace_state, AgentOption, ArrangeMode, BackendEvent, BranchCleanupInfo,
        BranchListEntry, BranchScope, CanvasViewport, FocusCycleDirection, KnowledgeKind,
        LaunchWizardAction, LaunchWizardContext, LaunchWizardState, PersistedWindowState,
        ProjectKind, QuickStartEntry, QuickStartLaunchMode, RuntimeHookEvent, RuntimeHookEventKind,
        ShellLaunchConfig, WindowGeometry, WindowPreset, WindowProcessStatus, WorkspaceState,
    };
    use gwt_agent::{AgentId, AgentLaunchBuilder, DockerLifecycleIntent, LaunchRuntimeTarget};
    use gwt_core::logging::{LogEvent, LogLevel};
    use gwt_core::update::UpdateState;
    use gwt_terminal::PaneStatus;

    use super::{
        app_state_view_from_parts, apply_host_package_runner_fallback_with_probe,
        apply_windows_host_shell_wrapper, broadcast_log_entry, broadcast_runtime_hook_event,
        build_frontend_sync_events, build_shell_process_launch, close_window_from_workspace,
        combined_window_id, current_git_branch, docker_bundle_mounts_for_home,
        docker_bundle_override_content, gui_front_door_launch_surface, hook_forward_authorized,
        install_launch_gwt_bin_env_with_lookup, knowledge_kind_for_preset,
        logging_dir_for_startup_path, resolve_project_target, should_auto_close_agent_window,
        should_auto_start_restored_window, ActiveAgentSession, AppEventProxy, AppRuntime,
        BlockingTaskSpawner, ClientHub, DispatchTarget, LaunchWizardSession, OutboundEvent,
        ProcessLaunch, ProjectTabRuntime, UserEvent, WindowAddress,
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
            agent_id: None,
            agent_color: None,
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

    #[test]
    fn log_entry_broadcast_reaches_all_registered_clients() {
        let clients = ClientHub::default();
        let mut native = clients.register("native".to_string());
        let mut browser = clients.register("browser".to_string());

        broadcast_log_entry(
            &clients,
            LogEvent::new(LogLevel::Warn, "pty", "reader stalled").with_detail("retrying"),
        );

        let native_payload = native.try_recv().expect("native payload");
        let browser_payload = browser.try_recv().expect("browser payload");
        assert_eq!(native_payload, browser_payload);
        assert!(native_payload.contains("\"kind\":\"log_entry_appended\""));
        assert!(native_payload.contains("\"severity\":\"Warn\""));
        assert!(native_payload.contains("\"message\":\"reader stalled\""));
    }

    #[test]
    fn logging_dir_for_startup_path_uses_project_scoped_fallback() {
        let temp = tempdir().expect("tempdir");
        let log_dir = logging_dir_for_startup_path(temp.path());
        let project_hash = gwt_core::repo_hash::compute_path_hash(temp.path());

        assert!(log_dir.ends_with(
            Path::new("projects")
                .join(project_hash.as_str())
                .join("logs")
        ));
    }

    #[test]
    fn logging_initialization_sources_do_not_use_legacy_log_dir() {
        let legacy_helper = ["gwt_core::paths::", "gwt_logs_dir", "()"].concat();
        let forbidden_init = [
            "LoggingConfig::new(",
            "gwt_core::paths::",
            "gwt_logs_dir",
            "()",
        ]
        .concat();

        let main_source = include_str!("main.rs");
        let runtime_source = include_str!("app_runtime.rs");

        assert!(
            !main_source.contains(&forbidden_init),
            "main logging initialization must use the project-scoped canonical resolver"
        );
        assert!(
            !runtime_source.contains(&legacy_helper),
            "AppRuntime log snapshots must use the project-scoped canonical resolver"
        );
    }

    #[test]
    fn gui_front_door_launch_surface_reuses_same_server_url_for_browser_and_native_webview() {
        let surface = gui_front_door_launch_surface("http://127.0.0.1:44557/");

        assert_eq!(surface.browser_url, "http://127.0.0.1:44557/");
        assert_eq!(surface.webview_url, "http://127.0.0.1:44557/");
    }

    #[test]
    fn gui_front_door_launch_surface_shares_one_embedded_bundle_contract() {
        let surface = gui_front_door_launch_surface("http://127.0.0.1:44557/");
        let html = crate::embedded_web::index_html();
        let app_js = crate::embedded_web::app_js();

        assert_eq!(surface.browser_url, surface.webview_url);
        assert!(
            html.contains("<script type=\"module\" src=\"/app.js\"></script>"),
            "expected browser and native front door modes to point at the same embedded frontend bundle entrypoint",
        );
        assert!(
            !app_js.contains("window.__POC__"),
            "expected browser and native front door modes to avoid the retired PoC debug export",
        );
        assert!(
            app_js.contains("frontendUnits.socketTransport.connect();"),
            "expected the shared embedded bundle to bootstrap socket transport once for both front door modes",
        );
    }

    fn drain_client_payloads(receiver: &mut tokio::sync::mpsc::Receiver<String>) -> Vec<String> {
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
        let log_dir = temp_root.join("logs");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::create_dir_all(&log_dir).expect("create log dir");
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
            launch_wizard: None,
            active_agent_sessions: HashMap::new(),
            window_pty_statuses: HashMap::new(),
            window_hook_states: HashMap::new(),
            hook_forward_target: None,
            issue_link_cache_dir: gwt_core::paths::gwt_cache_dir(),
            pending_update: None,
            pty_writers: Arc::new(RwLock::new(HashMap::new())),
        };
        runtime.rebuild_window_lookup();
        runtime.seed_window_pty_statuses();
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
            custom_agent: None,
        }]
    }

    fn sample_wizard_quick_start_entry(live_window_id: Option<&str>) -> QuickStartEntry {
        QuickStartEntry {
            session_id: "gwt-session-1".to_string(),
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("gpt-5.5".to_string()),
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
    fn startup_update_state_classification_covers_publish_stop_and_retry() {
        let checked_at = Utc::now();

        assert!(matches!(
            super::classify_startup_update_state(&UpdateState::Available {
                current: "9.7.0".to_string(),
                latest: "9.7.1".to_string(),
                release_url: "https://example.invalid/releases/9.7.1".to_string(),
                asset_url: Some("https://example.invalid/gwt.zip".to_string()),
                checked_at,
            }),
            super::StartupUpdateAction::Publish
        ));
        assert!(matches!(
            super::classify_startup_update_state(&UpdateState::Available {
                current: "9.7.0".to_string(),
                latest: "9.7.1".to_string(),
                release_url: "https://example.invalid/releases/9.7.1".to_string(),
                asset_url: None,
                checked_at,
            }),
            super::StartupUpdateAction::Stop
        ));
        assert!(matches!(
            super::classify_startup_update_state(&UpdateState::UpToDate { checked_at: None }),
            super::StartupUpdateAction::Stop
        ));
        assert!(matches!(
            super::classify_startup_update_state(&UpdateState::Failed {
                message: "network".to_string(),
                failed_at: checked_at,
            }),
            super::StartupUpdateAction::Retry
        ));
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
        assert!(super::same_worktree_path(
            &runtime.recent_projects[0].path,
            &repo
        ));

        let added = runtime
            .open_project_path(scratch.clone())
            .expect("open new project");

        assert!(!added);
        assert_eq!(runtime.tabs.len(), 3);
        assert!(super::same_worktree_path(
            &runtime.recent_projects[0].path,
            &scratch
        ));
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
            3
        );
        assert_eq!(
            runtime
                .create_window_events(WindowPreset::FileTree, bounds.clone())
                .len(),
            3
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
            Some(WindowProcessStatus::Running)
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
            gwt::KnowledgeListScope::Open,
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
            gwt::KnowledgeListScope::Open,
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
        assert_eq!(error_events.len(), 3);
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
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &claude_one_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            error_events[2].event,
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
            BackendEvent::WindowState { ref window_id, state }
                if window_id == "tab-1::missing" && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            failed_launch[1].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("launch failed")
        ));

        let missing_window_launch = runtime.handle_launch_complete(
            "tab-1::missing".to_string(),
            Ok((
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-3".to_string(),
                "feature/demo".to_string(),
                "Codex".to_string(),
                repo.clone(),
                AgentId::Codex,
                None,
            )),
        );
        assert!(matches!(
            missing_window_launch[0].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == "tab-1::missing" && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            missing_window_launch[1].event,
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
            BackendEvent::WindowState { ref window_id, state }
                if window_id == "tab-1::missing" && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            shell_launch[1].event,
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
            3
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
                    list_scope: None,
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
                    list_scope: None,
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
        assert_eq!(status_events.len(), 3);
        assert!(!runtime.window_details.contains_key(&shell_id));
        assert!(matches!(
            status_events[1].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &shell_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            status_events[2].event,
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
            Ok((
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-1".to_string(),
                "feature/demo".to_string(),
                "Codex".to_string(),
                repo.clone(),
                AgentId::Codex,
                None,
            )),
        );
        assert!(matches!(
            project_missing[0].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &project_missing_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            project_missing[1].event,
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
            Ok((
                ProcessLaunch {
                    command: "echo".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                },
                "session-2".to_string(),
                "feature/demo".to_string(),
                "Codex".to_string(),
                repo.clone(),
                AgentId::Codex,
                None,
            )),
        );
        assert!(matches!(
            raw_missing[0].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &raw_missing_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            raw_missing[1].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Window not found")
        ));

        let shell_project_missing = runtime.handle_shell_launch_complete(
            project_missing_id.clone(),
            Ok(ProcessLaunch {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            }),
        );
        assert!(matches!(
            shell_project_missing[0].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &project_missing_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            shell_project_missing[1].event,
            BackendEvent::TerminalStatus { ref detail, .. }
                if detail.as_deref() == Some("Project tab not found")
        ));

        let shell_raw_missing = runtime.handle_shell_launch_complete(
            raw_missing_id.clone(),
            Ok(ProcessLaunch {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
            }),
        );
        assert!(matches!(
            shell_raw_missing[0].event,
            BackendEvent::WindowState { ref window_id, state }
                if window_id == &raw_missing_id && state == WindowProcessStatus::Error
        ));
        assert!(matches!(
            shell_raw_missing[1].event,
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

    #[test]
    fn local_branch_exists_surfaces_git_errors_for_invalid_repo() {
        let temp = tempdir().expect("tempdir");
        let err = super::local_branch_exists(temp.path(), "feature/missing")
            .expect_err("non-repo path should surface git failure");

        assert!(err.contains("git show-ref --verify refs/heads/feature/missing"));
        assert!(err.contains(&temp.path().display().to_string()));
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

    fn sample_custom_bunx_launch_config() -> gwt_agent::LaunchConfig {
        let mut config = AgentLaunchBuilder::new(AgentId::Custom("claude-code-openai".to_string()))
            .working_dir("E:/gwt/develop")
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
    fn host_package_runner_fallback_switches_custom_bunx_to_npx_when_probe_fails() {
        let mut config = sample_custom_bunx_launch_config();

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

        assert!(changed, "expected custom bunx failure to switch to npx");
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
    fn probe_host_package_runner_times_out_and_returns_false() {
        #[cfg(target_os = "windows")]
        let (command, args) = (
            "cmd",
            vec!["/C".to_string(), "ping -n 6 127.0.0.1 >NUL".to_string()],
        );
        #[cfg(not(target_os = "windows"))]
        let (command, args) = ("sh", vec!["-c".to_string(), "sleep 5".to_string()]);

        let start = Instant::now();
        let ok = crate::probe_host_package_runner_with_timeout(
            command,
            args,
            &HashMap::new(),
            None,
            Duration::from_millis(100),
            Duration::from_millis(10),
        );

        assert!(!ok, "hanging package-runner probe should fail closed");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "probe timeout should return quickly"
        );
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
            windows_shell: None,
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
    fn build_shell_process_launch_for_host_uses_selected_windows_shell() {
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
            env_vars: HashMap::new(),
            windows_shell: Some(gwt_agent::WindowsShellKind::WindowsPowerShell),
        };

        let launch = build_shell_process_launch(&worktree, &mut config).expect("shell launch");

        if cfg!(windows) {
            assert_eq!(launch.command, "powershell");
            assert_eq!(launch.args, vec!["-NoLogo".to_string()]);
        } else {
            // On non-Windows, the defensive platform guard ignores windows_shell
            // and falls back to detect_shell_program().
            assert_ne!(launch.command, "powershell");
            assert_ne!(launch.command, "cmd.exe");
        }
    }

    #[test]
    fn windows_shell_process_command_mapping_is_owned_by_launch_runtime() {
        assert_eq!(
            super::launch_runtime::windows_shell_process_command(
                gwt_agent::WindowsShellKind::CommandPrompt
            ),
            "cmd.exe"
        );
        assert_eq!(
            super::launch_runtime::windows_shell_process_command(
                gwt_agent::WindowsShellKind::WindowsPowerShell
            ),
            "powershell"
        );
        assert_eq!(
            super::launch_runtime::windows_shell_process_command(
                gwt_agent::WindowsShellKind::PowerShell7
            ),
            "pwsh"
        );
    }

    #[test]
    fn command_prompt_agent_wrapper_preserves_spaced_cmd_path() {
        let mut config = sample_versioned_launch_config();
        config.command = r"C:\Program Files\nodejs\npx.cmd".to_string();
        config.args = vec![
            "--yes".to_string(),
            "@anthropic-ai/claude-code@latest".to_string(),
            "value with space".to_string(),
        ];
        config.windows_shell = Some(gwt_agent::WindowsShellKind::CommandPrompt);

        apply_windows_host_shell_wrapper(&mut config).expect("wrap command prompt");

        assert_eq!(config.command, "cmd.exe");
        assert_eq!(
            config.args,
            vec![
                "/d".to_string(),
                "/k".to_string(),
                "%GWT_WINDOWS_HOST_SHELL_EXPRESSION%".to_string()
            ]
        );
        assert_eq!(
            config
                .env_vars
                .get("GWT_WINDOWS_HOST_SHELL_EXPRESSION")
                .map(String::as_str),
            Some(
                r#"call "C:\Program Files\nodejs\npx.cmd" --yes @anthropic-ai/claude-code@latest "value with space" & exit"#
            )
        );
    }

    #[test]
    fn powershell_agent_wrapper_quotes_spaced_path_and_single_quotes() {
        let mut config = sample_versioned_launch_config();
        config.command = r"C:\Program Files\nodejs\npx.cmd".to_string();
        config.args = vec!["value's".to_string()];
        config.windows_shell = Some(gwt_agent::WindowsShellKind::PowerShell7);

        apply_windows_host_shell_wrapper(&mut config).expect("wrap powershell");

        assert_eq!(config.command, "pwsh");
        assert_eq!(config.args[0], "-NoLogo");
        assert_eq!(config.args[1], "-NoProfile");
        assert_eq!(config.args[2], "-Command");
        let script = config.args[3].as_str();
        assert!(script.contains(r"& 'C:\Program Files\nodejs\npx.cmd'"));
        assert!(script.contains("'value''s'"));
        assert!(script.contains("exit $LASTEXITCODE"));
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
                assert_eq!(command, "gwtd");
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
    fn docker_bundle_override_content_mounts_gwtd_only_for_agents() {
        let home = PathBuf::from("/home/example");
        let bundle = docker_bundle_mounts_for_home(&home);
        let content = docker_bundle_override_content("app", &bundle);

        assert!(content.contains("/home/example/.gwt/bin/gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!content.contains("/usr/local/bin/gwt:ro"));
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
        assert_eq!(defaults.compose_files.len(), 1);
        assert!(super::same_worktree_path(
            &defaults.compose_files[0],
            &compose_file
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
            platform: None,
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
    fn docker_launch_plan_merges_devcontainer_compose_files_and_rebases_relative_mounts() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let devcontainer_dir = project.join(".devcontainer");
        fs::create_dir_all(&devcontainer_dir).expect("create devcontainer dir");
        let base = devcontainer_dir.join("base.yml");
        let override_file = devcontainer_dir.join("override.yml");
        fs::write(
            &base,
            "services:\n  app:\n    image: alpine:3.19\n    volumes:\n      - ../:/workspace/base\n",
        )
        .expect("write base compose");
        fs::write(
            &override_file,
            "services:\n  app:\n    volumes:\n      - ../:/workspace/override\n",
        )
        .expect("write override compose");
        fs::write(
            devcontainer_dir.join("devcontainer.json"),
            r#"{
  "dockerComposeFile": ["base.yml", "override.yml"],
  "service": "app"
}"#,
        )
        .expect("write devcontainer");

        let plan = super::resolve_docker_launch_plan(&project, None).expect("launch plan");
        assert_eq!(plan.compose_files.len(), 2);
        assert!(super::same_worktree_path(&plan.compose_files[0], &base));
        assert!(super::same_worktree_path(
            &plan.compose_files[1],
            &override_file
        ));
        assert!(super::same_worktree_path(&plan.compose_file, &base));
        assert_eq!(plan.container_cwd, "/workspace/override");
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

        let detach = Command::new("git")
            .args(["checkout", "--detach", "HEAD"])
            .current_dir(&repo)
            .status()
            .expect("detach head");
        assert!(detach.success(), "git checkout --detach failed");

        let err = super::resolve_launch_worktree_request(
            &repo,
            Some("feature/detached"),
            None,
            &mut None,
            &mut HashMap::new(),
        )
        .expect_err("repo branch resolution failure should not silently skip");
        assert!(err.contains("git branch --show-current"));

        let attach = Command::new("git")
            .args(["checkout", "develop"])
            .current_dir(&repo)
            .status()
            .expect("reattach head");
        assert!(attach.success(), "git checkout develop failed");

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
        assert!(existing_dir
            .as_deref()
            .is_some_and(|value| super::same_worktree_path(value, &existing_worktree)));
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

        let local_only_branch = Command::new("git")
            .args(["branch", "feature/local-only"])
            .current_dir(&repo)
            .status()
            .expect("create local-only branch");
        assert!(
            local_only_branch.success(),
            "create local-only branch failed"
        );

        let mut local_only_dir = None;
        let mut local_only_env = HashMap::new();
        super::resolve_launch_worktree_request(
            &repo,
            Some("feature/local-only"),
            Some("develop"),
            &mut local_only_dir,
            &mut local_only_env,
        )
        .expect("local-only worktree");
        let local_only_dir = local_only_dir.expect("local-only worktree dir");
        let local_remote = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                "refs/remotes/origin/feature/local-only",
            ])
            .current_dir(&repo)
            .status()
            .expect("check local-only remote ref");
        assert_eq!(
            local_remote.code(),
            Some(1),
            "local-only branch should not publish origin/feature/local-only from base branch"
        );
        assert!(local_only_env
            .get("GWT_PROJECT_ROOT")
            .is_some_and(|value| super::same_worktree_path(Path::new(value), &local_only_dir)));

        let mut launch_config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .branch("feature/existing")
            .base_branch("develop")
            .build();
        launch_config.working_dir = None;
        launch_config.env_vars = HashMap::new();
        super::resolve_launch_worktree(&repo, &mut launch_config).expect("agent launch wrapper");
        assert!(launch_config
            .working_dir
            .as_deref()
            .is_some_and(|value| super::same_worktree_path(value, &existing_worktree)));

        let mut shell_config = ShellLaunchConfig {
            working_dir: None,
            branch: Some("feature/existing".to_string()),
            base_branch: Some("develop".to_string()),
            display_name: "Shell".to_string(),
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            env_vars: HashMap::new(),
            windows_shell: None,
        };
        super::resolve_shell_launch_worktree(&repo, &mut shell_config)
            .expect("shell launch wrapper");
        assert!(shell_config
            .working_dir
            .as_deref()
            .is_some_and(|value| super::same_worktree_path(value, &existing_worktree)));
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
    fn finalize_docker_agent_launch_config_wraps_runtime_command_in_compose_exec() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/app\n",
        )
        .expect("write compose file");

        let mut config = sample_versioned_launch_config();
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.working_dir = Some(project.clone());
        config.docker_service = Some("app".to_string());
        config
            .env_vars
            .insert("EXTRA_FLAG".to_string(), "1".to_string());

        super::finalize_docker_agent_launch_config(&project, &mut config)
            .expect("finalize docker launch");

        assert_eq!(config.command, super::docker_binary_for_launch());
        assert_eq!(
            config.args,
            vec![
                "compose".to_string(),
                "-f".to_string(),
                project.join("docker-compose.yml").display().to_string(),
                "exec".to_string(),
                "-w".to_string(),
                "/workspace/app".to_string(),
                "-e".to_string(),
                "EXTRA_FLAG=1".to_string(),
                "-e".to_string(),
                "TERM=xterm-256color".to_string(),
                "app".to_string(),
                "bunx".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ]
        );
    }

    #[test]
    fn finalize_docker_agent_launch_config_includes_override_file_when_present() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/app\n",
        )
        .expect("write compose file");
        fs::write(
            project.join("docker-compose.override.yml"),
            "services:\n  app:\n    environment:\n      EXTRA: 1\n",
        )
        .expect("write override file");

        let mut config = sample_versioned_launch_config();
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.working_dir = Some(project.clone());
        config.docker_service = Some("app".to_string());

        super::finalize_docker_agent_launch_config(&project, &mut config)
            .expect("finalize docker launch");

        assert_eq!(
            config.args[..6],
            [
                "compose".to_string(),
                "-f".to_string(),
                project.join("docker-compose.yml").display().to_string(),
                "-f".to_string(),
                project
                    .join("docker-compose.override.yml")
                    .display()
                    .to_string(),
                "exec".to_string(),
            ]
        );
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
            platform: None,
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

fn main() -> wry::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if !matches!(
        front_door_route(&argv),
        runtime_support::FrontDoorRoute::Gui
    ) {
        if let Err(error) = run_cli(&argv) {
            eprintln!("CLI dispatch failed: {error}");
            std::process::exit(1);
        }
    }

    let startup_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let _gui_instance_lock = match gwt::gui_single_instance::acquire_gui_instance_lock(
        &gwt_core::paths::gwt_home(),
        &startup_dir,
    ) {
        Ok(lock) => lock,
        Err(error @ gwt::gui_single_instance::GuiInstanceLockError::AlreadyRunning { .. }) => {
            eprintln!("gwt GUI startup failed: {error}");
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("gwt GUI startup failed: {error}");
            std::process::exit(1);
        }
    };
    let log_dir = logging_dir_for_startup_path(&startup_dir);

    // Install the tracing subscriber so that `tracing::debug!/info!` lands in
    // the startup project's canonical `gwt.log.YYYY-MM-DD`. The returned guard
    // must outlive the event loop; we bind it to `log_handles` and keep it
    // until `main` returns.
    //
    // Diagnostic trace for intermittent key-input drop (bugfix/input-key) is
    // emitted at `debug` level under `target: "gwt_input_trace"`. Enable with
    // `RUST_LOG=gwt_input_trace=debug`.
    let mut log_handles = gwt_core::logging::init(gwt_core::logging::LoggingConfig::new(log_dir))
        .map_err(|error| {
            eprintln!("gwt logging init failed: {error}");
        })
        .ok();

    if let Err(error) = gwt::cli::prepare_daemon_front_door_for_path(&startup_dir) {
        eprintln!("gwt daemon bootstrap: {error}");
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
    if let Some(log_handles) = log_handles.as_mut() {
        if let Some(mut ui_rx) = log_handles.take_ui_rx() {
            let log_proxy = proxy.clone();
            drop(runtime.handle().spawn(async move {
                while let Some(entry) = ui_rx.recv().await {
                    let _ = log_proxy.send_event(UserEvent::LogEntry { entry });
                }
            }));
        }
    }

    let mut server = EmbeddedServer::start(
        &runtime,
        AppEventProxy::new(proxy.clone()),
        clients.clone(),
        pty_writers.clone(),
    )
    .expect("embedded server");
    app.set_hook_forward_target(server.hook_forward_target());
    let front_door = gui_front_door_launch_surface(server.url());
    eprintln!("gwt browser URL: {}", front_door.browser_url);

    // Startup update check (T-031): keep only the wiring here.
    spawn_startup_update_check(&runtime, clients.clone(), proxy.clone());
    spawn_project_index_status_check(&runtime, proxy.clone());

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

    let builder = WebViewBuilder::new().with_url(front_door.webview_url);

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
                let refresh_index_status = matches!(event, FrontendEvent::FrontendReady);
                let events = app.handle_frontend_event(client_id, event);
                clients.dispatch(events);
                if refresh_index_status {
                    spawn_project_index_status_check(&runtime, proxy.clone());
                }
            }
            Event::UserEvent(UserEvent::LogEntry { entry }) => {
                broadcast_log_entry(&clients, entry);
            }
            Event::UserEvent(UserEvent::RuntimeOutput { id, data }) => {
                let events = app.handle_runtime_output(id, data);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::RuntimeStatus { id, status, detail }) => {
                let events = app.handle_runtime_status(id, status, detail);
                clients.dispatch(events);
            }
            Event::UserEvent(UserEvent::RuntimeHook(event)) => {
                let events = app.handle_runtime_hook_event(event);
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
            Event::UserEvent(UserEvent::ProjectIndexStatus { status }) => {
                clients.dispatch(vec![OutboundEvent::broadcast(
                    BackendEvent::ProjectIndexStatus { status },
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

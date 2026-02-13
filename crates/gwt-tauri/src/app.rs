//! Tauri app wiring (builder configuration + run event handling)

use crate::state::AppState;
use std::sync::atomic::Ordering;
use tauri::Manager;
use tauri::{Emitter, WebviewWindowBuilder};
use tracing::{info, warn};

#[cfg(not(test))]
use gwt_core::config::os_env;

#[cfg(not(test))]
use gwt_core::config::mcp_registration;

#[cfg(any(not(test), target_os = "macos"))]
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

fn should_prevent_window_close(is_quitting: bool) -> bool {
    !is_quitting
}

fn should_prevent_exit_request(is_quitting: bool) -> bool {
    !is_quitting
}

#[cfg(any(not(test), target_os = "macos"))]
fn has_running_agents(state: &AppState) -> bool {
    let Ok(mut manager) = state.pane_manager.lock() else {
        return false;
    };

    for pane in manager.panes_mut() {
        let _ = pane.check_status();
        if matches!(pane.status(), gwt_core::terminal::pane::PaneStatus::Running) {
            return true;
        }
    }

    false
}

#[cfg(any(not(test), target_os = "macos"))]
fn try_begin_exit_confirm(state: &AppState) -> bool {
    state
        .exit_confirm_inflight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

#[cfg(any(not(test), target_os = "macos"))]
fn end_exit_confirm(state: &AppState) {
    state.exit_confirm_inflight.store(false, Ordering::SeqCst);
}

fn menu_action_from_id(id: &str) -> Option<&'static str> {
    match id {
        crate::menu::MENU_ID_FILE_OPEN_PROJECT => Some("open-project"),
        crate::menu::MENU_ID_FILE_CLOSE_PROJECT => Some("close-project"),
        crate::menu::MENU_ID_GIT_CLEANUP_WORKTREES => Some("cleanup-worktrees"),
        crate::menu::MENU_ID_GIT_VERSION_HISTORY => Some("version-history"),
        crate::menu::MENU_ID_TOOLS_LAUNCH_AGENT => Some("launch-agent"),
        crate::menu::MENU_ID_TOOLS_LIST_TERMINALS => Some("list-terminals"),
        crate::menu::MENU_ID_TOOLS_TERMINAL_DIAGNOSTICS => Some("terminal-diagnostics"),
        crate::menu::MENU_ID_SETTINGS_PREFERENCES => Some("open-settings"),
        crate::menu::MENU_ID_HELP_ABOUT => Some("about"),
        _ => None,
    }
}

#[cfg_attr(test, allow(dead_code))]
fn show_best_window(app: &tauri::AppHandle<tauri::Wry>) {
    let Some(window) = best_window(app) else {
        recreate_main_window(app);
        return;
    };
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg_attr(test, allow(dead_code))]
fn recreate_main_window(app: &tauri::AppHandle<tauri::Wry>) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
            return;
        }

        let mut conf = match app.config().app.windows.first() {
            Some(c) => c.clone(),
            None => {
                info!(
                    category = "tauri",
                    event = "MainWindowConfigMissing",
                    "No window config found; skipping main window recreation"
                );
                return;
            }
        };
        conf.label = "main".to_string();

        let builder = WebviewWindowBuilder::from_config(&app, &conf);
        let window = match builder.and_then(|b| b.build()) {
            Ok(w) => w,
            Err(err) => {
                warn!(
                    category = "tauri",
                    event = "MainWindowRecreateFailed",
                    error = %err,
                    "Failed to recreate main window"
                );
                return;
            }
        };

        let _ = window.show();
        let _ = window.set_focus();
        let _ = crate::menu::rebuild_menu(&app);
    });
}

#[cfg_attr(test, allow(dead_code))]
fn best_window(app: &tauri::AppHandle<tauri::Wry>) -> Option<tauri::WebviewWindow<tauri::Wry>> {
    // Prefer the focused window.
    if let Some((_, w)) = app
        .webview_windows()
        .into_iter()
        .find(|(_, w)| w.is_focused().ok() == Some(true))
    {
        return Some(w);
    }

    // Next, prefer "main" if present.
    if let Some(w) = app.get_webview_window("main") {
        return Some(w);
    }

    // Finally, fall back to any existing window.
    app.webview_windows().into_iter().next().map(|(_, w)| w)
}

pub fn build_app(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
) -> tauri::Builder<tauri::Wry> {
    let builder = builder.manage(app_state);

    // Plugins are not required for unit tests and may rely on runtime features.
    #[cfg(not(test))]
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build());

    builder
        .setup(|_app| {
            #[cfg(not(test))]
            {
                // Clean up stale MCP state from a previous crash (FR-019)
                crate::mcp_ws_server::cleanup_stale_state_file();

                // Start MCP WebSocket server (FR-001, FR-005)
                {
                    let app_handle = _app.handle().clone();
                    let state = _app.state::<AppState>();
                    let mcp_handle_slot = state.mcp_ws_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        match crate::mcp_ws_server::start(app_handle).await {
                            Ok(handle) => {
                                tracing::info!(
                                    category = "mcp",
                                    port = handle.port,
                                    "MCP WebSocket server ready"
                                );
                                if let Ok(mut slot) = mcp_handle_slot.lock() {
                                    *slot = Some(handle);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    category = "mcp",
                                    error = %e,
                                    "Failed to start MCP WebSocket server"
                                );
                            }
                        }
                    });
                }

                // Native menubar (SPEC-4470704f)
                let _ = crate::menu::rebuild_menu(_app.handle());

                // System tray (SPEC-dfb1611a FR-310〜FR-313)
                let tray_menu = tauri::menu::Menu::new(_app)?;
                let show_item =
                    tauri::menu::MenuItem::with_id(_app, "tray-show", "Show", true, None::<&str>)?;
                let quit_item =
                    tauri::menu::MenuItem::with_id(_app, "tray-quit", "Quit", true, None::<&str>)?;
                tray_menu.append_items(&[&show_item, &quit_item])?;

                // NOTE: Requires `tauri` features `tray-icon` + `image-png`.
                // macOS: use a template icon so the system can tint it appropriately.
                // Others: use a high-contrast 2-tone icon for light/dark tray backgrounds.
                #[cfg(target_os = "macos")]
                let tray_icon_bytes = include_bytes!("../icons/trayTemplate.png");
                #[cfg(not(target_os = "macos"))]
                let tray_icon_bytes = include_bytes!("../icons/tray.png");
                let icon = tauri::image::Image::from_bytes(tray_icon_bytes)?;

                let _tray = tauri::tray::TrayIconBuilder::with_id("gwt-tray")
                    .icon(icon)
                    .tooltip("gwt")
                    .menu(&tray_menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "tray-show" => {
                            show_best_window(app);
                        }
                        "tray-quit" => {
                            let state = app.state::<AppState>();
                            if !has_running_agents(&state) {
                                state.request_quit();
                                app.exit(0);
                                return;
                            }

                            if !try_begin_exit_confirm(&state) {
                                return;
                            }

                            let app_handle = app.clone();
                            app.dialog()
                                .message("Agents are still running. Quit gwt anyway?")
                                .kind(MessageDialogKind::Warning)
                                .buttons(MessageDialogButtons::OkCancelCustom(
                                    "Quit".to_string(),
                                    "Cancel".to_string(),
                                ))
                                .show(move |ok| {
                                    let state = app_handle.state::<AppState>();
                                    end_exit_confirm(&state);
                                    if ok {
                                        state.request_quit();
                                        app_handle.exit(0);
                                    }
                                });
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_best_window(tray.app_handle());
                        }
                    })
                    .build(_app)?;

                #[cfg(target_os = "macos")]
                _tray.set_icon_as_template(true)?;

                // MCP bridge: cleanup stale registrations then register for all agents (T21).
                // Delay briefly so login-shell PATH capture can complete first.
                {
                    let app_handle = _app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_handle.state::<AppState>();
                        let resource_dir = app_handle.path().resource_dir().ok();
                        let _ = state.wait_os_env_ready(std::time::Duration::from_secs(2));

                        if let Err(e) = mcp_registration::cleanup_stale_registrations() {
                            warn!(
                                category = "mcp",
                                error = %e,
                                "Failed to cleanup stale MCP registrations"
                            );
                        }

                        match mcp_registration::detect_runtime() {
                            Ok(runtime) => {
                                match mcp_registration::resolve_bridge_path(resource_dir.as_deref())
                                {
                                    Ok(bridge_path) => {
                                        let config = mcp_registration::McpBridgeConfig {
                                            command: runtime,
                                            args: vec![bridge_path.to_string_lossy().into_owned()],
                                            env: std::collections::HashMap::new(),
                                        };
                                        if let Err(e) = mcp_registration::register_all(&config) {
                                            warn!(
                                                category = "mcp",
                                                error = %e,
                                                "Failed to register MCP server in agent configs"
                                            );
                                        } else {
                                            info!(
                                                category = "mcp",
                                                "MCP bridge server registered in all agent configs"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        info!(
                                            category = "mcp",
                                            error = %e,
                                            "MCP bridge JS not found; skipping registration"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                info!(
                                    category = "mcp",
                                    error = %e,
                                    "No JS runtime found; skipping MCP registration"
                                );
                            }
                        }
                    });
                }

                // Background task: capture login shell environment
                {
                    let state = _app.state::<AppState>();
                    let os_env_cell = state.os_env.clone();
                    let os_env_source_cell = state.os_env_source.clone();
                    let app_handle_clone = _app.handle().clone();

                    tauri::async_runtime::spawn(async move {
                        let result = os_env::capture_login_shell_env().await;

                        match &result.source {
                            os_env::EnvSource::LoginShell => {
                                tracing::info!(
                                    category = "os_env",
                                    count = result.env.len(),
                                    "Captured login shell environment"
                                );
                            }
                            os_env::EnvSource::ProcessEnv => {
                                tracing::info!(
                                    category = "os_env",
                                    count = result.env.len(),
                                    "Using process environment"
                                );
                            }
                            os_env::EnvSource::StdEnvFallback { reason } => {
                                tracing::warn!(
                                    category = "os_env",
                                    reason = %reason,
                                    "Login shell env capture failed, using process env fallback"
                                );
                                let _ = app_handle_clone.emit("os-env-fallback", reason.clone());
                            }
                        };

                        let _ = os_env_source_cell.set(result.source);
                        let _ = os_env_cell.set(result.env);
                    });
                }
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();

            if id == crate::menu::MENU_ID_FILE_NEW_WINDOW {
                open_new_window(app);
                return;
            }

            if let Some(project_path) = crate::menu::parse_recent_project_menu_id(id) {
                let action = format!("open-recent-project::{}", project_path);
                emit_menu_action(app, &action);
                return;
            }

            if let Some(target) = crate::menu::parse_window_focus_menu_id(id) {
                if let Some(w) = app.get_webview_window(target) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = crate::menu::rebuild_menu(app);
                return;
            }

            if let Some(tab_id) = crate::menu::parse_window_tab_focus_menu_id(id) {
                emit_menu_action(app, &format!("focus-agent-tab::{tab_id}"));
                return;
            }

            let Some(action) = menu_action_from_id(id) else {
                return;
            };
            emit_menu_action(app, action);
        })
        .on_window_event(|window, event| {
            // Keep the process alive when the user clicks the window close button (x).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                info!(
                    category = "tauri",
                    event = "CloseRequested",
                    "Window close requested"
                );
                let state = window.app_handle().state::<AppState>();
                let is_quitting = state.is_quitting.load(Ordering::SeqCst);

                if !should_prevent_window_close(is_quitting) {
                    return;
                }

                // Allow specific windows to actually close (used for macOS Cmd+Q behavior).
                if state.consume_window_close_permission(window.label()) {
                    info!(
                        category = "tauri",
                        event = "CloseAllowed",
                        "Window close allowed"
                    );
                    return;
                }

                api.prevent_close();
                let _ = window.hide();
                let _ = crate::menu::rebuild_menu(window.app_handle());
            }

            if let tauri::WindowEvent::Focused(true) = event {
                let _ = crate::menu::rebuild_menu(window.app_handle());
            }

            if let tauri::WindowEvent::Destroyed = event {
                window
                    .app_handle()
                    .state::<AppState>()
                    .clear_project_for_window(window.label());
                let _ = crate::menu::rebuild_menu(window.app_handle());

                // Exit the app when all windows are truly closed (hidden windows still count as open).
                let app_handle = window.app_handle().clone();
                let destroyed_label = window.label().to_string();
                let remaining_windows = app_handle
                    .webview_windows()
                    .into_iter()
                    .filter(|(label, _)| label != &destroyed_label)
                    .count();
                if remaining_windows == 0 {
                    let state = app_handle.state::<AppState>();
                    let is_quitting = state.is_quitting.load(Ordering::SeqCst);
                    if is_quitting {
                        info!(
                            category = "tauri",
                            event = "AllWindowsClosed",
                            "All windows closed; exiting app"
                        );
                        state.request_quit();
                        app_handle.exit(0);
                    } else {
                        info!(
                            category = "tauri",
                            event = "AllWindowsClosed",
                            "All windows closed; keeping app running"
                        );
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::greet,
            crate::commands::branches::list_branches,
            crate::commands::branches::list_worktree_branches,
            crate::commands::branches::list_remote_branches,
            crate::commands::branches::get_current_branch,
            crate::commands::project::open_project,
            crate::commands::project::probe_path,
            crate::commands::project::create_project,
            crate::commands::project::start_migration_job,
            crate::commands::project::close_project,
            crate::commands::project::get_project_info,
            crate::commands::project::is_git_repo,
            crate::commands::project::quit_app,
            crate::commands::docker::detect_docker_context,
            crate::commands::sessions::get_branch_quick_start,
            crate::commands::sessions::get_branch_session_summary,
            crate::commands::branch_suggest::suggest_branch_names,
            crate::commands::terminal::launch_terminal,
            crate::commands::terminal::launch_agent,
            crate::commands::terminal::start_launch_job,
            crate::commands::terminal::cancel_launch_job,
            crate::commands::terminal::write_terminal,
            crate::commands::terminal::send_keys_to_pane,
            crate::commands::terminal::send_keys_broadcast,
            crate::commands::terminal::resize_terminal,
            crate::commands::terminal::close_terminal,
            crate::commands::terminal::list_terminals,
            crate::commands::terminal::probe_terminal_ansi,
            crate::commands::terminal::capture_scrollback_tail,
            crate::commands::agent_mode::get_agent_mode_state_cmd,
            crate::commands::agent_mode::send_agent_mode_message,
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
            crate::commands::agents::detect_agents,
            crate::commands::agents::list_agent_versions,
            crate::commands::agent_config::get_agent_config,
            crate::commands::agent_config::save_agent_config,
            crate::commands::profiles::get_profiles,
            crate::commands::profiles::save_profiles,
            crate::commands::profiles::list_ai_models,
            crate::commands::cleanup::list_worktrees,
            crate::commands::cleanup::cleanup_worktrees,
            crate::commands::cleanup::cleanup_single_worktree,
            crate::commands::hooks::check_and_update_hooks,
            crate::commands::hooks::register_hooks,
            crate::commands::terminal::get_captured_environment,
            crate::commands::terminal::is_os_env_ready,
            crate::commands::git_view::get_git_change_summary,
            crate::commands::git_view::get_branch_diff_files,
            crate::commands::git_view::get_file_diff,
            crate::commands::git_view::get_branch_commits,
            crate::commands::git_view::get_working_tree_status,
            crate::commands::git_view::get_stash_list,
            crate::commands::git_view::get_base_branch_candidates,
            crate::commands::version_history::list_project_versions,
            crate::commands::version_history::get_project_version_history,
            crate::commands::window_tabs::sync_window_agent_tabs,
            crate::commands::recent_projects::get_recent_projects,
            crate::commands::issue::fetch_github_issues,
            crate::commands::issue::check_gh_cli_status,
            crate::commands::issue::find_existing_issue_branch,
            crate::commands::issue::link_branch_to_issue,
            crate::commands::issue::rollback_issue_branch,
        ])
}

fn focused_window_label(app: &tauri::AppHandle<tauri::Wry>) -> String {
    app.webview_windows()
        .into_iter()
        .find_map(|(label, w)| w.is_focused().ok().and_then(|f| f.then_some(label)))
        .or_else(|| app.get_webview_window("main").map(|_| "main".to_string()))
        .or_else(|| {
            app.webview_windows()
                .into_iter()
                .next()
                .map(|(label, _)| label)
        })
        .unwrap_or_else(|| "main".to_string())
}

fn emit_menu_action(app: &tauri::AppHandle<tauri::Wry>, action: &str) {
    let label = focused_window_label(app);
    let Some(window) = app
        .get_webview_window(&label)
        .or_else(|| app.get_webview_window("main"))
    else {
        return;
    };

    let _ = window.emit(
        crate::menu::MENU_ACTION_EVENT,
        crate::menu::MenuActionPayload {
            action: action.to_string(),
        },
    );
}

fn open_new_window(app: &tauri::AppHandle<tauri::Wry>) {
    let app = app.clone();
    let label = format!("project-{}", uuid::Uuid::new_v4());

    // NOTE: On Windows, window creation can deadlock in synchronous handlers.
    // Create the window on a separate thread (Tauri docs).
    std::thread::spawn(move || {
        let mut conf = match app.config().app.windows.first() {
            Some(c) => c.clone(),
            None => {
                info!(
                    category = "tauri",
                    event = "NewWindowConfigMissing",
                    "No window config found; skipping new window"
                );
                return;
            }
        };
        conf.label = label.clone();

        let builder = WebviewWindowBuilder::from_config(&app, &conf);
        let window = match builder.and_then(|b| b.build()) {
            Ok(w) => w,
            Err(err) => {
                info!(
                    category = "tauri",
                    event = "NewWindowFailed",
                    error = %err,
                    "Failed to create new window"
                );
                return;
            }
        };

        let _ = window.show();
        let _ = window.set_focus();
        let _ = crate::menu::rebuild_menu(&app);
    });
}

pub fn handle_run_event(app_handle: &tauri::AppHandle<tauri::Wry>, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            info!(
                category = "tauri",
                event = "ExitRequested",
                "Exit requested"
            );
            // SPEC-dfb1611a FR-314: only the tray "Quit" is allowed to exit.
            let is_quitting = app_handle
                .state::<AppState>()
                .is_quitting
                .load(Ordering::SeqCst);

            if should_prevent_exit_request(is_quitting) {
                api.prevent_exit();

                // macOS: Cmd+Q is treated as explicit app quit (with agent confirmation).
                #[cfg(target_os = "macos")]
                {
                    let state = app_handle.state::<AppState>();
                    if has_running_agents(&state) {
                        if !try_begin_exit_confirm(&state) {
                            return;
                        }

                        let app_handle = app_handle.clone();
                        app_handle
                            .dialog()
                            .message("Agents are still running. Quit gwt anyway?")
                            .kind(MessageDialogKind::Warning)
                            .buttons(MessageDialogButtons::OkCancelCustom(
                                "Quit".to_string(),
                                "Cancel".to_string(),
                            ))
                            .show(move |ok| {
                                let state = app_handle.state::<AppState>();
                                end_exit_confirm(&state);
                                if !ok {
                                    return;
                                }
                                state.request_quit();
                                app_handle.exit(0);
                            });
                        return;
                    }

                    app_handle.state::<AppState>().request_quit();
                    app_handle.exit(0);
                }

                // Other OSes: keep current behavior (exit request hides to tray).
                #[cfg(not(target_os = "macos"))]
                {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        info!(
                            category = "tauri",
                            event = "ExitPrevented",
                            "Exit prevented; hiding main window"
                        );
                        let _ = window.hide();
                    }
                }
            }
        }
        tauri::RunEvent::Exit => {
            info!(category = "tauri", event = "Exit", "App exiting");

            // T22: Unregister MCP bridge from all agent configs on exit
            #[cfg(not(test))]
            if let Err(e) = gwt_core::config::unregister_all_mcp() {
                warn!(
                    category = "mcp",
                    error = %e,
                    "Failed to unregister MCP server on exit"
                );
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            // Ensure the app is recoverable from dock reopen even if the window is hidden.
            if !has_visible_windows {
                show_best_window(app_handle);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_prevent_window_close_when_not_quitting() {
        assert!(should_prevent_window_close(false));
        assert!(!should_prevent_window_close(true));
    }

    #[test]
    fn should_prevent_exit_request_when_not_quitting() {
        assert!(should_prevent_exit_request(false));
        assert!(!should_prevent_exit_request(true));
    }

    #[test]
    fn app_state_request_quit_sets_flag() {
        let state = AppState::new();
        assert!(!state.is_quitting.load(Ordering::SeqCst));
        state.request_quit();
        assert!(state.is_quitting.load(Ordering::SeqCst));
    }

    #[test]
    fn menu_action_from_id_maps_git_cleanup() {
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_GIT_CLEANUP_WORKTREES),
            Some("cleanup-worktrees")
        );
    }
}

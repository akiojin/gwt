//! Tauri app wiring (builder configuration + run event handling)

use crate::state::AppState;
use std::sync::atomic::Ordering;
use tauri::Manager;
use tauri::{Emitter, WebviewWindowBuilder};
use tracing::info;

#[cfg(not(test))]
use gwt_core::config::os_env;

fn should_prevent_window_close(is_quitting: bool) -> bool {
    !is_quitting
}

fn should_prevent_exit_request(is_quitting: bool) -> bool {
    !is_quitting
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
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "tray-quit" => {
                            app.state::<AppState>().request_quit();
                            app.exit(0);
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
                            if let Some(window) = tray.app_handle().get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    })
                    .build(_app)?;

                #[cfg(target_os = "macos")]
                _tray.set_icon_as_template(true)?;

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

            if let Some(target) = crate::menu::parse_window_focus_menu_id(id) {
                if let Some(w) = app.get_webview_window(target) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = crate::menu::rebuild_menu(app);
                return;
            }

            let action = match id {
                crate::menu::MENU_ID_FILE_OPEN_PROJECT => Some("open-project"),
                crate::menu::MENU_ID_FILE_CLOSE_PROJECT => Some("close-project"),
                crate::menu::MENU_ID_TOOLS_LAUNCH_AGENT => Some("launch-agent"),
                crate::menu::MENU_ID_TOOLS_LIST_TERMINALS => Some("list-terminals"),
                crate::menu::MENU_ID_TOOLS_TERMINAL_DIAGNOSTICS => Some("terminal-diagnostics"),
                crate::menu::MENU_ID_SETTINGS_PREFERENCES => Some("open-settings"),
                crate::menu::MENU_ID_HELP_ABOUT => Some("about"),
                _ => None,
            };

            let Some(action) = action else { return };
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
                let is_quitting = window
                    .app_handle()
                    .state::<AppState>()
                    .is_quitting
                    .load(Ordering::SeqCst);

                if !should_prevent_window_close(is_quitting) {
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
            }
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::greet,
            crate::commands::branches::list_branches,
            crate::commands::branches::list_worktree_branches,
            crate::commands::branches::list_remote_branches,
            crate::commands::branches::get_current_branch,
            crate::commands::project::open_project,
            crate::commands::project::create_project,
            crate::commands::project::close_project,
            crate::commands::project::get_project_info,
            crate::commands::project::is_git_repo,
            crate::commands::docker::detect_docker_context,
            crate::commands::sessions::get_branch_quick_start,
            crate::commands::sessions::get_branch_session_summary,
            crate::commands::branch_suggest::suggest_branch_names,
            crate::commands::terminal::launch_terminal,
            crate::commands::terminal::launch_agent,
            crate::commands::terminal::write_terminal,
            crate::commands::terminal::resize_terminal,
            crate::commands::terminal::close_terminal,
            crate::commands::terminal::list_terminals,
            crate::commands::terminal::probe_terminal_ansi,
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
            crate::commands::agents::detect_agents,
            crate::commands::agents::list_agent_versions,
            crate::commands::agent_config::get_agent_config,
            crate::commands::agent_config::save_agent_config,
            crate::commands::profiles::get_profiles,
            crate::commands::profiles::save_profiles,
            crate::commands::cleanup::list_worktrees,
            crate::commands::cleanup::cleanup_worktrees,
            crate::commands::cleanup::cleanup_single_worktree,
            crate::commands::terminal::get_captured_environment,
            crate::commands::terminal::is_os_env_ready,
            crate::commands::git_view::get_git_change_summary,
            crate::commands::git_view::get_branch_diff_files,
            crate::commands::git_view::get_file_diff,
            crate::commands::git_view::get_branch_commits,
            crate::commands::git_view::get_working_tree_status,
            crate::commands::git_view::get_stash_list,
            crate::commands::git_view::get_base_branch_candidates,
        ])
}

fn focused_window_label(app: &tauri::AppHandle<tauri::Wry>) -> String {
    app.webview_windows()
        .into_iter()
        .find_map(|(label, w)| w.is_focused().ok().and_then(|f| f.then_some(label)))
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
        tauri::RunEvent::Exit => {
            info!(category = "tauri", event = "Exit", "App exiting");
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            // Ensure the app is recoverable from dock reopen even if the window is hidden.
            if !has_visible_windows {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
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
}

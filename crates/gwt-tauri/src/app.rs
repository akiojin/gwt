//! Tauri app wiring (builder configuration + run event handling)

use crate::state::AppState;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tauri::{Emitter, EventTarget, WebviewWindowBuilder};
use tracing::{info, warn};

#[cfg(not(test))]
use gwt_core::config::os_env;

#[cfg(not(test))]
use gwt_core::config::{skill_registration, Settings};
#[cfg(not(test))]
use tokio::io::AsyncReadExt;

#[cfg(not(test))]
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

fn should_prevent_window_close(is_quitting: bool) -> bool {
    !is_quitting
}

#[derive(Clone, Copy)]
enum WindowSwitchDirection {
    Next,
    Previous,
}

fn resolve_window_switch_target(
    state: &AppState,
    focused_label: &str,
    direction: WindowSwitchDirection,
) -> Option<String> {
    if state.project_for_window(focused_label).is_some() {
        return match direction {
            WindowSwitchDirection::Next => state.next_window(),
            WindowSwitchDirection::Previous => state.previous_window(),
        };
    }

    match direction {
        WindowSwitchDirection::Next => state.most_recent_window(),
        WindowSwitchDirection::Previous => state.least_recent_window(),
    }
}

fn should_prevent_exit_request(is_quitting: bool) -> bool {
    !is_quitting
}

fn captured_path_from_env(env: &HashMap<String, String>) -> Option<String> {
    env.get("PATH")
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[cfg_attr(test, allow(dead_code))]
pub(crate) fn apply_captured_path_to_process_env(env: &HashMap<String, String>) -> bool {
    let Some(path) = captured_path_from_env(env) else {
        return false;
    };
    std::env::set_var("PATH", path);
    true
}

#[cfg(not(test))]
pub(crate) fn spawn_login_shell_env_capture(app_handle: tauri::AppHandle<tauri::Wry>) {
    {
        let state = app_handle.state::<AppState>();
        if !state.begin_os_env_capture() {
            tracing::info!(
                category = "os_env",
                "Login shell environment capture already running; skip duplicate request"
            );
            return;
        }
    }

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
                let _ = app_handle.emit("os-env-fallback", reason.clone());
            }
        };

        if apply_captured_path_to_process_env(&result.env) {
            tracing::info!(
                category = "os_env",
                "Updated process PATH from captured environment"
            );
        }

        let source = result.source;
        let env = result.env;
        let state = app_handle.state::<AppState>();
        state.set_os_env_snapshot(env, source);
    });
}

#[cfg(not(test))]
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

#[cfg(not(test))]
fn try_begin_exit_confirm(state: &AppState) -> bool {
    state
        .exit_confirm_inflight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

#[cfg(not(test))]
fn end_exit_confirm(state: &AppState) {
    state.exit_confirm_inflight.store(false, Ordering::SeqCst);
}

fn menu_action_from_id(id: &str) -> Option<&'static str> {
    match id {
        crate::menu::MENU_ID_FILE_OPEN_PROJECT => Some("open-project"),
        crate::menu::MENU_ID_FILE_CLOSE_PROJECT => Some("close-project"),
        crate::menu::MENU_ID_GIT_CLEANUP_WORKTREES => Some("cleanup-worktrees"),
        crate::menu::MENU_ID_GIT_VERSION_HISTORY => Some("version-history"),
        crate::menu::MENU_ID_GIT_PULL_REQUESTS => Some("git-pull-requests"),
        crate::menu::MENU_ID_GIT_ISSUES => Some("git-issues"),
        crate::menu::MENU_ID_EDIT_COPY => Some("edit-copy"),
        crate::menu::MENU_ID_EDIT_PASTE => Some("edit-paste"),
        crate::menu::MENU_ID_EDIT_COPY_SCREEN => Some("screen-copy"),
        crate::menu::MENU_ID_TOOLS_NEW_TERMINAL => Some("new-terminal"),
        crate::menu::MENU_ID_TOOLS_LAUNCH_AGENT => Some("launch-agent"),
        crate::menu::MENU_ID_TOOLS_LIST_TERMINALS => Some("list-terminals"),
        crate::menu::MENU_ID_TOOLS_TERMINAL_DIAGNOSTICS => Some("terminal-diagnostics"),
        crate::menu::MENU_ID_TOOLS_PROJECT_INDEX => Some("project-index"),
        crate::menu::MENU_ID_SETTINGS_PREFERENCES => Some("open-settings"),
        crate::menu::MENU_ID_HELP_ABOUT => Some("about"),
        crate::menu::MENU_ID_HELP_CHECK_UPDATES => Some("check-updates"),
        crate::menu::MENU_ID_HELP_REPORT_ISSUE => Some("report-issue"),
        crate::menu::MENU_ID_HELP_SUGGEST_FEATURE => Some("suggest-feature"),
        crate::menu::MENU_ID_WINDOW_PREVIOUS_TAB => Some("previous-tab"),
        crate::menu::MENU_ID_WINDOW_NEXT_TAB => Some("next-tab"),
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

#[allow(deprecated)]
pub fn build_app(
    builder: tauri::Builder<tauri::Wry>,
    app_state: AppState,
    _single_instance_guard: Option<Arc<crate::single_instance::SingleInstanceGuard>>,
) -> tauri::Builder<tauri::Wry> {
    let builder = builder.manage(app_state);
    #[cfg(not(test))]
    let single_instance_guard = _single_instance_guard.clone();

    // Plugins are not required for unit tests and may rely on runtime features.
    #[cfg(not(test))]
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::default().build());

    builder
        .setup(move |_app| {
            #[cfg(not(test))]
            {
                if let Some(guard) = single_instance_guard.as_ref() {
                    spawn_single_instance_focus_listener(_app.handle().clone(), guard.clone());
                }

                // Native menubar (SPEC-4470704f)
                if let Err(e) = crate::menu::rebuild_menu(_app.handle()) {
                    warn!(category = "menu", error = %e, "Failed to build initial menu");
                } else {
                    info!(category = "menu", "Initial native menu built");
                }

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

                // Startup shell environment behavior.
                // On Unix, capture from login shell to get PATH extensions (nvm, pyenv, etc.).
                // On Windows, use process environment directly.
                {
                    #[cfg(unix)]
                    {
                        info!(
                            category = "os_env",
                            mode = "login_shell",
                            "Capturing environment from login shell"
                        );
                        spawn_login_shell_env_capture(_app.handle().clone());
                    }
                    #[cfg(not(unix))]
                    {
                        let state = _app.state::<AppState>();
                        state.set_os_env_process_env_snapshot();
                        info!(
                            category = "os_env",
                            mode = "process_env",
                            "Using process environment"
                        );
                    }
                }

                // Skill bundles: ensure managed skills are registered for supported agents.
                {
                    let app_handle = _app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let settings = match Settings::load_global() {
                            Ok(settings) => settings,
                            Err(err) => {
                                warn!(
                                    category = "skills",
                                    error = %err,
                                    "Failed to load settings before startup skill repair; using defaults"
                                );
                                Settings::default()
                            }
                        };
                        let status =
                            skill_registration::repair_skill_registration_with_settings(&settings);
                        let state = app_handle.state::<AppState>();
                        state.set_skill_registration_status(status.clone());
                        match status.overall.as_str() {
                            "ok" => {
                                info!(
                                    category = "skills",
                                    "Managed skills are registered for all supported agents"
                                );
                            }
                            _ => {
                                warn!(
                                    category = "skills",
                                    overall = %status.overall,
                                    error = %status
                                        .last_error_message
                                        .clone()
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    "Skill registration is degraded"
                                );
                            }
                        }
                    });
                }

                // Background task: check gh CLI authentication (SPEC-ad1ac432 T009)
                {
                    let app_handle = _app.handle().clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let available = gwt_core::git::gh_cli::check_auth();
                        let state = app_handle.state::<AppState>();
                        state
                            .gh_available
                            .store(available, std::sync::atomic::Ordering::Relaxed);
                        tracing::info!(
                            category = "gh_cli",
                            available = available,
                            "gh CLI authentication check completed"
                        );
                    });
                }

                // Background task: watch session files for agent status changes (SPEC-b80e7996 FR-820)
                {
                    let watcher_handle = _app.handle().clone();
                    if let Err(e) = crate::session_watcher::start_session_watcher(watcher_handle) {
                        warn!(
                            category = "session_watcher",
                            error = %e,
                            "Failed to start session watcher (agent status updates will use polling fallback)"
                        );
                    }
                }

                // Background task: check app update (best-effort, TTL cached).
                {
                    let mgr = _app.state::<AppState>().update_manager.clone();
                    let app_handle_clone = _app.handle().clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let current_exe = std::env::current_exe().ok();
                        let state = mgr.check_for_executable(false, current_exe.as_deref());
                        if let gwt_core::update::UpdateState::Failed { message, .. } = &state {
                            warn!(
                                category = "update",
                                force = false,
                                source = "startup-event",
                                error = %message,
                                "Startup update check failed"
                            );
                        }
                        let _ = app_handle_clone.emit("app-update-state", &state);
                    });
                }
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            info!(category = "menu", id = id, "Native menu event received");

            if id == crate::menu::MENU_ID_FILE_NEW_WINDOW {
                open_new_window(app);
                return;
            }

            // Window switching (rotation order)
            if id == crate::menu::MENU_ID_WINDOW_NEXT_WINDOW {
                let state = app.state::<AppState>();
                let focused_label = focused_window_label(app);
                if let Some(target) =
                    resolve_window_switch_target(&state, &focused_label, WindowSwitchDirection::Next)
                {
                    if let Some(w) = app.get_webview_window(&target) {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                return;
            }
            if id == crate::menu::MENU_ID_WINDOW_PREVIOUS_WINDOW {
                let state = app.state::<AppState>();
                let focused_label = focused_window_label(app);
                if let Some(target) = resolve_window_switch_target(
                    &state,
                    &focused_label,
                    WindowSwitchDirection::Previous,
                ) {
                    if let Some(w) = app.get_webview_window(&target) {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                return;
            }

            // macOS standard window items
            #[cfg(target_os = "macos")]
            {
                if id == crate::menu::MENU_ID_WINDOW_MINIMIZE {
                    if let Some(w) = focused_webview_window(app) {
                        let _ = w.minimize();
                    }
                    return;
                }
                if id == crate::menu::MENU_ID_WINDOW_ZOOM {
                    if let Some(w) = focused_webview_window(app) {
                        if w.is_maximized().unwrap_or(false) {
                            let _ = w.unmaximize();
                        } else {
                            let _ = w.maximize();
                        }
                    }
                    return;
                }
                if id == crate::menu::MENU_ID_WINDOW_BRING_ALL_TO_FRONT {
                    let focused_label = focused_window_label(app);

                    for (_, w) in app.webview_windows() {
                        let _ = w.show();
                    }

                    // Restore focus once to keep MRU/history deterministic.
                    if let Some(w) = app.get_webview_window(&focused_label) {
                        let _ = w.set_focus();
                    }
                    let _ = crate::menu::rebuild_menu(app);
                    return;
                }
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

                // Allow explicit close requests from trusted flows if explicitly permitted.
                if state.consume_window_close_permission(window.label()) {
                    info!(
                        category = "tauri",
                        event = "CloseAllowed",
                        "Window close allowed"
                    );
                    return;
                }

                let _ = window.emit("window-will-hide", ());
                api.prevent_close();
                let _ = window.hide();
                state.remove_window_from_history(window.label());
                let _ = crate::menu::rebuild_menu(window.app_handle());
            }

            if let tauri::WindowEvent::Focused(true) = event {
                let state = window.app_handle().state::<AppState>();
                if state.project_for_window(window.label()).is_some() {
                    state.push_window_focus(window.label());
                }
                let _ = crate::menu::rebuild_menu(window.app_handle());
            }

            if let tauri::WindowEvent::Destroyed = event {
                let state = window.app_handle().state::<AppState>();
                state.clear_window_state(window.label());
                state.remove_window_from_history(window.label());
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
            crate::commands::project::cancel_quit_confirm,
            crate::commands::docker::detect_docker_context,
            crate::commands::sessions::get_branch_quick_start,
            crate::commands::sessions::get_agent_sidebar_view,
            crate::commands::sessions::get_branch_session_summary,
            crate::commands::sessions::rebuild_all_branch_session_summaries,
            crate::commands::branch_suggest::suggest_branch_name,
            crate::commands::branch_suggest::is_ai_configured,
            crate::commands::terminal::launch_terminal,
            crate::commands::terminal::spawn_shell,
            crate::commands::terminal::launch_agent,
            crate::commands::terminal::start_launch_job,
            crate::commands::terminal::cancel_launch_job,
            crate::commands::terminal::poll_launch_job,
            crate::commands::terminal::write_terminal,
            crate::commands::terminal::send_keys_to_pane,
            crate::commands::terminal::send_keys_broadcast,
            crate::commands::terminal::resize_terminal,
            crate::commands::terminal::close_terminal,
            crate::commands::terminal::list_terminals,
            crate::commands::terminal::probe_terminal_ansi,
            crate::commands::terminal::capture_scrollback_tail,
            crate::commands::terminal::terminal_ready,
            crate::commands::project_mode::get_project_mode_state_cmd,
            crate::commands::project_mode::send_project_mode_message,
            crate::commands::project_mode::send_project_mode_message_cmd,
            crate::commands::project_mode::restore_project_mode_session_cmd,
            crate::commands::project_mode::list_project_mode_sessions_cmd,
            crate::commands::project_mode::stop_project_mode_session_cmd,
            crate::commands::skills::get_skill_registration_status_cmd,
            crate::commands::skills::repair_skill_registration_cmd,
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
            crate::commands::clause_docs::check_and_fix_agent_instruction_docs,
            crate::commands::voice::get_voice_capability,
            crate::commands::voice::ensure_voice_runtime,
            crate::commands::voice::prepare_voice_model,
            crate::commands::voice::transcribe_voice_audio,
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
            crate::commands::cleanup::check_gh_available,
            crate::commands::cleanup::get_cleanup_pr_statuses,
            crate::commands::cleanup::get_cleanup_settings,
            crate::commands::cleanup::set_cleanup_settings,
            crate::commands::update::check_app_update,
            crate::commands::update::apply_app_update,
            crate::commands::terminal::get_captured_environment,
            crate::commands::terminal::is_os_env_ready,
            crate::commands::terminal::get_available_shells,
            crate::commands::git_view::get_git_change_summary,
            crate::commands::git_view::get_branch_diff_files,
            crate::commands::git_view::get_file_diff,
            crate::commands::git_view::get_branch_commits,
            crate::commands::git_view::get_working_tree_status,
            crate::commands::git_view::get_stash_list,
            crate::commands::git_view::get_base_branch_candidates,
            crate::commands::version_history::list_project_versions,
            crate::commands::version_history::get_project_version_history,
            crate::commands::version_history::prefetch_version_history,
            crate::commands::window_tabs::sync_window_agent_tabs,
            crate::commands::window::get_current_window_label,
            crate::commands::window::open_gwt_window,
            crate::commands::window::try_acquire_window_restore_leader,
            crate::commands::window::release_window_restore_leader,
            crate::commands::recent_projects::get_recent_projects,
            crate::commands::issue::fetch_github_issues,
            crate::commands::issue::fetch_github_issue_detail,
            crate::commands::issue::fetch_branch_linked_issue,
            crate::commands::issue::check_gh_cli_status,
            crate::commands::issue::find_existing_issue_branch,
            crate::commands::issue::link_branch_to_issue,
            crate::commands::issue::rollback_issue_branch,
            crate::commands::issue::classify_issue_branch_prefix,
            crate::commands::issue_spec::create_spec_issue_cmd,
            crate::commands::issue_spec::update_spec_issue_cmd,
            crate::commands::issue_spec::upsert_spec_issue_cmd,
            crate::commands::issue_spec::get_spec_issue_detail_cmd,
            crate::commands::issue_spec::find_spec_issue_by_spec_id_cmd,
            crate::commands::issue_spec::append_spec_contract_comment_cmd,
            crate::commands::issue_spec::upsert_spec_issue_artifact_comment_cmd,
            crate::commands::issue_spec::list_spec_issue_artifact_comments_cmd,
            crate::commands::issue_spec::delete_spec_issue_artifact_comment_cmd,
            crate::commands::issue_spec::close_spec_issue_cmd,
            crate::commands::issue_spec::sync_spec_issue_project_cmd,
            crate::commands::pullrequest::fetch_pr_status,
            crate::commands::pullrequest::fetch_pr_detail,
            crate::commands::pullrequest::fetch_latest_branch_pr,
            crate::commands::pullrequest::fetch_ci_log,
            crate::commands::report::read_recent_logs,
            crate::commands::report::get_report_system_info,
            crate::commands::report::detect_report_target,
            crate::commands::report::create_github_issue,
            crate::commands::report::capture_screen_text,
            crate::commands::pullrequest::update_pr_branch,
            crate::commands::pullrequest::merge_pull_request,
            crate::commands::pullrequest::fetch_pr_list,
            crate::commands::pullrequest::fetch_github_user,
            crate::commands::pullrequest::merge_pr,
            crate::commands::pullrequest::review_pr,
            crate::commands::pullrequest::mark_pr_ready,
            crate::commands::system::get_system_info,
            crate::commands::system::get_stats,
            crate::commands::project_index::ensure_index_runtime,
            crate::commands::project_index::index_project_cmd,
            crate::commands::project_index::search_project_index_cmd,
            crate::commands::project_index::get_index_status_cmd,
        ])
}

#[cfg(not(test))]
fn spawn_single_instance_focus_listener(
    app_handle: tauri::AppHandle<tauri::Wry>,
    guard: Arc<crate::single_instance::SingleInstanceGuard>,
) {
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) => {
                warn!(
                    category = "single_instance",
                    error = %err,
                    "Failed to bind focus listener"
                );
                return;
            }
        };

        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(err) => {
                warn!(
                    category = "single_instance",
                    error = %err,
                    "Failed to resolve focus listener address"
                );
                return;
            }
        };

        if let Err(err) = guard.set_focus_port(Some(port)) {
            warn!(
                category = "single_instance",
                error = %err,
                "Failed to publish focus listener endpoint"
            );
            return;
        }

        info!(
            category = "single_instance",
            port = port,
            "Focus listener ready"
        );

        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(v) => v,
                Err(err) => {
                    warn!(
                        category = "single_instance",
                        error = %err,
                        "Focus listener accept failed"
                    );
                    break;
                }
            };

            let mut buf = [0u8; 64];
            let bytes_read = match socket.read(&mut buf).await {
                Ok(n) => n,
                Err(err) => {
                    warn!(
                        category = "single_instance",
                        error = %err,
                        "Focus listener read failed"
                    );
                    continue;
                }
            };
            if bytes_read == 0 {
                continue;
            }

            let payload = String::from_utf8_lossy(&buf[..bytes_read]);
            if payload.contains("focus") {
                show_best_window(&app_handle);
            }
        }
    });
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

#[cfg(target_os = "macos")]
fn focused_webview_window(
    app: &tauri::AppHandle<tauri::Wry>,
) -> Option<tauri::WebviewWindow<tauri::Wry>> {
    app.webview_windows()
        .into_iter()
        .find_map(|(_, w)| w.is_focused().ok().and_then(|f| f.then_some(w)))
}

fn emit_menu_action(app: &tauri::AppHandle<tauri::Wry>, action: &str) {
    let label = focused_window_label(app);
    let Some(window) = app
        .get_webview_window(&label)
        .or_else(|| app.get_webview_window("main"))
    else {
        warn!(
            category = "menu",
            action = action,
            requested_label = %label,
            "Skipping menu action emit because target window was not found"
        );
        return;
    };

    info!(
        category = "menu",
        action = action,
        target_label = window.label(),
        "Emitting menu action"
    );

    let emit_result = window.emit_to(
        EventTarget::webview_window(window.label()),
        crate::menu::MENU_ACTION_EVENT,
        crate::menu::MenuActionPayload {
            action: action.to_string(),
        },
    );

    if let Err(err) = emit_result {
        warn!(
            category = "menu",
            action = action,
            target_label = window.label(),
            error = %err,
            "Failed to emit menu action"
        );
    }
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
            let state = app_handle.state::<AppState>();
            let is_quitting = state.is_quitting.load(Ordering::SeqCst);

            if should_prevent_exit_request(is_quitting) {
                // Check if this is the 2nd Cmd+Q within the timeout window
                const QUIT_CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);
                if state.is_quit_confirm_active(QUIT_CONFIRM_TIMEOUT) {
                    // 2nd press within timeout → quit
                    state.cancel_quit_confirm();
                    state.request_quit();
                    app_handle.exit(0);
                    return;
                }

                // 1st press → show toast
                api.prevent_exit();
                state.begin_quit_confirm(QUIT_CONFIRM_TIMEOUT);

                if let Some(window) = best_window(app_handle) {
                    // Show window if hidden (FR-007)
                    let _ = window.show();
                    let _ = window.set_focus();
                    // Emit event to frontmost window only (FR-009)
                    let _ = window.emit_to(
                        EventTarget::webview_window(window.label()),
                        "quit-confirm-show",
                        (),
                    );
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
                show_best_window(app_handle);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_TOOLS_PROJECT_INDEX),
            Some("project-index")
        );
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_HELP_CHECK_UPDATES),
            Some("check-updates")
        );
    }

    #[test]
    fn menu_action_from_id_maps_edit_copy() {
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_EDIT_COPY),
            Some("edit-copy")
        );
    }

    #[test]
    fn menu_action_from_id_maps_edit_paste() {
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_EDIT_PASTE),
            Some("edit-paste")
        );
    }

    #[test]
    fn menu_action_from_id_maps_tab_switching() {
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_WINDOW_PREVIOUS_TAB),
            Some("previous-tab")
        );
        assert_eq!(
            menu_action_from_id(crate::menu::MENU_ID_WINDOW_NEXT_TAB),
            Some("next-tab")
        );
    }

    #[test]
    fn captured_path_from_env_returns_trimmed_path() {
        let env = HashMap::from([("PATH".to_string(), "  /usr/local/bin  ".to_string())]);
        assert_eq!(
            captured_path_from_env(&env),
            Some("/usr/local/bin".to_string())
        );
    }

    #[test]
    fn captured_path_from_env_rejects_missing_or_empty_path() {
        let no_path = HashMap::from([("HOME".to_string(), "/tmp".to_string())]);
        assert_eq!(captured_path_from_env(&no_path), None);

        let empty_path = HashMap::from([("PATH".to_string(), "   ".to_string())]);
        assert_eq!(captured_path_from_env(&empty_path), None);
    }

    #[test]
    fn capabilities_default_allows_event_listen() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = format!("{manifest_dir}/capabilities/default.json");
        let contents = fs::read_to_string(path).expect("read capabilities/default.json");
        let json: serde_json::Value =
            serde_json::from_str(&contents).expect("parse capabilities/default.json");
        let permissions = json
            .get("permissions")
            .and_then(|v| v.as_array())
            .expect("permissions array missing");

        let has_event_default = permissions
            .iter()
            .any(|v| v.as_str() == Some("core:event:default"));
        assert!(
            has_event_default,
            "capabilities/default.json must include core:event:default"
        );

        let windows = json
            .get("windows")
            .and_then(|v| v.as_array())
            .expect("windows array missing");
        let allows_all_windows = windows.iter().any(|v| v.as_str() == Some("*"));
        assert!(
            allows_all_windows,
            "capabilities/default.json must include windows: [\"*\"]"
        );
    }

    #[test]
    fn resolve_window_switch_target_non_project_focus_uses_recent_for_next() {
        let state = AppState::new();
        assert_eq!(
            state.claim_project_for_window_with_identity(
                "A",
                "/tmp/repo-a".to_string(),
                "/tmp/repo-a-id".to_string()
            ),
            Ok(())
        );
        assert_eq!(
            state.claim_project_for_window_with_identity(
                "B",
                "/tmp/repo-b".to_string(),
                "/tmp/repo-b-id".to_string()
            ),
            Ok(())
        );
        state.push_window_focus("A");
        state.push_window_focus("B");
        // History: [B, A]
        assert_eq!(
            resolve_window_switch_target(&state, "new-window", WindowSwitchDirection::Next),
            Some("B".to_string())
        );
    }

    #[test]
    fn resolve_window_switch_target_non_project_focus_uses_oldest_for_previous() {
        let state = AppState::new();
        assert_eq!(
            state.claim_project_for_window_with_identity(
                "A",
                "/tmp/repo-a".to_string(),
                "/tmp/repo-a-id".to_string()
            ),
            Ok(())
        );
        assert_eq!(
            state.claim_project_for_window_with_identity(
                "B",
                "/tmp/repo-b".to_string(),
                "/tmp/repo-b-id".to_string()
            ),
            Ok(())
        );
        state.push_window_focus("A");
        state.push_window_focus("B");
        // History: [B, A]
        assert_eq!(
            resolve_window_switch_target(&state, "new-window", WindowSwitchDirection::Previous),
            Some("A".to_string())
        );
    }
}

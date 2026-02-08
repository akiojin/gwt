#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;
use std::sync::atomic::Ordering;
use tauri::Manager;

fn main() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(app_state)
        .setup(|app| {
            // System tray (SPEC-dfb1611a FR-310〜FR-313)
            let tray_menu = tauri::menu::Menu::new(app)?;
            let show_item =
                tauri::menu::MenuItem::with_id(app, "tray-show", "Show", true, None::<&str>)?;
            let quit_item =
                tauri::menu::MenuItem::with_id(app, "tray-quit", "Quit", true, None::<&str>)?;
            tray_menu.append_items(&[&show_item, &quit_item])?;

            // NOTE: Requires `tauri` features `tray-icon` + `image-png`.
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;

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
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Keep the process alive when the user clicks the window close button (x).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window
                    .app_handle()
                    .state::<AppState>()
                    .is_quitting
                    .load(Ordering::SeqCst)
                {
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::branches::list_branches,
            commands::branches::list_remote_branches,
            commands::branches::get_current_branch,
            commands::project::open_project,
            commands::project::create_project,
            commands::project::get_project_info,
            commands::project::is_git_repo,
            commands::terminal::launch_terminal,
            commands::terminal::write_terminal,
            commands::terminal::resize_terminal,
            commands::terminal::close_terminal,
            commands::terminal::list_terminals,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::agents::detect_agents,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

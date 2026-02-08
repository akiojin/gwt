#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;

fn main() {
    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::branches::list_branches,
            commands::branches::list_remote_branches,
            commands::branches::get_current_branch,
            commands::project::open_project,
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

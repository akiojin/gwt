#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commands;
mod state;

use state::AppState;

fn main() {
    let app_state = AppState::new();

    let app = crate::app::build_app(tauri::Builder::default(), app_state)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(crate::app::handle_run_event);
}

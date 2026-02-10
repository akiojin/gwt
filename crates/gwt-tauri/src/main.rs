#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod commands;
mod menu;
mod state;

use state::AppState;
use std::io::Read;

fn main() {
    // Claude Code hooks call `/Applications/gwt.app/.../gwt-tauri hook <Event>` via `~/.claude/settings.json`.
    // In hook mode we must NOT start the GUI event loop; process stdin JSON and exit immediately.
    if handle_hook_cli() {
        return;
    }

    let app_state = AppState::new();

    let app = crate::app::build_app(tauri::Builder::default(), app_state)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(crate::app::handle_run_event);
}

fn handle_hook_cli() -> bool {
    let mut args = std::env::args();
    let _exe = args.next();

    let Some(subcommand) = args.next() else {
        return false;
    };
    if subcommand != "hook" {
        return false;
    }

    // Missing event name is treated as a no-op to avoid noisy hook errors.
    let Some(event) = args.next() else {
        return true;
    };

    let mut payload = String::new();
    let _ = std::io::stdin().read_to_string(&mut payload);

    // Best-effort: hook errors should not block Claude Code.
    let _ = gwt_core::config::process_claude_hook_event(&event, &payload);
    true
}

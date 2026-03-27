//! Tauri-specific wrapper around `gwt_core::agent::session_watcher`.
//!
//! Delegates file-system watching to the core library and bridges change
//! notifications into Tauri events so the frontend can refresh agent status.

use gwt_core::agent::session_watcher::{start_session_watcher, AGENT_STATUS_CHANGED_EVENT};
use tauri::{AppHandle, Emitter};

/// Start watching `~/.gwt/sessions/` and emit a Tauri event on changes.
///
/// Returns `Ok(())` if the watcher was successfully spawned.
/// The watcher runs until the app exits (channel close).
pub fn start_session_watcher_for_app(app_handle: AppHandle) -> Result<(), String> {
    let _handle = start_session_watcher(Box::new(move || {
        let _ = app_handle.emit(AGENT_STATUS_CHANGED_EVENT, ());
    }))?;

    // Leak the handle intentionally: the watcher should live for the entire
    // app lifetime. The thread exits when the debouncer channel closes at
    // process termination.
    std::mem::forget(_handle);

    Ok(())
}

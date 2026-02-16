//! File-system watcher for `~/.gwt/sessions/` (SPEC-b80e7996 FR-820)
//!
//! Monitors session file changes (written by `gwt-tauri hook <Event>`) and
//! emits a Tauri event so the frontend can refresh agent status indicators.

// In test binaries main() is not called, so the setup path that invokes
// start_session_watcher() appears dead to the compiler.
#![cfg_attr(test, allow(unused))]

use gwt_core::config::Session;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tracing::{debug, warn};

/// Tauri event name emitted when session files change.
pub const AGENT_STATUS_CHANGED_EVENT: &str = "agent-status-changed";

/// Start watching `~/.gwt/sessions/` in a background thread.
///
/// Returns `Ok(())` if the watcher was successfully spawned.
/// The watcher runs until the `AppHandle` is dropped (app exit).
pub fn start_session_watcher(app_handle: AppHandle) -> Result<(), String> {
    let sessions_dir = Session::sessions_dir();

    if !sessions_dir.exists() {
        // Create the directory so the watcher has something to observe.
        let _ = std::fs::create_dir_all(&sessions_dir);
    }

    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(500), tx)
        .map_err(|e| format!("Failed to create session watcher: {e}"))?;

    debouncer
        .watcher()
        .watch(&sessions_dir, notify::RecursiveMode::NonRecursive)
        .map_err(|e| format!("Failed to watch sessions dir: {e}"))?;

    std::thread::Builder::new()
        .name("session-watcher".into())
        .spawn(move || {
            // Keep the debouncer alive for the lifetime of this thread.
            let _debouncer = debouncer;

            loop {
                match rx.recv() {
                    Ok(Ok(events)) => {
                        let has_toml = events
                            .iter()
                            .any(|e| {
                                matches!(e.kind, DebouncedEventKind::Any)
                                    && e.path
                                        .extension()
                                        .map(|ext| ext == "toml")
                                        .unwrap_or(false)
                            });

                        if has_toml {
                            debug!(
                                category = "session_watcher",
                                "Session file changed, emitting agent-status-changed"
                            );
                            let _ = app_handle.emit(AGENT_STATUS_CHANGED_EVENT, ());
                        }
                    }
                    Ok(Err(err)) => {
                        warn!(
                            category = "session_watcher",
                            error = %err,
                            "Session watcher error"
                        );
                    }
                    Err(_) => {
                        // Channel closed — app is shutting down.
                        debug!(
                            category = "session_watcher",
                            "Session watcher channel closed, stopping"
                        );
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("Failed to spawn session watcher thread: {e}"))?;

    Ok(())
}

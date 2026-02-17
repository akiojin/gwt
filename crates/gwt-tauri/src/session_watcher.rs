//! File-system watcher for `~/.gwt/sessions/` (SPEC-b80e7996 FR-820)
//!
//! Monitors session file changes (written by `gwt-tauri hook <Event>`) and
//! emits a Tauri event so the frontend can refresh agent status indicators.

// In test binaries main() is not called, so the setup path that invokes
// start_session_watcher() appears dead to the compiler.
#![cfg_attr(test, allow(unused))]

use gwt_core::config::Session;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use std::path::Path;
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
                        if has_relevant_session_change(&events) {
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

/// Check if a path is a TOML session file (extracted for testability).
fn is_session_file(path: &Path) -> bool {
    path.extension().map(|ext| ext == "toml").unwrap_or(false)
}

/// Filter debounced events, returning true if any `.toml` file was changed.
fn has_relevant_session_change(events: &[DebouncedEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e.kind, DebouncedEventKind::Any) && is_session_file(&e.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_mini::{DebouncedEvent, DebouncedEventKind};
    use std::path::PathBuf;

    fn make_event(path: &str, kind: DebouncedEventKind) -> DebouncedEvent {
        DebouncedEvent {
            path: PathBuf::from(path),
            kind,
        }
    }

    // -- is_session_file tests --

    #[test]
    fn test_is_session_file_toml() {
        assert!(is_session_file(Path::new("/home/.gwt/sessions/abc.toml")));
    }

    #[test]
    fn test_is_session_file_non_toml() {
        assert!(!is_session_file(Path::new("/home/.gwt/sessions/abc.json")));
        assert!(!is_session_file(Path::new("/home/.gwt/sessions/abc.txt")));
        assert!(!is_session_file(Path::new("/home/.gwt/sessions/abc.lock")));
    }

    #[test]
    fn test_is_session_file_no_extension() {
        assert!(!is_session_file(Path::new("/home/.gwt/sessions/abc")));
    }

    // -- has_relevant_session_change tests --

    #[test]
    fn test_detects_toml_change() {
        let events = vec![make_event(
            "/home/.gwt/sessions/abc.toml",
            DebouncedEventKind::Any,
        )];
        assert!(has_relevant_session_change(&events));
    }

    #[test]
    fn test_ignores_non_toml_change() {
        let events = vec![make_event(
            "/home/.gwt/sessions/abc.json",
            DebouncedEventKind::Any,
        )];
        assert!(!has_relevant_session_change(&events));
    }

    #[test]
    fn test_ignores_non_any_event_kind() {
        let events = vec![make_event(
            "/home/.gwt/sessions/abc.toml",
            DebouncedEventKind::AnyContinuous,
        )];
        assert!(!has_relevant_session_change(&events));
    }

    #[test]
    fn test_empty_events() {
        let events: Vec<DebouncedEvent> = vec![];
        assert!(!has_relevant_session_change(&events));
    }

    #[test]
    fn test_mixed_events_detects_toml() {
        let events = vec![
            make_event("/home/.gwt/sessions/abc.json", DebouncedEventKind::Any),
            make_event("/home/.gwt/sessions/def.toml", DebouncedEventKind::Any),
        ];
        assert!(has_relevant_session_change(&events));
    }

    #[test]
    fn test_sessions_dir_exists_or_can_be_created() {
        let temp = tempfile::TempDir::new().unwrap();
        let sessions_dir = temp.path().join("sessions");
        // Directory doesn't exist yet
        assert!(!sessions_dir.exists());
        // But it can be created
        std::fs::create_dir_all(&sessions_dir).unwrap();
        assert!(sessions_dir.exists());
    }
}

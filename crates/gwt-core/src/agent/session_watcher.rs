//! File-system watcher for `~/.gwt/sessions/`
//!
//! Monitors session file changes and invokes a callback so the
//! UI layer can refresh agent status indicators.

use std::{path::Path, sync::mpsc, thread::JoinHandle, time::Duration};

use crate::config::Session;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use tracing::{debug, warn};

/// Event name constant for session status changes.
///
/// UI layers can use this to identify the event type (e.g. Tauri `emit()`).
pub const AGENT_STATUS_CHANGED_EVENT: &str = "agent-status-changed";

/// Handle returned by [`start_session_watcher`] for lifecycle management.
///
/// Dropping this handle does **not** stop the watcher thread immediately;
/// the thread exits when the internal debouncer channel closes.
pub struct SessionWatcherHandle {
    _thread: JoinHandle<()>,
}

/// Start watching `~/.gwt/sessions/` in a background thread.
///
/// `on_change` is invoked whenever a `.toml` session file is created, modified,
/// or removed.  Returns a [`SessionWatcherHandle`] on success.
pub fn start_session_watcher(
    on_change: Box<dyn Fn() + Send + 'static>,
) -> Result<SessionWatcherHandle, String> {
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

    let handle = std::thread::Builder::new()
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
                                "Session file changed, invoking on_change callback"
                            );
                            on_change();
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

    Ok(SessionWatcherHandle { _thread: handle })
}

/// Check if a path is a TOML session file (extracted for testability).
fn is_session_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "toml")
}

/// Filter debounced events, returning true if any `.toml` file was changed.
fn has_relevant_session_change(events: &[DebouncedEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e.kind, DebouncedEventKind::Any) && is_session_file(&e.path))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify_debouncer_mini::{DebouncedEvent, DebouncedEventKind};

    use super::*;

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
    fn test_sessions_dir_creation() {
        let temp = tempfile::TempDir::new().unwrap();
        let sessions_dir = temp.path().join("sessions");
        // Directory doesn't exist yet
        assert!(!sessions_dir.exists());
        // But it can be created
        std::fs::create_dir_all(&sessions_dir).unwrap();
        assert!(sessions_dir.exists());
    }
}

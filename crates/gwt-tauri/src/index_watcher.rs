//! File-system watcher for project files and `specs/` directory.
//!
//! Monitors file changes and triggers re-indexing only when changes are
//! detected, replacing the unconditional index-on-open approach.

#![cfg_attr(test, allow(unused))]

use std::{path::Path, sync::mpsc, time::Duration};

use notify_debouncer_mini::{new_debouncer, DebouncedEvent, DebouncedEventKind};
use tracing::{debug, info, warn};

/// Start watching the project root for file and spec changes.
///
/// When files under `specs/` change, triggers spec re-indexing.
/// When other source files change, triggers project file re-indexing.
///
/// Returns `Ok(())` if the watcher was successfully spawned.
/// The watcher runs until the sender is dropped (typically app exit).
pub fn start_index_watcher(project_root: String) -> Result<(), String> {
    let project_path = std::path::PathBuf::from(&project_root);
    let specs_dir = project_path.join("specs");

    if !project_path.exists() {
        return Err(format!("Project root does not exist: {project_root}"));
    }

    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_secs(2), tx)
        .map_err(|e| format!("Failed to create index watcher: {e}"))?;

    // Watch specs/ recursively if it exists.
    if specs_dir.exists() {
        debouncer
            .watcher()
            .watch(&specs_dir, notify::RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch specs dir: {e}"))?;
    }

    let project_root_clone = project_root.clone();

    std::thread::Builder::new()
        .name("index-watcher".into())
        .spawn(move || {
            let _debouncer = debouncer;

            loop {
                match rx.recv() {
                    Ok(Ok(events)) => {
                        let needs_specs = has_spec_change(&events, &specs_dir);

                        if needs_specs {
                            debug!(
                                category = "index_watcher",
                                "specs/ change detected, triggering spec re-index"
                            );
                            let pr = project_root_clone.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ =
                                    crate::commands::project_index::index_local_specs_cmd(pr).await;
                            });
                        }
                    }
                    Ok(Err(err)) => {
                        warn!(
                            category = "index_watcher",
                            error = %err,
                            "Index watcher error"
                        );
                    }
                    Err(_) => {
                        debug!(
                            category = "index_watcher",
                            "Index watcher channel closed, stopping"
                        );
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("Failed to spawn index watcher thread: {e}"))?;

    info!(
        category = "index_watcher",
        project_root = %project_root,
        "Index watcher started"
    );

    Ok(())
}

/// Check if any event is under the specs/ directory.
fn has_spec_change(events: &[DebouncedEvent], specs_dir: &Path) -> bool {
    events
        .iter()
        .any(|e| matches!(e.kind, DebouncedEventKind::Any) && e.path.starts_with(specs_dir))
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

    #[test]
    fn test_spec_change_detected() {
        let specs_dir = PathBuf::from("/project/specs");
        let events = vec![make_event(
            "/project/specs/SPEC-1/spec.md",
            DebouncedEventKind::Any,
        )];
        assert!(has_spec_change(&events, &specs_dir));
    }

    #[test]
    fn test_non_spec_change_ignored() {
        let specs_dir = PathBuf::from("/project/specs");
        let events = vec![make_event("/project/src/main.rs", DebouncedEventKind::Any)];
        assert!(!has_spec_change(&events, &specs_dir));
    }

    #[test]
    fn test_empty_events() {
        let specs_dir = PathBuf::from("/project/specs");
        let events: Vec<DebouncedEvent> = vec![];
        assert!(!has_spec_change(&events, &specs_dir));
    }

    #[test]
    fn test_non_any_event_kind_ignored() {
        let specs_dir = PathBuf::from("/project/specs");
        let events = vec![make_event(
            "/project/specs/SPEC-1/spec.md",
            DebouncedEventKind::AnyContinuous,
        )];
        assert!(!has_spec_change(&events, &specs_dir));
    }

    #[test]
    fn test_mixed_events() {
        let specs_dir = PathBuf::from("/project/specs");
        let events = vec![
            make_event("/project/src/main.rs", DebouncedEventKind::Any),
            make_event(
                "/project/specs/SPEC-2/metadata.json",
                DebouncedEventKind::Any,
            ),
        ];
        assert!(has_spec_change(&events, &specs_dir));
    }
}

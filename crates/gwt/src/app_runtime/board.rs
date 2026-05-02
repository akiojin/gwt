//! Board post handler split out of `app_runtime/mod.rs` for SPEC-2077 Phase A
//! (arch-review handoff, 2026-05-01).
//!
//! Owns:
//! - [`BoardPostRequest`] payload coming from the frontend
//! - [`AppRuntime::post_board_entry_events`] impl extension (validates the
//!   target window, sanitizes lists, and persists the entry through
//!   `gwt_core::coordination::post_entry`)
//! - [`sanitize_board_list`] helper that trims and de-duplicates string
//!   payloads
//!
//! SPEC-1974 Phase 9 / Phase 10 contracts (`target_owners`, `>>` marker,
//! reminder coordination axes) flow through here unchanged — the handler
//! still uses `BoardEntry::with_target_owners` from `gwt-core` and emits
//! the same `BackendEvent::BoardEntries` / `BackendEvent::BoardError`
//! responses.

use gwt_core::coordination::{self, BoardEntryKind};

use super::{AppRuntime, BackendEvent, OutboundEvent, WindowPreset};

#[derive(Debug, Clone)]
pub(crate) struct BoardPostRequest {
    pub(crate) id: String,
    pub(crate) entry_kind: BoardEntryKind,
    pub(crate) body: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) topics: Vec<String>,
    pub(crate) owners: Vec<String>,
    pub(crate) targets: Vec<String>,
}

impl AppRuntime {
    pub(crate) fn post_board_entry_events(
        &self,
        client_id: &str,
        request: BoardPostRequest,
    ) -> Vec<OutboundEvent> {
        let BoardPostRequest {
            id,
            entry_kind,
            body,
            parent_id,
            topics,
            owners,
            targets,
        } = request;

        let Some(address) = self.window_lookup.get(&id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Board {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Window is not a Board surface".to_string(),
                },
            )];
        }

        let trimmed_body = body.trim();
        if trimmed_body.is_empty() {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: "Board entry body is required".to_string(),
                },
            )];
        }

        let parent_id = parent_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let topics = sanitize_board_list(&topics);
        let owners = sanitize_board_list(&owners);
        let targets = sanitize_board_list(&targets);

        let snapshot = match coordination::load_snapshot(&tab.project_root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: error.to_string(),
                    },
                )];
            }
        };
        if let Some(parent_id) = parent_id.as_deref() {
            if !snapshot
                .board
                .entries
                .iter()
                .any(|entry| entry.id == parent_id)
            {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardError {
                        id,
                        message: "Reply target was not found".to_string(),
                    },
                )];
            }
        }

        let mut entry = coordination::BoardEntry::new(
            coordination::AuthorKind::User,
            "You",
            entry_kind,
            trimmed_body,
            None,
            parent_id,
            topics,
            owners,
        );
        if !targets.is_empty() {
            entry = entry.with_target_owners(targets);
        }
        match coordination::post_entry(&tab.project_root, entry) {
            Ok(snapshot) => {
                publish_board_change(&tab.project_root, snapshot.board.entries.len());
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::BoardEntries {
                        id,
                        entries: snapshot.board.entries,
                    },
                )]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BoardError {
                    id,
                    message: error.to_string(),
                },
            )],
        }
    }
}

/// Best-effort fan-out of a Board projection change to other gwt
/// instances connected to the same daemon (SPEC-2077 Phase H1).
///
/// This is a side-channel notification: the local file watcher already
/// triggers `UserEvent::BoardProjectionChanged` for the in-process GUI,
/// and the on-disk projection remains the source of truth. The daemon
/// publish gives **other** gwt instances on the same project a
/// deterministic push (instead of relying on each instance's file
/// watcher debounce). Any error is logged at debug level and ignored.
#[cfg(unix)]
fn publish_board_change(project_root: &std::path::Path, entries_count: usize) {
    let result = gwt::daemon_publisher::publish_event(
        project_root,
        "board",
        serde_json::json!({"entries_count": entries_count}),
    );
    if let Err(err) = result {
        tracing::debug!(
            error = %err,
            project_root = %project_root.display(),
            entries_count,
            "board projection daemon publish failed (non-fatal)"
        );
    }
}

#[cfg(not(unix))]
fn publish_board_change(_project_root: &std::path::Path, _entries_count: usize) {
    // Daemon publishing is gated on Unix; the local file watcher
    // continues to drive single-instance updates on other platforms.
}

fn sanitize_board_list(values: &[String]) -> Vec<String> {
    let mut sanitized = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || sanitized.iter().any(|item| item == trimmed) {
            continue;
        }
        sanitized.push(trimmed.to_string());
    }
    sanitized
}

//! Memo note handler split out of `app_runtime/mod.rs` for SPEC-2077 Phase B
//! (arch-review handoff, 2026-05-01).
//!
//! Owns:
//! - [`AppRuntime::load_memo_events`] — load memo notes for a Memo window
//! - [`AppRuntime::create_memo_note_events`] / [`AppRuntime::update_memo_note_events`]
//!   / [`AppRuntime::delete_memo_note_events`] — CRUD on memo notes
//! - private helpers `resolve_memo_window_context`,
//!   `memo_window_ids_for_tab`, and `memo_snapshot_events` for window
//!   address validation and snapshot broadcasting
//!
//! All persistence flows through `gwt_core::notes` and the responses use the
//! same `BackendEvent::MemoNotes` / `BackendEvent::MemoError` envelopes
//! consumed by the frontend Memo window.

use std::path::{Path, PathBuf};

use gwt_core::notes;

use super::{combined_window_id, AppRuntime, BackendEvent, OutboundEvent, WindowPreset};

impl AppRuntime {
    pub(crate) fn load_memo_events(&self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        let project_root = match self.resolve_memo_window_context(id) {
            Ok(context) => context,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        }
        .1;

        match notes::load_snapshot(&project_root) {
            Ok(snapshot) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::MemoNotes {
                    id: id.to_string(),
                    notes: snapshot.notes,
                    selected_note_id: None,
                },
            )],
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::MemoError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn create_memo_note_events(
        &self,
        client_id: &str,
        id: &str,
        title: String,
        body: String,
        pinned: bool,
    ) -> Vec<OutboundEvent> {
        let (tab_id, project_root) = match self.resolve_memo_window_context(id) {
            Ok(context) => context,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let created = match notes::create_note(
            &project_root,
            notes::MemoNoteDraft::new(title, body, pinned),
        ) {
            Ok(note) => note,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message: error.to_string(),
                    },
                )];
            }
        };

        self.memo_snapshot_events(&tab_id, id, Some(created.id), &project_root, client_id)
    }

    pub(crate) fn update_memo_note_events(
        &self,
        client_id: &str,
        id: &str,
        note_id: &str,
        title: String,
        body: String,
        pinned: bool,
    ) -> Vec<OutboundEvent> {
        let (tab_id, project_root) = match self.resolve_memo_window_context(id) {
            Ok(context) => context,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let updated = match notes::update_note(
            &project_root,
            note_id,
            notes::MemoNoteDraft::new(title, body, pinned),
        ) {
            Ok(note) => note,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message: error.to_string(),
                    },
                )];
            }
        };

        self.memo_snapshot_events(&tab_id, id, Some(updated.id), &project_root, client_id)
    }

    pub(crate) fn delete_memo_note_events(
        &self,
        client_id: &str,
        id: &str,
        note_id: &str,
    ) -> Vec<OutboundEvent> {
        let (tab_id, project_root) = match self.resolve_memo_window_context(id) {
            Ok(context) => context,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        if let Err(error) = notes::delete_note(&project_root, note_id) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::MemoError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )];
        }

        let selected_note_id = match notes::load_snapshot(&project_root) {
            Ok(snapshot) => snapshot.notes.first().map(|note| note.id.clone()),
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: id.to_string(),
                        message: error.to_string(),
                    },
                )];
            }
        };

        self.memo_snapshot_events(&tab_id, id, selected_note_id, &project_root, client_id)
    }

    fn resolve_memo_window_context(
        &self,
        id: &str,
    ) -> std::result::Result<(String, PathBuf), String> {
        let Some(address) = self.window_lookup.get(id) else {
            return Err("Window not found".to_string());
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return Err("Project tab not found".to_string());
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return Err("Window not found".to_string());
        };
        if window.preset != WindowPreset::Memo {
            return Err("Window is not a Memo surface".to_string());
        }

        Ok((address.tab_id.clone(), tab.project_root.clone()))
    }

    fn memo_window_ids_for_tab(&self, tab_id: &str) -> Vec<String> {
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        tab.workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Memo)
            .map(|window| combined_window_id(tab_id, &window.id))
            .collect()
    }

    fn memo_snapshot_events(
        &self,
        tab_id: &str,
        selected_window_id: &str,
        selected_note_id: Option<String>,
        project_root: &Path,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let snapshot = match notes::load_snapshot(project_root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::MemoError {
                        id: selected_window_id.to_string(),
                        message: error.to_string(),
                    },
                )];
            }
        };

        self.memo_window_ids_for_tab(tab_id)
            .into_iter()
            .map(|window_id| {
                OutboundEvent::broadcast(BackendEvent::MemoNotes {
                    id: window_id.clone(),
                    notes: snapshot.notes.clone(),
                    selected_note_id: if window_id == selected_window_id {
                        selected_note_id.clone()
                    } else {
                        None
                    },
                })
            })
            .collect()
    }
}

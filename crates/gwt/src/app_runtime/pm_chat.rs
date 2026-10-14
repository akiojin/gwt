//! The PM's text projection is separate from its unchanged PTY execution.

use gwt::pm_conversation::{PmConversationAvailability, PmConversationSnapshot};

use super::{AppRuntime, BackendEvent, ClientId, OutboundEvent, ProjectContext, UserEvent};

use gwt::protocol::{PmReportView, PmReportsMode, PmReportsProjection, SliceCursor, SliceRevision};
use gwt_core::pm_report::{PmReportsHistoryCursor, PmReportsReader, PmReportsUpdate};

pub(crate) struct PmReportsState {
    reader: PmReportsReader,
    epoch: String,
    revision: u64,
    tail: Vec<PmReportView>,
    delta: Option<PmReportsProjection>,
}

impl Default for PmReportsState {
    fn default() -> Self {
        Self {
            reader: Default::default(),
            epoch: uuid::Uuid::new_v4().to_string(),
            revision: 0,
            tail: Vec::new(),
            delta: None,
        }
    }
}

impl PmReportsState {
    fn read(
        &mut self,
        root: &std::path::Path,
        cursor: Option<&SliceCursor>,
        before: Option<&PmReportsHistoryCursor>,
    ) -> gwt_core::Result<PmReportsProjection> {
        let update = self.reader.refresh(root)?;
        if !matches!(update, PmReportsUpdate::Unchanged) {
            let base_revision = self.revision;
            self.revision = self
                .revision
                .checked_add(1)
                .filter(|value| *value <= 9_007_199_254_740_991)
                .ok_or_else(|| {
                    gwt_core::GwtError::Config("PM Reports revision exhausted".into())
                })?;
            let old = std::mem::take(&mut self.tail);
            self.tail = self
                .reader
                .tail()
                .iter()
                .map(|report| {
                    old.iter()
                        .find(|view| &view.report == report)
                        .cloned()
                        .unwrap_or_else(|| PmReportView::from(report.clone()))
                })
                .collect();
            self.delta = match update {
                PmReportsUpdate::Delta {
                    reports,
                    removed_ids,
                    ordered_ids,
                } => Some(PmReportsProjection {
                    reports_mode: PmReportsMode::Delta,
                    slice: Some(SliceRevision {
                        epoch: self.epoch.clone(),
                        base_revision,
                        revision: self.revision,
                    }),
                    reports: reports.into_iter().map(PmReportView::from).collect(),
                    removed_ids,
                    ordered_ids,
                    before: self.reader.history_cursor(),
                    request_id: None,
                }),
                _ => None,
            };
        }
        let mut projection = PmReportsProjection {
            slice: Some(SliceRevision {
                epoch: self.epoch.clone(),
                base_revision: cursor.map_or(0, |value| value.revision.min(self.revision)),
                revision: self.revision,
            }),
            before: self.reader.history_cursor(),
            ..Default::default()
        };
        if let Some(before) = before {
            let page = gwt_core::pm_report::load_reports_before(root, Some(before))?;
            projection.reports_mode = PmReportsMode::History;
            projection.reports = page.reports.into_iter().map(PmReportView::from).collect();
            projection.before = page.before;
        } else if cursor
            .is_some_and(|value| value.epoch == self.epoch && value.revision == self.revision)
        {
            projection.reports_mode = PmReportsMode::Unchanged;
        } else if let Some(delta) = self.delta.as_ref().filter(|delta| {
            cursor.is_some_and(|cursor| {
                cursor.epoch == self.epoch
                    && Some(cursor.revision)
                        == delta.slice.as_ref().map(|slice| slice.base_revision)
            })
        }) {
            return Ok(delta.clone());
        } else {
            projection.reports = self.tail.clone();
        }
        Ok(projection)
    }
}

fn empty(availability: PmConversationAvailability, detail: &str) -> PmConversationSnapshot {
    PmConversationSnapshot {
        conversation_id: None,
        availability,
        messages: Vec::new(),
        detail: Some(detail.to_owned()),
    }
}

impl AppRuntime {
    pub(super) fn load_pm_reports_events(
        &self,
        context: &ProjectContext,
        client_id: ClientId,
        window_id: &str,
        cursor: Option<SliceCursor>,
        before: Option<PmReportsHistoryCursor>,
        request_id: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let Some(session_id) = self.pm_chat_session_id(window_id).map(str::to_owned) else {
            return vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::PmReports {
                    id: window_id.to_owned(),
                    session_id: None,
                    projection: PmReportsProjection::default(),
                    current: None,
                    error: Some("This window has no registered PM.".to_owned()),
                },
            )];
        };
        let Some(address) = self.window_lookup.get(window_id) else {
            return Vec::new();
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return Vec::new();
        };
        let project_root = tab.project_root.clone();
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        let reader = std::sync::Arc::clone(&state.pm_reports_reader);
        let pending_update = self.pending_update.clone();
        let window_id = window_id.to_owned();
        let proxy = self.proxy.for_project(context.clone());
        self.blocking_tasks.spawn(move || {
            // Serialize read and enqueue: a later poll cannot overtake a delta.
            let mut reader = reader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (mut projection, error) =
                match reader.read(&project_root, cursor.as_ref(), before.as_ref()) {
                    Ok(projection) => (projection, None),
                    Err(error) => (
                        PmReportsProjection::default(),
                        Some(format!("PM reports could not be loaded: {error}")),
                    ),
                };
            projection.request_id = request_id;
            let (current, error) =
                match gwt::pm_current::snapshot(&project_root, pending_update.as_ref()) {
                    Ok(current) => (Some(Box::new(current)), error),
                    Err(current_error) => (None, Some(error.unwrap_or(current_error))),
                };
            proxy.send(UserEvent::PmReportsLoaded {
                client_id,
                window_id,
                session_id,
                projection,
                current,
                error,
            });
        });
        Vec::new()
    }

    pub(crate) fn pm_reports_loaded_events(
        &self,
        client_id: ClientId,
        window_id: &str,
        session_id: &str,
        projection: PmReportsProjection,
        current: Option<Box<gwt::pm_current::PmCurrentSnapshot>>,
        error: Option<String>,
    ) -> Vec<OutboundEvent> {
        if self.pm_chat_session_id(window_id) != Some(session_id) {
            return Vec::new();
        }
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::PmReports {
                id: window_id.to_owned(),
                session_id: Some(session_id.to_owned()),
                projection,
                current,
                error,
            },
        )]
    }

    fn pm_chat_session_id(&self, window_id: &str) -> Option<&str> {
        let address = self.window_lookup.get(window_id)?;
        let tab = self.tab(&address.tab_id)?;
        let window = tab.workspace.window(&address.raw_id)?;
        let session_id = window.session_id.as_deref()?;
        (self
            .pm_session_for_root(&tab.project_root)
            .map(String::as_str)
            == Some(session_id))
        .then_some(session_id)
    }

    pub(super) fn load_pm_conversation_events(
        &self,
        context: &ProjectContext,
        client_id: ClientId,
        window_id: &str,
    ) -> Vec<OutboundEvent> {
        let session_id = self.pm_chat_session_id(window_id).map(str::to_owned);
        let reply = |snapshot| {
            vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::PmConversation {
                    id: window_id.to_owned(),
                    session_id: session_id.clone(),
                    snapshot,
                },
            )]
        };
        let Some(id) = session_id.as_deref() else {
            return reply(empty(
                PmConversationAvailability::Unsupported,
                "This window uses the terminal view.",
            ));
        };
        // This small host-owned Session record is the only authority for the
        // native identity; the client cannot choose a provider path or ID.
        let session_path = self.sessions_dir.join(format!("{id}.toml"));
        if !session_path.exists() {
            return reply(empty(
                PmConversationAvailability::Waiting,
                "Waiting for the PM session.",
            ));
        }
        let Ok(session) = gwt_agent::Session::load(&session_path) else {
            return reply(empty(
                PmConversationAvailability::Unavailable,
                "The PM session is not readable.",
            ));
        };
        if session.id != id {
            return reply(empty(
                PmConversationAvailability::Unavailable,
                "The PM session identity does not match.",
            ));
        }
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        let reader = std::sync::Arc::clone(&state.pm_conversation_reader);
        let window_id = window_id.to_owned();
        let session_id = id.to_owned();
        let proxy = self.proxy.for_project(context.clone());
        self.blocking_tasks.spawn(move || {
            // Serialize reads and their completion enqueue so multiple viewers
            // cannot replay an older snapshot after a newer append.
            let mut reader = reader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let snapshot = reader.read_for_session(&session);
            proxy.send(UserEvent::PmConversationLoaded {
                client_id,
                window_id,
                session_id,
                snapshot,
            });
        });
        Vec::new()
    }

    pub(crate) fn pm_conversation_loaded_events(
        &self,
        client_id: ClientId,
        window_id: &str,
        session_id: &str,
        snapshot: PmConversationSnapshot,
    ) -> Vec<OutboundEvent> {
        // A delayed read must not replace the chat of a restarted PM. Project
        // completion dispatch separately rejects closed/reopened project tabs.
        if self.pm_chat_session_id(window_id) != Some(session_id) {
            return Vec::new();
        }
        // Native /clear can rotate a conversation inside the same gwt Session.
        // Recheck that small authoritative record before delivering a queued read.
        let session_path = self.sessions_dir.join(format!("{session_id}.toml"));
        let Ok(current_session) = gwt_agent::Session::load(&session_path) else {
            return Vec::new();
        };
        if current_session.id != session_id
            || current_session.exact_resume_session_id() != snapshot.conversation_id.as_deref()
        {
            return Vec::new();
        }
        vec![OutboundEvent::reply(
            client_id,
            BackendEvent::PmConversation {
                id: window_id.to_owned(),
                session_id: Some(session_id.to_owned()),
                snapshot,
            },
        )]
    }
}

#[cfg(test)]
mod reports_slice_tests {
    use super::*;

    #[test]
    fn pm_reports_slice_sends_delta_noop_and_recovers_stale_cursors() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = home.path().join("repo");
        let first = gwt_core::pm_report::post_report(
            &repo,
            gwt_core::pm_report::PmReportKind::Progress,
            "first",
            None,
        )
        .unwrap();
        let mut state = PmReportsState::default();
        let snapshot = state.read(&repo, None, None).unwrap();
        assert_eq!(
            snapshot.reports_mode,
            gwt::protocol::PmReportsMode::Snapshot
        );
        assert_eq!(snapshot.reports[0].report.id, first.id);
        let cursor = gwt::protocol::SliceCursor {
            epoch: snapshot.slice.as_ref().unwrap().epoch.clone(),
            revision: snapshot.slice.as_ref().unwrap().revision,
        };
        let second = gwt_core::pm_report::post_report(
            &repo,
            gwt_core::pm_report::PmReportKind::Progress,
            "second",
            None,
        )
        .unwrap();
        let delta = state.read(&repo, Some(&cursor), None).unwrap();
        assert_eq!(delta.reports_mode, gwt::protocol::PmReportsMode::Delta);
        assert_eq!(delta.reports.len(), 1);
        assert_eq!(delta.reports[0].report.id, second.id);
        assert_eq!(delta.ordered_ids, [first.id, second.id]);
        let latest = gwt::protocol::SliceCursor {
            epoch: cursor.epoch,
            revision: delta.slice.as_ref().unwrap().revision,
        };
        let noop = state.read(&repo, Some(&latest), None).unwrap();
        assert_eq!(noop.reports_mode, gwt::protocol::PmReportsMode::Unchanged);
        assert!(noop.reports.is_empty());
        let stale = gwt::protocol::SliceCursor {
            epoch: "retired-server".into(),
            revision: latest.revision,
        };
        let recovered = state.read(&repo, Some(&stale), None).unwrap();
        assert_eq!(
            recovered.reports_mode,
            gwt::protocol::PmReportsMode::Snapshot
        );
        assert_eq!(recovered.reports.len(), 2);
    }
}

//! The PM's text projection is separate from its unchanged PTY execution.

use gwt::pm_conversation::{PmConversationAvailability, PmConversationSnapshot};

use super::{AppRuntime, BackendEvent, ClientId, OutboundEvent, ProjectContext, UserEvent};

fn empty(availability: PmConversationAvailability, detail: &str) -> PmConversationSnapshot {
    PmConversationSnapshot {
        conversation_id: None,
        availability,
        messages: Vec::new(),
        detail: Some(detail.to_owned()),
    }
}

impl AppRuntime {
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

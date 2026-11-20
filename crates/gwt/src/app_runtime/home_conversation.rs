//! Issue #7639 (SPEC #7007 T-021): the Home conversation.
//!
//! Home talks to the PM of every open project from the Hub connection. The
//! durable source stays where it already is — each project's report journal —
//! and an answer travels the PM pane input path that the project page uses.
//! The Hub client names a project, never a session or a window: the host
//! resolves the registered PM from the project.

use super::*;

const NO_PM: &str =
    "No Project Manager is running for this project. Open the project, then try again.";

impl AppRuntime {
    /// Read the decisions every open project's PM is waiting on.
    pub(super) fn load_home_conversation_events(
        &self,
        client_id: ClientId,
        request_id: u64,
    ) -> Vec<OutboundEvent> {
        let projects: Vec<(String, String, PathBuf, bool)> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                Some((
                    self.project_key_for_tab(&tab.id)?.to_string(),
                    tab.title.clone(),
                    tab.project_root.clone(),
                    self.pm_session_for_root(&tab.project_root).is_some(),
                ))
            })
            .collect();
        let proxy = self.proxy.clone();
        let reply_client = client_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let projects = projects
                .into_iter()
                .map(|(project_key, title, root, pm_available)| {
                    gwt::HomeProjectConversationView::from_reports(
                        project_key,
                        title,
                        pm_available,
                        gwt_core::pm_report::load_reports(&root).map_err(|error| {
                            gwt_core::error_ledger::sanitize_error_message(&error.to_string())
                        }),
                    )
                })
                .collect();
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &reply_client,
                BackendEvent::HomeConversation {
                    request_id,
                    projects,
                },
            )]));
        });
        match spawn {
            Ok(()) => Vec::new(),
            // The read never started: answer with what is known without it, so
            // the client is not left waiting on this request.
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::HomeConversation {
                    request_id,
                    projects: self
                        .tabs
                        .iter()
                        .filter_map(|tab| {
                            Some(gwt::HomeProjectConversationView::from_reports(
                                self.project_key_for_tab(&tab.id)?.to_string(),
                                tab.title.clone(),
                                self.pm_session_for_root(&tab.project_root).is_some(),
                                Err(error.clone()),
                            ))
                        })
                        .collect(),
                },
            )],
        }
    }

    /// Send one Home message to the PM pane of an open project.
    pub(super) fn home_pm_send_events(
        &mut self,
        client_id: ClientId,
        project_key: &str,
        text: &str,
        request_id: u64,
    ) -> Vec<OutboundEvent> {
        let target = self.tabs.iter().find_map(|tab| {
            let key = self.project_key_for_tab(&tab.id)?;
            (key.as_str() == project_key).then(|| {
                (
                    key.clone(),
                    self.pm_session_for_root(&tab.project_root).cloned(),
                )
            })
        });
        let refuse = |error: &str| {
            vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::PaneSendResult {
                    ok: false,
                    window_id: None,
                    error: Some(error.to_string()),
                    request_id: Some(request_id),
                },
            )]
        };
        match target {
            None => refuse("This project is not open. Open it from Projects, then try again."),
            Some((_, None)) => refuse(NO_PM),
            Some((key, Some(session_id))) => self.pane_send_input_for_project_events(
                client_id,
                &key,
                &session_id,
                text,
                Some(request_id),
            ),
        }
    }
}

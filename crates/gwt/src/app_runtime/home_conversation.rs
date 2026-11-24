//! Issue #7639 (SPEC #7007 T-021): the Home conversation.
//!
//! Home talks to the PM of every open project from the Hub connection. The
//! decisions are each project's gwtd decision queue (Issue #7742): answering
//! or deferring one is a queue operation, and gwtd delivers the answer to the
//! waiting sessions. A free-form message travels the PM pane input path that
//! the project page uses. The Hub client names a project, never a session or
//! a window: the host resolves the rest from the project.

use super::*;
use gwt::protocol::HomePmView;

const NO_PM: &str =
    "No Project Manager is running for this project. Open the project, then try again.";
const NOT_OPEN: &str = "This project is not open. Open it from Projects, then try again.";
const QUEUE_UNREADABLE: &str = "The decision queue could not be read.";

fn home_pm() -> std::io::Result<Option<HomePmView>> {
    let Some(designation) = gwt::first_run_setup::load_designation()? else {
        return Ok(None);
    };
    let roster = gwt::agent_registry::SessionMemberSnapshot::load()?;
    let member = roster.designated_pm(&designation);
    Ok(Some(HomePmView {
        name: member.map_or_else(|| designation.name, |member| member.name.clone()),
        avatar: member.map_or_else(Default::default, |member| member.avatar.clone()),
    }))
}

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
                    gwt::HomeProjectConversationView::from_queue(
                        project_key,
                        title,
                        pm_available,
                        gwt::pm_current::load_decisions(&root)
                            .ok_or_else(|| QUEUE_UNREADABLE.to_string()),
                    )
                })
                .collect();
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &reply_client,
                BackendEvent::HomeConversation {
                    request_id,
                    pm: home_pm().ok(),
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
                    pm: None,
                    projects: self
                        .tabs
                        .iter()
                        .filter_map(|tab| {
                            Some(gwt::HomeProjectConversationView::from_queue(
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

    /// Issue #7742: answer (`Some`) or defer (`None`) one decision of an open
    /// project from Home. No PM has to be running: the queue is the project's.
    pub(super) fn home_decision_events(
        &self,
        client_id: ClientId,
        project_key: &str,
        request_id: u64,
        decision_id: String,
        answer: Option<String>,
    ) -> Vec<OutboundEvent> {
        let root = self.tabs.iter().find_map(|tab| {
            (self.project_key_for_tab(&tab.id)?.as_str() == project_key)
                .then(|| tab.project_root.clone())
        });
        let Some(root) = root else {
            return vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::DecisionResult {
                    id: project_key.to_owned(),
                    request_id,
                    decision_id,
                    action: if answer.is_some() { "answer" } else { "defer" }.to_owned(),
                    ok: false,
                    error: Some(NOT_OPEN.to_owned()),
                    decisions: None,
                },
            )];
        };
        // The Hub connection belongs to no project, so its reply is not
        // scoped to one.
        self.run_decision_request(
            self.proxy.clone(),
            root,
            client_id,
            project_key,
            request_id,
            decision_id,
            answer,
        )
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
            None => refuse(NOT_OPEN),
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

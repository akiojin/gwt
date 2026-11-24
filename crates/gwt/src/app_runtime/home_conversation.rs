//! Issue #7639 (SPEC #7007 T-021): the Home conversation.
//!
//! Home talks to the PM of every open project from the Hub connection. The
//! decisions are each project's gwtd decision queue (Issue #7742): answering
//! or deferring one is a queue operation, and gwtd delivers the answer to the
//! waiting sessions. A free-form message travels the PM pane input path that
//! the project page uses. The Hub client names a project, never a session or
//! a window: the host resolves the rest from the project.

use super::*;
use gwt::{pm_registry, protocol::HomePmView};

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
        history: std::collections::BTreeMap<String, String>,
    ) -> Vec<OutboundEvent> {
        let projects: Vec<_> = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let pm_available = self.pm_session_for_root(&tab.project_root).is_some();
                let reader = self
                    .project_state_for_root(&tab.project_root)
                    .map(|state| std::sync::Arc::clone(&state.pm_conversation_reader));
                Some((
                    self.project_key_for_tab(&tab.id)?.to_string(),
                    tab.title.clone(),
                    tab.project_root.clone(),
                    pm_available,
                    reader,
                ))
            })
            .collect();
        let sessions_dir = self.sessions_dir.clone();
        let proxy = self.proxy.clone();
        let reply_client = client_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let projects = projects
                .into_iter()
                .map(|(project_key, title, root, pm_available, reader)| {
                    let mut project = gwt::HomeProjectConversationView::from_queue(
                        project_key,
                        title,
                        pm_available,
                        gwt::pm_current::load_decisions(&root)
                            .ok_or_else(|| QUEUE_UNREADABLE.to_string()),
                    );
                    let conversation = reader.and_then(|reader| {
                        // The registration also identifies stopped sessions whose
                        // saved output is still part of the Home conversation.
                        let registration = pm_registry::load_pm_prefs(
                            &pm_registry::pm_prefs_path_for_repo_path(&root),
                        )
                        .ok()?
                        .registration?;
                        let id = registration.session_id;
                        let session =
                            gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml")))
                                .ok()?;
                        if session.id != id {
                            return None;
                        }
                        project.pm_interrupted = matches!(
                            session.status,
                            gwt_agent::AgentStatus::Stopped | gwt_agent::AgentStatus::Interrupted
                        );
                        Some(
                            reader
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .read_home_page(
                                    &session,
                                    history.get(&project.project_key).map(String::as_str),
                                ),
                        )
                    });
                    project.conversation = conversation;
                    project
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

    fn home_pm_target(
        &self,
        project_key: &str,
    ) -> Option<(gwt_core::repo_hash::ProjectKey, Option<String>)> {
        self.tabs.iter().find_map(|tab| {
            let key = self.project_key_for_tab(&tab.id)?;
            (key.as_str() == project_key).then(|| {
                (
                    key.clone(),
                    self.pm_session_for_root(&tab.project_root).cloned(),
                )
            })
        })
    }

    /// Send one Home message to the PM pane of an open project.
    pub(super) fn home_pm_send_events(
        &mut self,
        client_id: ClientId,
        project_key: &str,
        text: &str,
        request_id: u64,
        attachments: Vec<gwt::FileAttachment>,
    ) -> Vec<OutboundEvent> {
        let Some((key, session_id)) = self.home_pm_target(project_key) else {
            return home_send_refusal(&client_id, request_id, NOT_OPEN);
        };
        let Some(session_id) = session_id else {
            return home_send_refusal(&client_id, request_id, NO_PM);
        };
        if attachments.is_empty() {
            return self.pane_send_input_for_project_events(
                client_id,
                &key,
                &session_id,
                text,
                Some(request_id),
            );
        }
        let Some(session) = self
            .active_agent_sessions
            .values()
            .find(|session| session.session_id == session_id)
        else {
            return home_send_refusal(&client_id, request_id, NO_PM);
        };
        let Some(runtime) = self.runtimes.get(&session.window_id) else {
            return home_send_refusal(&client_id, request_id, NO_PM);
        };
        let target = gwt::protocol::AttachmentExpectedTarget {
            project_key: project_key.to_owned(),
            session_id,
            host_id: format!(
                "{}:{}",
                runtime.owner_identity.pid, runtime.owner_identity.host_started_at
            ),
            runtime_incarnation: runtime.incarnation,
        };
        let Ok(operation) = self.capture_attachment_operation(
            client_id.clone(),
            session.window_id.clone(),
            Some(format!("home-{request_id}")),
            Some(target),
            attachments.len(),
            None,
        ) else {
            return home_send_refusal(&client_id, request_id, NO_PM);
        };
        let worktree = session.worktree_path.clone();
        let uploads = self.attachment_uploads.clone();
        let proxy = self.proxy.clone();
        let text = text.to_owned();
        match self.blocking_tasks.try_spawn(move || {
            match stage_home_attachments(&worktree, &uploads, &text, &attachments) {
                Ok(prompt) => proxy.send(UserEvent::HomeAttachmentsReady {
                    operation: Box::new(operation),
                    request_id,
                    prompt,
                }),
                Err(error) => proxy.send(UserEvent::Dispatch(home_send_refusal(
                    &operation.client_id,
                    request_id,
                    &error,
                ))),
            };
        }) {
            Ok(()) => Vec::new(),
            Err(error) => home_send_refusal(&client_id, request_id, &error),
        }
    }

    pub(crate) fn deliver_home_message(
        &mut self,
        operation: super::AttachmentOperation,
        prompt: &str,
        request_id: u64,
    ) -> Vec<OutboundEvent> {
        let target = &operation.expected_target;
        if !self.attachment_operation_is_current(&operation)
            || !self
                .home_pm_target(&target.project_key)
                .is_some_and(|(_, current)| current.as_deref() == Some(target.session_id.as_str()))
        {
            return home_send_refusal(
                &operation.client_id,
                request_id,
                "Project Manager changed while preparing attachments.",
            );
        }
        self.pane_send_input_to_window_events(
            operation.client_id,
            &operation.id,
            prompt,
            Some(request_id),
        )
    }
}

fn home_send_refusal(client_id: &ClientId, request_id: u64, error: &str) -> Vec<OutboundEvent> {
    vec![OutboundEvent::reply(
        client_id,
        BackendEvent::PaneSendResult {
            ok: false,
            window_id: None,
            error: Some(error.to_owned()),
            request_id: Some(request_id),
        },
    )]
}

fn stage_home_attachments(
    worktree: &Path,
    uploads: &crate::attachment_upload::AttachmentUploadStore,
    text: &str,
    files: &[gwt::FileAttachment],
) -> Result<String, String> {
    use super::attachments::{
        format_file_attachment_prompt, prepare_file_attachment, save_file_attachment_with_progress,
    };
    if files.len() > 16 {
        return Err("Choose up to 16 uploaded attachments.".into());
    }
    let tokens = files
        .iter()
        .map(|file| {
            let gwt::FileAttachment::Uploaded { upload_id, .. } = file else {
                return Err("Home attachments must be uploaded first.".to_string());
            };
            uuid::Uuid::parse_str(upload_id)
                .map(|id| id.simple().to_string())
                .map_err(|_| "Invalid attachment upload ID.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths = Vec::with_capacity(files.len());
    for (file, token) in files.iter().zip(tokens) {
        let prepared = prepare_file_attachment(
            worktree,
            "",
            gwt_agent::LaunchRuntimeTarget::Host,
            file,
            &token,
            gwt::protocol::ContentLimits::default(),
            uploads,
        )
        .map_err(|error| error.to_string())?;
        save_file_attachment_with_progress(&prepared, |_, _| {})
            .map_err(|error| error.to_string())?;
        paths.push(prepared.agent_path);
    }
    Ok(format!(
        "{}\n\n{}\r",
        text.trim_end_matches('\r'),
        format_file_attachment_prompt(&paths)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_attachments_stage_uploaded_bytes_in_the_pm_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let uploads = crate::attachment_upload::AttachmentUploadStore::new(temp.path().into());
        let (upload_id, path) = uploads.allocate_path();
        std::fs::write(&path, b"PM evidence").unwrap();
        uploads
            .insert(
                upload_id.clone(),
                crate::attachment_upload::UploadedAttachment {
                    path,
                    filename: "evidence.txt".into(),
                    mime_type: None,
                    size: 11,
                },
            )
            .unwrap();
        let file = gwt::FileAttachment::Uploaded {
            upload_id,
            filename: "evidence.txt".into(),
            mime_type: None,
            size: 11,
        };
        let prompt =
            stage_home_attachments(temp.path(), &uploads, "Please review\r", &[file]).unwrap();
        let path = prompt
            .strip_prefix("Please review\n\nFile: ")
            .unwrap()
            .trim_end_matches('\r')
            .trim_matches('"');
        assert_eq!(
            std::fs::read(temp.path().join(path)).unwrap(),
            b"PM evidence"
        );
    }
}

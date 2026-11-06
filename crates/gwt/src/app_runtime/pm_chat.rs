//! The PM's text projection is separate from its unchanged PTY execution.

use gwt::pm_conversation::{PmConversationAvailability, PmConversationSnapshot};
use gwt::protocol::PmRequestTaskResult;
use gwt_core::pm_request_tasks::{Action, Actor, ActorKind, Request, TaskError, TaskStore};

use super::{AppRuntime, BackendEvent, ClientId, OutboundEvent, ProjectContext, UserEvent};

pub(super) enum PmRequestTaskOperation {
    Read {
        task_id: String,
        receipt_operation_id: Option<String>,
    },
    Reopen(gwt::protocol::PmRequestTaskReopenRequest),
}

fn task_error(code: &str, message: String, current_revision: Option<u64>) -> PmRequestTaskResult {
    PmRequestTaskResult::Error {
        code: code.into(),
        message,
        current_revision,
        recovery_operation: "load_pm_request_task".into(),
    }
}

fn domain_task_error(store: &TaskStore, error: TaskError) -> PmRequestTaskResult {
    let code = match &error {
        TaskError::ForeignProject => "foreign_project",
        TaskError::Unauthorized => "unauthorized",
        TaskError::Conflict { .. } => "conflict",
        TaskError::OperationConflict => "operation_conflict",
        TaskError::StaleReview => "stale_review",
        TaskError::ReassessmentRequired => "reassessment_required",
        TaskError::UnfinishedChildren => "unfinished_children",
        TaskError::Invalid(_) => "invalid_request",
        TaskError::MissingSource => "missing_source",
        TaskError::MissingTask => "missing_task",
        TaskError::Unreadable(_) => "unreadable_store",
        TaskError::PersistenceUnconfirmed(_) => "persistence_unconfirmed",
    };
    let revision = match &error {
        TaskError::Conflict { current_revision } => Some(*current_revision),
        TaskError::ForeignProject | TaskError::Unauthorized | TaskError::Unreadable(_) => None,
        // A publication may have renamed before losing its acknowledgement.
        // Observe after failure; never claim the pre-write revision is current.
        _ => store
            .read()
            .ok()
            .map(|state| state.map_or(0, |state| state.revision)),
    };
    task_error(code, error.to_string(), revision)
}

pub(super) fn rejected_request_task_events(
    client_id: &str,
    event: &gwt::FrontendEvent,
) -> Option<Vec<OutboundEvent>> {
    let (id, request_id) = match event {
        gwt::FrontendEvent::LoadPmRequestTask { id, request_id, .. }
        | gwt::FrontendEvent::ReopenPmRequestTask { id, request_id, .. } => (id, *request_id),
        _ => return None,
    };
    Some(vec![OutboundEvent::reply(
        client_id,
        BackendEvent::PmRequestTask {
            id: id.clone(),
            request_id,
            result: task_error(
                "foreign_project",
                "Reload the current project before reading or changing a request task.".into(),
                None,
            ),
        },
    )])
}

fn request_task_result(
    store: &TaskStore,
    client_id: &str,
    operation: PmRequestTaskOperation,
) -> Result<PmRequestTaskResult, TaskError> {
    match operation {
        PmRequestTaskOperation::Read {
            task_id,
            receipt_operation_id,
        } => {
            if task_id.trim().is_empty() {
                return Err(TaskError::Invalid("nonempty task_id is required".into()));
            }
            let state = store.read()?.ok_or(TaskError::MissingTask)?;
            let task = state
                .tasks
                .get(&task_id)
                .ok_or(TaskError::MissingTask)?
                .clone();
            let receipt = receipt_operation_id.map(|operation_id| {
                state.receipts.iter().find(|receipt| receipt.operation_id == operation_id)
                    .cloned().map(Box::new)
                    .ok_or_else(|| TaskError::Invalid("operation receipt not found; retain the unresolved operation and reread the current task".into()))
            }).transpose()?;
            Ok(PmRequestTaskResult::Ready {
                revision: state.revision,
                task: Box::new(task),
                receipt,
            })
        }
        PmRequestTaskOperation::Reopen(request) => {
            let receipt = store.apply(
                &Actor {
                    kind: ActorKind::User,
                    id: format!("browser:{client_id}"),
                },
                &Request {
                    operation_id: request.operation_id,
                    expected_revision: request.expected_revision,
                    reason: request.reason,
                    action: Action::Reopen {
                        task_id: request.task_id.clone(),
                    },
                },
            )?;
            // An exact retry returns its original applied snapshot, not a claim
            // about current state. Read/receipt reconciliation observes that separately.
            let task = receipt
                .tasks
                .iter()
                .find(|task| task.draft.id == request.task_id)
                .cloned()
                .ok_or_else(|| {
                    TaskError::PersistenceUnconfirmed(
                        "reopen receipt is missing its task; reconcile the exact operation".into(),
                    )
                })?;
            Ok(PmRequestTaskResult::Ready {
                revision: receipt.revision,
                task: Box::new(task),
                receipt: Some(Box::new(receipt)),
            })
        }
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
    pub(super) fn pm_request_task_events(
        &self,
        context: &ProjectContext,
        client_id: ClientId,
        window_id: &str,
        request_id: u64,
        operation: PmRequestTaskOperation,
    ) -> Vec<OutboundEvent> {
        // The connection's project gate runs before this window lookup. Missing
        // windows cannot become a fallback to another project's private store.
        let root = self
            .window_lookup
            .get(window_id)
            .and_then(|address| self.tab(&address.tab_id))
            .filter(|tab| tab.project_root == context.project_root)
            .map(|tab| tab.project_root.clone());
        let Some(root) = root else {
            return vec![OutboundEvent::reply(&client_id, BackendEvent::PmRequestTask {
                id: window_id.into(), request_id,
                result: task_error("unauthorized", "The selected project window is unavailable; reload the current project before retrying.".into(), None),
            })];
        };
        let proxy = self.proxy.for_project(context.clone());
        let reply_client = client_id.clone();
        let id = window_id.to_owned();
        let spawn = self.blocking_tasks.try_spawn(move || {
            let store = TaskStore::for_repo(&root);
            let result = request_task_result(&store, &reply_client, operation)
                .unwrap_or_else(|error| domain_task_error(&store, error));
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &reply_client,
                BackendEvent::PmRequestTask {
                    id,
                    request_id,
                    result,
                },
            )]));
        });
        match spawn {
            Ok(()) => Vec::new(),
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::PmRequestTask {
                    id: window_id.into(),
                    request_id,
                    result: task_error("unavailable", error, None),
                },
            )],
        }
    }

    pub(super) fn load_pm_reports_events(
        &self,
        context: &ProjectContext,
        client_id: ClientId,
        window_id: &str,
        request_id: u64,
        version: Option<gwt::protocol::PmReportsVersion>,
    ) -> Vec<OutboundEvent> {
        let Some(session_id) = self.pm_chat_session_id(window_id).map(str::to_owned) else {
            return vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::PmReports {
                    id: window_id.to_owned(),
                    session_id: None,
                    request_id,
                    version: None,
                    snapshot: version.is_none(),
                    reports: Vec::new(),
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
            // Keep reads and completion enqueue ordered across concurrent viewers.
            let mut reader = reader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some((next_version, data)) =
                reader.poll(&project_root, pending_update.as_ref(), version.as_ref())
            else {
                return;
            };
            proxy.send(UserEvent::PmReportsLoaded {
                client_id,
                window_id,
                session_id,
                request_id,
                version: Some(next_version),
                snapshot: version.is_none(),
                reports: data
                    .reports
                    .into_iter()
                    .map(gwt::protocol::PmReportView::from)
                    .collect(),
                current: data.current.map(Box::new),
                error: data.error,
            });
        });
        Vec::new()
    }

    pub(crate) fn pm_reports_loaded_events(
        &self,
        client_id: ClientId,
        event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        let BackendEvent::PmReports {
            id,
            session_id: Some(session_id),
            ..
        } = &event
        else {
            return Vec::new();
        };
        if self.pm_chat_session_id(id) != Some(session_id.as_str()) {
            return Vec::new();
        }
        vec![OutboundEvent::reply(client_id, event)]
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

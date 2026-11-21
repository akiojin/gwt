//! The PM's text projection is separate from its unchanged PTY execution.

use gwt::pm_conversation::{PmConversationAvailability, PmConversationSnapshot};
use gwt::protocol::PmRequestTaskResult;
use gwt_core::pm_request_tasks::{Action, Actor, ActorKind, Request, TaskError, TaskStore};
use sha2::{Digest, Sha256};

use super::{
    combined_window_id, AppRuntime, BackendEvent, ClientId, OutboundEvent, ProjectContext,
    UserEvent,
};

pub(super) enum PmRequestTaskOperation {
    Read {
        task_id: String,
        task_key: Option<String>,
        receipt_operation_id: Option<String>,
    },
    Reopen {
        request: gwt::protocol::PmRequestTaskUserRequest,
        task_key: Option<String>,
    },
    Cancel {
        request: gwt::protocol::PmRequestTaskUserRequest,
        task_key: Option<String>,
    },
    Update(gwt::protocol::PmRequestTaskUpdateRequest),
    Complete {
        request: gwt::protocol::PmRequestTaskCompleteRequest,
        task_key: Option<String>,
    },
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
        | gwt::FrontendEvent::ReopenPmRequestTask { id, request_id, .. }
        | gwt::FrontendEvent::CancelPmRequestTask { id, request_id, .. }
        | gwt::FrontendEvent::UpdatePmRequestTask { id, request_id, .. }
        | gwt::FrontendEvent::CompletePmRequestTask { id, request_id, .. } => (id, *request_id),
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

fn resolve_request_task_id(
    store: &TaskStore,
    task_id: String,
    task_key: Option<String>,
) -> Result<String, TaskError> {
    let Some(selected_key) = task_key else {
        return Ok(task_id);
    };
    if selected_key.trim().is_empty() || !task_id.is_empty() {
        return Err(TaskError::Invalid(
            "task_key requires a nonempty key without task_id".into(),
        ));
    }
    let state = store.read().map_err(|error| match error {
        TaskError::Unreadable(_) => TaskError::Unreadable(
            "Request tasks could not be read. Restore this project's saved task store and reload details.".into(),
        ),
        error => error,
    })?.ok_or(TaskError::MissingTask)?;
    state
        .tasks
        .keys()
        .find(|id| format!("{:x}", Sha256::digest(id.as_bytes())) == selected_key)
        .cloned()
        .ok_or(TaskError::MissingTask)
}

fn request_task_result(
    store: &TaskStore,
    client_id: &str,
    operation: PmRequestTaskOperation,
) -> Result<PmRequestTaskResult, TaskError> {
    let keyed_mutation = matches!(
        &operation,
        PmRequestTaskOperation::Reopen {
            task_key: Some(_),
            ..
        } | PmRequestTaskOperation::Cancel {
            task_key: Some(_),
            ..
        }
    );
    let (request, action) = match operation {
        PmRequestTaskOperation::Read {
            task_id,
            task_key,
            receipt_operation_id,
        } => {
            if let Some(selected_key) = task_key {
                if selected_key.trim().is_empty()
                    || !task_id.is_empty()
                    || receipt_operation_id.is_some()
                {
                    return Err(TaskError::Invalid(
                        "task_key requires a nonempty key without task_id or receipt_operation_id"
                            .into(),
                    ));
                }
                let state = store
                    .read()
                    .map_err(|error| match error {
                        TaskError::Unreadable(_) => TaskError::Unreadable(
                            "Request task details could not be read. Restore this project's saved task store and try again.".into(),
                        ),
                        error => error,
                    })?
                    .ok_or(TaskError::MissingTask)?;
                let detail = gwt::pm_current::task_detail(&state, &selected_key)
                    .ok_or(TaskError::MissingTask)?;
                return Ok(PmRequestTaskResult::Detail {
                    revision: state.revision,
                    key: selected_key,
                    detail: Box::new(detail),
                });
            }
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
            return Ok(PmRequestTaskResult::Ready {
                revision: state.revision,
                task: Box::new(task),
                receipt,
            });
        }
        PmRequestTaskOperation::Reopen {
            mut request,
            task_key,
        } => {
            request.task_id = resolve_request_task_id(store, request.task_id, task_key)?;
            let action = Action::Reopen {
                task_id: request.task_id.clone(),
            };
            (request, action)
        }
        PmRequestTaskOperation::Cancel {
            mut request,
            task_key,
        } => {
            request.task_id = resolve_request_task_id(store, request.task_id, task_key)?;
            let action = Action::Cancel {
                task_id: request.task_id.clone(),
            };
            (request, action)
        }
        PmRequestTaskOperation::Update(request) => {
            let task_id = request.task.id.clone();
            let receipt = store.apply(
                &Actor {
                    kind: ActorKind::User,
                    id: format!("browser:{client_id}"),
                },
                &Request {
                    operation_id: request.operation_id,
                    expected_revision: request.expected_revision,
                    reason: request.reason,
                    action: Action::Update { task: request.task },
                },
            )?;
            return current_request_task_result(store, &task_id, receipt);
        }
        PmRequestTaskOperation::Complete {
            mut request,
            task_key,
        } => {
            request.task_id = resolve_request_task_id(store, request.task_id, task_key)?;
            let receipt = store.apply(
                &Actor {
                    kind: ActorKind::User,
                    id: format!("browser:{client_id}"),
                },
                &Request {
                    operation_id: request.operation_id,
                    expected_revision: request.expected_revision,
                    reason: request.reason,
                    action: Action::Complete {
                        task_id: request.task_id.clone(),
                        condition_revision: request.condition_revision,
                        review_generation: request.review_generation,
                        evidence: request.evidence,
                    },
                },
            )?;
            return current_request_task_result(store, &request.task_id, receipt);
        }
    };
    let receipt = store.apply(
        &Actor {
            kind: ActorKind::User,
            id: format!("browser:{client_id}"),
        },
        &Request {
            operation_id: request.operation_id,
            expected_revision: request.expected_revision,
            reason: request.reason,
            action,
        },
    )?;
    if keyed_mutation {
        return current_request_task_result(store, &request.task_id, receipt);
    }
    // An exact retry returns its original applied snapshot, not a claim
    // about current state. Read/receipt reconciliation observes that separately.
    let task = receipt
        .tasks
        .iter()
        .find(|task| task.draft.id == request.task_id)
        .cloned()
        .ok_or_else(|| {
            TaskError::PersistenceUnconfirmed(
                "mutation receipt is missing its task; reconcile the exact operation".into(),
            )
        })?;
    Ok(PmRequestTaskResult::Ready {
        revision: receipt.revision,
        task: Box::new(task),
        receipt: Some(Box::new(receipt)),
    })
}

fn current_request_task_result(
    store: &TaskStore,
    task_id: &str,
    receipt: gwt_core::pm_request_tasks::Receipt,
) -> Result<PmRequestTaskResult, TaskError> {
    // Legal no-ops have an empty task delta. Observe current state separately;
    // the original receipt stays immutable on replay, even after a reopen.
    let state = store.read().map_err(|error| TaskError::PersistenceUnconfirmed(
        format!("Mutation was recorded but its current task read failed: {error}; retain the operation id and load the task and receipt"),
    ))?;
    let unconfirmed = || {
        TaskError::PersistenceUnconfirmed(
        "Mutation was recorded but its current task and receipt could not be reconciled; retain the operation id and load the task and receipt".into(),
    )
    };
    let state = state.ok_or_else(unconfirmed)?;
    if state.revision < receipt.revision || !state.receipts.iter().any(|saved| saved == &receipt) {
        return Err(unconfirmed());
    }
    let task = state.tasks.get(task_id).cloned().ok_or_else(unconfirmed)?;
    Ok(PmRequestTaskResult::Ready {
        revision: state.revision,
        task: Box::new(task),
        receipt: Some(Box::new(receipt)),
    })
}

fn empty(availability: PmConversationAvailability, detail: &str) -> PmConversationSnapshot {
    PmConversationSnapshot {
        conversation_id: None,
        availability,
        messages: Vec::new(),
        detail: Some(detail.to_owned()),
    }
}

/// Request ids of decision deliveries live far above the client-issued ones,
/// so a completion can be matched to its attempt without a second lane.
const DECISION_ANSWER_REQUEST_BASE: u64 = 1 << 62;
const DECISION_ANSWER_ACK_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(crate) struct DecisionAnswerInFlight {
    key: String,
    project_root: std::path::PathBuf,
    decision_id: String,
    waiter: gwt_core::decision_queue::Waiter,
    started: std::time::Instant,
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
                    current_error: None,
                    decisions: None,
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
                current_error: data.current_error,
                decisions: data.decisions,
            });
        });
        Vec::new()
    }

    /// Issue #7742: answer or defer one decision in the gwtd decision queue.
    ///
    /// The queue belongs to the project, not to a PM session: the window only
    /// names the project, so this works while the PM is stopped or replaced.
    pub(super) fn decision_events(
        &self,
        context: &ProjectContext,
        client_id: ClientId,
        window_id: &str,
        request_id: u64,
        decision_id: String,
        answer: Option<String>,
    ) -> Vec<OutboundEvent> {
        let root = self
            .window_lookup
            .get(window_id)
            .and_then(|address| self.tab(&address.tab_id))
            .filter(|tab| tab.project_root == context.project_root)
            .map(|tab| tab.project_root.clone());
        let Some(root) = root else {
            return vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::DecisionResult {
                    id: window_id.to_owned(),
                    request_id,
                    decision_id,
                    action: if answer.is_some() { "answer" } else { "defer" }.to_owned(),
                    ok: false,
                    error: Some("The selected project window is unavailable; reload the current project before retrying.".to_owned()),
                    decisions: None,
                },
            )];
        };
        self.run_decision_request(
            self.proxy.for_project(context.clone()),
            root,
            client_id,
            window_id,
            request_id,
            decision_id,
            answer,
        )
    }

    /// The one queue write behind every surface that answers or defers: the
    /// project window and Home. `reply_id` is echoed back as the result's `id`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_decision_request(
        &self,
        proxy: super::AppEventProxy,
        root: std::path::PathBuf,
        client_id: ClientId,
        reply_id: &str,
        request_id: u64,
        decision_id: String,
        answer: Option<String>,
    ) -> Vec<OutboundEvent> {
        let action = if answer.is_some() { "answer" } else { "defer" };
        let reply_client = client_id.clone();
        let id = reply_id.to_owned();
        let task_decision = decision_id.clone();
        let spawn = self.blocking_tasks.try_spawn(move || {
            use gwt_core::decision_queue as queue;
            let outcome = match &answer {
                Some(text) => queue::answer(&root, &task_decision, text, queue::AnsweredBy::User),
                None => queue::defer(&root, &task_decision),
            };
            let error = outcome.err().map(|error| error.to_string());
            if error.is_none() && answer.is_some() {
                if let Ok(deliveries) = queue::pending_deliveries(&root) {
                    proxy.send(UserEvent::DecisionAnswersPending {
                        project_root: root.clone(),
                        deliveries,
                    });
                }
            }
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &reply_client,
                BackendEvent::DecisionResult {
                    id,
                    request_id,
                    decision_id: task_decision,
                    action: action.to_owned(),
                    ok: error.is_none(),
                    error,
                    decisions: gwt::pm_current::load_decisions(&root),
                },
            )]));
        });
        match spawn {
            Ok(()) => Vec::new(),
            Err(error) => vec![OutboundEvent::reply(
                &client_id,
                BackendEvent::DecisionResult {
                    id: reply_id.to_owned(),
                    request_id,
                    decision_id,
                    action: action.to_owned(),
                    ok: false,
                    error: Some(error.to_string()),
                    decisions: None,
                },
            )],
        }
    }

    /// Look for recorded answers that have not reached their waiters yet.
    /// Runs on the convergence tick so answers written by another process —
    /// the PM closing a decision through gwtd — are delivered without the PM.
    pub(crate) fn scan_decision_answers(&self) {
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        for tab in &self.tabs {
            if !roots.contains(&tab.project_root) {
                roots.push(tab.project_root.clone());
            }
        }
        for root in roots {
            if !gwt_core::decision_queue::queue_path(&root).exists() {
                continue;
            }
            let Some(context) = self.project_context_for_root(&root) else {
                continue;
            };
            let proxy = self.proxy.for_project(context);
            let _ = self.blocking_tasks.try_spawn(move || {
                match gwt_core::decision_queue::pending_deliveries(&root) {
                    Ok(deliveries) if !deliveries.is_empty() => {
                        proxy.send(UserEvent::DecisionAnswersPending {
                            project_root: root,
                            deliveries,
                        });
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(
                        error = %gwt_core::error_ledger::sanitize_error_message(&error.to_string()),
                        "decision queue deliveries could not be read"
                    ),
                }
            });
        }
    }

    /// gwtd hands each answer straight to the session that waits for it: the
    /// text is submitted into that session's own pane, which resumes it. The
    /// delivery is recorded only once the pane owner acknowledged the write; a
    /// waiter without a live pane keeps its delivery pending for a later tick.
    pub(crate) fn deliver_decision_answers(
        &mut self,
        project_root: &std::path::Path,
        deliveries: Vec<gwt_core::decision_queue::PendingDelivery>,
    ) -> Vec<OutboundEvent> {
        use gwt_core::decision_queue::Waiter;
        // An unacknowledged attempt is retried instead of blocking the answer.
        self.decision_answers_in_flight
            .retain(|_, attempt| attempt.started.elapsed() < DECISION_ANSWER_ACK_WINDOW);
        let mut events = Vec::new();
        for delivery in deliveries {
            let key = format!("{}\n{:?}", delivery.decision_id, delivery.waiter);
            if self.decision_answers_sent.contains(&key)
                || self
                    .decision_answers_in_flight
                    .values()
                    .any(|attempt| attempt.key == key)
            {
                continue;
            }
            let session_id = match &delivery.waiter {
                Waiter::Session { session_id } => Some(session_id.clone()),
                Waiter::Pm => self.pm_session_for_root(project_root).cloned(),
            };
            let window_id = session_id.and_then(|session_id| {
                self.tabs
                    .iter()
                    .filter(|tab| tab.project_root == project_root)
                    .find_map(|tab| {
                        tab.workspace
                            .persisted()
                            .windows
                            .iter()
                            .find(|window| window.session_id.as_deref() == Some(&session_id))
                            .map(|window| combined_window_id(&tab.id, &window.id))
                    })
            });
            let Some(window_id) = window_id else {
                continue;
            };
            let Some(runtime) = self.runtimes.get(&window_id) else {
                continue;
            };
            let command = gwt::project_runtime::events::OwnerCommand::SendInput {
                id: runtime.owner_pane_id.clone(),
                incarnation: runtime.incarnation,
                text: format!("{}\r", delivery.message),
            };
            self.decision_answer_sequence += 1;
            let request_id = DECISION_ANSWER_REQUEST_BASE + self.decision_answer_sequence;
            self.decision_answers_in_flight.insert(
                request_id,
                DecisionAnswerInFlight {
                    key,
                    project_root: project_root.to_path_buf(),
                    decision_id: delivery.decision_id,
                    waiter: delivery.waiter,
                    started: std::time::Instant::now(),
                },
            );
            events.extend(self.queue_remote_runtime_command(
                &window_id,
                command,
                super::RemoteRuntimeCommandKind::Input {
                    reply_client: None,
                    request_id: Some(request_id),
                },
            ));
        }
        events
    }

    /// The pane owner answered one delivery attempt. Only an acknowledged
    /// write is recorded as delivered; a failed one is retried on a later tick.
    pub(crate) fn settle_decision_answer(&mut self, request_id: u64, written: bool) {
        let Some(attempt) = self.decision_answers_in_flight.remove(&request_id) else {
            return;
        };
        if !written {
            return;
        }
        self.decision_answers_sent.insert(attempt.key);
        let _ = self.blocking_tasks.try_spawn(move || {
            if let Err(error) = gwt_core::decision_queue::mark_delivered(
                &attempt.project_root,
                &attempt.decision_id,
                &attempt.waiter,
            ) {
                tracing::warn!(
                    error = %gwt_core::error_ledger::sanitize_error_message(&error.to_string()),
                    "decision delivery could not be recorded"
                );
            }
        });
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

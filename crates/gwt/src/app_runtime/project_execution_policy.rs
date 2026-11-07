//! Actual browser view receipts and the existing Close transaction boundary.
use super::*;
use gwt::project_execution_policy::{
    AcceptedCloseIntent, ActualViewReceipt, ObservationInvalidation, RuntimeObservation,
};
use gwt_core::project_execution_policy::{self as store, Choice, ChoiceRequest, EffectState};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct ProjectViewState {
    challenges: HashMap<String, String>,
    observation_unavailable: bool,
    pending_invalidation: Option<ObservationInvalidation>,
    invalidation_inflight: bool,
    views: HashMap<String, (String, ActualViewReceipt)>,
    pub(super) revision: u64,
    last_open_receipt: Option<ActualViewReceipt>,
    close_intents: BTreeMap<String, AcceptedCloseIntent>,
    pub(super) deferred_restores: Vec<PreparedProjectWindowRestore>,
    pub(super) previews: HashMap<String, ClosePreview>,
}

#[derive(Clone)]
pub(super) struct ClosePreview {
    pub(super) policy_revision: u64,
    pub(super) view_revision: u64,
    pub(super) choice_required: bool,
    pub(super) policy_corruption_digest: Option<String>,
    pub(super) view_receipts: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectExecutionChoiceCompleted {
    pub(crate) context: ProjectContext,
    pub(crate) client_id: String,
    pub(crate) operation_id: String,
    pub(crate) closing_receipts: BTreeMap<String, String>,
    pub(crate) result: Result<serde_json::Value, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectObservationInvalidationCompleted {
    pub(crate) context: ProjectContext,
    pub(crate) invalidation: ObservationInvalidation,
    pub(crate) result: Result<(), String>,
}

impl ProjectViewState {
    pub(super) fn view_count(&self) -> usize {
        self.views.len()
    }
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    pub(super) fn view_receipts(&self) -> BTreeMap<String, String> {
        self.views
            .iter()
            .map(|(client, (_, receipt))| (client.clone(), receipt.receipt_id.clone()))
            .collect()
    }
    #[cfg(test)]
    pub(super) fn seed_fixture_view(&mut self, receipt: ActualViewReceipt) {
        self.views.insert(
            "fixture-browser".into(),
            ("fixture-visible-view".into(), receipt.clone()),
        );
        self.last_open_receipt = Some(receipt);
    }
    #[cfg(test)]
    pub(super) fn clear_fixture_views(&mut self) {
        self.views.clear();
        self.last_open_receipt = None;
    }
    #[cfg(test)]
    pub(super) fn pending_invalidation_for_test(&self) -> Option<ObservationInvalidation> {
        self.pending_invalidation.clone()
    }
}

impl AppRuntime {
    pub(super) fn project_view_challenge_events(
        &mut self,
        context: &ProjectContext,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let nonce = uuid::Uuid::new_v4().to_string();
        let Some(state) = self.project_state_mut(context) else {
            return Vec::new();
        };
        state
            .project_views
            .challenges
            .insert(client_id.into(), nonce.clone());
        vec![
            OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectViewChallenge {
                    project_key: context.project_key.to_string(),
                    generation: context.generation,
                    nonce,
                },
            ),
            OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectExecutionStatus {
                    status: self.project_execution_status(context, None),
                },
            ),
        ]
    }

    pub(super) fn accept_project_view_ready(
        &mut self,
        context: &ProjectContext,
        client_id: &str,
        generation: u64,
        nonce: &str,
        view_id: &str,
    ) -> Vec<OutboundEvent> {
        let valid = generation == context.generation
            && !view_id.is_empty()
            && view_id.len() <= 128
            && self.project_state(context).is_some_and(|state| {
                state
                    .project_views
                    .challenges
                    .get(client_id)
                    .map(String::as_str)
                    == Some(nonce)
            });
        if !valid {
            return self.project_close_error(
                client_id,
                &context.project_key.to_string(),
                "StaleViewReceipt",
                "Project view receipt expired or belongs to another client.",
            );
        }
        let policy = store::load(&context.project_root).ok();
        if policy.as_ref().is_some_and(|policy| {
            policy.choice == Some(Choice::Stop)
                && policy
                    .last_operation()
                    .is_none_or(|receipt| receipt.effect_state != EffectState::Confirmed)
        }) {
            return self.project_close_error(client_id, &context.project_key.to_string(), "DrainUnconfirmed", "Project stop is still draining or unconfirmed. Reconcile project.execution.status before reopening.");
        }
        let receipt = ActualViewReceipt {
            receipt_id: uuid::Uuid::new_v4().to_string(),
            accepted_at: chrono::Utc::now(),
            admitted_policy_revision: policy.as_ref().map_or(0, |policy| policy.revision),
        };
        let state = self.project_state_mut(context).expect("current project");
        state.project_views.challenges.remove(client_id);
        state
            .project_views
            .views
            .insert(client_id.into(), (view_id.into(), receipt.clone()));
        state.project_views.last_open_receipt = Some(receipt);
        state.project_views.revision += 1;
        if let Err(error) = self.persist_actual_project_views(context) {
            self.project_state_mut(context)
                .expect("current project")
                .project_views
                .views
                .remove(client_id);
            return self.project_close_error(
                client_id,
                &context.project_key.to_string(),
                "ObservationSaveFailed",
                &error,
            );
        }
        self.project_state_mut(context)
            .expect("current project")
            .project_views
            .observation_unavailable = false;
        let restores = std::mem::take(
            &mut self
                .project_state_mut(context)
                .expect("current project")
                .project_views
                .deferred_restores,
        );
        let mut events = self.restore_prepared_open_project_windows(&context.tab_id, restores);
        events.extend(self.startup_auto_resume_ready_events());
        if self.agent_maintenance.busy {
            if !self.pending_startup_pm_tabs.contains(&context.tab_id) {
                self.pending_startup_pm_tabs.push(context.tab_id.clone());
            }
        } else {
            events.extend(
                self.ensure_pm_agent_for_tab(&context.tab_id, pm::PmEnsureTrigger::Automatic),
            );
        }
        events.push(self.hub_state_broadcast());
        events.push(OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::ProjectExecutionStatus {
                status: self.project_execution_status(context, None),
            },
        ));
        events
    }

    pub(super) fn persist_actual_project_views(
        &mut self,
        context: &ProjectContext,
    ) -> Result<(), String> {
        self.persist_actual_project_views_with_birth(
            context,
            gwt::process::host_process_start_time(std::process::id()),
        )
    }

    pub(super) fn persist_actual_project_views_with_birth(
        &mut self,
        context: &ProjectContext,
        host_started_at: Option<u64>,
    ) -> Result<(), String> {
        let state = self
            .project_state(context)
            .ok_or("Project generation changed")?;
        let host_pid = std::process::id();
        let Some(host_started_at) = host_started_at else {
            let revision = state.project_views.revision;
            self.project_state_mut(context)
                .expect("current project")
                .project_views
                .observation_unavailable = true;
            match gwt::project_execution_policy::revoke_saved_observation(
                &context.project_root,
                host_pid,
                context.generation,
                revision,
            ) {
                Ok(Some(invalidation)) => {
                    self.project_state_mut(context)
                        .expect("current project")
                        .project_views
                        .pending_invalidation = Some(invalidation);
                    self.schedule_project_observation_invalidation(context);
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(%error, "Project observation revocation failed after host birth lookup failure")
                }
            }
            return Err("Host process birth is unavailable".into());
        };
        let observation = RuntimeObservation {
            schema_version: 1,
            project_key: context.project_key.to_string(),
            project_root: context.project_root.clone(),
            generation: context.generation,
            host_pid,
            host_started_at,
            view_set_revision: state.project_views.revision,
            views: state
                .project_views
                .views
                .iter()
                .map(|(client, (_, receipt))| (client.clone(), receipt.clone()))
                .collect::<BTreeMap<_, _>>(),
            last_open_receipt: state.project_views.last_open_receipt.clone(),
            close_intents: state.project_views.close_intents.clone(),
            observed_at: chrono::Utc::now(),
        };
        let result =
            gwt::project_execution_policy::write_observation(&context.project_root, &observation)
                .map_err(|error| error.to_string());
        if result.is_err() {
            self.project_state_mut(context)
                .expect("current project")
                .project_views
                .observation_unavailable = true;
            self.project_state_mut(context)
                .expect("current project")
                .project_views
                .pending_invalidation = Some(ObservationInvalidation {
                host_pid: observation.host_pid,
                host_started_at: observation.host_started_at,
                generation: observation.generation,
                view_set_revision: observation.view_set_revision,
                last_open_receipt_id: observation
                    .last_open_receipt
                    .map(|receipt| receipt.receipt_id),
            });
            self.schedule_project_observation_invalidation(context);
        }
        result
    }

    fn schedule_project_observation_invalidation(&mut self, context: &ProjectContext) {
        let Some(state) = self.project_state_mut(context) else {
            return;
        };
        if state.project_views.invalidation_inflight {
            return;
        }
        let Some(invalidation) = state.project_views.pending_invalidation.clone() else {
            return;
        };
        let root = context.project_root.clone();
        let completion_context = context.clone();
        let proxy = self.proxy.clone();
        let queued = self.blocking_tasks.try_spawn(move || {
            let command = gwt::project_runtime::events::OwnerCommand::InvalidateProjectObservation {
                host_pid:invalidation.host_pid,host_started_at:invalidation.host_started_at,generation:invalidation.generation,
                view_set_revision:invalidation.view_set_revision,last_open_receipt_id:invalidation.last_open_receipt_id.clone(),
            };
            let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string()).and_then(|executor| executor.block_on(async {
                let owner = gwt::project_runtime::client::ProjectRuntimeClient::from_current_project(&root,&gwt_core::paths::gwt_home()).await?;
                owner.command(command).await.map(|_| ())
            }));
            proxy.send(UserEvent::ProjectObservationInvalidationCompleted(Box::new(ProjectObservationInvalidationCompleted { context:completion_context,invalidation,result })));
        });
        match queued {
            Ok(()) => {
                self.project_state_mut(context)
                    .expect("current project")
                    .project_views
                    .invalidation_inflight = true
            }
            Err(error) => {
                tracing::error!(%error,"Project observation invalidation could not start; retained for the next canonical tick")
            }
        }
    }

    pub(crate) fn retry_project_observation_invalidations(&mut self) {
        for context in self.project_contexts() {
            self.schedule_project_observation_invalidation(&context);
        }
    }

    pub(crate) fn project_observation_invalidation_completed(
        &mut self,
        completed: ProjectObservationInvalidationCompleted,
    ) {
        let Some(state) = self.project_state_mut(&completed.context) else {
            return;
        };
        state.project_views.invalidation_inflight = false;
        if completed.result.is_ok()
            && state.project_views.pending_invalidation.as_ref() == Some(&completed.invalidation)
        {
            state.project_views.pending_invalidation = None;
        } else if let Err(error) = completed.result {
            tracing::error!(%error,"Project observation invalidation unconfirmed; retained for the next canonical tick and execution state remains Unknown");
        }
    }

    pub(crate) fn reconcile_project_view_connections(
        &mut self,
        clients: &gwt::project_transport::ClientHub,
    ) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        for context in self.project_contexts() {
            let active = clients.project_browser_client_ids(&context.project_key);
            let Some(state) = self.project_state_mut(&context) else {
                continue;
            };
            let before = state.project_views.views.len();
            state
                .project_views
                .views
                .retain(|client, _| active.contains(client));
            state
                .project_views
                .challenges
                .retain(|client, _| active.contains(client));
            if before != state.project_views.views.len() {
                state.project_views.revision += 1;
                if let Err(error) = self.persist_actual_project_views(&context) {
                    tracing::warn!(%error, "Project view detach observation could not be saved");
                }
                events.push(OutboundEvent::project(
                    context.project_key.clone(),
                    BackendEvent::ProjectExecutionStatus {
                        status: self.project_execution_status(&context, None),
                    },
                ));
                events.push(self.hub_state_broadcast());
            }
        }
        events
    }

    pub(super) fn project_execution_admission(
        &self,
        context: &ProjectContext,
        lineage: &[String],
    ) -> Result<(), String> {
        let unavailable = self.project_state(context).is_none_or(|state| {
            state.project_views.observation_unavailable || state.project_views.views.is_empty()
        });
        if unavailable {
            gwt::project_execution_policy::admission_without_actual_view(
                &context.project_root,
                lineage,
            )
        } else {
            gwt::project_execution_policy::admission(&context.project_root, lineage)
        }
    }

    pub(super) fn project_execution_status(
        &self,
        context: &ProjectContext,
        operation: Option<&str>,
    ) -> gwt::protocol::ProjectExecutionStatusView {
        let mut status =
            gwt::project_execution_policy::local_status(&context.project_root, operation);
        status.generation = context.generation;
        if let Some(state) = self.project_state(context) {
            status.view_set_revision = state.project_views.revision;
            if state.project_views.observation_unavailable {
                status.effective_state = "unknown".into();
                status.start_reason = "runtime_observation_unavailable".into();
                status.degraded = true;
            }
        }
        let owner_observed = self.project_state(context).is_some_and(|state| {
            state.project_owner_identity.is_some()
                && state.project_owner_health.as_ref().is_some_and(|health| {
                    health.state == gwt::protocol::ProjectOwnerHealthState::Healthy
                })
        });
        if !owner_observed {
            status.effective_state = "unknown".into();
            status.degraded = true;
            status.recovery_operation = Some("project.execution.status".into());
            return status;
        }
        if matches!(
            status.start_reason.as_str(),
            "policy_unreadable" | "runtime_observation_unreadable" | "drain_unconfirmed"
        ) || self
            .project_state(context)
            .is_some_and(|state| state.project_views.observation_unavailable)
        {
            return status;
        }
        if let Some(cached) = self.project_state(context).and_then(|state| {
            (!state.project_views.observation_unavailable)
                .then_some(state)
                .and_then(|state| state.project_owner_execution_status.as_ref())
                .filter(|cached| {
                    cached.project_key == context.project_key.as_str()
                        && cached.generation == context.generation
                        && cached.revision == status.revision
                        && cached.view_set_revision == status.view_set_revision
                })
        }) {
            let mut observed = cached.clone();
            observed.operation_outcome = status.operation_outcome;
            return observed;
        }
        let running = self
            .runtimes
            .iter()
            .filter(|(id, runtime)| {
                !runtime.exit_confirmed
                    && self.project_key_for_window(id) == Some(&context.project_key)
            })
            .count();
        let continued_without_view = status.choice == Some(Choice::Continue)
            && status.start_reason == "runtime_observation_unavailable"
            && self
                .project_state(context)
                .is_some_and(|state| !state.project_views.observation_unavailable);
        if running > 0
            && (continued_without_view
                || (status.effective_state != "stopping"
                    && status.effective_state != "unknown"
                    && status.effective_state != "selection_pending"))
        {
            status.effective_state = "running".into();
            if continued_without_view {
                status.start_reason = "authorized_resume".into();
                status.degraded = false;
                status.recovery_operation = None;
            }
        }
        status
    }

    pub(super) fn project_close_error(
        &self,
        client: &str,
        key: &str,
        code: &str,
        message: &str,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client,
            BackendEvent::CloseProjectError {
                project_key: key.into(),
                code: code.into(),
                message: message.into(),
                operation_id: None,
                retry_same_intent: false,
            },
        )]
    }

    fn project_choice_error(
        &self,
        client: &str,
        key: &str,
        code: &str,
        message: &str,
        operation_id: &str,
        retry_same_intent: bool,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client,
            BackendEvent::CloseProjectError {
                project_key: key.into(),
                code: code.into(),
                message: message.into(),
                operation_id: Some(operation_id.into()),
                retry_same_intent,
            },
        )]
    }

    pub(super) fn detach_actual_project_view(
        &mut self,
        context: &ProjectContext,
        client: &str,
        expected_receipts: Option<&BTreeMap<String, String>>,
        completed_operation: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state_mut(context) else {
            return Vec::new();
        };
        let closing = match expected_receipts {
            Some(expected) => state
                .project_views
                .views
                .iter()
                .filter(|(client, (_, receipt))| expected.get(*client) == Some(&receipt.receipt_id))
                .map(|(client, _)| client.clone())
                .collect::<Vec<_>>(),
            None if state.project_views.views.contains_key(client) => vec![client.to_string()],
            None => Vec::new(),
        };
        if closing.is_empty()
            && completed_operation
                .is_some_and(|operation| !state.project_views.close_intents.contains_key(operation))
        {
            return Vec::new();
        }
        let removed_views: HashMap<_, _> = closing
            .iter()
            .filter_map(|client| {
                state
                    .project_views
                    .views
                    .remove(client)
                    .map(|view| (client.clone(), view))
            })
            .collect();
        let completed = completed_operation.map(|operation| {
            (
                operation.to_string(),
                state.project_views.close_intents.remove(operation),
                state.project_views.previews.remove(client),
                state.close_project_nonces.remove(client),
            )
        });
        state.project_views.revision += 1;
        if let Err(error) = self.persist_actual_project_views(context) {
            let state = self.project_state_mut(context).expect("current project");
            state.project_views.views.extend(removed_views);
            if let Some((operation, intent, preview, nonce)) = completed {
                if let Some(intent) = intent {
                    state.project_views.close_intents.insert(operation, intent);
                }
                if let Some(preview) = preview {
                    state.project_views.previews.insert(client.into(), preview);
                }
                if let Some(nonce) = nonce {
                    state.close_project_nonces.insert(client.into(), nonce);
                }
            }
            return self.project_close_error(
                client,
                &context.project_key.to_string(),
                "ObservationSaveFailed",
                &error,
            );
        }
        let mut events = vec![self.hub_state_broadcast()];
        for client in closing {
            events.push(OutboundEvent::reply(
                client,
                BackendEvent::ProjectViewClosed {
                    project_key: context.project_key.to_string(),
                },
            ));
        }
        events
    }

    fn confirmed_project_choice(
        &self,
        context: &ProjectContext,
        operation: &str,
    ) -> Option<store::Receipt> {
        store::load(&context.project_root)
            .ok()?
            .operation(operation)
            .filter(|receipt| {
                receipt.generation == context.generation
                    && receipt.effect_state == EffectState::Confirmed
            })
            .cloned()
    }

    pub(super) fn reconcile_project_choice(
        &mut self,
        context: &ProjectContext,
        operation: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(operation) = operation else {
            return Vec::new();
        };
        let Some(receipt) = self.confirmed_project_choice(context, operation) else {
            return Vec::new();
        };
        let Some(client) = receipt.actor.strip_prefix("browser:") else {
            return Vec::new();
        };
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        let Some(intent) = state.project_views.close_intents.get(operation) else {
            return Vec::new();
        };
        let Some(preview) = state.project_views.previews.get(client) else {
            return Vec::new();
        };
        if intent.request.generation != receipt.generation
            || intent.request.choice != receipt.choice
            || intent.request.expected_revision != receipt.before_revision
            || intent.request.actor != receipt.actor
            || intent.request.consent_source_id != receipt.consent_source_id
            || state.close_project_nonces.get(client) != Some(&receipt.consent_source_id)
            || preview.policy_revision != receipt.before_revision
            || preview.view_revision != intent.view_set_revision
        {
            return Vec::new();
        }
        let closing = preview.view_receipts.clone();
        self.detach_actual_project_view(context, client, Some(&closing), Some(operation))
    }

    pub(super) fn execute_project_choice(
        &mut self,
        context: &ProjectContext,
        client: &str,
        preview: ClosePreview,
        token: &gwt::protocol::CloseProjectToken,
        choice: Choice,
        operation_id: String,
    ) -> Vec<OutboundEvent> {
        let context = context.clone();
        let completion_context = context.clone();
        let client_id = client.to_string();
        let proxy = self.proxy.clone();
        let supervisor = self.daemon_supervisor.clone();
        let request = ChoiceRequest {
            operation_id,
            expected_revision: preview.policy_revision,
            generation: context.generation,
            choice,
            consent_source_id: token.nonce.clone(),
            actor: format!("browser:{client}"),
            authorized_work_lineage: Vec::new(),
        };
        let state = self.project_state_mut(&context).expect("current project");
        if state.project_views.close_intents.values().any(|intent| {
            intent.request.consent_source_id == request.consent_source_id
                && intent.request != request
        }) {
            return self.project_close_error(client, &context.project_key.to_string(), "CloseOperationInFlight", "This Close choice is already bound to an operation. Reconcile its exact operation_id.");
        }
        state.project_views.close_intents.insert(
            request.operation_id.clone(),
            AcceptedCloseIntent {
                request: request.clone(),
                view_set_revision: preview.view_revision,
                policy_corruption_digest: preview.policy_corruption_digest.clone(),
            },
        );
        if let Err(error) = self.persist_actual_project_views(&context) {
            return self.project_choice_error(
                client,
                &context.project_key.to_string(),
                "ChoiceProofSaveFailed",
                &error,
                &request.operation_id,
                true,
            );
        }
        let error_key = context.project_key.to_string();
        let operation_id = request.operation_id.clone();
        let retry_operation_id = operation_id.clone();
        let result = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string()).and_then(|executor| executor.block_on(async {
                let (owner, _gui_presence) = gwt::project_runtime::client::ProjectRuntimeClient::connect_for_gui(context.project_root.clone(), supervisor).await?;
                owner.command(gwt::project_runtime::events::OwnerCommand::ProjectExecutionChoose { request, caller_session_id: None, expected_view_set_revision: Some(preview.view_revision) }).await
            }));
            proxy.send(UserEvent::ProjectExecutionChoiceCompleted(Box::new(ProjectExecutionChoiceCompleted { context: completion_context, client_id, operation_id, closing_receipts: preview.view_receipts, result })));
        });
        match result {
            Ok(()) => Vec::new(),
            Err(error) => self.project_choice_error(
                client,
                &error_key,
                "ChoiceNotStarted",
                &error,
                &retry_operation_id,
                true,
            ),
        }
    }

    pub(crate) fn project_execution_choice_completed(
        &mut self,
        completed: ProjectExecutionChoiceCompleted,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(&completed.context) {
            return Vec::new();
        }
        match completed.result {
            Err(error) => self.project_choice_error(
                &completed.client_id,
                &completed.context.project_key.to_string(),
                "ChoiceUnconfirmed",
                &format!(
                    "{error}. Reconcile project.execution.status using the same operation_id."
                ),
                &completed.operation_id,
                false,
            ),
            Ok(value)
                if value
                    .get("choice_not_started")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true) =>
            {
                self.project_choice_error(
                    &completed.client_id,
                    &completed.context.project_key.to_string(),
                    "ChoiceNotStarted",
                    value
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("Project choice was refused before saving."),
                    &completed.operation_id,
                    true,
                )
            }
            Ok(value) => {
                let mut events = if self
                    .confirmed_project_choice(&completed.context, &completed.operation_id)
                    .is_some_and(|receipt| {
                        receipt.actor == format!("browser:{}", completed.client_id)
                    }) {
                    self.detach_actual_project_view(
                        &completed.context,
                        &completed.client_id,
                        Some(&completed.closing_receipts),
                        Some(&completed.operation_id),
                    )
                } else {
                    Vec::new()
                };
                let status = value
                    .get("status")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .unwrap_or_else(|| {
                        self.project_execution_status(
                            &completed.context,
                            Some(&completed.operation_id),
                        )
                    });
                events.push(OutboundEvent::reply(
                    completed.client_id,
                    BackendEvent::ProjectExecutionStatus { status },
                ));
                events
            }
        }
    }

    pub(crate) fn has_unfinished_continue_work(&self) -> bool {
        self.project_contexts().iter().any(|context| {
            let Ok(policy) = store::load(&context.project_root) else {
                return false;
            };
            if policy.choice != Some(Choice::Continue) {
                return false;
            }
            if gwt::project_execution_policy::has_queued_continue_work(&context.project_root) {
                return true;
            }
            if self.pending_launch_completions.values().any(|pending| {
                pending
                    .context
                    .as_ref()
                    .is_some_and(|pending| pending.project_key == context.project_key)
            }) {
                return true;
            }
            let pm_session = gwt::pm_registry::load_pm_prefs(
                &gwt::pm_registry::pm_prefs_path_for_repo_path(&context.project_root),
            )
            .ok()
            .and_then(|prefs| prefs.registration)
            .map(|registration| registration.session_id);
            self.runtimes.iter().any(|(id, runtime)| {
                if runtime.exit_confirmed
                    || self.project_key_for_window(id) != Some(&context.project_key)
                {
                    return false;
                }
                match self.active_agent_sessions.get(id) {
                    Some(active) => {
                        pm_session.as_deref() != Some(active.session_id.as_str())
                            && gwt_agent::Session::load(
                                &self
                                    .sessions_dir
                                    .join(format!("{}.toml", active.session_id)),
                            )
                            .is_ok_and(|session| {
                                gwt::project_execution_policy::session_work_is_unfinished(&session)
                                    && gwt::project_execution_policy::admission_without_actual_view(
                                        &context.project_root,
                                        &gwt::project_execution_policy::session_lineage(
                                            &context.project_root,
                                            &session,
                                        ),
                                    )
                                    .is_ok()
                            })
                    }
                    None => policy
                        .authorized_work_lineage
                        .contains(&format!("pane:{}", runtime.owner_pane_id)),
                }
            })
        })
    }
}

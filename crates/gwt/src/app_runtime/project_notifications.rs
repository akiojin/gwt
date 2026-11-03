//! Runtime adapters for the process-local notification policy.

use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

use gwt::project_notifications::{NotificationDelivery, NotificationIntent};

use super::{
    AppRuntime, BackendEvent, ClientHub, DispatchTarget, OutboundEvent, WindowProcessStatus,
};

pub(crate) struct PendingNativeNotification {
    pub(crate) intent: NotificationIntent,
    pub(crate) validity: Arc<AtomicBool>,
}

impl AppRuntime {
    pub(crate) fn retain_notification_projects(&mut self) {
        let projects: Vec<_> = self
            .project_contexts()
            .into_iter()
            .map(|context| (context.project_key, context.generation))
            .collect();
        self.notification_coordinator.retain_projects(&projects);
    }

    pub(crate) fn observe_notification_transition_at(
        &mut self,
        window_id: &str,
        status: WindowProcessStatus,
        now: Instant,
    ) {
        if !self
            .window_preset(window_id)
            .is_some_and(|preset| preset.is_agent_terminal())
        {
            return;
        }
        let Some(context) = self
            .window_lookup
            .get(window_id)
            .and_then(|address| self.project_context(&address.tab_id))
        else {
            return;
        };
        let incarnation = self
            .runtimes
            .get(window_id)
            .map(|runtime| runtime.incarnation)
            .or_else(|| {
                self.window_lifecycle_generations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(window_id)
                    .copied()
            })
            .unwrap_or(0);
        self.notification_coordinator
            .open_project(context.project_key.clone(), context.generation);
        self.notification_coordinator.observe_runtime(
            &context.project_key,
            context.generation,
            window_id,
            incarnation,
            status,
            now,
        );
    }

    pub(crate) fn observe_notification_toasts(&mut self, events: &[OutboundEvent]) {
        self.retain_notification_projects();
        for event in events {
            let (
                DispatchTarget::Project(key),
                BackendEvent::IssueMonitorToast {
                    notification_transition:
                        Some(gwt::issue_monitor::MonitorNotificationTransition::NeedsHuman),
                    notification_transition_id: Some(source_id),
                    issue_number: Some(_),
                    ..
                },
            ) = (&event.target, &event.event)
            else {
                continue;
            };
            if let Some(context) = self
                .project_contexts()
                .into_iter()
                .find(|context| &context.project_key == key)
            {
                self.notification_coordinator.observe_needs_human(
                    key,
                    context.generation,
                    source_id,
                );
            }
        }
    }

    pub(crate) fn collect_project_notifications(
        &mut self,
        clients: &ClientHub,
    ) -> (Vec<OutboundEvent>, Vec<PendingNativeNotification>) {
        self.retain_notification_projects();
        let mut events = self.refresh_project_aggregates();
        let mut native = Vec::new();
        for intent in self.notification_coordinator.drain_intents() {
            // A close/reopen drops old pending intents and invalidates native tokens.
            let Some(state) = self.project_aggregates.get_mut(&intent.project_key) else {
                continue;
            };
            self.next_project_aggregate_revision = self
                .next_project_aggregate_revision
                .checked_add(1)
                .expect("project aggregate revision exhausted");
            state.aggregate.unread = true;
            state.aggregate.revision = self.next_project_aggregate_revision;
            events.push(OutboundEvent::project(
                intent.project_key.clone(),
                BackendEvent::ProjectAgentAggregate {
                    aggregate: state.aggregate.clone(),
                },
            ));
            let live = clients.project_browser_client_ids(&intent.project_key);
            match self.notification_coordinator.route(&intent, &live) {
                NotificationDelivery::Browser { client_id } => events.push(OutboundEvent::reply(
                    &client_id,
                    BackendEvent::ProjectNotification {
                        notification: intent,
                    },
                )),
                NotificationDelivery::Native { validity } => {
                    native.push(PendingNativeNotification { intent, validity })
                }
                NotificationDelivery::Suppressed => {}
            }
        }
        (events, native)
    }
}

//! Private owner ACK requests and fenced GUI observations. The project owner
//! commits Monitor materialization and supervises startup beside its real PTY.
use super::*;
use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub(crate) struct LaunchDeliveryAckIdentity {
    generation: u64,
    context: ProjectContext,
    monitor_project_root: PathBuf,
    window_id: String,
    owner_pane_id: String,
    session_id: Option<String>,
    runtime_incarnation: u64,
    owner_identity: gwt::daemon_subscriber::ProjectOwnerIdentity,
    issue_number: u64,
    delivery_id: Option<String>,
    current: Arc<AtomicBool>,
}

pub(crate) struct PendingLaunchDeliveryAck {
    identity: LaunchDeliveryAckIdentity,
}

impl Drop for PendingLaunchDeliveryAck {
    fn drop(&mut self) {
        // A lost GUI completion invalidates only this observation. The owner
        // keeps the accepted operation and its durable Monitor receipt.
        self.identity.current.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub(crate) struct IssueMonitorLaunchDeliveryAcknowledged {
    identity: LaunchDeliveryAckIdentity,
    result: Result<serde_json::Value, String>,
}

impl std::fmt::Debug for IssueMonitorLaunchDeliveryAcknowledged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IssueMonitorLaunchDeliveryAcknowledged(<redacted>)")
    }
}

impl AppRuntime {
    pub(crate) fn invalidate_launch_delivery_ack(&mut self, window_id: &str) {
        self.pending_launch_delivery_acks.remove(window_id);
    }

    pub(super) fn queue_issue_monitor_launch_delivery_ack(
        &mut self,
        project_root: &Path,
        issue_number: u64,
        window_id: &str,
        delivery_id: Option<&str>,
        require_durable: bool,
        handoff_id: Option<&str>,
    ) -> Vec<OutboundEvent> {
        if self
            .pending_launch_delivery_acks
            .get(window_id)
            .is_some_and(|pending| {
                pending.identity.issue_number == issue_number
                    && pending.identity.delivery_id.as_deref() == delivery_id
                    && pending.identity.monitor_project_root == project_root
            })
        {
            return Vec::new();
        }
        let Some(address) = self.window_lookup.get(window_id) else {
            return Vec::new();
        };
        let Some(context) = self.project_context(&address.tab_id) else {
            return Vec::new();
        };
        if !super::same_worktree_path(&context.project_root, project_root) {
            return Vec::new();
        }
        let Some(runtime) = self.runtimes.get(window_id) else {
            return Vec::new();
        };
        let identity = LaunchDeliveryAckIdentity {
            generation: next_window_runtime_incarnation(),
            context,
            monitor_project_root: project_root.to_path_buf(),
            window_id: window_id.to_string(),
            owner_pane_id: runtime.owner_pane_id.clone(),
            session_id: self
                .active_agent_sessions
                .get(window_id)
                .map(|active| active.session_id.clone()),
            runtime_incarnation: runtime.incarnation,
            owner_identity: runtime.owner_identity.clone(),
            issue_number,
            delivery_id: delivery_id.map(str::to_string),
            current: Arc::new(AtomicBool::new(true)),
        };
        let client = runtime.client.clone();
        let command = if require_durable && handoff_id.is_none() {
            gwt::project_runtime::events::OwnerCommand::AcknowledgeMonitorLaunch {
                id: identity.owner_pane_id.clone(),
                incarnation: identity.runtime_incarnation,
                issue_number,
                delivery_id: identity.delivery_id.clone(),
            }
        } else {
            gwt::project_runtime::events::OwnerCommand::AdoptMonitorLaunch {
                id: identity.owner_pane_id.clone(),
                incarnation: identity.runtime_incarnation,
                issue_number,
                delivery_id: identity.delivery_id.clone(),
            }
        };
        self.invalidate_launch_delivery_ack(window_id);
        self.pending_launch_delivery_acks.insert(
            window_id.to_string(),
            PendingLaunchDeliveryAck {
                identity: identity.clone(),
            },
        );
        if let Some(delivery_id) = delivery_id {
            self.issue_monitor_launch_deliveries.insert(
                delivery_id.to_string(),
                IssueMonitorLaunchDeliveryState::LaunchedPendingAck {
                    window_id: window_id.to_string(),
                },
            );
        }
        let proxy = self.proxy.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| runtime.block_on(client.command(command)));
            proxy.send(UserEvent::IssueMonitorLaunchDeliveryAcknowledged(Box::new(
                IssueMonitorLaunchDeliveryAcknowledged { identity, result },
            )));
        }) {
            self.invalidate_launch_delivery_ack(window_id);
            return self.issue_monitor_control_error_events(
                Some(project_root),
                None,
                IssueMonitorControlPublishError::TransportUnavailable(error),
                "launch-succeeded",
                Some(issue_number),
            );
        }
        Vec::new()
    }

    pub(crate) fn handle_issue_monitor_launch_delivery_ack(
        &mut self,
        acknowledged: IssueMonitorLaunchDeliveryAcknowledged,
    ) -> Vec<OutboundEvent> {
        let identity = acknowledged.identity;
        let still_current = self
            .pending_launch_delivery_acks
            .get(&identity.window_id)
            .is_some_and(|pending| pending.identity.generation == identity.generation)
            && identity.current.load(Ordering::Acquire)
            && self.project_context_is_current(&identity.context)
            && self
                .runtimes
                .get(&identity.window_id)
                .is_some_and(|runtime| {
                    runtime.incarnation == identity.runtime_incarnation
                        && runtime.owner_pane_id == identity.owner_pane_id
                        && runtime.owner_identity == identity.owner_identity
                })
            && self
                .active_agent_sessions
                .get(&identity.window_id)
                .map(|active| active.session_id.clone())
                == identity.session_id;
        if !still_current {
            return Vec::new();
        }
        self.pending_launch_delivery_acks
            .remove(&identity.window_id);
        match acknowledged.result {
            Ok(receipt)
                if receipt["id"].as_str() == Some(identity.owner_pane_id.as_str())
                    && receipt["incarnation"].as_u64() == Some(identity.runtime_incarnation) =>
            {
                if receipt["acknowledged"] == true {
                    if let Some(delivery_id) = identity.delivery_id {
                        self.issue_monitor_launch_deliveries.insert(
                            delivery_id,
                            IssueMonitorLaunchDeliveryState::Launched {
                                window_id: identity.window_id,
                            },
                        );
                    }
                }
                Vec::new()
            }
            result => self.issue_monitor_control_error_events(
                Some(&identity.monitor_project_root),
                None,
                IssueMonitorControlPublishError::OutcomeUnknown(
                    result
                        .err()
                        .unwrap_or_else(|| "owner Monitor ACK identity changed".into()),
                ),
                "launch-succeeded",
                Some(identity.issue_number),
            ),
        }
    }
}

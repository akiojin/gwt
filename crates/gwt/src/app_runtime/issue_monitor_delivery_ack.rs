use super::*;
use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub(crate) struct LaunchDeliveryAckIdentity {
    generation: u64,
    context: ProjectContext,
    monitor_project_root: PathBuf,
    window_id: String,
    session_id: Option<String>,
    runtime_incarnation: Option<u64>,
    // Stale failed panes are identified on the worker. Capture only existing
    // runtime identities so cleanup cannot close a later same-id installation.
    live_runtime_incarnations: Arc<HashMap<String, u64>>,
    issue_number: u64,
    delivery_id: Option<String>,
    current: Arc<AtomicBool>,
    handoff_id: Option<String>,
}

pub(crate) struct PendingLaunchDeliveryAck {
    identity: LaunchDeliveryAckIdentity,
}

impl Drop for PendingLaunchDeliveryAck {
    fn drop(&mut self) {
        self.identity.current.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
pub(crate) struct IssueMonitorLaunchDeliveryAcknowledged {
    identity: LaunchDeliveryAckIdentity,
    result:
        Result<Option<LaunchDeliveryAckResult>, (IssueMonitorControlPublishError, &'static str)>,
}

impl std::fmt::Debug for IssueMonitorLaunchDeliveryAcknowledged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("IssueMonitorLaunchDeliveryAcknowledged(<redacted>)")
    }
}

#[derive(Clone, Default)]
struct LaunchDeliveryAckResult {
    monitor: Option<Box<gwt::IssueMonitorState>>,
    stale_window: Option<String>,
    semantic_receipt_settled: bool,
}

struct LaunchDeliveryAckInput {
    identity: LaunchDeliveryAckIdentity,
    materializer_id: String,
    fallback_timeout: Duration,
    barrier: Option<persist_dispatcher::DurableWorkspaceBarrier>,
    _persist: Arc<persist_dispatcher::PersistDispatcher>,
}

fn acknowledge_delivery(input: LaunchDeliveryAckInput) -> IssueMonitorLaunchDeliveryAcknowledged {
    let result = acknowledge_delivery_inner(&input);
    let result = result.map(|result| {
        let semantic_receipt_settled = input.identity.current.load(Ordering::Acquire)
            && input
                .identity
                .delivery_id
                .as_deref()
                .zip(input.identity.handoff_id.as_deref())
                .is_some_and(|(delivery_id, handoff_id)| {
                    AppRuntime::autonomous_answer_receipt_settled_delivery(
                        &input.identity.monitor_project_root,
                        delivery_id,
                        handoff_id,
                    )
                });
        if semantic_receipt_settled {
            let mut result = result.unwrap_or_default();
            result.semantic_receipt_settled = true;
            Some(result)
        } else {
            result
        }
    });
    IssueMonitorLaunchDeliveryAcknowledged {
        identity: input.identity,
        result,
    }
}

fn acknowledge_delivery_inner(
    input: &LaunchDeliveryAckInput,
) -> Result<Option<LaunchDeliveryAckResult>, (IssueMonitorControlPublishError, &'static str)> {
    let identity = &input.identity;
    let root = &identity.monitor_project_root;
    let is_current = || identity.current.load(Ordering::Acquire);
    if !is_current() {
        return Ok(None);
    }
    if let Some(barrier) = &input.barrier {
        let delivery_id = identity
            .delivery_id
            .as_deref()
            .expect("durable delivery id");
        if !AppRuntime::mark_issue_monitor_launch_delivery_materialized(
            root,
            &input.materializer_id,
            input.fallback_timeout,
            identity.issue_number,
            delivery_id,
            &identity.window_id,
        )
        .map_err(|error| (error, "mark-launch-delivery-materialized"))?
        {
            return Ok(None);
        }
        if !is_current() {
            return Ok(None);
        }
        barrier.allow_and_wait().map_err(|error| {
            (
                IssueMonitorControlPublishError::OutcomeUnknown(format!(
                    "launch delivery workspace persistence failed: {error}"
                )),
                "persist-launch-delivery-window",
            )
        })?;
        if !is_current() {
            return Ok(None);
        }
        if !AppRuntime::mark_issue_monitor_launch_delivery_workspace_durable(
            root,
            &input.materializer_id,
            input.fallback_timeout,
            identity.issue_number,
            delivery_id,
            &identity.window_id,
        )
        .map_err(|error| (error, "mark-launch-delivery-workspace-durable"))?
        {
            return Ok(None);
        }
    }
    if !is_current() {
        return Ok(None);
    }
    let stale_window =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(root))
            .ok()
            .and_then(|prefs| {
                prefs
                    .failed_issues
                    .into_iter()
                    .find(|failed| failed.issue_number == identity.issue_number)
                    .and_then(|failed| failed.window_id)
            })
            .filter(|stale| stale != &identity.window_id);
    let mut launched = serde_json::json!({ "issue_number": identity.issue_number, "window_id": identity.window_id });
    if let Some(delivery_id) = &identity.delivery_id {
        launched["delivery_id"] = serde_json::json!(delivery_id);
    }
    // Ownership changes cancel the ticket before the irreversible ACK begins.
    if !is_current() {
        return Ok(None);
    }
    match AppRuntime::publish_issue_monitor_control_owned(
        root,
        serde_json::json!({ "launched": launched }),
    ) {
        Ok(()) => Ok(Some(LaunchDeliveryAckResult {
            monitor: None,
            stale_window,
            semantic_receipt_settled: false,
        })),
        Err(error) if error.allows_local_fallback() => {
            let (monitor, (stale_window, accepted)) =
                AppRuntime::commit_local_issue_monitor_control_with_timeout(
                    root,
                    input.fallback_timeout,
                    |monitor| {
                        if !is_current() {
                            return (None, false);
                        }
                        let stale_window = monitor
                            .prefs()
                            .failed_issues
                            .iter()
                            .find(|failed| failed.issue_number == identity.issue_number)
                            .and_then(|failed| failed.window_id.clone())
                            .filter(|stale| stale != &identity.window_id);
                        let accepted = monitor.complete_active_launch_delivery(
                            identity.issue_number,
                            identity.window_id.clone(),
                            identity.delivery_id.as_deref(),
                        );
                        (stale_window, accepted)
                    },
                )
                .map_err(|error| (error, "launch-succeeded"))?;
            Ok(accepted.then(|| LaunchDeliveryAckResult {
                monitor: Some(Box::new(monitor)),
                stale_window,
                semantic_receipt_settled: false,
            }))
        }
        Err(error) => Err((error, "launch-succeeded")),
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
        let Some(tab) = self.tab(&address.tab_id) else {
            return Vec::new();
        };
        let identity = LaunchDeliveryAckIdentity {
            generation: next_window_runtime_incarnation(),
            context: context.clone(),
            monitor_project_root: project_root.to_path_buf(),
            window_id: window_id.to_string(),
            session_id: tab
                .workspace
                .window(&address.raw_id)
                .and_then(|window| window.session_id.clone()),
            runtime_incarnation: self
                .runtimes
                .get(window_id)
                .map(|runtime| runtime.incarnation),
            live_runtime_incarnations: Arc::new(
                self.runtimes
                    .iter()
                    .map(|(id, runtime)| (id.clone(), runtime.incarnation))
                    .collect(),
            ),
            issue_number,
            delivery_id: delivery_id.map(str::to_string),
            current: Arc::new(AtomicBool::new(true)),
            handoff_id: handoff_id.map(str::to_string),
        };
        let barrier = if require_durable && delivery_id.is_some() {
            match self.persist_dispatcher.reserve_workspace_durable(
                gwt::workspace_state_path(&context.project_root),
                self.persistable_workspace_state(tab),
            ) {
                Ok(barrier) => Some(barrier),
                Err(error) => {
                    return self.issue_monitor_control_error_events(
                        Some(project_root),
                        None,
                        IssueMonitorControlPublishError::OutcomeUnknown(format!(
                            "launch delivery workspace reservation failed: {error}"
                        )),
                        "persist-launch-delivery-window",
                        Some(issue_number),
                    )
                }
            }
        } else {
            None
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
        let input = LaunchDeliveryAckInput {
            identity: identity.clone(),
            barrier,
            materializer_id: self.issue_monitor_materializer_id.clone(),
            fallback_timeout: self.issue_monitor_fallback_commit_timeout,
            _persist: self.persist_dispatcher.clone(),
        };
        let proxy = self.proxy.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            proxy.send(UserEvent::IssueMonitorLaunchDeliveryAcknowledged(Box::new(
                acknowledge_delivery(input),
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
                .map(|runtime| runtime.incarnation)
                == identity.runtime_incarnation
            && self
                .window_lookup
                .get(&identity.window_id)
                .and_then(|address| {
                    self.tab(&address.tab_id)
                        .and_then(|tab| tab.workspace.window(&address.raw_id))
                })
                .and_then(|window| window.session_id.clone())
                == identity.session_id;
        if !still_current {
            return Vec::new();
        }
        self.pending_launch_delivery_acks
            .remove(&identity.window_id);
        match acknowledged.result {
            Ok(Some(result)) => {
                if let Some(delivery_id) = identity.delivery_id {
                    if result.semantic_receipt_settled {
                        self.issue_monitor_launch_deliveries.remove(&delivery_id);
                    } else {
                        self.issue_monitor_launch_deliveries.insert(
                            delivery_id,
                            IssueMonitorLaunchDeliveryState::Launched {
                                window_id: identity.window_id,
                            },
                        );
                    }
                }
                let mut events = result
                    .monitor
                    .map(|monitor| {
                        self.issue_monitor_snapshot_events_for(
                            None,
                            Some(&identity.monitor_project_root),
                            *monitor,
                        )
                    })
                    .unwrap_or_default();
                if let Some(stale_window) = result.stale_window {
                    let same_project = self
                        .window_lookup
                        .get(&stale_window)
                        .and_then(|address| self.project_context(&address.tab_id))
                        .as_ref()
                        == Some(&identity.context);
                    let predates_ack = self
                        .window_lifecycle_generations
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(&stale_window)
                        .is_some_and(|generation| *generation < identity.generation);
                    let same_runtime = self
                        .runtimes
                        .get(&stale_window)
                        .map(|runtime| runtime.incarnation)
                        == identity
                            .live_runtime_incarnations
                            .get(&stale_window)
                            .copied();
                    let launching = self.pending_launch_completions.contains_key(&stale_window)
                        || self
                            .inflight_launches
                            .values()
                            .any(|(id, _)| id == &stale_window);
                    if same_project && predates_ack && same_runtime && !launching {
                        events.extend(self.close_window_events(&stale_window));
                    }
                }
                events
            }
            Ok(None) => Vec::new(),
            Err((error, stage)) => self.issue_monitor_control_error_events(
                Some(&identity.monitor_project_root),
                None,
                error,
                stage,
                Some(identity.issue_number),
            ),
        }
    }
}

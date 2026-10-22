use super::*;
use gwt::runtime_daemon_events::IssueMonitorControlPublishError;
use std::sync::atomic::{AtomicBool, Ordering};

const MONITOR_STARTUP_TIMEOUT_PREFIX: &str = "Issue Monitor startup timed out after ";
const MONITOR_STARTUP_TIMEOUT: Duration = Duration::from_secs(180);

#[cfg(test)]
mod startup_tests {
    #[test]
    fn issue_5415_startup_deadline_records_the_unsent_prompt_and_provider_error() {
        assert_eq!(super::MONITOR_STARTUP_TIMEOUT.as_secs(), 180);
        let error =
            "Could not read Windows sandbox configuration and requirements from the app server.";
        let detail = super::monitor_startup_failure_detail(
            5415,
            Some(&format!("■ {error}\n› $gwt-execute #5415")),
        );
        assert!(detail.contains("initial prompt was not submitted"));
        assert!(detail.contains(error));
        assert!(gwt_agent::is_transient_launch_failure(&detail));
        let other_owner =
            super::monitor_startup_failure_detail(5415, Some("› $gwt-execute #54150"));
        assert!(!other_owner.contains("initial prompt was not submitted"));
    }

    #[test]
    fn issue_5694_unknown_ack_publications_accept_only_exact_committed_identity() {
        use super::*;

        let temp = tempfile::tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let issue = 5694;
        let delivery_id = "launch:effect-5694";
        let window_id = "tab-1::agent-1";
        let materializer = "gui-5694";
        let pending = gwt::issue_monitor::PendingIssueMonitorLaunchDelivery {
            issue_number: issue,
            delivery_id: delivery_id.into(),
            branch_name: "work/issue-5694".into(),
            linked_issue_kind: gwt::LinkedIssueKind::Issue,
            claim_id: "claim-5694".into(),
            claim_owner: "host/session".into(),
            selected_tier: None,
            materializer_id: Some(materializer.into()),
            materializer_pid: Some(std::process::id()),
            materializer_window_id: Some(window_id.into()),
            materialized_window_id: Some(window_id.into()),
            workspace_durable_window_id: Some(window_id.into()),
            launch_session_strategy: gwt::IssueMonitorLaunchSessionStrategy::FreshRequired,
            created_at: chrono::Utc::now().to_rfc3339(),
            requeue_reason: None,
        };
        let prefs = gwt::IssueMonitorPrefs {
            pending_launch_deliveries: vec![pending],
            ..Default::default()
        };
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(&repo);
        gwt::save_issue_monitor_prefs(&prefs_path, &prefs).expect("committed delivery markers");
        let unknown =
            || IssueMonitorControlPublishError::OutcomeUnknown("ACK response lost".into());
        for workspace_durable in [false, true] {
            assert_eq!(
                AppRuntime::issue_monitor_delivery_marker_result(
                    &repo,
                    issue,
                    delivery_id,
                    materializer,
                    window_id,
                    workspace_durable,
                    Err(unknown()),
                ),
                Ok(true),
                "a committed exact marker survives a lost daemon response"
            );
            assert_eq!(
                AppRuntime::issue_monitor_delivery_marker_result(
                    &repo,
                    issue,
                    delivery_id,
                    "another-gui",
                    window_id,
                    workspace_durable,
                    Err(unknown()),
                ),
                Err(unknown()),
                "foreign materialization cannot resolve an unknown publication"
            );
        }
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        assert!(monitor.complete_active_launch_delivery(issue, window_id, Some(delivery_id)));
        let acknowledged = monitor.prefs();
        gwt::save_issue_monitor_prefs(&prefs_path, &acknowledged).expect("committed launch ACK");
        assert!(reconcile_delivery_ack_result(
            &repo,
            issue,
            window_id,
            delivery_id,
            Err((unknown(), "launch-succeeded")),
        )
        .expect("exact completed ACK resolves the lost response")
        .is_some());
        assert!(matches!(
            reconcile_delivery_ack_result(
                &repo,
                issue,
                "tab-1::another-pane",
                delivery_id,
                Err((unknown(), "launch-succeeded")),
            ),
            Err((
                IssueMonitorControlPublishError::OutcomeUnknown(_),
                "launch-succeeded"
            ))
        ));
        assert_eq!(
            gwt::load_issue_monitor_prefs(&prefs_path).expect("readback never mutates prefs"),
            acknowledged
        );
    }
}

/// Only initial Monitor launches are supervised; native resumes, reviews and
/// answered handoffs retain their existing delivery/readiness contracts.
#[derive(Clone)]
pub(crate) struct MonitorStartupWatch {
    generation: u64,
    session_id: String,
    runtime_incarnation: Option<u64>,
    feedback: LaunchFeedbackContext,
}

pub(super) fn monitor_startup_failure_detail(issue: u64, screen: Option<&str>) -> String {
    let screen = screen.unwrap_or_default();
    let owner = format!("#{issue}");
    let unsubmitted = screen.lines().any(|line| {
        let Some(composer) = line.trim_start().strip_prefix('›') else {
            return false;
        };
        let mut words = composer.split_whitespace();
        words.next() == Some("$gwt-execute") && words.next() == Some(owner.as_str())
    });
    let observation = if unsubmitted {
        "initial prompt was not submitted"
    } else {
        "no initial prompt submission or tool activity"
    };
    let error = screen
        .lines()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("could not read windows sandbox configuration")
                || lower.contains("error")
                || lower.contains("failed")
        })
        .map(|line| {
            format!(
                "; last output: {}",
                line.trim().chars().take(800).collect::<String>()
            )
        })
        .unwrap_or_default();
    format!(
        "{MONITOR_STARTUP_TIMEOUT_PREFIX}180s: {observation}{error}; {}",
        gwt_agent::TRANSIENT_LAUNCH_RETRY_HINT
    )
}

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
    let mut result = acknowledge_delivery_inner(&input);
    let identity = &input.identity;
    if let Some(delivery_id) = identity
        .delivery_id
        .as_deref()
        .filter(|_| identity.current.load(Ordering::Acquire) && identity.handoff_id.is_none())
    {
        result = reconcile_delivery_ack_result(
            &identity.monitor_project_root,
            identity.issue_number,
            &identity.window_id,
            delivery_id,
            result,
        );
    }
    if let Err((error, stage)) = &result {
        tracing::warn!(
            error = %error,
            operation = *stage,
            issue_number = identity.issue_number,
            delivery_id = ?identity.delivery_id,
            window_id = %identity.window_id,
            project_root = %identity.monitor_project_root.display(),
            "issue monitor launch delivery ACK is unconfirmed; inspect pending_launch_deliveries"
        );
    }
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

fn reconcile_delivery_ack_result(
    project_root: &Path,
    issue_number: u64,
    window_id: &str,
    delivery_id: &str,
    result: Result<
        Option<LaunchDeliveryAckResult>,
        (IssueMonitorControlPublishError, &'static str),
    >,
) -> Result<Option<LaunchDeliveryAckResult>, (IssueMonitorControlPublishError, &'static str)> {
    if !matches!(
        &result,
        Ok(None) | Err((IssueMonitorControlPublishError::OutcomeUnknown(_), _))
    ) {
        return result;
    }
    // A missing pending row alone cannot distinguish a consumed ACK from expiration.
    let prefs = match gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
        project_root,
    )) {
        Ok(prefs) => prefs,
        Err(error) => {
            return if result.is_err() {
                result
            } else {
                Err((
                    IssueMonitorControlPublishError::OutcomeUnknown(format!(
                        "launch delivery ACK readback failed: {error}"
                    )),
                    "read-launch-delivery-ack",
                ))
            };
        }
    };
    let already_applied = prefs
        .launch_confirmations
        .get(&issue_number)
        .is_some_and(|ack| {
            ack.window_id == window_id
                && ack.delivery_id.as_deref() == Some(delivery_id)
                && ack
                    .claim_id
                    .as_ref()
                    .is_some_and(|claim| prefs.launched_claims.get(&issue_number) == Some(claim))
        })
        && prefs.launch_bindings.get(window_id) == Some(&issue_number)
        && prefs.launched_issues.iter().any(|launched| {
            launched.issue_number == issue_number && launched.window_id == window_id
        })
        && !prefs.pending_launch_deliveries.iter().any(|delivery| {
            delivery.issue_number == issue_number && delivery.delivery_id == delivery_id
        });
    if already_applied {
        Ok(Some(LaunchDeliveryAckResult::default()))
    } else {
        result
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
    match AppRuntime::publish_durable_issue_monitor_control_owned(
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
            if !accepted {
                if let Some(owner) = monitor
                    .conflicting_launch_window_owner(identity.issue_number, &identity.window_id)
                {
                    return Err((
                        IssueMonitorControlPublishError::Rejected(format!(
                            "window {} already belongs to Issue #{owner}; refused Issue #{}",
                            identity.window_id, identity.issue_number
                        )),
                        "launch-succeeded",
                    ));
                }
            }
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
    pub(crate) fn arm_issue_monitor_startup(
        &mut self,
        window_id: &str,
        feedback: &LaunchFeedbackContext,
    ) -> Option<u64> {
        feedback.issue_monitor_launch_binding_issue_number()?;
        if feedback.issue_monitor_review_dispatch
            || feedback.issue_monitor_autonomous_handoff.is_some()
            || feedback
                .issue_monitor_session_mode
                .is_some_and(|mode| mode != gwt_agent::SessionMode::Normal)
        {
            return None;
        }
        let active = self.active_agent_sessions.get(window_id)?;
        let generation = next_window_runtime_incarnation();
        self.pending_monitor_startups.insert(
            window_id.to_string(),
            MonitorStartupWatch {
                generation,
                session_id: active.session_id.clone(),
                runtime_incarnation: self
                    .runtimes
                    .get(window_id)
                    .map(|runtime| runtime.incarnation),
                feedback: feedback.clone(),
            },
        );
        self.rearm_issue_monitor_startup(window_id, generation, MONITOR_STARTUP_TIMEOUT);
        Some(generation)
    }

    fn rearm_issue_monitor_startup(&self, window_id: &str, generation: u64, delay: Duration) {
        let proxy = self.proxy.clone();
        let window_id = window_id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            proxy.send(UserEvent::IssueMonitorStartupTimeout {
                window_id,
                generation,
            });
        });
    }

    pub(crate) fn observe_issue_monitor_startup_activity(
        &mut self,
        window_id: &str,
        event: &gwt::RuntimeHookEvent,
    ) {
        let acknowledged = self
            .pending_monitor_startups
            .get(window_id)
            .is_some_and(|watch| {
                // The child can submit before its pane is installed and replay
                // buffered hooks afterwards. Its exact Session proves ownership.
                event.gwt_session_id.as_deref() == Some(watch.session_id.as_str())
                    && matches!(
                        event.source_event.as_deref(),
                        Some("UserPromptSubmit" | "PreToolUse" | "PostToolUse")
                    )
            });
        if acknowledged {
            self.pending_monitor_startups.remove(window_id);
        }
    }

    pub(crate) fn handle_issue_monitor_startup_timeout(
        &mut self,
        window_id: &str,
        generation: u64,
    ) -> Vec<OutboundEvent> {
        let Some(watch) = self
            .pending_monitor_startups
            .get(window_id)
            .filter(|watch| watch.generation == generation)
            .cloned()
        else {
            return Vec::new();
        };
        let current = self
            .active_agent_sessions
            .get(window_id)
            .is_some_and(|active| active.session_id == watch.session_id)
            && self
                .runtimes
                .get(window_id)
                .map(|runtime| runtime.incarnation)
                == watch.runtime_incarnation;
        if !current {
            self.pending_monitor_startups.remove(window_id);
            return Vec::new();
        }
        let issue = watch
            .feedback
            .issue_monitor_launch_binding_issue_number()
            .expect("Monitor startup owner");
        let screen = self.runtimes.get(window_id).and_then(|runtime| {
            runtime
                .pane
                .lock()
                .ok()
                .map(|pane| pane.screen().contents())
        });
        let detail = monitor_startup_failure_detail(issue, screen.as_deref());
        self.log_window_launch_error("monitor_startup_timeout", window_id, &detail);
        self.invalidate_launch_delivery_ack(window_id);
        let events = if self
            .pending_fresh_execution_launches
            .contains_key(window_id)
        {
            // The exact Prepared candidate rolls back; an Activated candidate
            // reconciles first and keeps this watch until its commit is settled.
            self.fresh_execution_launch_failed_events(window_id, &detail)
        } else if let Some(root) = watch.feedback.issue_monitor_project_root.as_deref() {
            let committed =
                gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(root))
                    .ok()
                    .and_then(|prefs| {
                        let pending = watch
                            .feedback
                            .issue_monitor_delivery_id
                            .as_deref()
                            .filter(|id| {
                                prefs.pending_launch_deliveries.iter().any(|delivery| {
                                    delivery.issue_number == issue && delivery.delivery_id == *id
                                })
                            })
                            .map(str::to_string);
                        let monitor = gwt::IssueMonitorState::with_prefs(
                            gwt::IssueMonitorConfig::default(),
                            prefs,
                        );
                        // ACK consumes the delivery id. Never reuse it or act on a
                        // replacement Issue binding when reporting a post-ACK failure.
                        (pending.is_some()
                            || monitor.launched_window_issue(window_id) == Some(issue))
                        .then(|| {
                            self.issue_monitor_launch_failed_delivery_committed_events_with_mode(
                                Some(root),
                                issue,
                                &detail,
                                pending.as_deref(),
                                gwt_agent::SessionMode::Normal,
                            )
                        })
                    });
            match committed {
                Some((mut events, true)) => {
                    self.stop_window_runtime_without_session_projection(window_id);
                    events.extend(self.close_window_after_issue_monitor_finalize_events(window_id));
                    events
                }
                Some((events, false)) => events,
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        if self.pending_monitor_startups.contains_key(window_id) {
            // Unavailable durable state or a commit in flight cannot authorize
            // teardown. Keep the correlated recovery alive instead of losing it.
            self.rearm_issue_monitor_startup(window_id, generation, Duration::from_secs(60));
        }
        events
    }

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
                gwt_core::paths::gwt_project_dir(&context.project_key).join("workspace.json"),
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
            Ok(None) if identity.delivery_id.is_some() && identity.handoff_id.is_none() => {
                // The claim expired while this exact launch was materializing.
                // Monitor already finalized it; close without another notification.
                self.issue_monitor_launch_deliveries
                    .remove(identity.delivery_id.as_ref().unwrap());
                self.close_window_after_issue_monitor_finalize_events(&identity.window_id)
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

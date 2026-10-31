//! Monitor commits made by the fixed project's physical owner.
use super::launch_payload::LaunchFeedbackContext;
use crate::runtime_daemon_events::{
    IssueMonitorAgentFailureSource, IssueMonitorControlPublishError,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone)]
pub(crate) struct MonitorLaunch {
    pub root: PathBuf,
    pub id: String,
    pub incarnation: u64,
    pub session_id: String,
    pub agent_id: String,
    pub materializer_id: String,
    pub feedback: LaunchFeedbackContext,
    pub current: Arc<AtomicBool>,
    pub source: Option<IssueMonitorAgentFailureSource>,
}

pub(crate) fn publish(
    root: &std::path::Path,
    payload: Value,
) -> Result<(), IssueMonitorControlPublishError> {
    crate::daemon_publisher::publish_durable_issue_monitor_control(
        root,
        crate::runtime_daemon_events::issue_monitor_payload("control", payload, std::process::id()),
    )
}

pub(crate) fn capture_restored_launch(
    root: &Path,
    id: &str,
    incarnation: u64,
    materializer_id: &str,
    current: Arc<AtomicBool>,
    source: &gwt_agent::Session,
    row: Option<&crate::PersistedWindowState>,
) -> Result<Option<MonitorLaunch>, String> {
    if row.and_then(|row| row.session_id.as_deref()) != Some(source.id.as_str()) {
        return Ok(None);
    }
    let Some(issue) = source.linked_issue_number else {
        return Ok(None);
    };
    let prefs =
        crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(root))
            .map_err(|error| error.to_string())?;
    let monitor = crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
    let captured = IssueMonitorAgentFailureSource::Bound {
        issue_number: issue,
        identity: monitor.launch_identity(issue),
    };
    if !captured.matches(&monitor, id) {
        return Ok(None);
    }
    Ok(Some(MonitorLaunch {
        root: root.into(),
        id: id.into(),
        incarnation,
        session_id: source.id.clone(),
        agent_id: source.agent_id.command().into(),
        materializer_id: materializer_id.into(),
        current,
        feedback: LaunchFeedbackContext {
            client_id: "__issue_monitor__".into(),
            title: "Issue Monitor".into(),
            issue_monitor_issue_number: Some(issue),
            issue_monitor_delivery_id: None,
            issue_monitor_project_root: Some(root.into()),
            issue_monitor_session_mode: Some(source.session_mode),
            issue_monitor_autonomous_handoff: None,
            issue_monitor_autonomous_submit_started: false,
            issue_monitor_review_dispatch: false,
        },
        source: Some(captured),
    }))
}

pub(crate) fn launch_failure_message(detail: &str) -> String {
    let detail = super::launch::sanitize_launch_log_error(detail);
    if crate::issue_monitor::is_git_https_auth_error(&detail) {
        crate::issue_monitor::git_https_auth_setup_message(&detail)
    } else {
        detail
    }
}

/// Settle a first facts rejection without constructing a Session, grant or PTY.
/// All payload fields are expectations; the owner proves the refusal again.
pub(crate) fn reject_preflight_launch(
    root: &Path,
    id: &str,
    materializer_id: &str,
    current: Arc<AtomicBool>,
    request: &super::events::OwnerMonitorLaunchRejection,
    snapshot: crate::IssueMonitorWindowSnapshot,
    #[cfg(any(test, feature = "test-gh-guard"))] checkpoint: Option<(
        Arc<super::test_support::OwnerLaunchCheckpoint>,
        crate::agent_capability::AgentCapabilityIssuer,
    )>,
) -> super::events::OwnerReply {
    let expected = request.expected_delivery.as_deref();
    let issue = expected
        .map(|delivery| delivery.issue_number)
        .or_else(|| {
            request
                .expected_handoff
                .as_deref()
                .map(|handoff| handoff.issue_number)
        })
        .filter(|issue| *issue != 0)
        .ok_or("Monitor preflight rejection has no Issue")?;
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let unbound = |prefs: &crate::IssueMonitorPrefs| {
        let monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let identity = monitor.launch_identity(issue);
        !identity.active
            && identity.claim_id.is_none()
            && identity.delivery_id.is_none()
            && identity.window_id.is_none()
            && !prefs
                .pending_launch_deliveries
                .iter()
                .any(|delivery| delivery.issue_number == issue)
            && monitor.conflicting_launch_window_owner(issue, id).is_none()
    };
    let settle = |source: Option<&gwt_agent::Session>| -> super::events::OwnerReply {
        if !current.load(Ordering::Acquire) {
            return Err("Monitor preflight rejection is no longer current".into());
        }
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(root);
        let prefs =
            crate::load_issue_monitor_prefs(&prefs_path).map_err(|error| error.to_string())?;
        if let Some(expected) = expected {
            if prefs
                .pending_launch_deliveries
                .iter()
                .find(|delivery| delivery.issue_number == issue)
                != Some(expected)
                || expected.materializer_id.is_some()
                || expected.materializer_pid.is_some()
                || expected.materializer_window_id.is_some()
                || expected.materialized_window_id.is_some()
                || expected.workspace_durable_window_id.is_some()
                || request.expected_profile_readset.is_some()
                || prefs
                    .effective_launch_tiers()
                    .first()
                    .and_then(|tier| tier.first())
                    != request.expected_profile.as_ref()
            {
                return Err("Monitor preflight delivery, claim or profile changed".into());
            }
        } else {
            let (head, pool) = request
                .expected_profile_readset
                .as_ref()
                .ok_or("answer rejection has no original profile readset")?;
            if (prefs.launch_profile == *head && prefs.launch_profiles == *pool) || !unbound(&prefs)
            {
                return Err(
                    "answer rejection profile change or unbound admission is not confirmed".into(),
                );
            }
        }
        let (environment, _) = gwt_agent::LaunchEnvironment::from_active_profile(
            &gwt_core::paths::gwt_config_path(),
            gwt_agent::LaunchRuntimeTarget::Host,
        )?
        .into_parts();
        let detail = if let Some(handoff) = request.expected_handoff.as_deref() {
            let source = source.ok_or("answered rejection has no exact source Session")?;
            if prefs
                .autonomous_handoffs
                .iter()
                .find(|known| known.handoff_id == handoff.handoff_id)
                != Some(handoff)
                || handoff.issue_number != issue
                || handoff.session_id != source.id
                || handoff.state != crate::autonomous_handoff::AutonomousHandoffState::Resumed
                || !matches!(
                    handoff.delivery,
                    crate::autonomous_handoff::AutonomousHandoffDeliveryState::Pending
                )
                || handoff.delivered_at.is_some()
                || !super::continuation::session_matches_project_state(source, root)
            {
                return Err("answered rejection source changed".into());
            }
            if expected.is_none() {
                "autonomous answer launch facts changed before submit".to_string()
            } else {
                let grok_home =
                    gwt_core::usage::grok::grok_home_from_env(&environment, &source.worktree_path);
                let availability =
                    super::continuation::provider_conversation_availability_with_grok_home(
                        source,
                        grok_home.as_deref(),
                    );
                match availability {
                    super::continuation::ProviderConversationAvailability::Missing
                    | super::continuation::ProviderConversationAvailability::Foreign => {}
                    _ => {
                        return Err(
                            "answered rejection native-store failure is not confirmed".into()
                        )
                    }
                }
                "answered autonomous handoff native conversation is unavailable or foreign"
                    .to_string()
            }
        } else {
            if expected.is_none() {
                return Err("unbound answer rejection has no exact handoff".into());
            }
            if source.is_some() {
                return Err("unexpected source Session in provider rejection".into());
            }
            let profile = request
                .expected_profile
                .as_ref()
                .ok_or("provider rejection has no profile")?;
            if profile.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
                return Err("provider rejection cannot prove remote authentication".into());
            }
            let codex_home = environment
                .get("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(gwt_core::usage::codex::codex_home);
            let claude_home = environment
                .get("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(gwt_core::usage::claude::claude_home);
            if codex_home.as_ref().is_some_and(|home| !home.is_absolute())
                || claude_home.as_ref().is_some_and(|home| !home.is_absolute())
            {
                return Err("provider rejection credential home is unresolved".into());
            }
            let auth = crate::issue_monitor::provider_auth_state(
                &profile.agent_id,
                &crate::issue_monitor::ProviderAuthProbe {
                    codex_home: codex_home.as_deref(),
                    claude_home: claude_home.as_deref(),
                    openai_api_key: environment.get("OPENAI_API_KEY").map(String::as_str),
                    anthropic_api_key: environment.get("ANTHROPIC_API_KEY").map(String::as_str),
                },
            );
            if auth != crate::issue_monitor::ProviderAuthState::Unauthenticated {
                return Err("provider rejection authentication failure is not confirmed".into());
            }
            crate::issue_monitor::provider_unauthenticated_message(&profile.agent_id)
        };
        let claimed = if let Some(expected) = expected {
            let claim = super::installation::IssueMonitorLaunchClaimInput {
                project_root: root.into(),
                issue_number: issue,
                delivery_id: expected.delivery_id.clone(),
                materializer_window_id: id.into(),
                materializer_id: materializer_id.into(),
                materializer_pid: std::process::id(),
                fallback_timeout:
                    super::installation::local_issue_monitor_fallback_projection_timeout(),
                snapshot: Some(snapshot.clone()),
                snapshot_tabs: std::collections::BTreeSet::from([snapshot.project_tab_id.clone()]),
            };
            if !claim.claim_or_reuse().map_err(|error| error.to_string())? {
                #[cfg(any(test, feature = "test-gh-guard"))]
                {
                    let mut observed = crate::IssueMonitorState::with_prefs(
                        crate::IssueMonitorConfig::default(),
                        prefs.clone(),
                    );
                    crate::issue_monitor_worker::bind_issue_monitor_source_repository(
                        &mut observed,
                        root,
                    );
                    let candidate = crate::issue_cache::issue_cache_root_for_repo_path(root)
                        .and_then(|cache| {
                            crate::issue_monitor_worker::load_cached_issue_monitor_candidate(
                                &cache, issue,
                            )
                        });
                    let readiness = candidate.as_ref().map(|candidate| candidate.readiness);
                    if let Some(candidate) = candidate {
                        observed.record_candidate(candidate);
                    }
                    if let Some(birth) = crate::process::host_process_start_time(std::process::id())
                    {
                        observed.record_window_snapshot_from_host(
                            snapshot.clone(),
                            std::process::id(),
                            birth,
                            claim.snapshot_tabs.clone(),
                        );
                    } else {
                        observed.record_window_snapshot_for_tabs(
                            snapshot.clone(),
                            claim.snapshot_tabs.clone(),
                        );
                    }
                    let status = observed.agent_status_at(&chrono::Utc::now().to_rfc3339());
                    let unissued = prefs
                        .pending_launch_deliveries
                        .iter()
                        .filter(|delivery| {
                            delivery.materializer_id.is_none()
                                && delivery.materialized_window_id.is_none()
                                && !prefs
                                    .launched_issues
                                    .iter()
                                    .any(|launched| launched.issue_number == delivery.issue_number)
                        })
                        .collect::<Vec<_>>();
                    return Err(format!("Monitor preflight rejection could not claim its exact delivery; source={:?}; readiness={readiness:?}; schedule={:?}; dependency={:?}; capacity={}; max={}; occupied={:?}; unissued={}; position={:?}; pane={}; snapshot_windows={}; live_claim_matches={}; local_preflight={}",
                    observed.launch_source_exclusion(issue), observed.agent_schedule_launch_exclusion(issue),
                    observed.launch_dependency_exclusion(issue), observed.has_capacity_for_monitor_spawn(issue, false),
                    observed.effective_max_active_agents(), status.occupied_slot_count, unissued.len(),
                    unissued.iter().position(|delivery| delivery.delivery_id == expected.delivery_id),
                    observed.has_monitor_pane_for_issue(issue), snapshot.windows.len(),
                    observed.live_claim_id(issue).as_deref() == Some(expected.claim_id.as_str()),
                    claim.test_preflight_claim(prefs.clone(), crate::process::host_process_start_time(std::process::id()))));
                }
                #[cfg(not(any(test, feature = "test-gh-guard")))]
                return Err(
                    "Monitor preflight rejection could not claim its exact delivery".into(),
                );
            }
            let claimed = crate::load_issue_monitor_prefs(&prefs_path)
                .map_err(|error| error.to_string())?
                .pending_launch_deliveries
                .into_iter()
                .find(|delivery| {
                    delivery.issue_number == issue
                        && delivery.delivery_id == expected.delivery_id
                        && delivery.claim_id == expected.claim_id
                        && delivery.materializer_id.as_deref() == Some(materializer_id)
                        && delivery.materializer_pid == Some(std::process::id())
                        && delivery.materializer_window_id.as_deref() == Some(id)
                })
                .ok_or("Monitor preflight claim readback changed")?;
            Some(claimed)
        } else {
            None
        };
        let mut launch = MonitorLaunch {
            root: root.into(),
            id: id.into(),
            incarnation: 0,
            session_id: source.map(|source| source.id.clone()).unwrap_or_default(),
            agent_id: source
                .map(|source| source.agent_id.command().to_string())
                .or_else(|| {
                    request
                        .expected_profile
                        .as_ref()
                        .map(|profile| profile.agent_id.clone())
                })
                .unwrap_or_default(),
            materializer_id: materializer_id.into(),
            current: current.clone(),
            source: None,
            feedback: LaunchFeedbackContext {
                client_id: "__issue_monitor__".into(),
                title: "Issue Monitor".into(),
                issue_monitor_issue_number: Some(issue),
                issue_monitor_delivery_id: expected.map(|expected| expected.delivery_id.clone()),
                issue_monitor_project_root: Some(root.into()),
                issue_monitor_session_mode: Some(if source.is_some() {
                    gwt_agent::SessionMode::Resume
                } else {
                    gwt_agent::SessionMode::Normal
                }),
                issue_monitor_autonomous_handoff: None,
                issue_monitor_autonomous_submit_started: false,
                issue_monitor_review_dispatch: false,
            },
        };
        launch.check_current()?;
        let mut answer_target = None;
        if let Some(source) = source {
            let prepared = if let Some(expected) = expected {
                prepare_issue_monitor_handoff_delivery(
                    &prefs_path,
                    issue,
                    source,
                    id,
                    Some(&expected.delivery_id),
                    materializer_id,
                    false,
                )?
            } else {
                let target = super::installation::autonomous_handoff_delivery_target_for_session(
                    source,
                    issue,
                    id,
                    None,
                    materializer_id,
                )?;
                let (_, prepared) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                    let (head, pool) =
                        request.expected_profile_readset.as_ref().ok_or_else(|| {
                            std::io::Error::other(
                                "answer rejection lost its original profile readset",
                            )
                        })?;
                    if !current.load(Ordering::Acquire)
                        || !unbound(prefs)
                        || (prefs.launch_profile == *head && prefs.launch_profiles == *pool)
                        || prefs.autonomous_handoffs.iter().find(|handoff| {
                            request
                                .expected_handoff
                                .as_deref()
                                .is_some_and(|expected| handoff.handoff_id == expected.handoff_id)
                        }) != request.expected_handoff.as_deref()
                    {
                        return Err(std::io::Error::other(
                            "answer rejection source or admission changed before write-ahead",
                        ));
                    }
                    let mut monitor = crate::IssueMonitorState::with_prefs(
                        crate::IssueMonitorConfig::default(),
                        prefs.clone(),
                    );
                    let prepared = monitor.prepare_autonomous_handoff_delivery(
                        issue,
                        &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    );
                    *prefs = monitor.prefs();
                    Ok(prepared)
                })
                .map_err(|error| error.to_string())?;
                answer_target = Some(target);
                prepared
            };
            let Some(crate::AutonomousHandoffDeliveryPreparation::Ready(attempt)) = prepared else {
                return Err("answered rejection write-ahead is not available".into());
            };
            if let Some(target) = answer_target.as_ref() {
                if !crate::bind_autonomous_handoff_delivery_target_from_prefs(
                    &prefs_path,
                    &attempt.handoff_id,
                    &attempt.session_id,
                    attempt.attempt,
                    target,
                )
                .map_err(|error| error.to_string())?
                {
                    return Err("answered rejection exact target bind remains unconfirmed".into());
                }
            }
            launch.feedback.issue_monitor_autonomous_handoff = Some(attempt);
        }
        launch.check_current()?;
        #[cfg(any(test, feature = "test-gh-guard"))]
        if let Some((checkpoint, issuer)) = checkpoint.as_ref().filter(|(checkpoint, _)| {
            checkpoint.stage == super::test_support::OwnerLaunchStage::PreparedBeforeMailbox
        }) {
            checkpoint.pause(super::test_support::OwnerLaunchEvidence {
                id: id.into(),
                incarnation: 0,
                operation_id: request.operation_id.clone(),
                session_id: source.map(|source| source.id.clone()),
                current: current.clone(),
                session: source.cloned(),
                pty: None,
                issuer: issuer.clone(),
                grant: None,
                pending_continue_work: None,
                pending_fresh_execution: None,
            });
        }
        // This function has no process/PTY/input call. The real owner claim
        // and, for answers, its exact write-ahead fence are the only effects.
        if !launch.commit_unspawned_failure(&detail, true, answer_target.as_ref())? {
            return Err("Monitor preflight failure settlement remains unconfirmed".into());
        }
        let after =
            crate::load_issue_monitor_prefs(&prefs_path).map_err(|error| error.to_string())?;
        if after
            .pending_launch_deliveries
            .iter()
            .any(|delivery| delivery.issue_number == issue)
            || (source.is_none()
                && !after
                    .failed_issues
                    .iter()
                    .any(|failed| failed.issue_number == issue && failed.message == detail))
        {
            return Err("Monitor preflight failure readback changed".into());
        }
        Ok(
            json!({"operation_id":request.operation_id,"claimed_delivery":claimed,
            "no_pty":true,"no_submit":true,"settled":true,"message":detail,
            "handoff_retry":source.is_some(),"answer_target":answer_target}),
        )
    };
    if let Some(expected_source) = request.expected_session.as_deref() {
        gwt_agent::with_session_lease(&sessions, &expected_source.id, |source| {
            if serde_json::to_value(source).map_err(std::io::Error::other)?
                != serde_json::to_value(expected_source).map_err(std::io::Error::other)?
            {
                return Err(std::io::Error::other("preflight rejection Session changed"));
            }
            settle(Some(source)).map_err(std::io::Error::other)
        })
        .map_err(|error| error.to_string())
    } else {
        settle(None)
    }
}

impl MonitorLaunch {
    fn issue(&self) -> Result<u64, String> {
        self.feedback
            .issue_monitor_launch_binding_issue_number()
            .ok_or_else(|| "owner launch has no Monitor Issue".into())
    }

    fn check_current(&self) -> Result<(), String> {
        if self.current.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("Monitor launch incarnation is no longer current".into())
        }
    }

    /// The registry commit is the owner's barrier; GUI layout persistence is
    /// never allowed to authorize a materialized Session.
    pub fn commit_launch(
        &self,
        ready: bool,
    ) -> Result<Option<IssueMonitorAgentFailureSource>, String> {
        self.check_current()?;
        let issue = self.issue()?;
        let before = self.read_monitor()?;
        // OFF revokes existing reservations, not the materialized owner. Auto
        // may already be OFF while a valid manual reservation awaits its ACK.
        if !before.prefs().enabled && !before.launch_identity(issue).active {
            self.check_current()?;
            return Ok(None);
        }
        if ready {
            if let Some(source) = self.confirmed_source(&before) {
                return Ok(Some(source));
            }
        }
        if self.feedback.issue_monitor_autonomous_submit_started
            && before.live_claim_id(issue).is_none()
            && before.pending_launch_delivery_id(issue).is_none()
            && before.launched_window_id(issue).is_none()
            && before.conflicting_launch_window_owner(issue, &self.id).is_none()
            && self.feedback.issue_monitor_autonomous_handoff.as_ref().is_some_and(|attempt| {
                attempt.issue_number == issue
                    && before.prefs().autonomous_handoffs.iter().any(|handoff| {
                        handoff.handoff_id == attempt.handoff_id
                            && handoff.session_id == attempt.session_id
                            && handoff.issue_number == issue
                            && handoff.state == crate::autonomous_handoff::AutonomousHandoffState::AwaitingHuman
                            && handoff.delivered_at.is_none()
                            && matches!(&handoff.delivery,
                                crate::autonomous_handoff::AutonomousHandoffDeliveryState::Ambiguous {
                                    attempt: number, prompt_sha256, ..
                                } if *number == attempt.attempt && prompt_sha256 == &attempt.prompt_sha256)
                    })
            })
        {
            // An exact parked answer is settled without an ACK. Its missing
            // delivery is not expiry authority over the existing live holder.
            self.check_current()?;
            return Ok(None);
        }
        let expected_claim = before.live_claim_id(issue);
        if let Some(delivery) = self.feedback.issue_monitor_delivery_id.as_deref() {
            for durable in [false, true] {
                if durable {
                    let registry = crate::persistence::load_workspace_state(
                        &crate::persistence::workspace_state_path(&self.root),
                    )
                    .map_err(|error| error.to_string())?;
                    let raw = self
                        .id
                        .split_once("::")
                        .map(|(_, raw)| raw)
                        .ok_or("invalid owner pane ID")?;
                    if !registry.windows.iter().any(|row| {
                        row.id == raw && row.session_id.as_deref() == Some(&self.session_id)
                    }) {
                        return Err(
                            "owner launch registry commit does not contain its exact Session"
                                .into(),
                        );
                    }
                }
                self.check_current()?;
                let key = if durable {
                    "launch_delivery_workspace_durable"
                } else {
                    "launch_delivery_materialized"
                };
                let publication = publish(
                    &self.root,
                    json!({key: {
                        "issue_number":issue, "delivery_id":delivery,
                        "materializer_id":self.materializer_id, "materializer_window_id":self.id,
                    }}),
                );
                let publication = match publication {
                    Err(error) if error.allows_local_fallback() => {
                        super::installation::try_commit_local_issue_monitor_control_with_timeout(
                            &self.root,
                            super::installation::local_issue_monitor_fallback_projection_timeout(),
                            |monitor| {
                                if !self.current.load(Ordering::Acquire)
                                    || !self.owns_pending_delivery(monitor)
                                    || monitor.live_claim_id(issue) != expected_claim
                                    || !monitor.prefs().pending_launch_deliveries.iter().any(|pending| {
                                        pending.issue_number == issue
                                            && pending.delivery_id == delivery
                                            && Some(pending.claim_id.as_str()) == expected_claim.as_deref()
                                    })
                                {
                                    return Err(std::io::Error::other(
                                        "Monitor materialization no longer matches this owner claim",
                                    ));
                                }
                                let committed = if durable {
                                    monitor.mark_launch_delivery_workspace_durable(
                                        issue, delivery, &self.materializer_id, &self.id,
                                    )
                                } else {
                                    monitor.mark_launch_delivery_materialized(
                                        issue, delivery, &self.materializer_id, &self.id,
                                    )
                                };
                                committed.then_some(()).ok_or_else(|| std::io::Error::other(
                                    "Monitor materialization no longer matches this owner",
                                ))
                            },
                        )
                        .map(|_| ())
                    }
                    result => result,
                };
                if !super::installation::issue_monitor_delivery_marker_result(
                    &self.root,
                    issue,
                    delivery,
                    &self.materializer_id,
                    &self.id,
                    durable,
                    publication,
                )
                .map_err(|error| error.to_string())?
                {
                    return Err("Monitor materialization no longer matches this owner".into());
                }
            }
        }
        if !ready {
            return Ok(None);
        }
        self.check_current()?;
        let mut launched = json!({"issue_number":issue,"window_id":self.id});
        if let Some(delivery) = &self.feedback.issue_monitor_delivery_id {
            launched["delivery_id"] = json!(delivery);
        }
        let publication = publish(&self.root, json!({"launched":launched}));
        let publication =
            match (
                self.feedback.issue_monitor_delivery_id.as_deref(),
                publication,
            ) {
                (Some(delivery), Err(error)) if error.allows_local_fallback() => {
                    super::installation::try_commit_local_issue_monitor_control_with_timeout(
                        &self.root,
                        super::installation::local_issue_monitor_fallback_projection_timeout(),
                        |monitor| {
                            if !self.current.load(Ordering::Acquire)
                                || !self.owns_pending_delivery(monitor)
                                || monitor.live_claim_id(issue) != expected_claim
                                || !monitor.prefs().pending_launch_deliveries.iter().any(
                                    |pending| {
                                        pending.issue_number == issue
                                            && pending.delivery_id == delivery
                                            && Some(pending.claim_id.as_str())
                                                == expected_claim.as_deref()
                                    },
                                )
                            {
                                return Err(std::io::Error::other(
                                    "Monitor ACK no longer matches this owner claim",
                                ));
                            }
                            monitor
                                .complete_active_launch_delivery(issue, &self.id, Some(delivery))
                                .then_some(())
                                .ok_or_else(|| {
                                    std::io::Error::other(
                                        "Monitor launch ACK no longer matches this owner",
                                    )
                                })
                        },
                    )
                    .map(|_| ())
                }
                (_, result) => result,
            };
        if let Some(delivery) = self.feedback.issue_monitor_delivery_id.as_deref() {
            super::installation::reconcile_delivery_ack_result(
                &self.root,
                issue,
                &self.id,
                delivery,
                publication
                    .map(|_| Some(super::installation::LaunchDeliveryAckResult::default()))
                    .map_err(|error| (error, "launch-succeeded")),
            )
            .map_err(|(error, _)| error.to_string())?
            .ok_or("Monitor launch ACK is unconfirmed")?;
        } else if let Err(error) = publication {
            let monitor = self.read_monitor()?;
            if monitor.launched_window_issue(&self.id) != Some(issue)
                || monitor.live_claim_id(issue) != expected_claim
            {
                return Err(error.to_string());
            }
        }
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&self.root),
        )
        .map_err(|error| error.to_string())?;
        let monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let source = IssueMonitorAgentFailureSource::Bound {
            issue_number: issue,
            identity: monitor.launch_identity(issue),
        };
        if !source.matches(&monitor, &self.id) || monitor.live_claim_id(issue) != expected_claim {
            return Err("Monitor ACK did not bind the exact owner pane".into());
        }
        Ok(Some(source))
    }

    pub fn heartbeat(&self, at: Option<&str>, source_event: Option<&str>) -> Result<(), String> {
        self.check_current()?;
        publish(&self.root,json!({"heartbeat":{
            "issue_number":self.issue()?, "at":at.map(str::to_string).unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            "agent_id":self.agent_id, "source_event":source_event,
        }})).map_err(|error| error.to_string())
    }

    pub fn failure(
        &self,
        detail: &str,
        failure: Option<&crate::IssueMonitorFailure>,
        terminal: bool,
    ) -> Result<(), String> {
        if !terminal {
            self.check_current()?;
        }
        let payload = if let Some(source) = &self.source {
            super::runtime_events::issue_monitor_agent_failed_from_launch_payload(
                &self.id, detail, source, failure,
            )
        } else {
            let mut payload = super::runtime_events::issue_monitor_launch_failed_payload(
                self.issue()?,
                detail,
                self.feedback.issue_monitor_delivery_id.as_deref(),
                Some(&self.materializer_id),
                self.feedback
                    .issue_monitor_session_mode
                    .unwrap_or(gwt_agent::SessionMode::Normal),
            );
            if let Some(failure) = failure {
                payload["launch_failed"]["failure"] = json!(failure);
            }
            payload
        };
        publish(&self.root, payload).map_err(|error| error.to_string())
    }

    /// A failed worker has proved that no physical candidate remains. Only
    /// its actual pending delivery or exact unbound answer target can settle admission.
    pub(crate) fn commit_unspawned_failure(
        &self,
        detail: &str,
        retryable: bool,
        no_delivery_target: Option<&crate::autonomous_handoff::AutonomousHandoffDeliveryTarget>,
    ) -> Result<bool, String> {
        let message = launch_failure_message(detail);
        let detail = message.as_str();
        let before = self.read_monitor()?;
        if let Some(source) = self.source.as_ref() {
            if !source.matches(&before, &self.id) {
                return Ok(false);
            }
            let publication = self.failure(detail, None, true);
            let after = self.read_monitor()?;
            if source.matches(&after, &self.id) {
                return Err(publication
                    .err()
                    .unwrap_or_else(|| "restored Monitor failure is unconfirmed".into()));
            }
            self.closed(None)?;
            return Ok(after
                .conflicting_launch_window_owner(self.issue()?, &self.id)
                .is_none());
        }
        if self.feedback.issue_monitor_delivery_id.is_some() && !self.owns_pending_delivery(&before)
        {
            return Ok(false);
        }
        if let Some(attempt) = self.feedback.issue_monitor_autonomous_handoff.as_ref() {
            let issue = self.issue()?;
            let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&self.root);
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let (_, committed) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                // Zero identifies the no-pane preflight rejection. Captured
                // failed runtimes may compensate after their writer is revoked.
                if self.incarnation == 0 {
                    self.check_current().map_err(std::io::Error::other)?;
                }
                let mut monitor = crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(), prefs.clone(),
                );
                let owns_admission = if self.feedback.issue_monitor_delivery_id.is_some() {
                    self.owns_pending_delivery(&monitor)
                } else {
                    let identity = monitor.launch_identity(issue);
                    no_delivery_target.is_some_and(|target| {
                        target.gwt_session_id == self.session_id
                            && target.window_id == self.id
                            && target.materializer_id == self.materializer_id
                            && target.materializer_pid == std::process::id()
                            && target.materializer_started_at != 0
                            && crate::process::host_process_start_time(target.materializer_pid)
                                == Some(target.materializer_started_at)
                            && target.issue_number == issue
                            && attempt.issue_number == issue
                            && target.delivery_id.is_none()
                            && crate::launch_runtime::same_worktree_path(
                                Path::new(&target.project_state_root), &self.root,
                            )
                    }) && !identity.active && identity.claim_id.is_none()
                        && identity.delivery_id.is_none() && identity.window_id.is_none()
                        && !prefs.pending_launch_deliveries.iter()
                            .any(|pending| pending.issue_number == issue)
                        && monitor.conflicting_launch_window_owner(issue, &self.id).is_none()
                };
                let exact_attempt = prefs.autonomous_handoffs.iter().any(|handoff| {
                    handoff.handoff_id == attempt.handoff_id
                        && handoff.session_id == attempt.session_id
                        && handoff.issue_number == attempt.issue_number
                        && matches!(&handoff.delivery,
                            crate::autonomous_handoff::AutonomousHandoffDeliveryState::Attempting {
                                attempt: number, prompt_sha256, materializer_pid,
                                materializer_started_at, target, ..
                            } if *number == attempt.attempt && prompt_sha256 == &attempt.prompt_sha256
                                && *materializer_pid == std::process::id()
                                && crate::process::host_process_start_time(*materializer_pid) == Some(*materializer_started_at)
                                && if self.feedback.issue_monitor_delivery_id.is_some() {
                                    target.as_ref().is_none_or(|target| {
                                        target.gwt_session_id == self.session_id
                                            && target.window_id == self.id
                                            && target.materializer_id == self.materializer_id
                                            && target.delivery_id == self.feedback.issue_monitor_delivery_id
                                    })
                                } else {
                                    handoff.state == crate::autonomous_handoff::AutonomousHandoffState::Resumed
                                        && handoff.delivered_at.is_none()
                                        && target.as_deref().zip(no_delivery_target)
                                            .is_some_and(|(target, expected)| {
                                                target == expected && handoff.provider == target.provider
                                            })
                                })
                });
                if !owns_admission || !exact_attempt {
                    return Ok(false);
                }
                let committed = if self.feedback.issue_monitor_autonomous_submit_started {
                    monitor.mark_autonomous_handoff_delivery_ambiguous(
                        &attempt.handoff_id, &attempt.session_id, attempt.attempt, detail, &now,
                    );
                    // A reaped child does not prove that its answer was never submitted.
                    false
                } else {
                    !matches!(monitor.record_autonomous_handoff_delivery_failure(
                        &attempt.handoff_id, &attempt.session_id, attempt.attempt, detail, &now,
                    ), crate::AutonomousHandoffDeliveryFailureOutcome::Rejected)
                };
                *prefs = monitor.prefs();
                Ok(committed)
            }).map_err(|error| error.to_string())?;
            let after = self.read_monitor()?;
            return Ok(committed && !self.owns_pending_delivery(&after)
                && after.prefs().autonomous_handoffs.iter().any(|handoff| {
                    handoff.handoff_id == attempt.handoff_id
                        && handoff.session_id == attempt.session_id
                        && (self.feedback.issue_monitor_delivery_id.is_some()
                            || handoff.issue_number == issue && handoff.delivered_at.is_none())
                        && matches!(handoff.delivery,
                            crate::autonomous_handoff::AutonomousHandoffDeliveryState::RetryBackoff { attempt: number, .. }
                            | crate::autonomous_handoff::AutonomousHandoffDeliveryState::Exhausted { attempt: number, .. }
                            if number == attempt.attempt)
                }));
        }
        if !self.owns_pending_delivery(&before) {
            return Ok(false);
        }
        let payload = super::runtime_events::issue_monitor_launch_failed_payload_with_retryability(
            self.issue()?,
            detail,
            self.feedback.issue_monitor_delivery_id.as_deref(),
            Some(&self.materializer_id),
            self.feedback
                .issue_monitor_session_mode
                .unwrap_or(gwt_agent::SessionMode::Normal),
            retryable,
        );
        let publication = publish(&self.root, payload);
        let after = self.read_monitor()?;
        if self.owns_pending_delivery(&after) {
            return Err(publication
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unspawned Monitor failure is unconfirmed".into()));
        }
        Ok(after
            .conflicting_launch_window_owner(self.issue()?, &self.id)
            .is_none())
    }

    pub(crate) fn expired_delivery(&self) -> Result<bool, String> {
        let Some(delivery) = self.feedback.issue_monitor_delivery_id.as_deref() else {
            return Ok(false);
        };
        let monitor = self.read_monitor()?;
        let issue = self.issue()?;
        Ok(monitor.prefs().enabled
            && self.confirmed_source(&monitor).is_none()
            && !monitor
                .prefs()
                .pending_launch_deliveries
                .iter()
                .any(|pending| pending.issue_number == issue && pending.delivery_id == delivery)
            && monitor
                .conflicting_launch_window_owner(issue, &self.id)
                .is_none())
    }

    pub(crate) fn commit_startup_failure(&self, detail: &str) -> Result<bool, String> {
        let before = self.read_monitor()?;
        let source_matches = self
            .source
            .as_ref()
            .is_some_and(|source| source.matches(&before, &self.id));
        if !source_matches && !self.owns_pending_delivery(&before) {
            return Ok(false);
        }
        let publication = self.failure(detail, None, true);
        let after = self.read_monitor()?;
        if self
            .source
            .as_ref()
            .is_some_and(|source| source.matches(&after, &self.id))
            || self.owns_pending_delivery(&after)
        {
            return Err(publication
                .err()
                .unwrap_or_else(|| "Monitor startup failure is unconfirmed".into()));
        }
        Ok(after
            .conflicting_launch_window_owner(self.issue()?, &self.id)
            .is_none())
    }

    fn read_monitor(&self) -> Result<crate::IssueMonitorState, String> {
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&self.root),
        )
        .map_err(|error| format!("Monitor owner readback failed: {error}"))?;
        Ok(crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs,
        ))
    }

    fn owns_pending_delivery(&self, monitor: &crate::IssueMonitorState) -> bool {
        monitor
            .prefs()
            .pending_launch_deliveries
            .iter()
            .any(|pending| {
                Some(pending.issue_number)
                    == self.feedback.issue_monitor_launch_binding_issue_number()
                    && Some(pending.delivery_id.as_str())
                        == self.feedback.issue_monitor_delivery_id.as_deref()
                    && pending.materializer_id.as_deref() == Some(self.materializer_id.as_str())
                    && pending.materializer_pid == Some(std::process::id())
                    && pending.materializer_window_id.as_deref() == Some(self.id.as_str())
            })
    }

    fn confirmed_source(
        &self,
        monitor: &crate::IssueMonitorState,
    ) -> Option<IssueMonitorAgentFailureSource> {
        let issue = self.feedback.issue_monitor_launch_binding_issue_number()?;
        let delivery = self.feedback.issue_monitor_delivery_id.as_deref()?;
        let prefs = monitor.prefs();
        let confirmation = prefs.launch_confirmations.get(&issue)?;
        let identity = monitor.launch_identity(issue);
        (confirmation.window_id == self.id
            && confirmation.delivery_id.as_deref() == Some(delivery)
            && confirmation.claim_id == identity.claim_id
            && identity.delivery_id.as_deref() == Some(delivery)
            && identity.window_id.as_deref() == Some(self.id.as_str())
            && identity.active)
            .then_some(IssueMonitorAgentFailureSource::Bound {
                issue_number: issue,
                identity,
            })
    }

    /// Session/tree cleanup has already committed. Settle only this owner's
    /// captured source; a lost reply is resolved by a fresh exact readback.
    pub fn closed(
        &self,
        terminal_failure: Option<&(String, Option<crate::IssueMonitorFailure>)>,
    ) -> Result<(), String> {
        let mut monitor = self.read_monitor()?;
        let mut source = self
            .source
            .clone()
            .or_else(|| self.confirmed_source(&monitor));
        if source.is_none() && self.owns_pending_delivery(&monitor) {
            // Before ACK there is no bound window generation. The actual
            // materializer/delivery tuple is the authority for LaunchFailed.
            let detail = terminal_failure
                .map(|(detail, _)| detail.as_str())
                .unwrap_or("Agent window closed before Monitor launch acknowledgement");
            let payload = json!({"launch_failed": {
                "issue_number": self.issue()?, "message": detail,
                "delivery_id": self.feedback.issue_monitor_delivery_id,
                "materializer_id": self.materializer_id,
            }});
            let publication = publish(&self.root, payload);
            monitor = self.read_monitor()?;
            if self.owns_pending_delivery(&monitor) {
                return Err(publication
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "Monitor pre-ACK close is unconfirmed".into()));
            }
            // An ACK already admitted before the close may have won the race.
            source = self.confirmed_source(&monitor);
        }
        let Some(source @ IssueMonitorAgentFailureSource::Bound { .. }) = source else {
            return Ok(());
        };
        if let Some((detail, failure)) = terminal_failure {
            if source.matches(&monitor, &self.id) {
                let publication = publish(
                    &self.root,
                    super::runtime_events::issue_monitor_agent_failed_from_launch_payload(
                        &self.id,
                        detail,
                        &source,
                        failure.as_ref(),
                    ),
                );
                monitor = self.read_monitor()?;
                if let Err(error) = publication {
                    if source.matches(&monitor, &self.id) {
                        return Err(error.to_string());
                    }
                }
            }
        }
        let classification = terminal_failure
            .and_then(|(_, failure)| match failure {
                Some(crate::IssueMonitorFailure::Termination { classification }) => {
                    Some(*classification)
                }
                _ => None,
            })
            .unwrap_or(crate::IssueMonitorFailureClass::Agent);
        let publication = publish(
            &self.root,
            json!({"window_closed_from_launch":{
                "window_id":self.id, "source":source, "classification":classification,
            }}),
        );
        let publication = match publication {
            Err(error) if error.allows_local_fallback() => {
                super::installation::try_commit_local_issue_monitor_control_with_timeout(
                    &self.root,
                    super::installation::local_issue_monitor_fallback_projection_timeout(),
                    |monitor| {
                        // The caller already reaped and finalized this exact runtime.
                        // A newer same-id launch must remain an inert CAS miss.
                        if !source.matches(monitor, &self.id) {
                            return Err(std::io::Error::other(
                                "Monitor close source changed before local settlement",
                            ));
                        }
                        let issue = self.issue().map_err(std::io::Error::other)?;
                        let target = crate::IssueMonitorStopTarget {
                            issue_number: issue,
                            claim_id: monitor.live_claim_id(issue),
                            delivery_id: monitor.pending_launch_delivery_id(issue),
                            window_id: Some(self.id.clone()),
                        };
                        monitor
                            .requeue_exact_window_classified(&target, classification)
                            .map(|_| ())
                            .ok_or_else(|| {
                                std::io::Error::other(
                                    "Monitor close no longer matches its exact launch",
                                )
                            })
                    },
                )
                .map(|_| ())
            }
            result => result,
        };
        let monitor = self.read_monitor()?;
        if source.matches(&monitor, &self.id) {
            return Err(publication
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "Monitor window close is unconfirmed".into()));
        }
        Ok(())
    }
}

const MONITOR_STARTUP_TIMEOUT_PREFIX: &str = "Issue Monitor startup timed out after ";
pub fn monitor_startup_failure_detail(issue: u64, screen: Option<&str>) -> String {
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

#[doc(hidden)]
pub fn prepare_issue_monitor_handoff_delivery(
    prefs_path: &Path,
    issue_number: u64,
    session: &gwt_agent::Session,
    target_window_id: &str,
    delivery_id: Option<&str>,
    materializer_id: &str,
    bind_existing_session: bool,
) -> Result<Option<crate::AutonomousHandoffDeliveryPreparation>, String> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let preparation =
        crate::prepare_autonomous_handoff_delivery_from_prefs(prefs_path, issue_number, &now)
            .map_err(|error| format!("failed to prepare the autonomous answer: {error}"))?;
    // A new Resume pane receives a new gwt Session; its launch worker binds that final identity.
    // Only an existing writable pane can bind the asking Session here.
    if let (true, Some(crate::AutonomousHandoffDeliveryPreparation::Ready(prepared))) =
        (bind_existing_session, preparation.as_ref())
    {
        let bound = super::installation::autonomous_handoff_delivery_target_for_session(
            session,
            issue_number,
            target_window_id,
            delivery_id,
            materializer_id,
        )
        .and_then(|target| {
            crate::bind_autonomous_handoff_delivery_target_from_prefs(
                prefs_path,
                &prepared.handoff_id,
                &prepared.session_id,
                prepared.attempt,
                &target,
            )
            .map_err(|error| format!("failed to bind the autonomous answer target: {error}"))
        });
        if !matches!(bound, Ok(true)) {
            let error = bound.err().unwrap_or_else(|| {
                "autonomous answer target no longer matches its durable attempt".to_string()
            });
            let _ = crate::record_autonomous_handoff_delivery_failure_from_prefs(
                prefs_path,
                &prepared.handoff_id,
                &prepared.session_id,
                prepared.attempt,
                &error,
                &now,
            );
            return Err(error);
        }
    }
    Ok(preparation)
}

pub(crate) struct OwnerAnswerDelivery {
    pub feedback: LaunchFeedbackContext,
    pub source: Option<IssueMonitorAgentFailureSource>,
    pub submitted: bool,
    pub result: super::events::OwnerReply,
}

/// The existing protected answer transaction, executed by the physical owner.
/// The caller supplies an immutable Issue/delivery intent; the attempt, target,
/// grant fence and exact Session lease are all captured here.
pub(crate) fn deliver_existing_answer(
    mut launch: MonitorLaunch,
    session: gwt_agent::Session,
    issuer: crate::agent_capability::AgentCapabilityIssuer,
    token: Option<String>,
    writers: super::agent_dispatch::PtyWriterRegistry,
    writer: Arc<super::agent_dispatch::PtyWriterEntry>,
    snapshot: crate::IssueMonitorWindowSnapshot,
) -> Result<OwnerAnswerDelivery, String> {
    launch.check_current()?;
    let issue = launch.issue()?;
    if session.linked_issue_number != Some(issue) {
        return Err("autonomous answer target Session is not linked to this Issue".into());
    }
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&launch.root);
    let pending = crate::pending_autonomous_handoff_resumption_from_prefs(&prefs_path, issue)
        .map_err(|error| error.to_string())?
        .ok_or("autonomous answer is no longer pending")?;
    let sessions = gwt_core::paths::gwt_sessions_dir();
    let asking = gwt_agent::Session::load(&sessions.join(format!("{}.toml", pending.session_id)))
        .map_err(|error| error.to_string())?;
    if asking.agent_id != session.agent_id
        || asking.exact_resume_session_id() != session.exact_resume_session_id()
        || session.exact_resume_session_id().is_none()
    {
        return Err("autonomous answer native conversation changed before owner delivery".into());
    }
    if let Some(delivery_id) = &launch.feedback.issue_monitor_delivery_id {
        let snapshot_tabs = std::collections::BTreeSet::from([snapshot.project_tab_id.clone()]);
        let claim = super::installation::IssueMonitorLaunchClaimInput {
            project_root: launch.root.clone(),
            issue_number: issue,
            delivery_id: delivery_id.clone(),
            materializer_window_id: launch.id.clone(),
            materializer_id: launch.materializer_id.clone(),
            materializer_pid: std::process::id(),
            fallback_timeout: super::installation::local_issue_monitor_fallback_projection_timeout(
            ),
            snapshot: Some(snapshot),
            snapshot_tabs,
        };
        if !claim.claim_or_reuse().map_err(|error| error.to_string())? {
            return Err("autonomous answer delivery belongs to another materializer".into());
        }
    }
    let preparation = prepare_issue_monitor_handoff_delivery(
        &prefs_path,
        issue,
        &session,
        &launch.id,
        launch.feedback.issue_monitor_delivery_id.as_deref(),
        &launch.materializer_id,
        true,
    )?;
    let attempt = match preparation {
        Some(crate::AutonomousHandoffDeliveryPreparation::Ready(attempt)) => attempt,
        Some(crate::AutonomousHandoffDeliveryPreparation::Backoff {
            retry_not_before, ..
        }) => {
            return Err(format!(
                "answered autonomous handoff delivery is in backoff until {retry_not_before}"
            ))
        }
        Some(crate::AutonomousHandoffDeliveryPreparation::Ambiguous { reason, .. }) => {
            return Err(reason)
        }
        _ => {
            return Ok(OwnerAnswerDelivery {
                feedback: launch.feedback,
                source: launch.source,
                submitted: false,
                result: Ok(
                    json!({"id":launch.id,"incarnation":launch.incarnation,"submitted":false,"pending":true}),
                ),
            })
        }
    };
    launch.feedback.issue_monitor_autonomous_handoff = Some(attempt.clone());
    launch.feedback.issue_monitor_autonomous_submit_started = true;
    let prompt = format!("{}\r", attempt.prompt);
    let write = || -> Result<(), String> {
        launch.check_current()?;
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)?;
        let commit = || -> Result<(), String> {
            launch.check_current()?;
            if let Some(binding) = &session.execution_binding {
                if !token
                    .as_deref()
                    .is_some_and(|token| issuer.active_token_is_current(token, binding))
                {
                    return Err(
                        "autonomous answer capability changed before physical submit".into(),
                    );
                }
            }
            super::agent_dispatch::write_current_pane_input(&writers, &launch.id, &writer, &prompt)
        };
        if let Some(identity) = identity {
            crate::cli::execution_state::with_current_active_session_execution_identity_lease(
                &sessions, &identity, commit,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "autonomous answer producing Session changed".to_string())?
        } else {
            gwt_agent::with_session_lease(&sessions, &session.id, |current| {
                if current.created_at != session.created_at
                    || current.worktree_path != session.worktree_path
                    || current.agent_id != session.agent_id
                    || current.execution_binding.is_some()
                    || current.exact_resume_session_id() != session.exact_resume_session_id()
                {
                    return Err(std::io::Error::other(
                        "autonomous answer Session changed before physical submit",
                    ));
                }
                commit().map_err(std::io::Error::other)
            })
            .map_err(|error| error.to_string())
        }
    };
    if let Err(error) = write() {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let marker = crate::mark_autonomous_handoff_delivery_ambiguous_from_prefs(
            &prefs_path,
            &attempt.handoff_id,
            &attempt.session_id,
            attempt.attempt,
            &error,
            &now,
        );
        let reason = match marker {
            Ok(true) => {
                format!("{error}; autonomous answer outcome is ambiguous and requires human review")
            }
            Ok(false) => format!("{error}; autonomous answer durable attempt changed"),
            Err(marker) => {
                format!("{error}; ambiguous durable state could not be recorded: {marker}")
            }
        };
        return Ok(OwnerAnswerDelivery {
            feedback: launch.feedback,
            source: launch.source,
            submitted: false,
            result: Err(reason),
        });
    }
    let committed = launch.commit_launch(true);
    let (source, result) = match committed {
        Ok(source) => (
            source,
            Ok(json!({"id":launch.id,"incarnation":launch.incarnation,
            "submitted":true,"handoff_id":attempt.handoff_id,"session_id":attempt.session_id,"attempt":attempt.attempt})),
        ),
        Err(error) => (
            launch.source,
            Err(format!(
                "autonomous answer submitted; Monitor ACK requires reconciliation: {error}"
            )),
        ),
    };
    Ok(OwnerAnswerDelivery {
        feedback: launch.feedback,
        source,
        submitted: true,
        result,
    })
}

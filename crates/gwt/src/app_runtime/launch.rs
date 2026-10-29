//! Session / launch lifecycle split out of `app_runtime/mod.rs` for
//! SPEC-3064 Phase 1 (Pass 2).
//!
//! Owns:
//! - Reexports of the shared launch payload types ([`ProcessLaunch`],
//!   [`AgentLaunchCompletion`], [`AgentLaunchResult`])
//! - Reexports of [`LaunchWizardMemoryCache`] and
//!   `launch_config_from_persisted_session` from the shared library
//! - SPEC-2809 launch stage presentation (`emit_agent_launch_stage`) and the
//!   SPEC-2359 in-flight launch identity (`inflight_launch_key`)
//! - Issue<->branch link persistence (`IssueBranchLinkStore`,
//!   `record_issue_branch_link_with_cache_dir`, ...) and the codex managed
//!   hook discovery / trust registration helpers
//! - The launch / spawn / close-work method surface
//!   ([`AppRuntime::start_window`],
//!   [`AppRuntime::spawn_agent_window`], [`AppRuntime::close_work`], ...)
//!
//! Behavior-preserving move: `WindowRuntime`, `LaunchWizardSession`, and
//! `AppRuntime::new` stay in `mod.rs` and are reached via `super`.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use gwt::launch_wizard::LaunchWizardMemoryCache;
#[cfg(test)]
pub use gwt::project_runtime::launch_payload::AgentLaunchCompletion;
#[cfg(test)]
pub use gwt::project_runtime::launch_payload::AgentLaunchRuntimeContext;
#[cfg(test)]
pub use gwt::project_runtime::launch_payload::{AgentLaunchResult, ProcessLaunch};
pub(super) use gwt::session_launch::launch_config_from_persisted_session;
#[cfg(test)]
pub(super) use gwt::session_launch::{
    apply_resume_identity_to_session, resume_handle_unavailable_reason,
};

use super::agent_maintenance;
use super::continuation::durable_launch_recovery_exists;
use super::{
    active_agent_session_matches_work, combined_window_id, normalize_branch_name,
    ActiveAgentSession, AgentKanbanLaunchTarget, AppRuntime, LaunchFeedbackContext,
    LiveSessionEntry, OutboundEvent, PendingContinueWork, UserEvent, WindowPreset,
    WindowProcessStatus, WorkspaceResumeContext,
};
#[cfg(test)]
use super::{AgentCapabilityIssuer, AppEventProxy};

#[cfg(test)]
pub(super) use gwt::project_runtime::continuation::claim_prepared_manual_successor_launch;

pub(crate) struct PendingLaunchCompletion {
    pub(crate) generation: u64,
    pub(crate) context: Option<super::ProjectContext>,
    pub(crate) current: Arc<AtomicBool>,
    pub(crate) operation_id: String,
    pub(crate) owner_pane_id: String,
    pub(crate) client_cell: super::RemoteLaunchClientCell,
    pub(crate) cancel_ack: super::RemoteLaunchCancelAck,
    pub(crate) manual_request: Option<super::RemoteOwnerLaunchRequest>,
    pub(crate) manual_wizard: Option<super::LaunchWizardSession>,
}

impl Drop for PendingLaunchCompletion {
    fn drop(&mut self) {
        self.current.store(false, Ordering::Release);
    }
}

#[cfg(test)]
pub(super) fn dispatch_agent_launch_success<F>(
    proxy: AppEventProxy,
    window_id: String,
    window_generation: Option<u64>,
    completion: AgentLaunchCompletion,
    spawn_project_index_bootstrap: F,
) where
    F: FnOnce(AppEventProxy, PathBuf),
{
    let project_index_root = completion.4.clone();
    proxy.send(UserEvent::LaunchComplete {
        window_id,
        window_generation,
        result: Box::new(Ok(completion)),
    });
    spawn_project_index_bootstrap(proxy, project_index_root);
}

fn initial_agent_window_status(_config: &gwt_agent::LaunchConfig) -> WindowProcessStatus {
    #[cfg(windows)]
    if _config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host
        && matches!(
            _config.agent_id,
            gwt_agent::AgentId::Codex | gwt_agent::AgentId::ClaudeCode
        )
    {
        return WindowProcessStatus::Starting;
    }
    WindowProcessStatus::Running
}

#[cfg(test)]
use gwt::project_runtime::installation::issue_monitor_prefs_fence_digest;
/// Disk and process inputs captured before the GUI accepts a Monitor launch.
pub(super) use gwt::project_runtime::installation::{
    prepare_host_update_drain_launch_hold, IssueMonitorSpawnAdmissionInput,
    PreparedIssueMonitorSpawnAdmission, ISSUE_MONITOR_SPAWN_ADMISSION_STALE,
};

#[derive(Debug, Clone, Default)]
struct AgentWindowSpawnOptions {
    selected_agent_member_id: Option<String>,
    inherited_agent_name: Option<String>,
    workspace_resume_context: Option<WorkspaceResumeContext>,
    launch_feedback_context: Option<LaunchFeedbackContext>,
    agent_kanban_target: Option<AgentKanbanLaunchTarget>,
    continuation: Option<PendingContinueWork>,
    prepared_monitor_admission: Option<PreparedIssueMonitorSpawnAdmission>,
    monitor_requested_tier: Option<u8>,
    deliver_pending_autonomous_answer: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchPermissionChecked {
    context: super::ProjectContext,
    operation_id: String,
    inflight_key: String,
    config: gwt_agent::LaunchConfig,
    options: AgentWindowSpawnOptions,
    result: Result<
        (
            gwt::daemon_subscriber::ProjectOwnerIdentity,
            gwt::project_runtime::events::OwnerLaunchPermissionCheck,
        ),
        String,
    >,
}

#[cfg(test)]
pub(super) use gwt::project_runtime::launch::{
    issue_monitor_trust_candidate_from_feedback, IssueMonitorTrustCandidate,
};

pub(super) use gwt::project_runtime::installation::IssueBranchLinkStore;

/// Allocate fixture stage IDs for the console presentation contracts.
/// Production launch stages use the physical owner's incarnation ID.
#[cfg(test)]
pub(crate) fn next_agent_launch_stage_id(counter: &std::sync::atomic::AtomicU64) -> u64 {
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Emit a `gwt.process.summary` event for one Launch Wizard stage so the
/// Console window's `agent` tab surfaces the pipeline that ends in the
/// PTY spawn. Stage semantics (`start`, `done`, `error`) follow the same
/// vocabulary as the `spawn_logged` summary contract.
#[cfg(test)]
pub(super) use gwt::project_runtime::installation::emit_agent_launch_stage;

#[cfg(test)]
pub(super) use gwt::project_runtime::installation::launch_argv_summary;

#[cfg(test)]
pub use gwt::project_runtime::launch_readiness::{
    continue_work_readiness_decision, ContinueWorkReadinessWatch, ReadinessDeadlineDecision,
    ReadinessPaneEvidence,
};

/// Identity of a launch for in-flight dedup. Includes the agent and the
/// resume conversation so parallel restores of *different* Sessions on the
/// same Work (startup auto-resume) and multi-agent launches on one Work stay
/// allowed — only a re-request of the *same* launch dedupes. `None` when the
/// config carries neither a branch nor a working dir: such launches have no
/// stable Work identity and must never dedup against each other.
pub(super) use gwt::project_runtime::installation::inflight_launch_key;

#[cfg(test)]
pub(super) use gwt::project_runtime::launch::{
    codex_hook_discovery_mode_for_launch_config,
    codex_hook_discovery_mode_from_codex_version_output,
    codex_hook_discovery_mode_from_detected_codex_version, effective_host_codex_config_path,
    maybe_register_codex_managed_hook_trust_for_launch,
    register_codex_managed_project_trust_for_resolved_launch_with_host_context,
    validate_issue_monitor_managed_codex_worktree, HostEnvKeySemantics,
};

impl AppRuntime {
    pub(super) fn latest_resumable_branch_session(
        &self,
        project_root: &Path,
        branch_name: &str,
    ) -> Option<gwt_agent::Session> {
        // Resolve from the in-memory cache so the Resume click never blocks the
        // main UI thread on disk I/O. Freshness is guaranteed by
        // [`apply_refreshed_launch_wizard_sessions`], which the off-thread
        // branch load dispatches before any Resume button is enabled (#2995).
        let normalized_branch_name = normalize_branch_name(branch_name);
        self.launch_wizard_cache
            .latest_resumable_branch_session(project_root, &normalized_branch_name)
    }

    /// Apply a freshly disk-loaded session set to the Launch Wizard cache.
    /// Dispatched from the off-thread branch load (#2995) so branch Resume
    /// availability and the subsequent cache-based resume resolution reflect
    /// session TOMLs the hook CLI wrote out-of-process after launch — without
    /// the main thread ever performing the session-directory scan.
    pub(crate) fn apply_refreshed_launch_wizard_sessions(
        &mut self,
        mut sessions: Vec<gwt_agent::Session>,
    ) {
        sessions.retain(|session| !durable_launch_recovery_exists(&self.sessions_dir, &session.id));
        self.launch_wizard_cache.replace_sessions(sessions);
    }

    /// SPEC-2359 US-83 / FR-444: update the live wizard's "open existing branch"
    /// picker candidates (computed off the UI thread after `fetch_origin`) and
    /// re-emit its state so the picker renders. Scoped to the matching
    /// `wizard_id` so a stale background result can't clobber a newer wizard.
    pub(crate) fn apply_launch_wizard_branch_candidates(
        &mut self,
        wizard_id: String,
        candidates: Vec<String>,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_wizard(&wizard_id) else {
            return Vec::new();
        };
        let Some(session) = self.launch_wizard_for_mut(&context) else {
            return Vec::new();
        };
        session.wizard.open_branch_candidates = candidates;
        vec![self.launch_wizard_state_outbound(&context)]
    }

    pub(crate) fn live_sessions_for_branch(
        &self,
        tab_id: &str,
        branch_name: &str,
    ) -> Vec<LiveSessionEntry> {
        let mut entries = self
            .active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id && session.branch_name == branch_name)
            .map(|session| LiveSessionEntry {
                session_id: session.session_id.clone(),
                window_id: session.window_id.clone(),
                agent_id: session.agent_id.clone(),
                kind: "agent".to_string(),
                name: session.display_name.clone(),
                detail: Some(session.worktree_path.display().to_string()),
                active: true,
                runtime_status: self
                    .window_status(&session.window_id)
                    .unwrap_or(WindowProcessStatus::Running),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            match (
                self.launch_wizard_cache.session_by_id(&left.session_id),
                self.launch_wizard_cache.session_by_id(&right.session_id),
            ) {
                (Some(left_session), Some(right_session)) => right_session
                    .last_activity_at
                    .cmp(&left_session.last_activity_at)
                    .then_with(|| right_session.updated_at.cmp(&left_session.updated_at))
                    .then_with(|| right_session.created_at.cmp(&left_session.created_at))
                    .then_with(|| right_session.id.cmp(&left_session.id)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => left.name.cmp(&right.name),
            }
        });
        entries
    }

    pub(crate) fn active_session_branches_for_tab(
        &self,
        tab_id: &str,
    ) -> std::collections::HashSet<String> {
        self.active_agent_sessions
            .values()
            .filter(|session| session.tab_id == tab_id)
            .map(|session| session.branch_name.clone())
            .collect()
    }

    /// A stale GUI completion may cancel only its captured owner booking.
    pub(crate) fn cleanup_stale_project_completion(&self, event: &UserEvent) {
        match event {
            UserEvent::ProjectCompletion { event, .. } => {
                self.cleanup_stale_project_completion(event)
            }
            UserEvent::RemoteOwnerLaunchComplete(completion) => {
                let completion = completion.clone();
                let _ = self.blocking_tasks.try_spawn(move || {
                    if let Ok(executor) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        let _ = executor.block_on(super::cancel_remote_owner_launch(
                            &completion.owner_pane_id,
                            &completion.operation_id,
                            &completion.client_cell,
                            &completion.cancel_ack,
                        ));
                    }
                });
            }
            _ => {}
        }
    }

    pub(super) fn has_pending_agent_launch_worker(&mut self, window_id: &str) -> bool {
        if self
            .pending_launch_completions
            .get(window_id)
            .is_some_and(|pending| pending.current.load(Ordering::Acquire))
        {
            return true;
        }
        let Some(worker) = self.initial_launch_workers.get_mut(window_id) else {
            return false;
        };
        let Some(handle) = worker.as_ref() else {
            return true; // A normally returned worker has queued its completion.
        };
        if !handle.is_finished() {
            return true;
        }
        let returned = worker
            .take()
            .expect("finished launch worker")
            .join()
            .is_ok();
        if !returned {
            self.initial_launch_workers.remove(window_id);
        }
        returned
    }

    pub(crate) fn start_window(
        &mut self,
        tab_id: &str,
        raw_id: &str,
        preset: WindowPreset,
    ) -> Vec<OutboundEvent> {
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        if tab.workspace.window(raw_id).is_none() {
            return self.status_events(
                combined_window_id(tab_id, raw_id),
                WindowProcessStatus::Error,
                Some("Window not found".into()),
            );
        }
        self.register_window(tab_id, raw_id);
        let window_id = combined_window_id(tab_id, raw_id);
        if preset.is_agent_terminal() && self.agent_maintenance.busy {
            let detail = agent_maintenance::AGENT_MAINTENANCE_BUSY.to_string();
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
            self.window_details
                .insert(window_id.clone(), detail.clone());
            return self.status_events(window_id, WindowProcessStatus::Error, Some(detail));
        }
        if preset.is_agent_terminal() {
            if let Some(detail) = self.host_update_drain_launch_hold().unwrap_or_else(Some) {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details
                    .insert(window_id.clone(), detail.clone());
                return self.status_events(window_id, WindowProcessStatus::Error, Some(detail));
            }
        }
        if !preset.requires_process() {
            self.set_window_status(tab_id, raw_id, WindowProcessStatus::Running);
            return self.status_events(window_id, WindowProcessStatus::Running, None);
        }
        let Some(context) = self.project_context(tab_id) else {
            return Vec::new();
        };
        let generation = self
            .window_lifecycle_generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&window_id)
            .copied();
        let result = match preset {
            WindowPreset::Claude | WindowPreset::Codex => {
                let agent = if preset == WindowPreset::Claude {
                    gwt_agent::AgentId::ClaudeCode
                } else {
                    gwt_agent::AgentId::Codex
                };
                let config = gwt_agent::AgentLaunchBuilder::new(agent)
                    .working_dir(&context.project_root)
                    .build();
                self.queue_project_owner_launch(
                    &context,
                    window_id.clone(),
                    generation,
                    Box::new(config),
                    Box::default(),
                )
            }
            WindowPreset::Shell => {
                let config = gwt::launch_wizard::ShellLaunchConfig {
                    working_dir: Some(context.project_root.clone()),
                    branch: None,
                    base_branch: None,
                    display_name: preset.title().to_string(),
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                    docker_service: None,
                    docker_lifecycle_intent: Default::default(),
                    windows_shell: None,
                    env_vars: Default::default(),
                    remove_env: Vec::new(),
                    command_override: None,
                    command_args_override: None,
                };
                self.queue_project_owner_shell(
                    &context,
                    window_id.clone(),
                    generation,
                    Box::new(config),
                )
            }
            _ => Err(format!("{} cannot be launched directly", preset.title())),
        };
        match result {
            Ok(mut events) => {
                events.extend(self.status_events(
                    window_id,
                    WindowProcessStatus::Starting,
                    Some("Launching...".into()),
                ));
                events
            }
            Err(error) => {
                self.set_window_status(tab_id, raw_id, WindowProcessStatus::Error);
                self.window_details.insert(window_id.clone(), error.clone());
                self.status_events(window_id, WindowProcessStatus::Error, Some(error))
            }
        }
    }

    pub(crate) fn spawn_agent_window(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                selected_agent_member_id: None,
                inherited_agent_name: None,
                workspace_resume_context,
                launch_feedback_context: None,
                agent_kanban_target: None,
                continuation: None,
                prepared_monitor_admission: None,
                ..Default::default()
            },
        )
    }

    pub(crate) fn spawn_agent_window_with_feedback(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: LaunchFeedbackContext,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                selected_agent_member_id: None,
                inherited_agent_name: None,
                workspace_resume_context,
                launch_feedback_context: Some(launch_feedback_context),
                agent_kanban_target: None,
                continuation: None,
                prepared_monitor_admission: None,
                ..Default::default()
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn_agent_window_with_monitor_intent(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        feedback: LaunchFeedbackContext,
        prepared: Option<PreparedIssueMonitorSpawnAdmission>,
        requested_tier: Option<u8>,
        deliver_pending_answer: bool,
        selected_agent_member_id: Option<String>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                selected_agent_member_id,
                workspace_resume_context,
                launch_feedback_context: Some(feedback),
                prepared_monitor_admission: prepared,
                monitor_requested_tier: requested_tier,
                deliver_pending_autonomous_answer: deliver_pending_answer,
                ..Default::default()
            },
        )
    }

    pub(crate) fn spawn_agent_window_in_agent_kanban(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        launch_feedback_context: Option<LaunchFeedbackContext>,
        target: AgentKanbanLaunchTarget,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                selected_agent_member_id: None,
                inherited_agent_name: None,
                workspace_resume_context,
                launch_feedback_context,
                agent_kanban_target: Some(target),
                continuation: None,
                prepared_monitor_admission: None,
                ..Default::default()
            },
        )
    }

    pub(crate) fn spawn_agent_window_with_identity(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        workspace_resume_context: Option<WorkspaceResumeContext>,
        inherited_agent_name: Option<String>,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options(
            tab_id,
            config,
            AgentWindowSpawnOptions {
                selected_agent_member_id: None,
                inherited_agent_name,
                workspace_resume_context,
                launch_feedback_context: None,
                agent_kanban_target: None,
                continuation: None,
                prepared_monitor_admission: None,
                ..Default::default()
            },
        )
    }

    pub(crate) fn live_agent_window_for_work(
        &self,
        tab_id: &str,
        branch: Option<&str>,
        worktree_path: Option<&Path>,
    ) -> Option<String> {
        let normalized_branch = branch
            .map(normalize_branch_name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.active_agent_sessions
            .iter()
            .find(|(window_id, session)| {
                session.tab_id == tab_id
                    && self.window_lookup.contains_key(window_id.as_str())
                    && self
                        .window_status(window_id.as_str())
                        .is_some_and(|status| {
                            !matches!(
                                status,
                                WindowProcessStatus::Stopped | WindowProcessStatus::Error
                            )
                        })
                    && active_agent_session_matches_work(
                        session,
                        normalized_branch.as_deref(),
                        worktree_path,
                    )
            })
            .map(|(window_id, _)| window_id.clone())
    }

    pub(crate) fn focus_existing_live_work_agent_events(
        &mut self,
        window_id: &str,
    ) -> Vec<OutboundEvent> {
        let events = self.focus_window_events(window_id);
        if events.is_empty() {
            self.window_lookup
                .get(window_id)
                .and_then(|address| self.project_context(&address.tab_id))
                .map(|context| self.workspace_state_broadcast(&context))
                .into_iter()
                .collect()
        } else {
            events
        }
    }

    pub(super) fn host_update_drain_launch_hold(&self) -> Result<Option<String>, String> {
        let project_roots = self
            .project_contexts()
            .into_iter()
            .map(|context| context.project_root)
            .collect::<Vec<_>>();
        prepare_host_update_drain_launch_hold(&project_roots)
    }

    pub(super) fn capture_issue_monitor_spawn_admission(
        &self,
        project_root: &Path,
        issue_number: u64,
        target_branch: &str,
    ) -> IssueMonitorSpawnAdmissionInput {
        IssueMonitorSpawnAdmissionInput {
            project_root: project_root.to_path_buf(),
            host_project_roots: self
                .project_contexts()
                .into_iter()
                .map(|context| context.project_root)
                .collect(),
            issue_number,
            target_branch: target_branch.to_string(),
            issue_link_cache_dir: self.issue_link_cache_dir.clone(),
        }
    }

    fn spawn_agent_window_with_options(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        options: AgentWindowSpawnOptions,
    ) -> Result<Vec<OutboundEvent>, String> {
        self.spawn_agent_window_with_options_inner(tab_id, config, options, false)
    }

    fn spawn_agent_window_with_options_inner(
        &mut self,
        tab_id: &str,
        config: gwt_agent::LaunchConfig,
        options: AgentWindowSpawnOptions,
        permission_checked: bool,
    ) -> Result<Vec<OutboundEvent>, String> {
        use gwt::project_runtime::launch_payload::{ManualSuccessorChoice, OwnerLaunchMetadata};
        if self.agent_maintenance.busy {
            return Err(agent_maintenance::AGENT_MAINTENANCE_BUSY.to_string());
        }
        let context = self
            .project_context(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let mut permission_options = None;
        if !permission_checked
            && !config.is_ephemeral
            && !config.suppress_execution_control
            && config.session_mode == gwt_agent::SessionMode::Normal
            && options.continuation.is_none()
        {
            if let Some(number) = config.linked_issue_number {
                // Pure display-side screening. Only the fixed owner may
                // determine Inspection or persist a permission refusal.
                let decision = if config.skip_permissions {
                    // Derive provider/argv facts instead of accepting a stored
                    // verdict. Custom mappings require the native config read.
                    gwt_agent::validate_materialized_launch(
                        gwt_agent::decide_permission_mode(&gwt_agent::PermissionModeInputs {
                            agent_id: &config.agent_id,
                            custom_agent: None,
                            source: config.permission_decision.source,
                            entrypoint: &gwt_agent::permission_mode::entrypoint_from_launch_args(
                                config.entrypoint_args(),
                                false,
                            ),
                            launch_route: config.launch_route,
                            requested_skip_permissions: Some(true),
                            producing_work: true,
                        }),
                        &config.args,
                        &config.env_vars,
                    )
                } else {
                    config.permission_decision.clone()
                };
                if gwt::cli::permission_readiness::pre_launch_block(
                    "issue",
                    number,
                    "pending-permission-audit",
                    &decision,
                )
                .is_some()
                {
                    permission_options = Some(options.clone());
                }
            }
        }
        let AgentWindowSpawnOptions {
            selected_agent_member_id,
            inherited_agent_name,
            workspace_resume_context,
            launch_feedback_context,
            agent_kanban_target,
            continuation,
            prepared_monitor_admission,
            monitor_requested_tier,
            deliver_pending_autonomous_answer,
        } = options;
        if continuation.is_some() {
            return Err("Continue Work must be coordinated by the project execution owner".into());
        }
        let is_review = config
            .env_vars
            .get("GWT_REVIEW_DISPATCH")
            .is_some_and(|value| value == "1");
        let issue_number = launch_feedback_context
            .as_ref()
            .and_then(|feedback| feedback.issue_monitor_issue_number);
        let resume_context = if is_review {
            None
        } else {
            workspace_resume_context
        };
        let metadata = OwnerLaunchMetadata {
            selected_agent_member_id,
            inherited_agent_name: inherited_agent_name.clone(),
            workspace_resume_context: resume_context.clone(),
            launch_feedback_context,
            manual_successor: ManualSuccessorChoice::from_intent(&config.execution_intent)?,
            monitor_requested_tier,
            deliver_pending_autonomous_answer,
        };
        // Focus and layout are GUI projections. Admission, names, claims,
        // Session creation and the physical launch belong to the owner.
        if issue_number.is_none() && metadata.manual_successor.is_none() {
            if let Some(window_id) = self.live_agent_window_for_work(
                tab_id,
                config.branch.as_deref(),
                config.working_dir.as_deref(),
            ) {
                return Ok(self.focus_existing_live_work_agent_events(&window_id));
            }
        }
        let inflight_key = inflight_launch_key(tab_id, &config);
        if issue_number.is_none() && metadata.manual_successor.is_none() {
            if let Some((window_id, _)) = inflight_key
                .as_deref()
                .and_then(|key| self.inflight_launches.get(key))
            {
                return Ok(self.focus_existing_live_work_agent_events(&window_id.clone()));
            }
        }
        if let Some(options) = permission_options {
            return self.queue_launch_permission_check(context, config, options);
        }
        let purpose = if is_review {
            config
                .linked_issue_number
                .map(|number| format!("Review · Issue #{number}"))
        } else {
            resume_context
                .as_ref()
                .and_then(WorkspaceResumeContext::purpose_title)
                .or_else(|| {
                    prepared_monitor_admission
                        .as_ref()
                        .and_then(|prepared| prepared.purpose_title.clone())
                })
        };
        let tab = self
            .tab_mut(tab_id)
            .ok_or_else(|| "Project tab not found".to_string())?;
        let window = tab.workspace.add_window_with_title(
            WindowPreset::Agent,
            config.display_name.clone(),
            true,
        );
        if let Some(name) = inherited_agent_name {
            tab.workspace.set_agent_name(&window.id, Some(name));
        }
        if let Some(purpose) = purpose {
            let _ = tab.workspace.set_purpose_title(&window.id, Some(purpose));
        }
        let _ = tab
            .workspace
            .set_agent_id(&window.id, config.agent_id.command().to_string());
        let _ = tab
            .workspace
            .set_linked_issue_number(&window.id, config.linked_issue_number);
        if let Some(target) = agent_kanban_target {
            let _ = tab.workspace.place_agent_window_in_kanban(
                &window.id,
                &target.board_id,
                target.lane_id,
                None,
            );
        } else if let Some(issue_number) = issue_number {
            if let Some(host) = tab.workspace.issue_preview_host_window_id() {
                let _ = tab.workspace.place_agent_window_in_issue_preview(
                    &window.id,
                    &host,
                    issue_number,
                );
            }
        }
        let raw_id = window.id.clone();
        self.register_window(tab_id, &raw_id);
        let window_id = combined_window_id(tab_id, &raw_id);
        if is_review {
            self.issue_monitor_review_dispatch_windows
                .insert(window_id.clone());
        } else {
            self.issue_monitor_review_dispatch_windows
                .remove(&window_id);
        }
        let generation = self
            .window_lifecycle_generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&window_id)
            .copied();
        self.window_pty_statuses
            .insert(window_id.clone(), initial_agent_window_status(&config));
        self.window_hook_states.remove(&window_id);
        if let Some(feedback) = metadata.launch_feedback_context.as_ref() {
            self.pending_launch_feedback_contexts
                .insert(window_id.clone(), feedback.clone());
            if let Some(delivery_id) = feedback.issue_monitor_delivery_id.as_ref() {
                // This cache tracks the local placeholder. The owner alone
                // claims the canonical delivery and owns its physical launch.
                self.issue_monitor_launch_deliveries.insert(
                    delivery_id.clone(),
                    super::IssueMonitorLaunchDeliveryState::Materializing {
                        window_id: window_id.clone(),
                        started_at: std::time::Instant::now(),
                    },
                );
            }
        }
        if let Some(key) = inflight_key {
            self.inflight_launches
                .insert(key, (window_id.clone(), std::time::Instant::now()));
        }
        let mut events = self.queue_project_owner_launch(
            &context,
            window_id.clone(),
            generation,
            Box::new(config),
            Box::new(metadata),
        )?;
        events.push(self.workspace_state_broadcast(&context));
        events.extend(self.status_events(
            window_id,
            WindowProcessStatus::Starting,
            Some("Launching...".into()),
        ));
        Ok(events)
    }

    fn queue_launch_permission_check(
        &mut self,
        context: super::ProjectContext,
        config: gwt_agent::LaunchConfig,
        options: AgentWindowSpawnOptions,
    ) -> Result<Vec<OutboundEvent>, String> {
        let work_key = inflight_launch_key(&context.tab_id, &config)
            .ok_or("Choose a worktree before checking producing-launch permissions")?;
        let key = format!(
            "permission:{}:{}:{work_key}",
            context.project_key, context.generation
        );
        if self.inflight_launches.contains_key(&key) {
            return Ok(Vec::new());
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        // An attempt marker only; no GUI row, pane or Session is allocated.
        self.inflight_launches.insert(
            key.clone(),
            (
                format!("permission-check:{operation_id}"),
                std::time::Instant::now(),
            ),
        );
        // This completion must release its own marker even when the project
        // closed. Its handler performs the context fence before any GUI effect.
        let proxy = self.proxy.clone();
        let supervisor = self.daemon_supervisor.clone();
        let inflight_key = key.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(async {
                        let client =
                            gwt::project_runtime::client::ProjectRuntimeClient::connect_for_gui(
                                context.project_root.clone(),
                                supervisor,
                            )
                            .await?;
                        let identity = client
                            .identity()
                            .cloned()
                            .ok_or("Permission check owner is unconfirmed")?;
                        let reply = client
                            .command(
                                gwt::project_runtime::events::OwnerCommand::CheckLaunchPermission {
                                    operation_id: operation_id.clone(),
                                    config: Box::new(config.clone()),
                                },
                            )
                            .await?;
                        let checked =
                            serde_json::from_value(reply).map_err(|error| error.to_string())?;
                        Ok((identity, checked))
                    })
                });
            proxy.send(UserEvent::LaunchPermissionChecked(Box::new(
                LaunchPermissionChecked {
                    context,
                    operation_id,
                    inflight_key,
                    config,
                    options,
                    result,
                },
            )));
        }) {
            self.inflight_launches.remove(&key);
            return Err(format!("Permission check could not start: {error}"));
        }
        Ok(Vec::new())
    }

    pub(crate) fn handle_launch_permission_checked(
        &mut self,
        completion: LaunchPermissionChecked,
    ) -> Vec<OutboundEvent> {
        let LaunchPermissionChecked {
            context,
            operation_id,
            inflight_key,
            config,
            options,
            result,
        } = completion;
        let marker = format!("permission-check:{operation_id}");
        if !self
            .inflight_launches
            .get(&inflight_key)
            .is_some_and(|(pending, _)| pending == &marker)
        {
            return Vec::new();
        }
        self.inflight_launches.remove(&inflight_key);
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        let result = result
            .and_then(|(identity, checked)| {
                if checked.operation_id != operation_id
                    || checked.project_key != context.project_key.as_str()
                    || checked.host_instance_id != identity.host_instance_id
                    || checked.config_hash
                        != gwt::project_runtime::events::OwnerLaunchPermissionCheck::config_hash(
                            &config,
                        )?
                    || checked.blocked != checked.record.is_some()
                    || self
                        .project_state(&context)
                        .and_then(|state| state.project_owner_identity.as_ref())
                        .is_some_and(|current| current != &identity)
                {
                    return Err("Permission check identity changed; retry the launch".into());
                }
                if let Some(record) = checked.record {
                    if !record.integrity_ok()
                        || Some(record.owner_number) != config.linked_issue_number
                        || record.session_id != format!("permission-check:{operation_id}")
                    {
                        return Err("Permission refusal identity changed; retry the launch".into());
                    }
                    return Err(format!("launch refused: {}", record.describe()));
                }
                Ok(())
            })
            .and_then(|()| {
                self.spawn_agent_window_with_options_inner(&context.tab_id, config, options, true)
            });
        result.unwrap_or_else(|message| {
            vec![OutboundEvent::project(
                context.project_key,
                gwt::BackendEvent::IssueMonitorToast {
                    level: "error".into(),
                    message,
                    issue_number: None,
                    notification_transition: None,
                    notification_transition_id: None,
                },
            )]
        })
    }

    #[cfg(test)]
    pub(crate) fn spawn_agent_window_async(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        config: gwt_agent::LaunchConfig,
        profile_config_path: PathBuf,
        agent_capability_issuer: Option<AgentCapabilityIssuer>,
    ) {
        let prepared_manual_launch_claim = match claim_prepared_manual_successor_launch(
            &sessions_dir,
            Path::new(&project_root),
            &config,
        ) {
            Ok(claim) => claim,
            Err(error) => {
                proxy.send(UserEvent::LaunchComplete {
                    window_id,
                    window_generation: None,
                    result: Box::new(Err(error.into())),
                });
                return;
            }
        };
        Self::spawn_agent_window_async_with_claim(
            proxy,
            sessions_dir,
            project_root,
            window_id,
            None,
            config,
            profile_config_path,
            agent_capability_issuer,
            prepared_manual_launch_claim,
            None,
            false,
        );
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn spawn_agent_window_async_with_claim(
        proxy: AppEventProxy,
        sessions_dir: PathBuf,
        project_root: String,
        window_id: String,
        window_generation: Option<u64>,
        config: gwt_agent::LaunchConfig,
        profile_config_path: PathBuf,
        agent_capability_issuer: Option<AgentCapabilityIssuer>,
        prepared_manual_launch_claim: Option<gwt_agent::SessionActiveLaunchHandshake>,
        issue_monitor_trust_candidate: Option<IssueMonitorTrustCandidate>,
        issue_monitor_launch: bool,
    ) {
        use gwt::project_runtime::launch::{AgentLaunchEvent, AgentLaunchInput};
        let event_proxy = proxy.clone();
        let event_window_id = window_id.clone();
        let project_key = gwt_core::paths::resolve_project_scope(Path::new(&project_root)).hash;
        let result = gwt::project_runtime::launch::prepare_agent_launch(
            AgentLaunchInput {
                sessions_dir,
                project_root,
                window_id: window_id.clone(),
                config,
                profile_config_path,
                agent_capability_issuer,
                prepared_manual_launch_claim,
                issue_monitor_trust_candidate,
                issue_monitor_launch,
            },
            move |event| match event {
                AgentLaunchEvent::Progress { message } => {
                    event_proxy.send(UserEvent::LaunchProgress {
                        window_id: event_window_id.clone(),
                        message,
                    })
                }
                AgentLaunchEvent::TerminalOutput { data } => {
                    event_proxy.send(UserEvent::LaunchTerminalOutput {
                        window_id: event_window_id.clone(),
                        data,
                    })
                }
                AgentLaunchEvent::Toast {
                    level,
                    message,
                    issue_number,
                } => event_proxy.send(UserEvent::Dispatch(vec![OutboundEvent::project(
                    project_key.clone(),
                    gwt::protocol::BackendEvent::IssueMonitorToast {
                        level,
                        message,
                        issue_number,
                        notification_transition: None,
                        notification_transition_id: None,
                    },
                )])),
            },
        );
        match result {
            Ok(completion) => dispatch_agent_launch_success(
                proxy,
                window_id,
                window_generation,
                completion,
                |proxy, project_index_root| {
                    crate::project_index_bootstrap::request_project_index_refresh(
                        &proxy,
                        project_index_root,
                    );
                },
            ),
            Err(error) => proxy.send(UserEvent::LaunchComplete {
                window_id,
                window_generation,
                result: Box::new(Err(error)),
            }),
        }
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): handle a user-initiated Work close
    /// from the Work surface. `close_kind` is `"done"` or `"discarded"`.
    ///
    /// Behavior:
    /// - If the owning agent session (derived from `work_id`) is still live, the
    ///   close is blocked and the worktree is left untouched (FR-352). The
    ///   owning agent must be stopped first.
    /// - Otherwise (a Paused Work with no running agent), only the terminal close
    ///   is recorded in Work history. Worktree, branch, and PR are unchanged;
    ///   worktree deletion remains an independent vetted cleanup operation. A
    ///   `done` close records a Done event and `discarded` records a Discard
    ///   event. Re-closing an already-closed Work is a noop.
    pub(crate) fn close_work(
        &mut self,
        context: &super::ProjectContext,
        work_id: &str,
        close_kind: &str,
    ) -> Vec<OutboundEvent> {
        let work_id = work_id.trim().to_string();
        let close_kind = close_kind.trim().to_ascii_lowercase();
        if work_id.is_empty()
            || !matches!(close_kind.as_str(), "done" | "discarded")
            || !self.project_context_is_current(context)
        {
            return Vec::new();
        }
        let context = context.clone();
        let supervisor = Arc::clone(&self.daemon_supervisor);
        let proxy = self.proxy.clone().for_project(context.clone());
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|executor| {
                    executor.block_on(async {
                        let client =
                            gwt::project_runtime::client::ProjectRuntimeClient::connect_for_gui(
                                context.project_root.clone(),
                                supervisor,
                            )
                            .await?;
                        client
                            .command(gwt::project_runtime::events::OwnerCommand::CloseWork {
                                work_id,
                                close_kind,
                            })
                            .await
                    })
                });
            match result {
                Ok(_) => proxy.send(UserEvent::ActiveWorkProjectionChanged {
                    project_root: context.project_root,
                }),
                Err(error) => tracing::warn!(%error, "project owner declined native Work close"),
            }
        }) {
            tracing::warn!(%error, "could not enqueue native Work close");
        }
        Vec::new()
    }

    /// SPEC-3214 (codex #3235 review): whether a stopped session uses an
    /// ephemeral worktree. The `.intake-*` basename alone is not enough —
    /// a normal branch worktree a user happens to name `.intake-*` must keep its
    /// Paused-Work / resume behavior. The definitive signal is that the
    /// ephemeral worktree is DETACHED (branchless), which only
    /// `create_detached` produces.
    /// A worktree that is already gone is treated as ephemeral (it was reaped).
    pub(super) fn session_uses_ephemeral_worktree(&self, session: &ActiveAgentSession) -> bool {
        let project_root = self
            .tab(&session.tab_id)
            .map(|tab| tab.project_root.as_path());
        Self::session_uses_ephemeral_worktree_for_project(project_root, session)
    }

    pub(super) fn session_uses_ephemeral_worktree_for_project(
        project_root: Option<&Path>,
        session: &ActiveAgentSession,
    ) -> bool {
        gwt::project_runtime::workspace::session_uses_ephemeral_worktree_for_project(
            project_root,
            session,
        )
    }
}

#[cfg(test)]
#[path = "agent_launch_stage_tests.rs"]
mod agent_launch_stage_tests;

#[cfg(test)]
mod lazy_session_ledger_tests {
    use super::LaunchWizardMemoryCache;

    #[test]
    fn launch_complete_worker_resolves_session_ledger_before_gui_apply() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(dir.path());
        crate::app_runtime::tests::init_repo_with_initial_commit(dir.path());
        let (mut runtime, recorded, tasks, window_id, result) =
            crate::app_runtime::tests::queued_agent_completion_fixture(dir.path());
        runtime.sessions_dir = gwt_core::paths::gwt_sessions_dir();
        std::fs::create_dir_all(&runtime.sessions_dir).unwrap();
        runtime.launch_wizard_cache =
            LaunchWizardMemoryCache::load_with_agent_options(&runtime.sessions_dir, Vec::new());
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture executor");
        let owner = executor
            .block_on(gwt::project_runtime::test_support::ProjectOwnerHarness::start(dir.path()))
            .expect("production owner fixture");
        executor
            .block_on(owner.set_pty_start_gate(
                std::env::current_exe().expect("fixture binary"),
                vec![
                    "app_runtime::tests::pty_start_gate_helper".into(),
                    "--exact".into(),
                    "--nocapture".into(),
                ],
            ))
            .expect("use actual fixture gate");
        let (process, ..) = result.expect("immutable fixture command");
        let mut config =
            gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Custom("fixture".into()))
                .working_dir(dir.path())
                .build();
        config.command = process.command;
        config.args = if cfg!(windows) {
            config.command = "powershell".into();
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                "while ($null -ne [Console]::ReadLine()) {}".into(),
            ]
        } else {
            vec!["-c".into(), "while IFS= read -r line; do :; done".into()]
        };
        let address = runtime
            .window_lookup
            .get(&window_id)
            .expect("window address")
            .clone();
        runtime.register_window(&address.tab_id, &address.raw_id);
        let context = runtime
            .project_context(&address.tab_id)
            .expect("project context");
        let generation = runtime
            .window_lifecycle_generations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&window_id)
            .copied();
        runtime
            .queue_project_owner_launch(
                &context,
                window_id.clone(),
                generation,
                Box::new(config),
                Box::default(),
            )
            .expect("queue actual owner launch");
        crate::app_runtime::tests::drain_queued_blocking_tasks(&tasks);
        let completion = crate::app_runtime::tests::take_remote_owner_launch_completion(&recorded);
        assert!(
            completion.result.is_ok(),
            "owner launch failed: {:?}",
            completion.result.as_ref().err()
        );
        // This cache ingest panics if the launch worker did not resolve its ledger.
        runtime.handle_remote_owner_launch_complete(completion);
        assert!(runtime.active_agent_sessions.contains_key(&window_id));
        executor.block_on(owner.shutdown());
    }
}

#[cfg(all(test, windows))]
mod agent_window_status_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn targeted_windows_official_provider_stays_starting_during_pre_spawn_health_checks() {
        for agent_id in [gwt_agent::AgentId::Codex, gwt_agent::AgentId::ClaudeCode] {
            let config = gwt_agent::AgentLaunchBuilder::new(agent_id).build();
            assert_eq!(
                initial_agent_window_status(&config),
                WindowProcessStatus::Starting
            );
        }

        let generic = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::OpenCode).build();
        assert_eq!(
            initial_agent_window_status(&generic),
            WindowProcessStatus::Running
        );
    }
}

#[cfg(test)]
mod agent_schedule_admission_tests {
    use super::*;
    use std::fs;
    #[test]
    fn agent_schedule_prepared_admission_fences_member_change_but_not_other_plans() {
        let temp = tempfile::tempdir().unwrap();
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(temp.path());
        let path = temp.path().join("issue-monitor.json");
        let mut value = serde_json::json!({"agent_issue_schedule":{"revision":1,"reservations":[
            {"host_id":gwt::process::current_hostname(),"agent_member_id":"a","issue_number":42,"reason":"queue_order"},
            {"host_id":gwt::process::current_hostname(),"agent_member_id":"b","issue_number":43,"reason":"queue_order"}
        ]}});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let original = issue_monitor_prefs_fence_digest(&path, Some(42)).unwrap();
        value["agent_issue_schedule"]["revision"] = serde_json::json!(2);
        value["agent_issue_schedule"]["reservations"][1]["agent_member_id"] =
            serde_json::json!("c");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            issue_monitor_prefs_fence_digest(&path, Some(42)).unwrap(),
            original,
            "other member's next Issue must not invalidate this admission"
        );
        value["agent_issue_schedule"]["reservations"][0]["agent_member_id"] =
            serde_json::json!("b");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_ne!(
            issue_monitor_prefs_fence_digest(&path, Some(42)).unwrap(),
            original,
            "a prepared launch cannot silently change its selected member"
        );
    }
}

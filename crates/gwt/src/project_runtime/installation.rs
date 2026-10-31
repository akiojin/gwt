//! Exact owner-local Session/PTY installation and publication.
use super::workspace::{save_resumed_workspace_projection, save_start_work_workspace_projection};
use super::{launch_finalization::*, launch_payload::*, ActiveLaunchHandshakeCleanup};
use crate::launch_runtime::same_worktree_path;
use crate::session_launch::{
    durable_launch_recovery_exists, recovery::clear_durable_launch_recovery,
};
use crate::WindowProcessStatus;
use gwt_terminal::Pane;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(crate) fn prepare_owner_shell_launch(
    project_root: &Path,
    window_id: &str,
    config: &mut crate::launch_wizard::ShellLaunchConfig,
    live_session_ids: &HashSet<String>,
    current: &AtomicBool,
    mut progress: impl FnMut(&str),
) -> Result<ProcessLaunch, String> {
    if !current.load(Ordering::Acquire) {
        return Err("shell launch cancelled before preparation".into());
    }
    if let Err(error) = super::workspace::save_shell_work_projection(
        project_root,
        window_id,
        config.working_dir.clone(),
        config.branch.clone(),
        live_session_ids,
    ) {
        tracing::warn!(project_root = %project_root.display(), window_id, %error,
            "shell Work projection registration skipped");
    }
    progress("Preparing worktree...");
    crate::launch_runtime::resolve_shell_launch_worktree(project_root, config)?;
    let worktree_path = config
        .working_dir
        .clone()
        .unwrap_or_else(|| project_root.into());
    gwt_agent::LaunchEnvironment::from_active_profile(
        &gwt_core::paths::gwt_config_path(),
        config.runtime_target,
    )?
    .with_project_root(&worktree_path)
    .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
    if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
        progress("Starting Docker service...");
    }
    let launch = crate::launch_runtime::build_shell_process_launch(project_root, config)?;
    if !current.load(Ordering::Acquire) {
        return Err("shell launch cancelled before PTY spawn".into());
    }
    Ok(launch)
}

#[derive(Clone)]
pub(crate) struct OwnerSavedSessionRestore {
    pub request: serde_json::Value,
    pub source: gwt_agent::Session,
    pub origin: gwt_agent::SessionLaunchOrigin,
    pub registry: Arc<std::sync::Mutex<crate::WindowRegistry>>,
    pub placeholders: Vec<crate::PersistedWindowState>,
    pub worktree_reserved: bool,
    pub registry_after: Option<Result<crate::PersistedWindowRegistry, String>>,
    pub launch_started: bool,
    pub member_reservation: Option<(String, String)>,
    pub refusal: Option<OwnerRestoreRefusal>,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct OwnerRestoreRefusal {
    pub message: String,
    pub code: String,
    pub remove_placeholder: bool,
    pub source_session_id: String,
    pub retryable: bool,
}

impl OwnerSavedSessionRestore {
    pub fn refuse(&mut self, code: &str, message: impl Into<String>, retryable: bool) -> String {
        let message = message.into();
        self.refusal = Some(OwnerRestoreRefusal {
            message: message.clone(),
            code: code.into(),
            remove_placeholder: false,
            source_session_id: self.source.id.clone(),
            retryable,
        });
        message
    }
    pub fn outcome(&self, detail: &str) -> serde_json::Value {
        let refusal = self.refusal.clone().unwrap_or_else(|| OwnerRestoreRefusal {
            message: detail.into(),
            code: "restore_launch_failed".into(),
            remove_placeholder: false,
            source_session_id: self.source.id.clone(),
            retryable: true,
        });
        serde_json::json!({"restore_refusal":refusal})
    }
}

pub(crate) fn mark_restore_source_completed_exact(
    sessions_dir: &Path,
    expected: &gwt_agent::Session,
) -> Result<(), String> {
    let outcome = gwt_agent::update_session_if_matches_with(
        sessions_dir,
        expected,
        |current, expected| {
            Ok(current.id == expected.id
                && current.created_at == expected.created_at
                && current.worktree_path == expected.worktree_path
                && current.project_state_root == expected.project_state_root
                && current.repo_hash == expected.repo_hash
                && current.agent_id == expected.agent_id
                && current.branch == expected.branch
                && current.linked_issue_number == expected.linked_issue_number
                && current.execution_binding == expected.execution_binding)
        },
        |session| {
            session.update_status(gwt_agent::AgentStatus::Stopped);
            session.restore_window_on_startup = false;
            Ok(())
        },
    )
    .map_err(|error| error.to_string())?;
    match outcome {
        gwt_agent::SessionSnapshotUpdateOutcome::Updated(()) => Ok(()),
        gwt_agent::SessionSnapshotUpdateOutcome::SnapshotChanged => {
            Err("restore source Session changed before retirement".into())
        }
        gwt_agent::SessionSnapshotUpdateOutcome::MutationFailed(error)
        | gwt_agent::SessionSnapshotUpdateOutcome::SnapshotUnreadable(error) => {
            Err(error.to_string())
        }
    }
}

pub fn retire_restore_source_after_launch(
    sessions_dir: &Path,
    source_id: &str,
    expected_source: Option<&gwt_agent::Session>,
    launched_session_id: &str,
) -> Result<(), String> {
    if source_id == launched_session_id {
        return Ok(());
    }
    let expected = expected_source
        .filter(|snapshot| snapshot.id == source_id)
        .ok_or("restore source retirement requires its owner-captured Session snapshot")?;
    mark_restore_source_completed_exact(sessions_dir, expected)
}

fn remove_restore_placeholders(
    root: &Path,
    restore: &OwnerSavedSessionRestore,
    empty_only: bool,
) -> Result<(), String> {
    if empty_only {
        #[cfg(any(test, feature = "test-gh-guard"))]
        super::agent_frontend::run_owner_agent_cleanup_test_hook(root, &restore.source.id);
        let mut registry = restore.registry.lock().map_err(|error| error.to_string())?;
        for expected in &restore.placeholders {
            if registry
                .window(&expected.id)
                .is_some_and(|row| restore_row_is_empty(row, &restore.source.id))
            {
                registry.close_window(&expected.id);
            }
        }
        return crate::persistence::save_workspace_state_durable(
            &crate::persistence::workspace_state_path(root),
            &registry.persistable_state(),
        )
        .map_err(|error| error.to_string());
    }
    for row in &restore.placeholders {
        super::agent_dispatch::OwnedWindowRecordCleanup {
            registry: restore.registry.clone(),
            registry_path: crate::persistence::workspace_state_path(root),
            root: root.into(),
            project_key: gwt_core::paths::project_scope_hash(root).to_string(),
            canonical_id: format!("{}::{}", gwt_core::paths::project_scope_hash(root), row.id),
            expected_session_id: Some(restore.source.id.clone()),
        }
        .finish()?;
    }
    Ok(())
}

pub(crate) fn finish_owner_restore_failure(
    root: &Path,
    sessions_dir: &Path,
    restore: &mut OwnerSavedSessionRestore,
    detail: &str,
) -> Result<(), String> {
    if let Some((member, reservation)) = restore.member_reservation.take() {
        crate::agent_names::release_project_member(root, sessions_dir, &member, &reservation)?;
    }
    if restore.refusal.is_none() {
        restore.refuse("restore_launch_failed", detail, true);
        if restore.launch_started
            && restore.origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore
        {
            mark_restore_source_completed_exact(sessions_dir, &restore.source)?;
            remove_restore_placeholders(root, restore, false)?;
            restore
                .refusal
                .as_mut()
                .expect("restore failure")
                .remove_placeholder = true;
        } else {
            let mut registry = restore.registry.lock().map_err(|error| error.to_string())?;
            for expected in &restore.placeholders {
                if registry
                    .window(&expected.id)
                    .is_some_and(|row| row.session_id == expected.session_id)
                {
                    registry.set_status(&expected.id, WindowProcessStatus::Error);
                }
            }
            crate::persistence::save_workspace_state_durable(
                &crate::persistence::workspace_state_path(root),
                &registry.persistable_state(),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Preserve the existing local landed-worktree refusal without fetching.
pub fn landed_restore_refusal(
    session: &gwt_agent::Session,
    project_root: &Path,
    placeholder_known_empty: bool,
) -> Option<&'static str> {
    if !crate::pm_registry::registered_pm_worktree_authority(
        project_root,
        &session.id,
        &session.worktree_path,
    ) && gwt_core::process::run_git_logged(
        &[
            "merge-base",
            "--is-ancestor",
            "HEAD",
            "refs/remotes/origin/develop",
        ],
        Some(&session.worktree_path),
    )
    .is_ok_and(|output| output.status.success())
    {
        Some(if placeholder_known_empty {
            "empty_landed_worktree"
        } else {
            "landed_worktree"
        })
    } else {
        None
    }
}

fn restore_row_is_empty(row: &crate::PersistedWindowState, source: &str) -> bool {
    row.session_id.as_deref() == Some(source)
        && row.linked_issue_number.is_none()
        && row.status == WindowProcessStatus::Stopped
        && row
            .dynamic_title_detail
            .as_ref()
            .is_none_or(|detail| detail.trim().is_empty())
}

fn owner_restore_placeholder_known_empty(restore: &OwnerSavedSessionRestore) -> bool {
    let session = &restore.source;
    if session.linked_issue_number.is_some()
        || session.execution_binding.is_some()
        || !matches!(
            session.status,
            gwt_agent::AgentStatus::Idle | gwt_agent::AgentStatus::Stopped
        )
        || session.last_exit_code.is_some_and(|code| code != 0)
        || session.last_exit_signal.is_some()
        || crate::cli::execution_state::diagnose_for_projection(
            &session.worktree_path,
            Some(&session.id),
        )
        .ecr_status
            != crate::cli::execution_state::ExecutionDiagnosisState::Missing
    {
        return false;
    }
    let Ok(registry) = restore.registry.lock() else {
        return false;
    };
    restore.placeholders.iter().any(|expected| {
        registry
            .window(&expected.id)
            .is_some_and(|row| restore_row_is_empty(row, &session.id))
    })
}

pub(crate) fn prepare_owner_saved_session_restore(
    root: &Path,
    sessions_dir: &Path,
    restore: &mut OwnerSavedSessionRestore,
    config: &mut gwt_agent::LaunchConfig,
    metadata: &mut OwnerLaunchMetadata,
    current: &AtomicBool,
) -> Result<(), String> {
    let path = sessions_dir.join(format!("{}.toml", restore.source.id));
    let session = gwt_agent::Session::load(&path)
        .map_err(|error| restore.refuse("source_unreadable", error.to_string(), true))?;
    if serde_json::to_value(&session).ok() != serde_json::to_value(&restore.source).ok() {
        return Err(restore.refuse(
            "source_changed",
            "The saved Session changed; retry with the current Session",
            true,
        ));
    }
    if !current.load(Ordering::Acquire) {
        return Err(restore.refuse(
            "cancelled",
            "Restore was cancelled before preparation",
            false,
        ));
    }
    let own_project_dir = gwt_core::paths::gwt_project_dir_for_repo_path(root);
    let is_pm = crate::pm_registry::is_pm_worktree(&session.worktree_path);
    if crate::pm_registry::is_foreign_pm_worktree(&session.worktree_path, &own_project_dir) {
        return Err(restore.refuse(
            "foreign_pm",
            "The saved Session belongs to another project store's PM worktree",
            false,
        ));
    }
    let scope = gwt_core::paths::project_scope_hash(
        session
            .project_state_root
            .as_deref()
            .unwrap_or(&session.worktree_path),
    );
    if !is_pm && scope != gwt_core::paths::project_scope_hash(root) {
        return Err(restore.refuse(
            "foreign_project",
            "The saved Session belongs to another project",
            false,
        ));
    }
    if is_pm
        && !crate::pm_registry::session_is_registered_pm(
            &crate::pm_registry::pm_prefs_path_for_repo_path(root),
            &session.id,
        )
    {
        mark_restore_source_completed_exact(sessions_dir, &restore.source)?;
        remove_restore_placeholders(root, restore, false)?;
        let error = restore.refuse(
            "unregistered_pm",
            "The saved Session is not the registered PM",
            false,
        );
        restore
            .refusal
            .as_mut()
            .expect("PM refusal")
            .remove_placeholder = true;
        return Err(error);
    }
    if !session.worktree_path.is_dir() {
        return Err(restore.refuse(
            "worktree_missing",
            "The saved Session worktree is no longer a directory",
            false,
        ));
    }
    if restore.origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore {
        if !session.restore_window_on_startup {
            return Err(restore.refuse(
                "window_closed",
                "The saved Session is no longer marked for restore",
                false,
            ));
        }
        let placeholder = restore
            .placeholders
            .first()
            .map(|row| format!("{}::{}", gwt_core::paths::project_scope_hash(root), row.id));
        let admission =
            restore_saved_session_work_terminality(&session, root, placeholder.as_deref());
        if admission != RestoreAdmission::Admit
            && admission != RestoreAdmission::RefuseCompletedWork
        {
            let terminal = matches!(admission, RestoreAdmission::RefuseTerminal(_));
            let reason = match admission {
                RestoreAdmission::RefuseTerminal(reason) => reason.as_str(),
                RestoreAdmission::RefuseRetainedTerminal => "closed_work_diagnostic_retained",
                RestoreAdmission::RefuseCompletedWork => "completed_work_retained",
                RestoreAdmission::RefuseFreshSession => "fresh_session_required",
                RestoreAdmission::RefuseHeld(reason)
                | RestoreAdmission::RefuseUnprovable(reason) => reason,
                RestoreAdmission::Admit => unreachable!(),
            };
            if terminal {
                mark_restore_source_completed_exact(sessions_dir, &restore.source)?;
                remove_restore_placeholders(root, restore, false)?;
            }
            let error = restore.refuse(
                reason,
                format!("Automatic restore refused: {reason}"),
                !terminal,
            );
            restore
                .refusal
                .as_mut()
                .expect("terminal refusal")
                .remove_placeholder = terminal;
            return Err(error);
        }
        if let Some(code) = landed_restore_refusal(
            &session,
            root,
            owner_restore_placeholder_known_empty(restore),
        ) {
            let remove_placeholder = code == "empty_landed_worktree";
            if remove_placeholder {
                mark_restore_source_completed_exact(sessions_dir, &restore.source)?;
                remove_restore_placeholders(root, restore, true)?;
            }
            let error = restore.refuse(code, format!("Automatic restore refused: {code}"), false);
            restore
                .refusal
                .as_mut()
                .expect("landed refusal")
                .remove_placeholder = remove_placeholder;
            return Err(error);
        }
        if admission == RestoreAdmission::RefuseCompletedWork {
            return Err(restore.refuse(
                "completed_work_retained",
                "Automatic restore refused: completed_work_retained",
                false,
            ));
        }
        if session.exact_resume_session_id().is_none() {
            return Err(restore.refuse(
                "no_resume_session",
                "The saved Session has no native conversation handle",
                false,
            ));
        }
    }
    if restore.worktree_reserved {
        return Err(restore.refuse(
            "worktree_already_restoring",
            "The saved Session worktree already has a live or pending owner pane",
            true,
        ));
    }
    if is_pm {
        crate::pm_registry::refresh_pm_worktree_for_repo_path(root)
            .map_err(|error| error.to_string())?;
    }
    let member_root = (!crate::pm_registry::is_canonical_pm_worktree(&session.worktree_path)
        && !session.review_dispatch)
        .then_some(root);
    if let Some(member_root) = member_root {
        if restore.source.agent_name.is_none() {
            let previous = restore.source.clone();
            let existing_name = restore
                .placeholders
                .iter()
                .find_map(|row| row.agent_name.clone());
            let name = match existing_name {
                Some(name) => name,
                None => {
                    let known = restore
                        .registry
                        .lock()
                        .map_err(|error| error.to_string())?
                        .persisted()
                        .windows
                        .iter()
                        .filter_map(|row| row.agent_name.clone())
                        .collect();
                    let language = gwt_config::Settings::load()
                        .map(|settings| settings.ai.effective_language())
                        .unwrap_or("en");
                    crate::agent_names::allocate(
                        &own_project_dir.join("agent-names.jsonl"),
                        language,
                        &HashSet::new(),
                        &known,
                    )?
                }
            };
            restore.source.agent_name = Some(name);
            if !restore
                .source
                .save_if_unchanged(sessions_dir, &previous)
                .map_err(|error| error.to_string())?
            {
                return Err(restore.refuse(
                    "source_changed",
                    "Session changed during restore naming; retry with the current Session",
                    true,
                ));
            }
        }
        crate::agent_names::adopt_project_member_session(
            member_root,
            sessions_dir,
            &mut restore.source,
        )?;
    }
    *config = crate::session_launch::launch_config_from_persisted_session(&restore.source);
    config.automatic_restore = restore.origin == gwt_agent::SessionLaunchOrigin::AutomaticRestore;
    if restore.origin == gwt_agent::SessionLaunchOrigin::UserRestart {
        config.launch_route = gwt_agent::LaunchRoute::Manual;
    }
    config.predecessor_session_id = Some(restore.source.id.clone());
    let mut agent_name = restore.source.agent_name.clone().or_else(|| {
        restore
            .placeholders
            .iter()
            .filter(|row| {
                matches!(
                    row.status,
                    WindowProcessStatus::Stopped | WindowProcessStatus::Error
                )
            })
            .find_map(|row| row.agent_name.clone())
    });
    if let Some(member_root) = member_root {
        let token = uuid::Uuid::new_v4().to_string();
        let member = crate::agent_names::reserve_project_member_for_agent(
            member_root,
            sessions_dir,
            &token,
            restore.source.linked_issue_number,
            restore.source.linked_issue_number.and_then(|number| {
                crate::issue_monitor_worker::cached_agent_member_spec_owner(root, number)
            }),
            None,
            restore.source.agent_member_id.as_deref(),
            restore.source.agent_id.command(),
            None,
        )?;
        let reservation = member
            .assignment
            .as_ref()
            .expect("reserved restore assignment")
            .reservation_id
            .clone();
        config
            .env_vars
            .insert("GWT_AGENT_MEMBER_ID".into(), member.agent_member_id.clone());
        config
            .env_vars
            .insert("GWT_AGENT_RESERVATION_ID".into(), reservation.clone());
        agent_name = Some(member.name);
        restore.member_reservation = Some((member.agent_member_id, reservation));
    }
    metadata.inherited_agent_name = agent_name;
    let state_root = restore.source.project_state_root.as_deref().unwrap_or(root);
    metadata.workspace_resume_context = Some(workspace_resume_context_for_work_item(
        state_root,
        Some(&restore.source.branch),
        &restore.source.worktree_path,
    ));
    Ok(())
}

pub struct InstallAgentLaunchInput {
    pub auto_resume_source_session_id: Option<String>,
    pub auto_resume_source_snapshot: Option<gwt_agent::Session>,
    pub current: Arc<AtomicBool>,
    pub failure_session: Option<(ActiveAgentSession, gwt_agent::Session)>,
    /// Set only at the actual OS child-spawn call; cleanup never clears it.
    pub physical_spawn_started: bool,
    pub failed_launch: Option<Box<PreparedAgentLaunchSuccess>>,
    pub had_window: bool,
    pub is_continue_work: bool,
    pub issue_link_cache_dir: PathBuf,
    pub launch_feedback_context: Option<LaunchFeedbackContext>,
    pub live_pm_session_ids: HashSet<String>,
    pub materializer_id: String,
    pub origin: gwt_agent::SessionLaunchOrigin,
    pub pending_continue_work: Option<PendingContinueWork>,
    pub pending_fresh_execution: Option<PendingFreshExecutionLaunch>,
    pub pending_pm_project_root: Option<PathBuf>,
    pub project_root: PathBuf,
    pub removed_genesis_session: Option<gwt_agent::SessionExecutionIdentity>,
    pub sessions_dir: PathBuf,
    pub stage_id: u64,
    pub tab_id: String,
    pub window_id: String,
    pub workspace_resume_context: Option<WorkspaceResumeContext>,
    pub incarnation: u64,
}

pub struct PreparedAgentLaunchSuccess {
    pub project_root: PathBuf,
    pub active_session: ActiveAgentSession,
    pub pane: Pane,
    pub incarnation: u64,
    pub initial_prompt_file: Option<Arc<tempfile::TempPath>>,
    pub capability_token: Option<String>,
    pub member_reservation: Option<String>,
    pub pending_fresh_execution: Option<PendingFreshExecutionLaunch>,
    pub pending_continue_work: Option<PendingContinueWork>,
    pub workspace_projection_updated: bool,
    pub pm_registration: Option<(
        PathBuf,
        Option<crate::pm_registry::PmRegistration>,
        crate::BackendEvent,
    )>,
    pub cached_session: Option<gwt_agent::Session>,
    pub session_snapshot: Option<gwt_agent::Session>,
    pub predecessor_session_id: Option<String>,
    pub materialized_genesis: Option<MaterializedGenesisLaunch>,
}

pub fn install_agent_launch(
    input: &mut InstallAgentLaunchInput,
    launch: AgentLaunchCompletion,
    pty_gate_launch_parts: impl Fn() -> Result<(PathBuf, Vec<String>), String>,
) -> Result<PreparedAgentLaunchSuccess, String> {
    let (
        process_launch,
        session_id,
        branch_name,
        display_name,
        worktree_path,
        agent_id,
        linked_issue_number,
        base_branch,
        runtime_target,
        session_mode,
        had_prepared_execution,
        runtime_context,
    ) = launch;
    let AgentLaunchRuntimeContext {
        agent_project_root,
        predecessor_session_id,
        expected_execution_identity,
        active_launch_handshake,
    } = runtime_context;
    let mut handshake =
        ActiveLaunchHandshakeCleanup::new(input.sessions_dir.clone(), active_launch_handshake);
    if let Some((context, handoff)) = input.launch_feedback_context.as_ref().and_then(|context| {
        context
            .issue_monitor_autonomous_handoff
            .as_ref()
            .map(|handoff| (context, handoff))
    }) {
        let root = context
            .issue_monitor_project_root
            .as_deref()
            .ok_or("answered handoff target Project State is unavailable")?;
        let session =
            gwt_agent::Session::load(&input.sessions_dir.join(format!("{session_id}.toml")))
                .map_err(|error| {
                    format!("answered handoff target Session is unavailable: {error}")
                })?;
        if session.id != session_id || session.agent_id != agent_id {
            return Err("answered handoff target Session changed before provider spawn".into());
        }
        let target = autonomous_handoff_delivery_target_for_session(
            &session,
            handoff.issue_number,
            &input.window_id,
            context.issue_monitor_delivery_id.as_deref(),
            &input.materializer_id,
        )?;
        if !crate::bind_autonomous_handoff_delivery_target_from_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(root),
            &handoff.handoff_id,
            &handoff.session_id,
            handoff.attempt,
            &target,
        )
        .map_err(|error| format!("failed to bind answered handoff target: {error}"))?
        {
            return Err("answered handoff target no longer matches its durable attempt".into());
        }
    }
    let capability_token = process_launch
        .env
        .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .cloned();
    let member_reservation = process_launch.env.get("GWT_AGENT_RESERVATION_ID").cloned();
    let readiness_nonce = process_launch
        .env
        .get(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV)
        .cloned();
    let pending_fresh_execution = if input.is_continue_work {
        None
    } else if let Some(nonce) = readiness_nonce.as_deref() {
        if !had_prepared_execution && session_mode != gwt_agent::SessionMode::Normal {
            return Err("a resume launch cannot carry fresh execution readiness".into());
        }
        match pending_fresh_execution_launch_from_session(
            &input.sessions_dir,
            &session_id,
            &worktree_path,
            &agent_project_root,
            linked_issue_number,
            base_branch.clone(),
            input.workspace_resume_context.clone(),
            input.launch_feedback_context.clone(),
            nonce,
            &agent_id,
        ) {
            Ok(pending) => Some(pending),
            Err(error) => {
                let rollback = rollback_materialized_fresh_execution_launch(
                    &input.sessions_dir,
                    &session_id,
                    &worktree_path,
                    "fresh launch readiness reconstruction failed",
                    &agent_id,
                );
                return Err(match rollback {
                    Ok(()) => format!("fresh linked-owner launch readiness could not be recovered: {error}"),
                    Err(rollback) => format!("fresh linked-owner launch readiness could not be recovered: {error}; candidate rollback requires reconciliation: {rollback}"),
                });
            }
        }
    } else {
        None
    };
    input.pending_fresh_execution = pending_fresh_execution.clone();
    let is_fresh = pending_fresh_execution.is_some();
    let genesis = if (had_prepared_execution || session_mode == gwt_agent::SessionMode::Normal)
        && !input.is_continue_work
        && !is_fresh
    {
        materialized_genesis_launch_from_session(
            &input.sessions_dir,
            &session_id,
            &worktree_path,
            &input.project_root,
            &branch_name,
            linked_issue_number,
            &agent_id,
        )
        .map_err(|error| {
            format!("materialized genesis launch authority could not be authenticated: {error}")
        })?
    } else {
        None
    };
    if let Some(expected) = expected_execution_identity.as_ref() {
        let current = gwt_agent::Session::load(
            &input
                .sessions_dir
                .join(format!("{}.toml", expected.session_id)),
        )
        .ok()
        .and_then(|session| gwt_agent::SessionExecutionIdentity::from_session(&session).ok())
        .flatten();
        if current.as_ref() != Some(expected) {
            return Err("bound launch Session identity changed before PTY spawn".into());
        }
    }
    let canonical_owner = expected_execution_identity
        .as_ref()
        .map(|expected| {
            let binding = &expected.execution_binding;
            let kind = match binding.owner_kind.as_str() {
                "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
                "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
                _ => return Err("bound launch owner kind is not canonical".to_string()),
            };
            Ok(crate::cli::execution_state::ExecutionOwnerKey {
                kind,
                number: binding.owner_number,
            })
        })
        .transpose()?
        .or_else(|| genesis.as_ref().map(|genesis| genesis.owner));
    let active = ActiveAgentSession {
        window_id: input.window_id.clone(),
        session_id: session_id.clone(),
        agent_id: agent_id.command().to_string(),
        branch_name,
        display_name,
        worktree_path: worktree_path.clone(),
        agent_project_root,
        runtime_target,
        tab_id: input.tab_id.clone(),
    };
    input.had_window.then_some(()).ok_or_else(|| {
        let Some(genesis) = genesis.as_ref() else {
            return "Window not found".to_string();
        };
        match rollback_worker_genesis(
            &input.sessions_dir,
            &input.project_root,
            &active,
            genesis,
            "genesis launch window disappeared before PTY spawn",
            &mut input.removed_genesis_session,
        ) {
            Ok(()) => "Window not found; genesis authority was terminalized".to_string(),
            Err(error) => format!(
                "Window not found; failed genesis terminalization retained exact evidence for retry: {error}"
            ),
        }
    })?;
    // Manual Prepared retry needs the actual pre-child snapshot for exact
    // provisional retirement; fresh diagnostic publication remains excluded.
    let manual_prepared = input
        .pending_fresh_execution
        .as_ref()
        .is_some_and(|pending| pending.request.principal_id == "gwt-host-manual-launch");
    let failure_snapshot = |session: gwt_agent::Session| {
        (!input.is_continue_work
            && (!is_fresh || manual_prepared)
            && genesis.is_none()
            && session.id == session_id
            && session.agent_id == agent_id
            && session.branch == active.branch_name
            && same_worktree_path(&session.worktree_path, &worktree_path)
            && expected_execution_identity.as_ref().is_none_or(|expected| {
                gwt_agent::SessionExecutionIdentity::from_session(&session)
                    .ok()
                    .flatten()
                    .as_ref()
                    == Some(expected)
            }))
        .then(|| (active.clone(), session))
    };
    // Cancellation still needs the actual materialized, pre-child Session to
    // retire its captured provisional runtime under the existing exact CAS.
    input.failure_session =
        gwt_agent::Session::load(&input.sessions_dir.join(format!("{session_id}.toml")))
            .ok()
            .and_then(failure_snapshot);
    if !input.current.load(Ordering::Acquire) {
        if let Some(genesis) = genesis.as_ref() {
            rollback_worker_genesis(
                &input.sessions_dir,
                &input.project_root,
                &active,
                genesis,
                "launch window closed before PTY spawn",
                &mut input.removed_genesis_session,
            )?;
        }
        return Err("launch window closed before PTY spawn".into());
    }
    match gwt_agent::update_session_with_wait(
        &input.sessions_dir,
        &session_id,
        std::time::Duration::from_secs(2),
        |session| {
            session.restore_window_on_startup = true;
            session.updated_at = chrono::Utc::now();
            session.launch_origin = input.origin;
            session.restore_source_session_id = input.auto_resume_source_session_id.clone();
            Ok(())
        },
    ) {
        Ok(session) => input.failure_session = failure_snapshot(session),
        Err(error) => {
            tracing::warn!(%session_id, %error, "failed to persist launched Session window metadata");
        }
    }
    let pm_root = input.pending_pm_project_root.clone().or_else(|| {
        let source = input.auto_resume_source_session_id.as_ref()?;
        let prefs = crate::pm_registry::load_pm_prefs(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&input.project_root),
        )
        .ok()?;
        (prefs.registration?.session_id == *source).then(|| input.project_root.clone())
    });
    if let Some(source) = input.auto_resume_source_session_id.as_ref() {
        retire_restore_source_after_launch(
            &input.sessions_dir,
            source,
            input.auto_resume_source_snapshot.as_ref(),
            &session_id,
        )?;
    }
    emit_agent_launch_stage(
        input.stage_id,
        "resolve_binary",
        &format!("wizard launch {}", process_launch.command),
    );
    emit_agent_launch_stage(
        input.stage_id,
        "prepare_env",
        &format!("worktree={}", worktree_path.display()),
    );
    emit_agent_launch_stage(
        input.stage_id,
        "spawn_pty",
        &launch_argv_summary(&process_launch.args),
    );
    // Bootstrap a standard grid; the mounted terminal sends UpdateTerminalGrid.
    let (cols, rows) = (80, 24);
    let initial_prompt_file = process_launch.initial_prompt_file;
    let spawn_config = gwt_terminal::pty::SpawnConfig {
        command: process_launch.command,
        args: process_launch.args,
        cols,
        rows,
        env: process_launch.env,
        remove_env: process_launch.remove_env,
        cwd: process_launch.cwd,
    };
    let incarnation = input.incarnation;
    let monitor_admission = pm_root.is_none()
        && input
            .launch_feedback_context
            .as_ref()
            .is_some_and(|context| context.issue_monitor_issue_number.is_some());
    let spawn_attempted = std::cell::Cell::new(false);
    let spawn_pane = || {
        if let Some(expected) = expected_execution_identity.as_ref() {
            crate::project_runtime::spawn_bound_pane_with_attempt_observer(
                &input.window_id,
                spawn_config,
                process_launch.resource_policy,
                pty_gate_launch_parts()?,
                incarnation,
                expected,
                &mut handshake,
                || spawn_attempted.set(true),
            )
        } else {
            let policy_gate = process_launch
                .resource_policy
                .map(|policy| {
                    pty_gate_launch_parts().map(|(program, args)| (policy, program, args))
                })
                .transpose()?;
            crate::project_runtime::spawn_unbound_pane_with_attempt_observer(
                &input.window_id,
                spawn_config,
                policy_gate,
                incarnation,
                Some((&input.sessions_dir, &session_id)),
                monitor_admission,
                || spawn_attempted.set(true),
            )
        }
    };
    // This function already runs on a blocking worker. Keep the file guard
    // entirely within the physical spawn, never across a GUI event handoff.
    let spawn = if monitor_admission {
        crate::machine_agent_admission::with_admission(&input.sessions_dir, spawn_pane)
    } else {
        spawn_pane()
    };
    input.physical_spawn_started |= spawn_attempted.get();
    let pane = match spawn {
        Ok(pane) => pane,
        Err(error) => {
            if handshake.child_started() {
                if let Some(context) = input.launch_feedback_context.as_mut() {
                    context.issue_monitor_autonomous_submit_started =
                        context.issue_monitor_autonomous_handoff.is_some();
                }
                handshake.retain_for_reconciliation();
            }
            return Err(if let Some(genesis) = genesis.as_ref() {
                match rollback_worker_genesis(&input.sessions_dir, &input.project_root, &active, genesis,
                    "genesis PTY spawn failed before launch readiness", &mut input.removed_genesis_session) {
                    Ok(()) => error,
                    Err(rollback) => format!("{error}; failed genesis recovery retained exact evidence for retry: {rollback}"),
                }
            } else {
                error
            });
        }
    };
    input.failed_launch = Some(Box::new(PreparedAgentLaunchSuccess {
        project_root: input.project_root.clone(),
        active_session: active.clone(),
        pane,
        incarnation,
        initial_prompt_file: initial_prompt_file.clone(),
        capability_token: capability_token.clone(),
        member_reservation: member_reservation.clone(),
        pending_fresh_execution: pending_fresh_execution.clone(),
        pending_continue_work: input.pending_continue_work.clone(),
        workspace_projection_updated: false,
        pm_registration: None,
        cached_session: None,
        session_snapshot: gwt_agent::Session::load(
            &input.sessions_dir.join(format!("{session_id}.toml")),
        )
        .ok()
        .or_else(|| {
            input
                .failure_session
                .as_ref()
                .map(|(_, snapshot)| snapshot.clone())
        }),
        predecessor_session_id: predecessor_session_id.clone(),
        materialized_genesis: genesis.clone(),
    }));
    let pane = &input
        .failed_launch
        .as_ref()
        .expect("captured physical candidate")
        .pane;
    if let Some(context) = input.launch_feedback_context.as_mut() {
        context.issue_monitor_autonomous_submit_started =
            context.issue_monitor_autonomous_handoff.is_some();
    }
    if let Err(error) = handshake.finish() {
        if !crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &input.window_id)
            .map_err(|cleanup| format!("{error}; exact child reap failed: {cleanup}"))?
        {
            return Err(format!("{error}; exact child exit remains unconfirmed"));
        }
        return Err(error);
    }
    let pm_registration = pm_root.and_then(|root| {
        let candidate = crate::pm_registry::PmRegistration {
            session_id: session_id.clone(),
            agent_id: agent_id.command().to_string(),
            worktree_path: worktree_path.to_string_lossy().into_owned(),
            created_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            consecutive_crashes: 0,
            next_not_before: None,
        };
        let registration = crate::pm_registry::try_register_pm(
            &crate::pm_registry::pm_prefs_path_for_repo_path(&root),
            candidate,
            |existing| {
                input.live_pm_session_ids.contains(&existing.session_id)
                    || gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path(
                        &input.sessions_dir,
                        &existing.session_id,
                    ))
                    .ok()
                    .and_then(|runtime| runtime.child_pid.zip(runtime.child_started_at))
                    .is_some_and(|(pid, started_at)| {
                        crate::process::host_process_start_time(pid) == Some(started_at)
                    })
            },
        )
        .map(|(prefs, outcome)| {
            if let crate::pm_registry::PmRegisterOutcome::ReplacedStale { previous } = outcome {
                if previous.session_id != session_id {
                    mark_auto_resume_source_completed(&input.sessions_dir, &previous.session_id);
                }
            }
            let status = super::pm_delivery::pm_status_from_launch_prefs(
                &input.sessions_dir,
                &prefs,
                prefs.registration.as_ref(),
            );
            (prefs.registration, status)
        })
        .map_err(|error| {
            tracing::warn!(%error, "failed to write PM registration after launch");
        })
        .ok()?;
        Some((root, registration.0, registration.1))
    });
    emit_agent_launch_stage(input.stage_id, "ready", "PTY handoff complete");
    if !is_fresh {
        let linkage = match linked_issue_number {
            Some(number) => record_issue_branch_link_with_cache_dir(
                &worktree_path,
                &active.branch_name,
                number,
                &input.issue_link_cache_dir,
            ),
            None => clear_issue_branch_link_with_cache_dir(
                &worktree_path,
                &active.branch_name,
                &input.issue_link_cache_dir,
            ),
        };
        if let Err(error) = linkage {
            tracing::warn!(%error, "issue branch linkage update skipped after agent launch");
        }
    }
    let projection = if !input.is_continue_work && !is_fresh {
        if let Some(base) = base_branch.as_deref() {
            save_start_work_workspace_projection(
                &input.project_root,
                &active,
                base,
                linked_issue_number,
                canonical_owner,
                input.workspace_resume_context.as_ref(),
                None,
            )
            .map(|()| true)
        } else if let Some(context) = input.workspace_resume_context.as_ref() {
            save_resumed_workspace_projection(
                &input.project_root,
                &active,
                None,
                linked_issue_number,
                canonical_owner,
                context,
                None,
            )
            .map(|()| true)
        } else {
            Ok(false)
        }
    } else {
        Ok(false)
    };
    let workspace_projection_updated = match projection {
        Ok(updated) if updated || genesis.is_none() => updated,
        result => {
            let detail = result
                .err()
                .map(|error| error.to_string())
                .unwrap_or_else(|| "genesis launch did not publish its Work projection".into());
            if !crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &input.window_id)
                .map_err(|cleanup| format!("{detail}; exact child reap failed: {cleanup}"))?
            {
                return Err(format!("{detail}; exact child exit remains unconfirmed"));
            }
            return Err(if let Some(genesis) = genesis.as_ref() {
                match rollback_worker_genesis(&input.sessions_dir, &input.project_root, &active, genesis,
                    "genesis Work publication failed before launch readiness", &mut input.removed_genesis_session) {
                    Ok(()) => detail,
                    Err(error) => format!("{detail}; failed genesis recovery retained exact evidence for retry: {error}"),
                }
            } else {
                detail
            });
        }
    };
    if let Some(genesis) = genesis.as_ref() {
        #[cfg(any(test, feature = "test-gh-guard"))]
        super::agent_frontend::run_owner_genesis_receipt_cleanup_test_hook(
            &input.project_root,
            &genesis.session_id,
        );
        if let Err(error) = clear_durable_launch_recovery(&input.sessions_dir, &genesis.session_id)
        {
            if !crate::session_finalizer::kill_and_reap(&pane.shared_pty(), &input.window_id)
                .map_err(|cleanup| format!("{error}; exact child reap failed: {cleanup}"))?
            {
                return Err(format!("{error}; exact child exit remains unconfirmed"));
            }
            let rollback = rollback_worker_genesis(
                &input.sessions_dir,
                &input.project_root,
                &active,
                genesis,
                "genesis recovery receipt could not be settled",
                &mut input.removed_genesis_session,
            );
            return Err(format!(
                "genesis launch readiness could not be committed: {error}; rollback: {rollback:?}"
            ));
        }
    }
    let cached_session = if durable_launch_recovery_exists(&input.sessions_dir, &session_id) {
        None
    } else {
        gwt_agent::Session::load_and_migrate(&input.sessions_dir.join(format!("{session_id}.toml")))
            .ok()
    };
    let session_snapshot = cached_session.clone().or_else(|| {
        gwt_agent::Session::load(&input.sessions_dir.join(format!("{session_id}.toml"))).ok()
    });
    Ok(PreparedAgentLaunchSuccess {
        project_root: input.project_root.clone(),
        active_session: active,
        pane: input
            .failed_launch
            .take()
            .expect("successful physical candidate")
            .pane,
        incarnation,
        initial_prompt_file,
        capability_token,
        member_reservation,
        pending_fresh_execution,
        pending_continue_work: input.pending_continue_work.clone(),
        workspace_projection_updated,
        pm_registration,
        cached_session,
        session_snapshot,
        predecessor_session_id,
        materialized_genesis: genesis,
    })
}

pub fn emit_agent_launch_stage(spawn_id: u64, stage: &str, detail: &str) {
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = spawn_id,
        stage = stage,
        detail = detail,
        "agent launch stage",
    );
    // Also push a synthetic line into the hub so the agent tab shows the
    // stage banner in real time (the summary event alone lives in
    // canonical log + Logs window only).
    let hub = gwt_core::process_console::global();
    let label = format!("[{stage}] {detail}");
    hub.push(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::AgentBootstrap,
        spawn_id,
        gwt_core::process_console::ProcessStream::Stdout,
        label,
    ));
}

pub fn launch_argv_summary(args: &[String]) -> String {
    format!("argc={}", args.len())
}

pub fn record_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: u64,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, Some(issue_number), cache_dir)
}

pub fn clear_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    update_issue_branch_link_with_cache_dir(repo_path, branch_name, None, cache_dir)
}

pub fn update_issue_branch_link_with_cache_dir(
    repo_path: &Path,
    branch_name: &str,
    issue_number: Option<u64>,
    cache_dir: &Path,
) -> Result<(), String> {
    let branch_name = branch_name.trim();
    if branch_name.is_empty() {
        return Ok(());
    }
    let Some(repo_hash) = crate::index_worker::detect_repo_hash(repo_path) else {
        return Err("repository hash is unavailable for issue linkage".to_string());
    };
    let path = cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));

    let mut store = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<IssueBranchLinkStore>(&bytes)
            .map_err(|error| format!("failed to parse issue linkage store: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            IssueBranchLinkStore::default()
        }
        Err(error) => return Err(format!("failed to read issue linkage store: {error}")),
    };
    match issue_number {
        Some(issue_number) => {
            store.branches.insert(branch_name.to_string(), issue_number);
        }
        None => {
            if store.branches.remove(branch_name).is_none() {
                return Ok(());
            }
        }
    }

    let bytes = serde_json::to_vec_pretty(&store)
        .map_err(|error| format!("failed to serialize issue linkage store: {error}"))?;
    gwt_github::cache::write_atomic(&path, &bytes)
        .map_err(|error| format!("failed to write issue linkage store: {error}"))
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct IssueBranchLinkStore {
    #[serde(default)]
    pub branches: HashMap<String, u64>,
}

pub fn mark_auto_resume_source_completed(sessions_dir: &Path, session_id: &str) {
    let _ = gwt_agent::update_session_with_wait(
        sessions_dir,
        session_id,
        std::time::Duration::from_secs(2),
        |session| {
            session.update_status(gwt_agent::AgentStatus::Stopped);
            session.restore_window_on_startup = false;
            Ok(())
        },
    );
}

pub fn autonomous_handoff_delivery_target_for_session(
    session: &gwt_agent::Session,
    issue_number: u64,
    window_id: &str,
    delivery_id: Option<&str>,
    materializer_id: &str,
) -> Result<crate::autonomous_handoff::AutonomousHandoffDeliveryTarget, String> {
    if session.linked_issue_number != Some(issue_number) {
        return Err(format!(
            "answered handoff target Session {} is not linked to Issue #{issue_number}",
            session.id
        ));
    }
    let native_session_id = session
        .exact_resume_session_id()
        .filter(|identity| !identity.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "answered handoff target Session {} has no exact native identity",
                session.id
            )
        })?;
    let repo_hash = session
        .repo_hash
        .clone()
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| {
            format!(
                "answered handoff target Session {} has no repository identity",
                session.id
            )
        })?;
    let project_state_root = crate::validated_project_state_root_for_session_recovery(session)
        .map_err(|error| {
            format!(
                "answered handoff target Session {} has invalid Project State identity: {error}",
                session.id
            )
        })?;
    let materializer_pid = std::process::id();
    let materializer_started_at = crate::process::host_process_start_time(materializer_pid)
        .ok_or_else(|| {
            "answered handoff materializer process identity is unavailable".to_string()
        })?;
    Ok(crate::autonomous_handoff::AutonomousHandoffDeliveryTarget {
        gwt_session_id: session.id.clone(),
        native_session_id: native_session_id.to_string(),
        provider: session.agent_id.to_string(),
        issue_number,
        repo_hash,
        project_state_root: project_state_root.to_string_lossy().into_owned(),
        window_id: window_id.to_string(),
        materializer_id: materializer_id.to_string(),
        materializer_pid,
        materializer_started_at,
        delivery_id: delivery_id.map(str::to_string),
    })
}

/// Issue #6096: top-level prefs keys that only record observations (control
/// receipt dedupe, scan heartbeat/driver). Launch admission, capacity and the
/// update drain never read them, yet the daemon rewrites them every few
/// seconds, so a metadata fence over them kept every prepared launch stale.
const ISSUE_MONITOR_PREFS_OBSERVATIONAL_KEYS: [&str; 4] = [
    "last_control_receipt",
    "last_scan_at",
    "last_scan_driver",
    // Claim readback evidence; never an arbitration input.
    "claim_diagnostics",
];

/// Issue #6096: per-record fields that every scan re-stamps without a
/// lifecycle change. `closure_records[].generation` advances on each
/// same-state re-observation; state and revision floor still fence.
const ISSUE_MONITOR_PREFS_OBSERVATIONAL_RECORD_FIELDS: [(&str, &str); 2] = [
    ("closure_records", "generation"),
    ("autonomous_records", "last_heartbeat"),
];

/// Content digest of Issue Monitor prefs bytes that ignores observation-only
/// churn. `open_pr_observations` keeps only each Issue's non-null `hold`
/// (launch admission reads the hold; `observed_at` only orders merges).
/// Unparsable bytes hash raw so any byte change still invalidates.
pub fn issue_monitor_prefs_content_digest(bytes: &[u8]) -> [u8; 32] {
    issue_monitor_prefs_digests(bytes).0
}

/// The content digest plus per top-level key digests of the admission
/// projection. The key digests only name, in diagnostics, which keys a stale
/// launch saw rewritten; the content digest is derived from them in one pass.
fn issue_monitor_prefs_digests(
    bytes: &[u8],
) -> ([u8; 32], std::collections::BTreeMap<String, [u8; 32]>) {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    let mut keys = std::collections::BTreeMap::new();
    match issue_monitor_prefs_admission_map(bytes) {
        Some(map) => {
            hasher.update(b"json:");
            for (key, value) in map {
                let digest: [u8; 32] =
                    Sha256::digest(serde_json::to_vec(&value).unwrap_or_default()).into();
                hasher.update(serde_json::to_vec(&key).unwrap_or_default());
                hasher.update(digest);
                keys.insert(key, digest);
            }
        }
        None => {
            hasher.update(b"raw:");
            hasher.update(bytes);
        }
    }
    (hasher.finalize().into(), keys)
}

/// The prefs object without observation-only churn, or `None` when the bytes
/// are not a JSON object.
fn issue_monitor_prefs_admission_map(
    bytes: &[u8],
) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Object(mut map)) => {
            for key in ISSUE_MONITOR_PREFS_OBSERVATIONAL_KEYS {
                map.remove(key);
            }
            for (key, field) in ISSUE_MONITOR_PREFS_OBSERVATIONAL_RECORD_FIELDS {
                if let Some(serde_json::Value::Array(records)) = map.get_mut(key) {
                    for record in records.iter_mut() {
                        if let serde_json::Value::Object(record) = record {
                            record.remove(field);
                        }
                    }
                }
            }
            if let Some(serde_json::Value::Object(reclaim)) = map.get_mut("generation_reclaim") {
                reclaim.remove("recorded_at");
            }
            match map.remove("open_pr_observations") {
                Some(serde_json::Value::Object(observations)) => {
                    let holds = observations
                        .into_iter()
                        .filter_map(|(issue, observation)| {
                            observation
                                .get("hold")
                                .filter(|hold| !hold.is_null())
                                .cloned()
                                .map(|hold| (issue, hold))
                        })
                        .collect::<serde_json::Map<_, _>>();
                    if !holds.is_empty() {
                        map.insert(
                            "open_pr_observations".to_string(),
                            serde_json::Value::Object(holds),
                        );
                    }
                }
                Some(other) => {
                    map.insert("open_pr_observations".to_string(), other);
                }
                None => {}
            }
            Some(map)
        }
        _ => None,
    }
}

/// Freshness fence for Issue Monitor prefs consumed by launch preparation.
/// Metadata is read before the content, so an unchanged metadata pair always
/// still describes the digested bytes; a metadata change falls back to the
/// content digest so observation-only rewrites keep prepared facts current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorPrefsStamp {
    metadata: (u64, std::time::SystemTime),
    content: [u8; 32],
    keys: std::collections::BTreeMap<String, [u8; 32]>,
}

impl IssueMonitorPrefsStamp {
    pub fn read(path: &Path) -> std::io::Result<Option<Self>> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => (metadata.len(), metadata.modified()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let (content, keys) = issue_monitor_prefs_digests(&bytes);
        Ok(Some(Self {
            metadata,
            content,
            keys,
        }))
    }

    /// Issue #6096 diagnostics: the admission-relevant top-level keys whose
    /// content differs between `expected` and the current file.
    pub fn changed_keys(expected: Option<&Self>, path: &Path) -> Vec<String> {
        let current = std::fs::read(path)
            .map(|bytes| issue_monitor_prefs_digests(&bytes).1)
            .unwrap_or_default();
        let empty = std::collections::BTreeMap::new();
        let expected = expected.map_or(&empty, |stamp| &stamp.keys);
        expected
            .keys()
            .chain(current.keys())
            .filter(|key| expected.get(*key) != current.get(*key))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// True when `path` still carries the same admission-relevant content.
    /// Unchanged metadata needs no read; otherwise compare the digest.
    pub fn matches_current(expected: Option<&Self>, path: &Path) -> bool {
        let metadata = std::fs::metadata(path);
        let Some(expected) = expected else {
            return matches!(
                metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            );
        };
        let unchanged_metadata = metadata.is_ok_and(|metadata| {
            metadata
                .modified()
                .is_ok_and(|modified| (metadata.len(), modified) == expected.metadata)
        });
        unchanged_metadata
            || std::fs::read(path)
                .is_ok_and(|bytes| issue_monitor_prefs_content_digest(&bytes) == expected.content)
    }
}

pub fn workspace_projection_owner_title(
    project_root: &Path,
    branch_name: Option<&str>,
) -> gwt_core::error::Result<Option<String>> {
    let Some(branch_name) = branch_name
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    else {
        return Ok(None);
    };
    let Some(projection) = gwt_core::workspace_projection::load_workspace_projection(project_root)?
    else {
        return Ok(None);
    };
    let projection_branch = projection
        .git_details
        .as_ref()
        .and_then(|details| details.branch.as_deref())
        .map(str::trim);
    if projection_branch != Some(branch_name) {
        return Ok(None);
    }
    Ok(projection
        .owner
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty()))
}

pub fn agent_launch_purpose_title(
    project_root: &Path,
    linked_issue_number: Option<u64>,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> gwt_core::error::Result<Option<String>> {
    let issue_title = linked_issue_number
        .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        .or_else(|| {
            linked_issue_number_for_branch(project_root, branch_name, issue_link_cache_dir)
                .and_then(|issue_number| issue_title_from_cache(project_root, issue_number))
        });
    match issue_title {
        Some(title) => Ok(Some(title)),
        None => workspace_projection_owner_title(project_root, branch_name),
    }
}

pub fn issue_title_from_cache(project_root: &Path, issue_number: u64) -> Option<String> {
    // #3426: use the canonical workspace-home-aware cache resolution (with the
    // detached fallback) so managed roots read the same cache as execution
    // owner-kind detection and title sync.
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
    let entry =
        gwt_github::Cache::new(cache_root).load_entry(gwt_github::IssueNumber(issue_number))?;
    let title = entry.snapshot.title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

fn linked_issue_number_for_branch(
    project_root: &Path,
    branch_name: Option<&str>,
    issue_link_cache_dir: &Path,
) -> Option<u64> {
    let branch_name = branch_name?.trim();
    if branch_name.is_empty() {
        return None;
    }
    let repo_hash = crate::index_worker::detect_repo_hash(project_root)?;
    let path = issue_link_cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let bytes = std::fs::read(path).ok()?;
    let store = serde_json::from_slice::<IssueBranchLinkStore>(&bytes).ok()?;
    store.branches.get(branch_name).copied()
}

#[derive(Debug, Clone)]
pub struct IssueMonitorSpawnAdmissionInput {
    pub project_root: PathBuf,
    pub host_project_roots: Vec<PathBuf>,
    pub issue_number: u64,
    pub target_branch: String,
    pub issue_link_cache_dir: PathBuf,
}

pub const ISSUE_MONITOR_SPAWN_ADMISSION_STALE: &str =
    "Issue Monitor spawn admission changed; prepare the launch again";

#[derive(Debug, Clone, PartialEq, Eq)]
enum IssueMonitorAdmissionFileVersion {
    Metadata(Option<(std::time::SystemTime, u64)>),
    /// Issue #6096: Issue Monitor prefs are fenced by admission-relevant
    /// content, because the daemon rewrites observation-only keys every few
    /// seconds on every host project.
    IssueMonitorPrefs(Option<IssueMonitorPrefsStamp>),
    /// Issue #6096: the safety projection of a prefs file. Another host
    /// project's prefs only feed the host-wide update drain hold, so its scans,
    /// claims and launches must not invalidate this admission. The target
    /// project's projection adds the launched Issue's PR hold, so both stay
    /// strict fences after a decision re-evaluation stops fencing the rest.
    PrefsFence {
        issue_number: Option<u64>,
        digest: Option<[u8; 32]>,
    },
    /// Issue #6096: cached Issue evidence. A cache sync rewrites every entry
    /// with a fresh `generation` token; only the evidence content fences.
    IssueCache(Option<[u8; 32]>),
}

/// How a dependency fences a prepared admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueMonitorAdmissionDependencyRole {
    /// Any change refuses the admission and needs a fresh preparation.
    Strict,
    /// The target project's prefs: a change asks the worker to re-evaluate
    /// the admission decision instead of restarting the whole launch.
    TargetPrefs,
    /// The shared machine capacity sample, rewritten every few seconds.
    /// A change is compared by the slot limit it grants, not its bytes.
    Capacity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IssueMonitorAdmissionFileStamp {
    path: PathBuf,
    role: IssueMonitorAdmissionDependencyRole,
    version: IssueMonitorAdmissionFileVersion,
}

pub fn issue_monitor_prefs_fence_digest(
    path: &Path,
    issue_number: Option<u64>,
) -> std::io::Result<Option<[u8; 32]>> {
    use sha2::{Digest as _, Sha256};
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hasher = Sha256::new();
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(map)) => {
            hasher.update(b"drain:");
            hasher.update(
                serde_json::to_vec(map.get("update_drain").unwrap_or(&serde_json::Value::Null))
                    .unwrap_or_default(),
            );
            if let Some(issue_number) = issue_number {
                let hold = map
                    .get("open_pr_observations")
                    .and_then(|observations| observations.get(issue_number.to_string()))
                    .and_then(|observation| observation.get("hold"))
                    .unwrap_or(&serde_json::Value::Null);
                hasher.update(b"hold:");
                hasher.update(serde_json::to_vec(hold).unwrap_or_default());
                // Freeze this Issue's selected member across a queued refresh.
                // A delivery pin survives consumption of the future reservation.
                let pinned = map
                    .get("pending_launch_deliveries")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|entries| {
                        entries.iter().find(|entry| {
                            entry
                                .get("issue_number")
                                .and_then(serde_json::Value::as_u64)
                                == Some(issue_number)
                        })
                    })
                    .and_then(|entry| entry.get("agent_member_id"))
                    .filter(|id| id.is_string());
                let planned = map
                    .get("agent_issue_schedule")
                    .and_then(|schedule| schedule.get("reservations"))
                    .and_then(serde_json::Value::as_array)
                    .and_then(|entries| {
                        entries.iter().find(|entry| {
                            entry
                                .get("issue_number")
                                .and_then(serde_json::Value::as_u64)
                                == Some(issue_number)
                        })
                    });
                let member = match pinned {
                    Some(member) => serde_json::json!({"member":member,"host":crate::process::current_hostname()}),
                    None => planned.map(|entry| serde_json::json!({"member":entry.get("agent_member_id"),"host":entry.get("host_id")})).unwrap_or(serde_json::Value::Null),
                };
                hasher.update(b"member:");
                hasher.update(serde_json::to_vec(&member).unwrap_or_default());
            }
        }
        // Unparsable bytes fail closed: any byte change invalidates.
        _ => {
            hasher.update(b"raw:");
            hasher.update(&bytes);
        }
    }
    Ok(Some(hasher.finalize().into()))
}

fn issue_cache_evidence_digest(path: &Path) -> std::io::Result<Option<[u8; 32]>> {
    use sha2::{Digest as _, Sha256};
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut hasher = Sha256::new();
    let meta = path.file_name().is_some_and(|name| name == "meta.json");
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(mut map)) if meta => {
            map.remove("generation");
            hasher.update(b"meta:");
            hasher.update(serde_json::to_vec(&serde_json::Value::Object(map)).unwrap_or_default());
        }
        _ => {
            hasher.update(b"raw:");
            hasher.update(&bytes);
        }
    }
    Ok(Some(hasher.finalize().into()))
}

impl IssueMonitorAdmissionFileStamp {
    fn read_issue_cache(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            role: IssueMonitorAdmissionDependencyRole::Strict,
            version: IssueMonitorAdmissionFileVersion::IssueCache(issue_cache_evidence_digest(
                path,
            )?),
        })
    }

    fn read_prefs_fence(path: &Path, issue_number: Option<u64>) -> std::io::Result<Self> {
        Ok(Self {
            path: path.to_path_buf(),
            role: IssueMonitorAdmissionDependencyRole::Strict,
            version: IssueMonitorAdmissionFileVersion::PrefsFence {
                issue_number,
                digest: issue_monitor_prefs_fence_digest(path, issue_number)?,
            },
        })
    }

    fn read_with_role(
        path: &Path,
        role: IssueMonitorAdmissionDependencyRole,
    ) -> std::io::Result<Self> {
        let version = if is_issue_monitor_prefs_path(path)
            && role == IssueMonitorAdmissionDependencyRole::TargetPrefs
        {
            IssueMonitorAdmissionFileVersion::IssueMonitorPrefs(IssueMonitorPrefsStamp::read(path)?)
        } else {
            IssueMonitorAdmissionFileVersion::Metadata(match std::fs::metadata(path) {
                Ok(metadata) => Some((metadata.modified()?, metadata.len())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            })
        };
        Ok(Self {
            path: path.to_path_buf(),
            role,
            version,
        })
    }

    pub(super) fn is_current(&self) -> bool {
        match &self.version {
            IssueMonitorAdmissionFileVersion::IssueMonitorPrefs(expected) => {
                IssueMonitorPrefsStamp::matches_current(expected.as_ref(), &self.path)
            }
            IssueMonitorAdmissionFileVersion::PrefsFence {
                issue_number,
                digest,
            } => issue_monitor_prefs_fence_digest(&self.path, *issue_number)
                .is_ok_and(|current| current == *digest),
            IssueMonitorAdmissionFileVersion::IssueCache(expected) => {
                issue_cache_evidence_digest(&self.path).is_ok_and(|current| current == *expected)
            }
            IssueMonitorAdmissionFileVersion::Metadata(_) => {
                Self::read_with_role(&self.path, self.role).is_ok_and(|current| current == *self)
            }
        }
    }
}

/// Issue #6096: the outcome the GUI spawn admission derives from a prepared
/// Monitor state. A re-evaluation that reaches the same outcome keeps the
/// prepared launch, however often unrelated prefs records were rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueMonitorSpawnAdmissionDecision {
    update_drain: bool,
    source_exclusion: Option<String>,
    dependency_exclusion: Option<String>,
    wait_in_force: bool,
    monitor_pane: bool,
    implementation_capacity: bool,
    review_capacity: bool,
}

impl IssueMonitorSpawnAdmissionDecision {
    fn evaluate(monitor: &crate::IssueMonitorState, issue_number: u64) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            update_drain: monitor.update_drain().is_some(),
            source_exclusion: monitor.launch_source_exclusion(issue_number),
            dependency_exclusion: monitor.launch_dependency_exclusion(issue_number),
            wait_in_force: monitor
                .autonomous_wait(issue_number)
                .is_some_and(|wait| crate::issue_monitor::wait_declaration_in_force(wait, &now)),
            monitor_pane: monitor.has_monitor_pane_for_issue(issue_number),
            implementation_capacity: monitor.has_capacity_for_monitor_spawn(issue_number, false),
            review_capacity: monitor.has_capacity_for_monitor_spawn(issue_number, true),
        }
    }
}

fn is_issue_monitor_prefs_path(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == "issue-monitor.json")
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == "project-state")
}

#[derive(Debug, Clone)]
pub struct PreparedIssueMonitorSpawnAdmission {
    pub project_root: PathBuf,
    pub issue_number: u64,
    pub monitor: crate::IssueMonitorState,
    pub host_started_at: Option<u64>,
    pub purpose_title: Option<String>,
    host_project_roots: std::collections::BTreeSet<PathBuf>,
    dependencies: Vec<IssueMonitorAdmissionFileStamp>,
    capacity_expires_at: Option<u64>,
    /// Slot limit granted by the capacity sample this admission evaluated.
    capacity_limit: usize,
    decision: IssueMonitorSpawnAdmissionDecision,
    /// Issue #6096: set once the worker re-evaluated this admission and
    /// reached the original decision. The target prefs then stop fencing it:
    /// a busy project rewrites them faster than any host can re-evaluate.
    decision_revalidated: bool,
}

impl PreparedIssueMonitorSpawnAdmission {
    fn dependencies_with_role(
        &self,
        role: IssueMonitorAdmissionDependencyRole,
    ) -> impl Iterator<Item = &IssueMonitorAdmissionFileStamp> {
        self.dependencies
            .iter()
            .filter(move |dependency| dependency.role == role)
    }

    /// True while a worker re-evaluation can keep this launch: the host set and
    /// every strict source (update manifest/payload, machine policy, other
    /// projects' update drains, Issue cache evidence) are unchanged. Target
    /// prefs and capacity churn are decided by the re-evaluation itself.
    pub fn can_revalidate(&self, host_project_roots: &[PathBuf]) -> bool {
        self.host_project_roots == host_project_roots.iter().cloned().collect()
            && self
                .dependencies_with_role(IssueMonitorAdmissionDependencyRole::Strict)
                .all(IssueMonitorAdmissionFileStamp::is_current)
    }

    /// Issue #6096: a capacity resample keeps the admission while the slot
    /// limit it grants is not lower. Only an unchanged, unexpired sample is
    /// trusted without re-deriving that limit.
    fn capacity_admits(&self) -> bool {
        let sample_current = self.capacity_expires_at.is_none_or(|expires| {
            chrono::Utc::now().timestamp() < i64::try_from(expires).unwrap_or(i64::MAX)
        }) && self
            .dependencies_with_role(IssueMonitorAdmissionDependencyRole::Capacity)
            .all(IssueMonitorAdmissionFileStamp::is_current);
        sample_current
            || self.monitor.project_capacity_limit_now(&self.project_root) >= self.capacity_limit
    }

    /// Metadata checks preserve worker admission when another project or the
    /// shared machine observation changes before the GUI creates its pane.
    pub fn is_current(&self, host_project_roots: &[PathBuf]) -> bool {
        self.can_revalidate(host_project_roots)
            && (self.decision_revalidated
                || self
                    .dependencies_with_role(IssueMonitorAdmissionDependencyRole::TargetPrefs)
                    .all(IssueMonitorAdmissionFileStamp::is_current))
            && self.capacity_admits()
    }
}

impl IssueMonitorSpawnAdmissionInput {
    pub fn prepare(&self) -> Result<PreparedIssueMonitorSpawnAdmission, String> {
        self.prepare_with_identity(None)
    }

    pub fn refresh(
        &self,
        previous: &PreparedIssueMonitorSpawnAdmission,
    ) -> Result<PreparedIssueMonitorSpawnAdmission, String> {
        let mut refreshed = self.prepare_with_identity(Some(previous))?;
        // Keep the original source/policy fence across the queued refresh too.
        // Issue #6096: the target prefs and capacity sample may have moved
        // while this ran; the launch survives only if the decision did not.
        if !previous.can_revalidate(&self.host_project_roots)
            || refreshed.decision != previous.decision
        {
            return Err(ISSUE_MONITOR_SPAWN_ADMISSION_STALE.to_string());
        }
        refreshed.decision_revalidated = true;
        Ok(refreshed)
    }

    fn prepare_with_identity(
        &self,
        previous: Option<&PreparedIssueMonitorSpawnAdmission>,
    ) -> Result<PreparedIssueMonitorSpawnAdmission, String> {
        let mut phase_started = std::time::Instant::now();
        let mut paths = self
            .host_project_roots
            .iter()
            .chain(std::iter::once(&self.project_root))
            .map(|root| crate::issue_monitor_prefs_path_for_repo_path(root))
            .collect::<std::collections::BTreeSet<_>>();
        paths.insert(gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json"));
        paths.insert(crate::machine_agent_settings::path());
        let pending_manifest = gwt_core::update::pending_update_manifest_path();
        paths.insert(pending_manifest.clone());
        // Manifest admission also depends on its payload's existence. Resolve
        // that path on the worker, even if the staged payload is not present yet.
        if let Some(manifest) = std::fs::read(&pending_manifest).ok().and_then(|bytes| {
            serde_json::from_slice::<gwt_core::update::PendingUpdateManifest>(&bytes).ok()
        }) {
            let payload_path = match manifest.payload {
                gwt_core::update::PreparedPayload::PortableBinary { path }
                | gwt_core::update::PreparedPayload::Installer { path, .. } => path,
            };
            paths.insert(payload_path);
        }
        let target_prefs = crate::issue_monitor_prefs_path_for_repo_path(&self.project_root);
        let capacity_path = gwt_core::paths::gwt_home().join("machine-state/agent-capacity.json");
        let mut dependencies = paths
            .iter()
            .flat_map(|path| {
                if *path == target_prefs {
                    vec![
                        IssueMonitorAdmissionFileStamp::read_with_role(
                            path,
                            IssueMonitorAdmissionDependencyRole::TargetPrefs,
                        ),
                        IssueMonitorAdmissionFileStamp::read_prefs_fence(
                            path,
                            Some(self.issue_number),
                        ),
                    ]
                } else if is_issue_monitor_prefs_path(path) {
                    vec![IssueMonitorAdmissionFileStamp::read_prefs_fence(path, None)]
                } else if *path == capacity_path {
                    vec![IssueMonitorAdmissionFileStamp::read_with_role(
                        path,
                        IssueMonitorAdmissionDependencyRole::Capacity,
                    )]
                } else {
                    vec![IssueMonitorAdmissionFileStamp::read_with_role(
                        path,
                        IssueMonitorAdmissionDependencyRole::Strict,
                    )]
                }
            })
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| format!("Cannot inspect Monitor spawn admission: {error}"))?;
        tracing::info!(
            target: "gwt.launch.timing",
            phase = "monitor_admission_metadata",
            elapsed_ms = phase_started.elapsed().as_secs_f64() * 1000.0,
            thread = "worker",
            "Issue Monitor launch phase"
        );
        phase_started = std::time::Instant::now();
        let hold = prepare_host_update_drain_launch_hold(&self.host_project_roots)?;
        tracing::info!(
            target: "gwt.launch.timing",
            phase = "monitor_host_drain",
            elapsed_ms = phase_started.elapsed().as_secs_f64() * 1000.0,
            thread = "worker",
            "Issue Monitor launch phase"
        );
        phase_started = std::time::Instant::now();
        if let Some(hold) = hold {
            return Err(hold);
        }
        let monitor = prepare_issue_monitor_spawn_capacity(
            &self.project_root,
            self.issue_number,
            &mut dependencies,
        )?;
        tracing::info!(
            target: "gwt.launch.timing",
            phase = "monitor_shared_capacity",
            elapsed_ms = phase_started.elapsed().as_secs_f64() * 1000.0,
            thread = "worker",
            "Issue Monitor launch phase"
        );
        let capacity = monitor
            .agent_status_at(&chrono::Utc::now().to_rfc3339())
            .agent_capacity;
        let capacity_expires_at = capacity.is_fresh().then_some(capacity.expires_at).flatten();
        let capacity_limit = monitor.effective_max_active_agents();
        let decision = IssueMonitorSpawnAdmissionDecision::evaluate(&monitor, self.issue_number);
        // Preparation is read-only: adoption and stale results must not consume
        // a member. The final materializer reserves after its admission fences.
        let purpose_title = if let Some(previous) = previous {
            previous.purpose_title.clone()
        } else {
            agent_launch_purpose_title(
                &self.project_root,
                Some(self.issue_number),
                Some(&self.target_branch),
                &self.issue_link_cache_dir,
            )
            .map_err(|error| error.to_string())?
        };
        // The GUI checks is_current before applying this result and again at
        // pane creation. Keep stale observations available for a narrow refresh
        // instead of discarding their already prepared identity and launch facts.
        Ok(PreparedIssueMonitorSpawnAdmission {
            project_root: self.project_root.clone(),
            issue_number: self.issue_number,
            monitor,
            host_started_at: crate::process::host_process_start_time(std::process::id()),
            purpose_title,
            host_project_roots: self.host_project_roots.iter().cloned().collect(),
            dependencies,
            capacity_expires_at,
            capacity_limit,
            decision,
            decision_revalidated: false,
        })
    }
}

pub(super) fn prepare_issue_monitor_spawn_capacity(
    project_root: &Path,
    issue_number: u64,
    dependencies: &mut Vec<IssueMonitorAdmissionFileStamp>,
) -> Result<crate::IssueMonitorState, String> {
    let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
        project_root,
    ))
    .map_err(|error| format!("Monitor capacity read failed: {error}"))?;
    let mut monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
    crate::issue_monitor_worker::bind_issue_monitor_source_repository(&mut monitor, project_root);
    if let Some(cache_root) = crate::issue_cache::issue_cache_root_for_repo_path(project_root) {
        let issues = crate::issue_monitor_worker::load_cached_issue_monitor_dependency_candidates(
            &cache_root,
            issue_number,
            |number| {
                let source = cache_root.join(number.to_string());
                for path in [source.join("meta.json"), source.join("body.md")] {
                    dependencies.push(
                        IssueMonitorAdmissionFileStamp::read_issue_cache(&path).map_err(
                            |error| format!("Cannot inspect Monitor spawn admission: {error}"),
                        )?,
                    );
                }
                // Cache::load_entry hydrates only metadata-declared comments.
                // Stamp missing declarations too: their later arrival changes
                // SPEC readiness and the composed dependency source.
                if let Some(meta) = std::fs::read(source.join("meta.json"))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<gwt_github::CacheMeta>(&bytes).ok())
                {
                    for id in meta.comment_ids {
                        let path = source.join("comments").join(format!("{id}.md"));
                        dependencies.push(
                            IssueMonitorAdmissionFileStamp::read_issue_cache(&path).map_err(
                                |error| format!("Cannot inspect Monitor spawn admission: {error}"),
                            )?,
                        );
                    }
                }
                Ok(())
            },
        )?;
        monitor.observe_dependency_candidates(&issues);
        if let Some(issue) = issues
            .into_iter()
            .find(|issue| issue.number == issue_number)
        {
            monitor.record_candidate(issue);
        }
    }
    monitor.refresh_agent_capacity(project_root);
    Ok(monitor)
}

pub fn prepare_host_update_drain_launch_hold(
    project_roots: &[PathBuf],
) -> Result<Option<String>, String> {
    let pending = gwt_core::update::load_pending_update_manifest().filter(|manifest| {
        !manifest.auto_apply_deferred
            && gwt_core::update::pending_version_is_newer(
                &manifest.version,
                env!("CARGO_PKG_VERSION"),
            )
    });
    let mut hold = None;
    for project_root in project_roots {
        let prefs = crate::issue_monitor::load_issue_monitor_prefs_for_running_version(
            &crate::issue_monitor_prefs_path_for_repo_path(project_root),
        )
        .map_err(|error| format!("Cannot check host update_drain admission: {error}"))?;
        if let Some(drain) = prefs.update_drain.filter(|drain| {
            pending
                .as_ref()
                .is_some_and(|manifest| manifest.version == drain.version)
        }) {
            hold.get_or_insert_with(|| {
                format!(
                    "Host update_drain for v{} in {} holds new agent launches; release.update.defer {{\"version\":\"{}\"}} (host-wide)",
                    drain.version,
                    project_root.display(),
                    drain.version,
                )
            });
        }
    }
    Ok(hold)
}

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! { static LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(any(test, feature = "test-gh-guard"))]
pub fn reset_local_issue_monitor_fallback_commit_count() {
    LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.set(0);
}
#[cfg(any(test, feature = "test-gh-guard"))]
pub fn local_issue_monitor_fallback_commit_count() -> usize {
    LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.get()
}
pub fn local_issue_monitor_fallback_projection_timeout() -> std::time::Duration {
    #[cfg(any(test, feature = "test-gh-guard"))]
    if let Some(timeout) = std::env::var_os("GWT_TEST_ISSUE_MONITOR_FALLBACK_PROJECTION_TIMEOUT_MS")
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .filter(|timeout| *timeout <= 60_000)
    {
        return std::time::Duration::from_millis(timeout);
    }
    // Functional launch tests share the fallback commit's allowance for native
    // Git/fsync latency. Explicit budget tests keep the override above.
    std::time::Duration::from_secs(if cfg!(any(test, feature = "test-gh-guard")) {
        10
    } else {
        1
    })
}

#[derive(Debug, Clone)]
pub struct IssueMonitorLaunchClaimInput {
    pub project_root: PathBuf,
    pub issue_number: u64,
    pub delivery_id: String,
    pub materializer_window_id: String,
    pub materializer_id: String,
    pub materializer_pid: u32,
    pub fallback_timeout: std::time::Duration,
    pub snapshot: Option<crate::IssueMonitorWindowSnapshot>,
    pub snapshot_tabs: std::collections::BTreeSet<String>,
}

impl IssueMonitorLaunchClaimInput {
    pub fn claim(
        &self,
    ) -> Result<bool, crate::runtime_daemon_events::IssueMonitorControlPublishError> {
        self.claim_with_publishers(
            |snapshot, tabs| {
                crate::daemon_publisher::publish_issue_monitor_window_snapshot(
                    &self.project_root,
                    snapshot,
                    tabs,
                )
            },
            |payload| {
                let payload = crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    payload,
                    self.materializer_pid,
                );
                crate::daemon_publisher::publish_issue_monitor_control(&self.project_root, payload)
            },
        )
    }

    /// Claim from a launch-preparation worker. The worker is off the GUI
    /// event loop, so it waits for the daemon's serialized prefs commit
    /// instead of the 200ms GUI budget: a busy daemon scan otherwise turns
    /// every claim into an uncertain outcome and the launch never creates a
    /// window (Issue #6096).
    pub fn claim_from_worker(
        &self,
    ) -> Result<bool, crate::runtime_daemon_events::IssueMonitorControlPublishError> {
        self.claim_with_publishers(
            |snapshot, tabs| {
                crate::daemon_publisher::publish_issue_monitor_window_snapshot_with_timeout(
                    &self.project_root,
                    snapshot,
                    tabs,
                    std::time::Duration::from_secs(5),
                )
            },
            |payload| {
                let payload = crate::runtime_daemon_events::issue_monitor_payload(
                    "control",
                    payload,
                    self.materializer_pid,
                );
                crate::daemon_publisher::publish_durable_issue_monitor_control(
                    &self.project_root,
                    payload,
                )
            },
        )
    }

    /// Reuse an exact durable reservation after checking it against the latest
    /// prefs and this retry's current canvas. Re-publishing an unchanged claim
    /// would invalidate other launches preparing against the same prefs file.
    pub fn claim_or_reuse(
        &self,
    ) -> Result<bool, crate::runtime_daemon_events::IssueMonitorControlPublishError> {
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&self.project_root),
        )
        .map_err(|error| {
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(format!(
                "launch delivery capacity read failed: {error}"
            ))
        })?;
        if self.matches_durable_claim(&prefs)
            && self.preflight_claim(
                prefs,
                crate::process::host_process_start_time(self.materializer_pid),
            )
        {
            return Ok(true);
        }
        self.claim_from_worker()
    }

    fn matches_durable_claim(&self, prefs: &crate::IssueMonitorPrefs) -> bool {
        prefs.pending_launch_deliveries.iter().any(|delivery| {
            delivery.issue_number == self.issue_number
                && delivery.delivery_id == self.delivery_id
                && delivery.materializer_id.as_deref() == Some(self.materializer_id.as_str())
                && delivery.materializer_pid == Some(self.materializer_pid)
                && delivery.materializer_window_id.as_deref()
                    == Some(self.materializer_window_id.as_str())
        })
    }

    fn observe_windows(
        &self,
        monitor: &mut crate::IssueMonitorState,
        host_started_at: Option<u64>,
    ) {
        if let Some(snapshot) = self.snapshot.as_ref() {
            if let Some(started) = host_started_at {
                monitor.record_window_snapshot_from_host(
                    snapshot.clone(),
                    self.materializer_pid,
                    started,
                    self.snapshot_tabs.clone(),
                );
            } else {
                monitor
                    .record_window_snapshot_for_tabs(snapshot.clone(), self.snapshot_tabs.clone());
            }
        }
    }

    #[cfg(any(test, feature = "test-gh-guard"))]
    #[doc(hidden)]
    pub fn test_preflight_claim(
        &self,
        prefs: crate::IssueMonitorPrefs,
        host_started_at: Option<u64>,
    ) -> bool {
        self.preflight_claim(prefs, host_started_at)
    }

    fn preflight_claim(
        &self,
        prefs: crate::IssueMonitorPrefs,
        host_started_at: Option<u64>,
    ) -> bool {
        let mut preflight =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        crate::issue_monitor_worker::bind_issue_monitor_source_repository(
            &mut preflight,
            &self.project_root,
        );
        if let Some(issue) = crate::issue_cache::issue_cache_root_for_repo_path(&self.project_root)
            .and_then(|cache_root| {
                crate::issue_monitor_worker::load_cached_issue_monitor_candidate(
                    &cache_root,
                    self.issue_number,
                )
            })
        {
            preflight.record_candidate(issue);
        }
        self.observe_windows(&mut preflight, host_started_at);
        preflight.claim_launch_delivery(
            self.issue_number,
            &self.delivery_id,
            &self.materializer_id,
            self.materializer_pid,
            &self.materializer_window_id,
            crate::process::is_host_process_alive,
        )
    }

    pub fn claim_with_publishers(
        &self,
        publish_snapshot: impl FnOnce(
            &crate::IssueMonitorWindowSnapshot,
            &std::collections::BTreeSet<String>,
        ) -> Result<
            (),
            crate::runtime_daemon_events::IssueMonitorControlPublishError,
        >,
        publish_claim: impl FnOnce(
            serde_json::Value,
        ) -> Result<
            (),
            crate::runtime_daemon_events::IssueMonitorControlPublishError,
        >,
    ) -> Result<bool, crate::runtime_daemon_events::IssueMonitorControlPublishError> {
        let project_root = &self.project_root;
        let issue_number = self.issue_number;
        let delivery_id = self.delivery_id.as_str();
        let materializer_window_id = self.materializer_window_id.as_str();
        let materializer_id = self.materializer_id.clone();
        let materializer_pid = self.materializer_pid;
        let snapshot = self.snapshot.clone();
        let snapshot_tabs = self.snapshot_tabs.clone();
        let host_started_at = crate::process::host_process_start_time(materializer_pid);
        // A rejected idempotent claim can still be present in prefs. Validate
        // against the current canvas before trusting the daemon readback.
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(project_root),
        )
        .map_err(|error| {
            crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(format!(
                "launch delivery capacity read failed: {error}"
            ))
        })?;
        if !self.preflight_claim(prefs, host_started_at) {
            return Ok(false);
        }
        // The daemon must judge this claim against the same current canvas.
        // Waiting for its periodic scan can strand an existing-pane adoption
        // at capacity even though this GUI's preflight accepted it.
        if let Some(snapshot) = snapshot.as_ref() {
            match publish_snapshot(snapshot, &snapshot_tabs) {
                Ok(()) => {}
                Err(error) if error.allows_local_fallback() => {}
                Err(error) => return Err(error),
            }
        }
        let publication = publish_claim(serde_json::json!({
            "claim_launch_delivery": {
                "issue_number": issue_number,
                "delivery_id": delivery_id,
                "materializer_id": materializer_id.clone(),
                "materializer_pid": materializer_pid,
                "materializer_window_id": materializer_window_id,
            }
        }));
        match publication {
            outcome @ (Ok(())
            | Err(
                crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(_),
            )) => {
                let prefs = crate::load_issue_monitor_prefs(
                    &crate::issue_monitor_prefs_path_for_repo_path(project_root),
                )
                .map_err(|error| {
                    crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                        format!("launch delivery claim readback failed: {error}"),
                    )
                })?;
                // A receipt timeout can follow a durable claim. Only the
                // exact tuple proves ownership; absence remains uncertain.
                if self.matches_durable_claim(&prefs) {
                    Ok(true)
                } else {
                    outcome.map(|()| false)
                }
            }
            Err(error) if error.allows_local_fallback() => {
                let source = crate::issue_cache::issue_cache_root_for_repo_path(project_root)
                    .and_then(|cache_root| {
                        crate::issue_monitor_worker::load_cached_issue_monitor_candidate(
                            &cache_root,
                            issue_number,
                        )
                    });
                commit_local_issue_monitor_control_with_timeout(
                    project_root,
                    self.fallback_timeout,
                    |monitor| {
                        if let Some(source) = source {
                            monitor.record_candidate(source);
                        }
                        self.observe_windows(monitor, host_started_at);
                        monitor.claim_launch_delivery(
                            issue_number,
                            delivery_id,
                            &materializer_id,
                            materializer_pid,
                            materializer_window_id,
                            crate::process::is_host_process_alive,
                        )
                    },
                )
                .map(|(monitor, accepted)| {
                    accepted
                        && monitor.pending_launch_delivery_id(issue_number).as_deref()
                            == Some(delivery_id)
                })
            }
            Err(crate::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(_)) => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

pub type LocalIssueMonitorFallbackProjection = (
    Vec<crate::IssueMonitorIssue>,
    Option<String>,
    String,
    Option<(String, String)>,
);

pub fn commit_local_issue_monitor_control_with_timeout<T>(
    project_root: &Path,
    fallback_timeout: std::time::Duration,
    mutation: impl FnOnce(&mut crate::IssueMonitorState) -> T,
) -> Result<
    (crate::IssueMonitorState, T),
    crate::runtime_daemon_events::IssueMonitorControlPublishError,
> {
    try_commit_local_issue_monitor_control_with_timeout(project_root, fallback_timeout, |monitor| {
        Ok(mutation(monitor))
    })
}

pub fn try_commit_local_issue_monitor_control_with_timeout<T>(
    project_root: &Path,
    fallback_timeout: std::time::Duration,
    mutation: impl FnOnce(&mut crate::IssueMonitorState) -> std::io::Result<T>,
) -> Result<
    (crate::IssueMonitorState, T),
    crate::runtime_daemon_events::IssueMonitorControlPublishError,
> {
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project_root);
    let (cached_issues, projection_error, now, source_repository) =
        load_local_issue_monitor_fallback_projection(project_root);
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + fallback_timeout,
    );
    crate::try_mutate_issue_monitor_prefs_without_authority_fence(&prefs_path, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        monitor.set_source_repository(
            source_repository
                .as_ref()
                .map(|(owner, repo)| (owner.as_str(), repo.as_str())),
        );
        monitor.refresh_agent_capacity(project_root);
        let result = mutation(&mut monitor)?;
        apply_local_issue_monitor_fallback_projection(
            &mut monitor,
            &cached_issues,
            projection_error.as_deref(),
            &now,
        );
        monitor.synchronize_agent_roster_capacity(&prefs_path)?;
        *prefs = monitor.prefs();
        Ok((monitor, result))
    })
    .map(|(_, committed)| {
        #[cfg(any(test, feature = "test-gh-guard"))]
        LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.set(LOCAL_ISSUE_MONITOR_FALLBACK_COMMITS.get() + 1);
        committed
    })
    .map_err(|error| {
        crate::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(format!(
            "local fallback control commit failed: {error}"
        ))
    })
}

pub fn load_local_issue_monitor_fallback_projection(
    project_root: &Path,
) -> LocalIssueMonitorFallbackProjection {
    let _cache_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + local_issue_monitor_fallback_projection_timeout(),
    );
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root);
    let (cached_issues, cache_error) =
        match crate::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root) {
            Ok(issues) => (issues, None),
            Err(error) => (Vec::new(), Some(format!("issue cache failed: {error}"))),
        };
    let source_repository = crate::issue_monitor_worker::github_remote_owner_and_repo(project_root);
    let origin_error = source_repository.as_ref().err().map(ToString::to_string);
    (
        cached_issues,
        cache_error.or(origin_error),
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        source_repository.ok(),
    )
}

pub fn apply_local_issue_monitor_fallback_projection(
    monitor: &mut crate::IssueMonitorState,
    cached_issues: &[crate::IssueMonitorIssue],
    projection_error: Option<&str>,
    now: &str,
) {
    monitor.expire_stale_unbound_launches(now);
    crate::scan_issue_monitor_candidates(monitor, cached_issues, now);
    if monitor.config.enabled {
        monitor.set_gui_connected(true);
    }
    if let Some(error) = projection_error {
        monitor.record_scan_error(now, error);
    }
}

/// Current facts captured by the one physical owner, never supplied as a claim over IPC.
pub(crate) struct OwnerAgentAdmissionInput<'a> {
    pub project_root: &'a Path,
    pub sessions_dir: &'a Path,
    pub issue_link_cache_dir: &'a Path,
    pub window_id: &'a str,
    pub materializer_id: &'a str,
    pub host_project_roots: &'a [PathBuf],
    pub snapshot: crate::IssueMonitorWindowSnapshot,
    pub snapshot_tabs: std::collections::BTreeSet<String>,
    pub known_names: &'a std::collections::HashSet<String>,
    pub metadata: OwnerLaunchMetadata,
    pub is_continue_work: bool,
    pub restore_source: Option<&'a gwt_agent::Session>,
}
pub(crate) struct PreparedOwnerAgentAdmission {
    pub workspace_resume_context: Option<WorkspaceResumeContext>,
    pub launch_feedback_context: Option<LaunchFeedbackContext>,
    pub prepared_manual_launch_claim: Option<gwt_agent::SessionActiveLaunchHandshake>,
    pub issue_monitor_trust_candidate: Option<super::launch::IssueMonitorTrustCandidate>,
    pub issue_monitor_launch: bool,
    pub purpose_title: Option<String>,
}
pub(crate) fn prepare_owner_agent_admission(
    input: OwnerAgentAdmissionInput<'_>,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<PreparedOwnerAgentAdmission, String> {
    // Manual preparation has already validated this immutable source and
    // converted the intent to Prepared. Reuse its name exclusion without
    // changing Fresh launch/restore semantics on config.predecessor_session_id.
    let name_predecessor_session_id = config
        .predecessor_session_id
        .clone()
        .or_else(|| input.restore_source.map(|source| source.id.clone()))
        .or_else(|| {
            matches!(
                config.execution_intent,
                gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(_)
            )
            .then(|| {
                input
                    .metadata
                    .manual_successor
                    .as_ref()
                    .and_then(|choice| choice.expected_predecessor.as_deref())
                    .map(|source| source.session_id.clone())
            })
            .flatten()
        });
    let OwnerLaunchMetadata {
        selected_agent_member_id,
        inherited_agent_name,
        workspace_resume_context,
        mut launch_feedback_context,
        monitor_requested_tier,
        deliver_pending_autonomous_answer,
        ..
    } = input.metadata;
    let is_review = config
        .env_vars
        .get("GWT_REVIEW_DISPATCH")
        .is_some_and(|value| value == "1");
    let workspace_resume_context = if is_review {
        None
    } else {
        workspace_resume_context
    };
    let issue_monitor_issue_number = launch_feedback_context
        .as_ref()
        .and_then(|feedback| feedback.issue_monitor_issue_number);
    let issue_monitor_trust_candidate = super::launch::issue_monitor_trust_candidate_from_feedback(
        &config.agent_id,
        launch_feedback_context.as_ref(),
    )?;
    let mut prepared = if let Some(issue_number) = issue_monitor_issue_number {
        Some(
            IssueMonitorSpawnAdmissionInput {
                project_root: input.project_root.into(),
                host_project_roots: input.host_project_roots.to_vec(),
                issue_number,
                target_branch: config.branch.clone().unwrap_or_default(),
                issue_link_cache_dir: input.issue_link_cache_dir.into(),
            }
            .prepare()?,
        )
    } else {
        None
    };
    if !config.suppress_execution_control && prepared.is_none() {
        if let Some(hold) = prepare_host_update_drain_launch_hold(input.host_project_roots)? {
            return Err(hold);
        }
    }
    let roster_spec_number = config.linked_issue_number.and_then(|number| {
        prepared
            .as_ref()
            .and_then(|prepared| {
                prepared.monitor.inbox_item(number).and_then(|item| {
                    if item.issue.labels.iter().any(|label| label == "gwt-spec") {
                        Some(number)
                    } else {
                        item.issue
                            .body
                            .as_deref()
                            .and_then(crate::knowledge_bridge::parent_spec_from_body)
                    }
                })
            })
            .or_else(|| {
                crate::issue_monitor_worker::cached_agent_member_spec_owner(
                    input.project_root,
                    number,
                )
            })
    });
    if let Some(issue_number) = issue_monitor_issue_number {
        let review_dispatch = launch_feedback_context
            .as_ref()
            .is_some_and(|feedback| feedback.issue_monitor_review_dispatch);
        let prepared = prepared.as_mut().expect("Monitor admission");
        if !prepared.is_current(input.host_project_roots) {
            return Err(ISSUE_MONITOR_SPAWN_ADMISSION_STALE.into());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let monitor = &mut prepared.monitor;
        if !review_dispatch
            && monitor
                .autonomous_wait(issue_number)
                .is_some_and(|wait| crate::issue_monitor::wait_declaration_in_force(wait, &now))
        {
            return Err(format!("Issue Monitor active wait protects Issue #{issue_number}; resume the waiting implementation before launching another producer"));
        }
        if let Some(started) = crate::process::host_process_start_time(std::process::id()) {
            monitor.record_window_snapshot_from_host(
                input.snapshot.clone(),
                std::process::id(),
                started,
                input.snapshot_tabs.clone(),
            );
        } else {
            monitor.record_window_snapshot_for_tabs(
                input.snapshot.clone(),
                input.snapshot_tabs.clone(),
            );
        }
        if !review_dispatch && monitor.has_monitor_pane_for_issue(issue_number) {
            return Err(format!(
                "Issue Monitor implementation pane already exists for Issue #{issue_number}"
            ));
        }
        if let Some(reason) = monitor.launch_source_exclusion(issue_number) {
            return Err(format!(
                "Issue Monitor source admission refused Issue #{issue_number}: {reason}"
            ));
        }
        if !review_dispatch {
            if let Some(reason) = monitor.launch_dependency_exclusion(issue_number) {
                return Err(format!(
                    "Issue Monitor dependency admission refused Issue #{issue_number}: {reason}"
                ));
            }
        }
        if !monitor.has_capacity_for_monitor_spawn(issue_number, review_dispatch) {
            let status = monitor.agent_status_at(&now);
            return Err(format!(
                "Issue Monitor max_active reached ({}/{}) before launching Issue #{issue_number}",
                status
                    .occupied_slot_count
                    .unwrap_or(status.active_launches.len()),
                status.max_active
            ));
        }
        if !review_dispatch {
            if let Some(delivery_id) = launch_feedback_context
                .as_ref()
                .and_then(|feedback| feedback.issue_monitor_delivery_id.as_ref())
            {
                let claim = IssueMonitorLaunchClaimInput {
                    project_root: input.project_root.into(),
                    issue_number,
                    delivery_id: delivery_id.clone(),
                    materializer_window_id: input.window_id.into(),
                    materializer_id: input.materializer_id.into(),
                    materializer_pid: std::process::id(),
                    fallback_timeout: std::time::Duration::from_secs(10),
                    snapshot: Some(input.snapshot),
                    snapshot_tabs: input.snapshot_tabs,
                };
                if !claim.claim_or_reuse().map_err(|error| error.to_string())? {
                    return Err(
                        "Issue Monitor launch delivery claim changed before owner materialization"
                            .into(),
                    );
                }
            }
        }
    }
    if let Some(issue_number) = issue_monitor_issue_number {
        let feedback = launch_feedback_context.as_mut().expect("Monitor feedback");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(input.project_root);
        if let Some(tier) =
            monitor_requested_tier.filter(|_| !feedback.issue_monitor_review_dispatch)
        {
            crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                let tiers = prefs.effective_launch_tiers();
                let eligible = tiers.get(usize::from(tier)).is_some_and(|pool| {
                    pool.iter().any(|profile| {
                        profile
                            .agent_id
                            .eq_ignore_ascii_case(config.agent_id.command())
                            && profile.model == config.model
                            && profile.reasoning == config.reasoning_level
                            && profile.runtime_target == config.runtime_target
                            && profile.fast_mode == config.fast_mode
                    })
                });
                if !prefs.launch_auto || !eligible {
                    return Err(std::io::Error::other(
                        "requested Monitor tier/config changed before owner admission",
                    ));
                }
                if let Some(delivery_id) = feedback.issue_monitor_delivery_id.as_deref() {
                    if !prefs.record_launch_delivery_tier(
                        issue_number,
                        delivery_id,
                        input.materializer_id,
                        input.window_id,
                        tier,
                    ) {
                        return Err(std::io::Error::other(
                            "launch tier no longer matches the claimed delivery window",
                        ));
                    }
                }
                prefs.record_tier_launch(issue_number, tier);
                Ok(())
            })
            .map_err(|error| format!("Could not record Issue Monitor launch tier: {error}"))?;
        }
        if deliver_pending_autonomous_answer {
            crate::preserve_answered_handoff_resume_strategy_from_prefs(&prefs_path, issue_number)
                .map_err(|error| error.to_string())?;
            // The immutable GUI intent may have waited behind another worker.
            // Apply the current canonical mode before preparing its answer attempt.
            if crate::load_issue_monitor_prefs(&prefs_path)
                .map_err(|error| error.to_string())?
                .autonomous_mode
            {
                config.env_vars.insert(
                    crate::autonomous_handoff::GWT_AUTONOMOUS_EXECUTION_ENV.into(),
                    "1".into(),
                );
                config.env_vars.insert(
                    crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV.into(),
                    issue_number.to_string(),
                );
            }
            if config.session_mode != gwt_agent::SessionMode::Resume {
                return Err("pending autonomous answer requires an exact Resume request".into());
            }
            let source_id = config
                .predecessor_session_id
                .as_deref()
                .ok_or("pending autonomous answer has no exact asking Session")?;
            let source =
                gwt_agent::Session::load(&input.sessions_dir.join(format!("{source_id}.toml")))
                    .map_err(|error| error.to_string())?;
            let pending =
                crate::pending_autonomous_handoff_resumption_from_prefs(&prefs_path, issue_number)
                    .map_err(|error| error.to_string())?
                    .ok_or("pending autonomous answer is no longer available")?;
            if pending.session_id != source.id {
                return Err("pending autonomous answer asking Session changed".into());
            }
            let preparation = super::monitor::prepare_issue_monitor_handoff_delivery(
                &prefs_path,
                issue_number,
                &source,
                input.window_id,
                feedback.issue_monitor_delivery_id.as_deref(),
                input.materializer_id,
                false,
            )?;
            match preparation {
                Some(crate::AutonomousHandoffDeliveryPreparation::Ready(attempt)) => {
                    config.args.push(attempt.prompt.clone());
                    feedback.issue_monitor_autonomous_handoff = Some(attempt);
                }
                Some(crate::AutonomousHandoffDeliveryPreparation::Backoff {
                    retry_not_before,
                    ..
                }) => {
                    return Err(format!(
                    "answered autonomous handoff delivery is in backoff until {retry_not_before}"
                ))
                }
                Some(crate::AutonomousHandoffDeliveryPreparation::Ambiguous { reason, .. }) => {
                    return Err(reason)
                }
                _ => return Err("answered autonomous handoff delivery is already in flight".into()),
            }
        }
    }
    let resume_title = workspace_resume_context
        .as_ref()
        .and_then(WorkspaceResumeContext::purpose_title);
    let purpose_title = if is_review {
        config
            .linked_issue_number
            .map(|number| format!("Review · Issue #{number}"))
    } else if resume_title.is_some() {
        resume_title
    } else if let Some(prepared) = &prepared {
        prepared.purpose_title.clone()
    } else {
        agent_launch_purpose_title(
            input.project_root,
            config.linked_issue_number,
            config.branch.as_deref(),
            input.issue_link_cache_dir,
        )
        .map_err(|error| error.to_string())?
    };
    let is_pm = config
        .working_dir
        .as_deref()
        .is_some_and(crate::pm_registry::is_canonical_pm_worktree);
    if is_pm || is_review {
        config.env_vars.remove("GWT_AGENT_MEMBER_ID");
        config.env_vars.remove("GWT_AGENT_RESERVATION_ID");
        let history = gwt_core::paths::gwt_project_dir_for_repo_path(input.project_root)
            .join("agent-names.jsonl");
        let inherited = inherited_agent_name.or_else(|| {
            name_predecessor_session_id
                .as_ref()
                .and_then(|id| {
                    gwt_agent::Session::load(&input.sessions_dir.join(format!("{id}.toml"))).ok()
                })
                .and_then(|session| session.agent_name)
        });
        let name = match inherited {
            Some(name) => {
                let mut known = std::collections::HashSet::from([name.clone()]);
                if is_review && !is_pm {
                    crate::agent_names::ensure_available_initial(&name, input.known_names)?;
                    known.extend(input.known_names.iter().cloned());
                }
                crate::agent_names::remember(&history, &known)?;
                name
            }
            None => {
                let language = gwt_config::Settings::load()
                    .map(|settings| settings.ai.effective_language())
                    .unwrap_or("en");
                crate::agent_names::allocate(
                    &history,
                    language,
                    input.known_names,
                    input.known_names,
                )?
            }
        };
        config.env_vars.insert("GWT_AGENT_NAME".into(), name);
    } else {
        let reservation_id = config
            .env_vars
            .get("GWT_AGENT_RESERVATION_ID")
            .filter(|_| inherited_agent_name.is_some())
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let member = crate::agent_names::reserve_project_member_for_agent(
            input.project_root,
            input.sessions_dir,
            &reservation_id,
            config.linked_issue_number,
            roster_spec_number,
            launch_feedback_context
                .as_ref()
                .and_then(|feedback| feedback.issue_monitor_delivery_id.as_deref()),
            config
                .env_vars
                .get("GWT_AGENT_MEMBER_ID")
                .filter(|_| inherited_agent_name.is_some() || input.is_continue_work)
                .map(String::as_str),
            config.agent_id.command(),
            selected_agent_member_id.as_deref(),
        )?;
        let assignment = member.assignment.as_ref().expect("reserved assignment");
        config
            .env_vars
            .insert("GWT_AGENT_MEMBER_ID".into(), member.agent_member_id);
        config.env_vars.insert(
            "GWT_AGENT_RESERVATION_ID".into(),
            assignment.reservation_id.clone(),
        );
        config.env_vars.insert("GWT_AGENT_NAME".into(), member.name);
    }
    let prepared_manual_launch_claim = super::continuation::claim_prepared_manual_successor_launch(
        input.sessions_dir,
        input.project_root,
        config,
    )
    .inspect_err(|_| {
        if let Some((member, reservation)) = config
            .env_vars
            .get("GWT_AGENT_MEMBER_ID")
            .zip(config.env_vars.get("GWT_AGENT_RESERVATION_ID"))
        {
            let _ = crate::agent_names::release_project_member(
                input.project_root,
                input.sessions_dir,
                member,
                reservation,
            );
        }
    })?;
    Ok(PreparedOwnerAgentAdmission {
        workspace_resume_context,
        launch_feedback_context,
        prepared_manual_launch_claim,
        issue_monitor_trust_candidate,
        issue_monitor_launch: issue_monitor_issue_number.is_some(),
        purpose_title,
    })
}

pub fn inflight_launch_key(tab_id: &str, config: &gwt_agent::LaunchConfig) -> Option<String> {
    let branch = config
        .branch
        .as_deref()
        .map(crate::launch_runtime::normalize_branch_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_default();
    let dir = config
        .working_dir
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    if branch.is_empty() && dir.is_empty() {
        return None;
    }
    let agent = config.agent_id.command();
    let resume = config.resume_session_id.as_deref().unwrap_or_default();
    Some(format!(
        "{tab_id}\u{001f}{agent}\u{001f}{branch}\u{001f}{dir}\u{001f}{resume}"
    ))
}

/// Read the existing host project catalog; it does not grant project mutation authority.
pub(crate) fn owner_host_project_roots(project_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots =
        crate::persistence::load_session_state(&gwt_core::paths::gwt_session_state_path())
            .map_err(|error| error.to_string())?
            .tabs
            .into_iter()
            .map(|tab| tab.project_root)
            .collect::<Vec<_>>();
    roots.push(project_root.into());
    let projects = match std::fs::read_dir(gwt_core::paths::gwt_projects_dir()) {
        Ok(projects) => Some(projects),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if let Some(projects) = projects {
        for project in projects {
            let project = project.map_err(|error| error.to_string())?;
            let descriptors = match std::fs::read_dir(project.path().join("runtime/daemon")) {
                Ok(descriptors) => descriptors,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            for descriptor in descriptors {
                let descriptor = descriptor.map_err(|error| error.to_string())?;
                if descriptor
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
                {
                    continue;
                }
                if let Some(endpoint) = std::fs::read(descriptor.path()).ok().and_then(|bytes| {
                    serde_json::from_slice::<gwt_core::daemon::DaemonEndpoint>(&bytes).ok()
                }) {
                    if endpoint.scope.target == gwt_core::daemon::RuntimeTarget::Host {
                        roots.push(endpoint.scope.project_root);
                    }
                }
            }
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

/// Facts read from the exact Worktree's execution diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionTerminalFacts {
    pub status: crate::cli::execution_state::ExecutionDiagnosisState,
    pub binding: crate::cli::execution_state::ExecutionBindingState,
    pub owner_number: Option<u64>,
    /// Completion belongs to this window's Session, not a previous launch.
    pub session_matches_execution: bool,
    /// `settlement_severity == "clear"`.
    pub settlement_clear: bool,
    /// Warning-only bookkeeping with authenticated current-generation provenance.
    pub settlement_warning_authorized: bool,
    pub settlement_obligation_open: bool,
    pub open_obligations: usize,
    /// Issue #4783 AC-1: the generation was Blocked by a revoked-launch
    /// release — what `issue.monitor.stop` writes on the ledger. Durable on
    /// the execution side, so it still says "stopped through the Monitor"
    /// after the Monitor prefs lost the hold (a requeue, a prefs reset, a
    /// different store) and until `execution.reopen` clears it.
    pub launch_revoked: bool,
}

impl ExecutionTerminalFacts {
    pub fn matches_completed_session(
        &self,
        session: &gwt_agent::Session,
        read_record: impl FnOnce(
            &Path,
        ) -> std::io::Result<
            Option<crate::cli::execution_state::ExecutionControlRecord>,
        >,
    ) -> bool {
        if self.status != crate::cli::execution_state::ExecutionDiagnosisState::Completed {
            return false;
        }
        read_record(&session.worktree_path)
            .ok()
            .flatten()
            .is_some_and(|record| {
                crate::cli::execution_state::integrity_ok(&record)
                    && record.primary_session_id == session.id
                    && Some(record.owner_number) == self.owner_number
                    && record.status
                        == crate::cli::execution_state::ExecutionControlStatus::Completed
            })
    }

    pub fn from_diagnosis(
        diagnosis: &crate::cli::execution_state::ExecutionDiagnosisSnapshot,
    ) -> Self {
        Self {
            status: diagnosis.ecr_status,
            binding: diagnosis.binding_state,
            owner_number: diagnosis.owner_number,
            session_matches_execution: false,
            settlement_clear: diagnosis.settlement_severity == "clear",
            settlement_warning_authorized: diagnosis.settlement_severity == "warning"
                && diagnosis.work_event_receipt_matches_current_generation == Some(true),
            settlement_obligation_open: diagnosis.settlement_obligation_open,
            open_obligations: diagnosis.open_obligations.len(),
            launch_revoked: diagnosis.ecr_status
                == crate::cli::execution_state::ExecutionDiagnosisState::Blocked
                && diagnosis.missing_verification.as_deref()
                    == Some(crate::cli::execution_state::REVOKED_LAUNCH_MISSING_VERIFICATION),
        }
    }
}

/// Facts read from the repository-scoped Issue Monitor prefs for one window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonitorTerminalFacts {
    /// The Monitor durably binds this exact window to the linked Issue.
    pub binds_this_window: bool,
    /// The Monitor durably binds the linked Issue to a *different* window:
    /// this launch was replaced (failover / relaunch), i.e. revoked.
    pub binds_other_window: bool,
    /// A durable closed-Issue record (Released) or validated Issue-wide
    /// completion (Merged) exists for the linked Issue.
    pub issue_closed: bool,
    /// The row is parked for a human (`needs_human`).
    pub needs_human: bool,
    /// The Issue holds a failure record (launch / agent failure, or an
    /// operator stop hold).
    pub failure_hold: bool,
    /// Issue #4802 AC-3: the hold is an operator stop through the Monitor.
    pub monitor_stopped: bool,
}

impl MonitorTerminalFacts {
    pub fn from_monitor(
        monitor: &crate::IssueMonitorState,
        issue_number: u64,
        window_id: &str,
    ) -> Self {
        let facts = monitor.terminal_window_facts(issue_number, window_id);
        Self {
            binds_this_window: facts.binds_this_window,
            binds_other_window: facts.binds_other_window,
            issue_closed: facts.issue_closed,
            needs_human: facts.needs_human,
            failure_hold: facts.failure_hold,
            monitor_stopped: facts.monitor_stopped,
        }
    }
}

/// Everything the canonical terminal predicate consumes. `None` for a fact
/// means it could not be read; every such reading fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalWindowFacts {
    pub linked_issue: Option<u64>,
    pub session_status: Option<gwt_agent::AgentStatus>,
    pub window_status: WindowProcessStatus,
    pub execution: Option<ExecutionTerminalFacts>,
    pub monitor: Option<MonitorTerminalFacts>,
}

/// Why a window is eligible for automatic terminal cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalCloseReason {
    /// FR-044 (a): Completed ECR, Terminal binding, clear settlement, no open
    /// obligation.
    SettledExecution,
    /// FR-044 (b): durable closed / completed Issue record.
    ClosedIssue,
    /// A completed idle writer of a closed Issue retains only bookkeeping warnings.
    ClosedIssueBookkeepingWarning,
    /// FR-044 (c): the Monitor durably replaced this launch with another
    /// window and this one is no longer running.
    RevokedLaunch,
    /// Issue #4802 AC-3: an idle pane whose Issue the Monitor bound to another
    /// window, and whose Session is not the Issue's execution writer — the
    /// duplicate a relaunch left behind.
    LostOwnership,
    /// Issue #4802 AC-3: an idle pane of an Issue the operator stopped
    /// through the Monitor.
    MonitorStopped,
}

impl TerminalCloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SettledExecution => "settled_execution",
            Self::ClosedIssue => "closed_issue",
            Self::ClosedIssueBookkeepingWarning => "closed_issue_bookkeeping_warning",
            Self::RevokedLaunch => "revoked_launch",
            Self::LostOwnership => "lost_ownership",
            Self::MonitorStopped => "monitor_stopped",
        }
    }
}

/// The canonical terminal predicate (FR-044). Pure; every fact is an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalCloseEligibility {
    Eligible(TerminalCloseReason),
    Ineligible(&'static str),
}

/// FR-044: decide whether one Issue-linked Agent window may be closed by the
/// runtime. Missing, stale, corrupt, or unreadable facts fail closed; so do
/// NeedsHuman, Error / Interrupted / Blocked, open action obligations, and windows
/// the Monitor still tracks without a terminal fact.
pub fn classify_terminal_window(facts: &TerminalWindowFacts) -> TerminalCloseEligibility {
    use crate::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};
    use TerminalCloseEligibility::{Eligible, Ineligible};

    let Some(issue_number) = facts.linked_issue else {
        return Ineligible("no_linked_issue");
    };
    // An Error pane is a diagnostic the operator may still need to read
    // (AS-42 / AS-45); pre-PTY Monitor failures converge through their own
    // acknowledged path instead.
    if facts.window_status == WindowProcessStatus::Error {
        return Ineligible("window_error");
    }
    let Some(session_status) = facts.session_status else {
        return Ineligible("session_unreadable");
    };
    if session_status == gwt_agent::AgentStatus::Interrupted {
        return Ineligible("session_interrupted");
    }
    let Some(monitor) = facts.monitor.as_ref() else {
        return Ineligible("monitor_unreadable");
    };
    // Issue #4802 AC-3: ahead of the hold checks, because a Monitor stop is
    // itself a hold, and ahead of the obligation checks, because a pane that
    // lost ownership is not the writer those obligations belong to.
    if let Some(reason) = orphaned_idle_pane_reason(facts, monitor, session_status) {
        return Eligible(reason);
    }
    if monitor.needs_human {
        return Ineligible("needs_human");
    }
    if monitor.failure_hold {
        return Ineligible("failure_hold");
    }
    let Some(execution) = facts.execution.as_ref() else {
        return Ineligible("execution_unreadable");
    };
    if matches!(
        execution.status,
        ExecutionDiagnosisState::Blocked | ExecutionDiagnosisState::Corrupt
    ) || execution.binding == ExecutionBindingState::Corrupt
    {
        return Ineligible("execution_blocked_or_corrupt");
    }
    // Issue #3590 / T-050: retain the warning and its original receipt, but
    // let this exact completed idle writer return its physical Monitor slot.
    if monitor.issue_closed
        && execution.status == ExecutionDiagnosisState::Completed
        && execution.binding == ExecutionBindingState::Terminal
        && execution.owner_number == Some(issue_number)
        && execution.session_matches_execution
        && execution.settlement_warning_authorized
        && execution.open_obligations == 0
        && matches!(
            facts.window_status,
            WindowProcessStatus::Idle | WindowProcessStatus::Stopped
        )
        && matches!(
            session_status,
            gwt_agent::AgentStatus::Idle | gwt_agent::AgentStatus::Stopped
        )
    {
        return Eligible(TerminalCloseReason::ClosedIssueBookkeepingWarning);
    }
    if execution.settlement_obligation_open || execution.open_obligations > 0 {
        return Ineligible("obligation_open");
    }

    // (b) durable closed / completed Issue record.
    if monitor.issue_closed {
        return Eligible(TerminalCloseReason::ClosedIssue);
    }
    // Issue #4783 AC-R1: a fresh launch may still read its predecessor's
    // Completed record before activation. It is not this Session's delivery.
    if execution.status == ExecutionDiagnosisState::Completed
        && !execution.session_matches_execution
    {
        return Ineligible("execution_session_mismatch");
    }
    // (a) fully settled execution for this exact owner.
    if execution.status == ExecutionDiagnosisState::Completed
        && execution.binding == ExecutionBindingState::Terminal
        && execution.settlement_clear
        && execution.owner_number == Some(issue_number)
    {
        return Eligible(TerminalCloseReason::SettledExecution);
    }
    // (c) the Monitor durably replaced this launch and nothing runs here.
    if monitor.binds_other_window
        && !monitor.binds_this_window
        && facts.window_status == WindowProcessStatus::Stopped
        && session_status == gwt_agent::AgentStatus::Stopped
    {
        return Eligible(TerminalCloseReason::RevokedLaunch);
    }
    if monitor.binds_this_window {
        return Ineligible("monitor_tracking_unsettled");
    }
    Ineligible("not_terminal")
}

/// Issue #4802 AC-3: an idle pane nothing owns any more.
///
/// The close predicate used to reach only a fully Stopped revoked launch, so
/// the prompt-ready duplicates a relaunch left behind and the panes of Issues
/// the operator stopped stayed on the canvas indefinitely. Both are closed
/// here, and only while idle: a pane in a turn (Running / Starting / Waiting)
/// or held for recovery (Interrupted) is never a candidate, which keeps the
/// #3482 rule that a working pane is not killed. A pane the Monitor still
/// binds is never orphaned, and an unreadable or corrupt execution record
/// fails closed.
fn orphaned_idle_pane_reason(
    facts: &TerminalWindowFacts,
    monitor: &MonitorTerminalFacts,
    session_status: gwt_agent::AgentStatus,
) -> Option<TerminalCloseReason> {
    use crate::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

    if monitor.binds_this_window {
        return None;
    }
    let idle = matches!(
        facts.window_status,
        WindowProcessStatus::Idle | WindowProcessStatus::Stopped
    ) && matches!(
        session_status,
        gwt_agent::AgentStatus::Idle | gwt_agent::AgentStatus::Stopped
    );
    if !idle {
        return None;
    }
    let execution = facts.execution.as_ref()?;
    if execution.status == ExecutionDiagnosisState::Corrupt
        || execution.binding == ExecutionBindingState::Corrupt
    {
        return None;
    }
    if monitor.monitor_stopped {
        return Some(TerminalCloseReason::MonitorStopped);
    }
    // The Session that still holds the execution generation is the writer:
    // closing it would strand the Work, whichever window the Monitor binds.
    if monitor.binds_other_window
        && !monitor.needs_human
        && execution.binding != ExecutionBindingState::Bound
    {
        return Some(TerminalCloseReason::LostOwnership);
    }
    None
}

/// Issue #4441: translate one close-side ineligibility cause into a
/// restore-side admission.
///
/// [`classify_terminal_window`] answers a single question — *may the runtime
/// close this window?* — so every fact it cannot prove comes back as
/// `Ineligible`, which is the safe answer for closing. Restore has the
/// opposite polarity, and it used to read the whole `Ineligible` family as
/// permission to spawn. That inversion is why `issue.monitor.stop` respawned
/// the row it stopped: the stop writes `failed_issues`, which reads as
/// `failure_hold`, which the close side reports as "do not close" and restore
/// then took as "do spawn". The operator's only lever recreated the window it
/// was pressed to remove.
///
/// So the mapping is an allowlist. Only the causes that positively establish
/// that the Work is still live admit a restore; everything else refuses, and
/// **a cause added to the close predicate later refuses by default** instead of
/// silently becoming a new way to respawn a finished window.
fn restore_admission_for_ineligible(cause: &'static str) -> RestoreAdmission {
    match cause {
        // The Work is live, or there is no Issue-linked Work to read at all
        // (a PM pane, a manual launch). These are the restores that should
        // happen.
        //
        // Interrupted Sessions retain exact auto-resume. Blocked generations
        // are refused separately below; corrupt-state behavior stays unchanged.
        "no_linked_issue"
        | "not_terminal"
        | "monitor_tracking_unsettled"
        | "obligation_open"
        | "execution_blocked_or_corrupt"
        | "session_interrupted" => RestoreAdmission::Admit,
        // Issue #4441 (AC-3): the Issue Monitor is holding this row. A hold is
        // reversible, so the placeholder and the restore flag stay: releasing
        // the row brings the window back.
        "needs_human" | "failure_hold" => RestoreAdmission::RefuseHeld(cause),
        // Issue #4143: the canonical facts could not be read. Spawning on an
        // unreadable fact is what turned one descriptor exhaustion (#4142) into
        // 254 respawned windows, 135 of which died before PTY start. An
        // unreadable fact is no evidence that the window is finished either, so
        // the placeholder survives for the next generation to answer.
        //
        // `window_error` is the same answer for a different reason: an error
        // pane is a diagnostic the operator may still need, and respawning it
        // automatically is what #4143 (AC-3) removed.
        "session_unreadable"
        | "monitor_unreadable"
        | "execution_unreadable"
        | "window_error"
        | "execution_session_mismatch" => RestoreAdmission::RefuseUnprovable(cause),
        // A cause this function does not name is, by definition, one it could
        // not establish. Refusing here is what keeps a later addition to the
        // close predicate from becoming a new way to respawn a finished window.
        _ => RestoreAdmission::RefuseUnprovable(cause),
    }
}

/// Issue #4143 (AC-2): the admission decision for one restore candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreAdmission {
    /// The Work is live, or the Session carries no Issue-linked Work at all.
    Admit,
    /// The Work is provably terminal: disable restore and drop the
    /// placeholder so the window stops coming back.
    RefuseTerminal(TerminalCloseReason),
    /// The owner is closed, but cleanup must retain its diagnostic window.
    RefuseRetainedTerminal,
    /// Issue #4802 (AC-4): the execution reached Completed, so the Work is
    /// finished even while a settlement obligation is open. Do not spawn;
    /// keep the placeholder as the diagnostic.
    RefuseCompletedWork,
    /// Issue #5510 AC-3: keep the placeholder but let the stale Active holder
    /// reach the reaper and the Monitor's existing fresh-launch recovery.
    RefuseFreshSession,
    /// Issue #4441 (AC-3): the Issue Monitor is holding this row. Do not
    /// spawn, but keep the placeholder — the hold is reversible.
    RefuseHeld(&'static str),
    /// The canonical facts could not be read. Do not spawn, but keep the
    /// placeholder: the next generation may be able to prove the answer.
    RefuseUnprovable(&'static str),
}

/// FR-047 / Issue #4143 (AC-2) / Issue #4802 (AC-4): the restore decision for
/// one Issue-linked Session, from the canonical facts. `has_window` is whether
/// a persisted placeholder names the exact window; an orphan Session has no
/// identity to compare against the Monitor binding.
pub fn restore_admission_for_facts(
    facts: &TerminalWindowFacts,
    has_window: bool,
) -> RestoreAdmission {
    match classify_terminal_window(facts) {
        TerminalCloseEligibility::Eligible(
            TerminalCloseReason::RevokedLaunch | TerminalCloseReason::LostOwnership,
        ) if !has_window => RestoreAdmission::Admit,
        // Issue #4441 AC-3 / #4802 AC-4: a stop is a reversible hold, so
        // nothing spawns but the placeholder stays for the release.
        TerminalCloseEligibility::Eligible(TerminalCloseReason::MonitorStopped) => {
            RestoreAdmission::RefuseHeld("monitor_stopped")
        }
        TerminalCloseEligibility::Eligible(reason) => RestoreAdmission::RefuseTerminal(reason),
        // Reopened #4143: diagnostic retention is not permission to
        // restart a closed owner's process. Keep its placeholder, but
        // refuse automatic spawn even for Blocked/open-obligation ECRs.
        // This outranks the per-cause mapping below, which answers "could
        // this cause be established", not "is this owner finished".
        TerminalCloseEligibility::Ineligible(_)
            if facts
                .monitor
                .as_ref()
                .is_some_and(|monitor| monitor.issue_closed) =>
        {
            RestoreAdmission::RefuseRetainedTerminal
        }
        // Issue #4783 AC-1: preserve the specific operator-stop reason even
        // after Monitor prefs lose their hold. The ledger marker keeps the
        // diagnostic placeholder without spawning; explicit reopen lifts it.
        TerminalCloseEligibility::Ineligible(_)
            if facts
                .execution
                .as_ref()
                .is_some_and(|execution| execution.launch_revoked) =>
        {
            RestoreAdmission::RefuseHeld("launch_revoked")
        }
        // Issue #4802 AC-4: an execution that reached Completed is
        // finished Work even while a settlement obligation is still open.
        // Restarting its agent is what brought finished Issues' windows
        // back after a restart; the placeholder is retained as the
        // diagnostic, but nothing spawns.
        TerminalCloseEligibility::Ineligible(_)
            if facts.execution.as_ref().is_some_and(|execution| {
                execution.status == crate::cli::execution_state::ExecutionDiagnosisState::Completed
            }) =>
        {
            RestoreAdmission::RefuseCompletedWork
        }
        // Issue #4783 recurrence: Blocked is terminal even when it was not
        // an operator revocation. Recovery must be explicit, never startup.
        TerminalCloseEligibility::Ineligible(_)
            if facts.execution.as_ref().is_some_and(|execution| {
                execution.status == crate::cli::execution_state::ExecutionDiagnosisState::Blocked
            }) =>
        {
            RestoreAdmission::RefuseHeld("execution_blocked")
        }
        TerminalCloseEligibility::Ineligible(cause) => restore_admission_for_ineligible(cause),
    }
}

pub fn read_terminal_window_facts(
    session: &gwt_agent::Session,
    window_id: &str,
    window_status: WindowProcessStatus,
    project_root: &Path,
) -> TerminalWindowFacts {
    let monitor = session.linked_issue_number.and_then(|_| {
        load_terminal_monitor(&crate::issue_monitor_prefs_path_for_repo_path(project_root))
    });
    read_terminal_window_facts_with_monitor(session, window_id, window_status, monitor.as_ref())
}

pub fn load_terminal_monitor(prefs_path: &Path) -> Option<crate::IssueMonitorState> {
    crate::load_issue_monitor_prefs(prefs_path)
        .ok()
        .map(|prefs| {
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs)
        })
}

pub fn read_terminal_window_facts_with_monitor(
    session: &gwt_agent::Session,
    window_id: &str,
    window_status: WindowProcessStatus,
    monitor: Option<&crate::IssueMonitorState>,
) -> TerminalWindowFacts {
    let Some(issue_number) = session.linked_issue_number else {
        return TerminalWindowFacts {
            linked_issue: None,
            session_status: Some(session.status),
            window_status,
            execution: None,
            monitor: None,
        };
    };
    let mut execution = ExecutionTerminalFacts::from_diagnosis(
        &crate::cli::execution_state::diagnose_for_projection_with_session(
            &session.worktree_path,
            Some(&session.id),
            Some(session),
        ),
    );
    execution.session_matches_execution =
        execution.matches_completed_session(session, crate::cli::execution_state::load);
    let execution = Some(execution);
    let monitor =
        monitor.map(|monitor| MonitorTerminalFacts::from_monitor(monitor, issue_number, window_id));
    TerminalWindowFacts {
        linked_issue: Some(issue_number),
        session_status: Some(session.status),
        window_status,
        execution,
        monitor,
    }
}

pub fn restore_saved_session_work_terminality(
    session: &gwt_agent::Session,
    project_root: &Path,
    window_id: Option<&str>,
) -> RestoreAdmission {
    if session.linked_issue_number.is_none() {
        // A Session with no Issue link has no Work whose terminality this
        // predicate can read (a PM pane, a manual launch). Restore keeps
        // the "everything the user did not explicitly close" rule.
        return RestoreAdmission::Admit;
    }
    let facts = read_terminal_window_facts(
        session,
        window_id.unwrap_or_default(),
        WindowProcessStatus::Stopped,
        project_root,
    );
    let admission = restore_admission_for_facts(&facts, window_id.is_some());
    if admission != RestoreAdmission::Admit {
        return admission;
    }
    // Work may already be Done while verification and delivery still keep
    // its execution Active. Selecting it would protect the old generation
    // from the reaper, then Resume would refuse and launch without authority.
    let project_state_root = session
        .project_state_root
        .as_deref()
        .unwrap_or(project_root);
    match crate::continuation_requires_new_session(project_state_root, session) {
        Ok(true) => RestoreAdmission::RefuseFreshSession,
        Ok(false) => admission,
        Err(_) => RestoreAdmission::RefuseUnprovable("work_authority_unreadable"),
    }
}

/// #3065: build the Workspace Resume context from the resumed branch's own
/// Work item. The repo-shared current projection (`current.json`) must NOT be
/// the source here: it carries the identity of whatever Work last wrote it,
/// and replaying that identity into a different Work's resume event is how
/// one Work's owner/title leaked into every other Workspace row. When no
/// Work item matches the container, the context is neutral — never the
/// shared identity.
pub fn workspace_resume_context_for_work_item(
    project_state_root: &Path,
    branch: Option<&str>,
    worktree_path: &Path,
) -> WorkspaceResumeContext {
    let item = gwt_core::workspace_projection::load_workspace_work_items(project_state_root)
        .ok()
        .flatten()
        .and_then(|projection| {
            gwt_core::workspace_projection::find_work_item_for_container(
                &projection,
                project_state_root,
                branch,
                Some(worktree_path),
            )
            .cloned()
        });
    match item {
        Some(item) => WorkspaceResumeContext {
            title: super::workspace::non_empty_workspace_text(Some(&item.title)),
            owner: super::workspace::non_empty_workspace_text(item.owner.as_deref()),
            summary: super::workspace::non_empty_workspace_text(item.summary.as_deref())
                .or_else(|| super::workspace::non_empty_workspace_text(item.intent.as_deref())),
            next_action: item.latest_next_action().map(str::to_string),
        },
        None => WorkspaceResumeContext {
            title: None,
            owner: None,
            summary: None,
            next_action: None,
        },
    }
}

#[cfg(test)]
mod terminal_window_tests {
    use super::*;
    use crate::cli::execution_state::{ExecutionBindingState, ExecutionDiagnosisState};

    #[test]
    fn closed_bookkeeping_warning_preserves_live_and_untrusted_panes() {
        let original = TerminalWindowFacts {
            linked_issue: Some(42),
            session_status: Some(gwt_agent::AgentStatus::Idle),
            window_status: WindowProcessStatus::Idle,
            execution: Some(ExecutionTerminalFacts {
                status: ExecutionDiagnosisState::Completed,
                binding: ExecutionBindingState::Terminal,
                owner_number: Some(42),
                session_matches_execution: true,
                settlement_clear: false,
                settlement_warning_authorized: true,
                settlement_obligation_open: true,
                open_obligations: 0,
                launch_revoked: false,
            }),
            monitor: Some(MonitorTerminalFacts {
                issue_closed: true,
                ..Default::default()
            }),
        };
        assert_eq!(
            classify_terminal_window(&original),
            TerminalCloseEligibility::Eligible(TerminalCloseReason::ClosedIssueBookkeepingWarning)
        );

        let invalid: [fn(&mut TerminalWindowFacts); 9] = [
            |facts| facts.monitor.as_mut().unwrap().issue_closed = false,
            |facts| facts.window_status = WindowProcessStatus::Running,
            |facts| facts.session_status = Some(gwt_agent::AgentStatus::Running),
            |facts| facts.execution.as_mut().unwrap().session_matches_execution = false,
            |facts| facts.execution.as_mut().unwrap().owner_number = Some(7),
            |facts| {
                facts
                    .execution
                    .as_mut()
                    .unwrap()
                    .settlement_warning_authorized = false
            },
            |facts| facts.execution.as_mut().unwrap().open_obligations = 1,
            |facts| facts.execution.as_mut().unwrap().status = ExecutionDiagnosisState::Active,
            |facts| facts.execution.as_mut().unwrap().binding = ExecutionBindingState::Corrupt,
        ];
        for invalidate in invalid {
            let mut facts = original.clone();
            invalidate(&mut facts);
            assert!(
                matches!(
                    classify_terminal_window(&facts),
                    TerminalCloseEligibility::Ineligible(_)
                ),
                "{facts:?}"
            );
        }
    }
}

#[cfg(test)]
mod delivery_ack_tests {
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
            agent_member_id: None,
            dependency_observations: Vec::new(),
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
                super::issue_monitor_delivery_marker_result(
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
                super::issue_monitor_delivery_marker_result(
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

use crate as gwt;
use crate::runtime_daemon_events::IssueMonitorControlPublishError;

#[derive(Clone, Default)]
pub struct LaunchDeliveryAckResult {
    pub monitor: Option<Box<gwt::IssueMonitorState>>,
    pub stale_window: Option<String>,
    pub semantic_receipt_settled: bool,
}

pub fn reconcile_delivery_ack_result(
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

pub fn issue_monitor_delivery_marker_result(
    project_root: &Path,
    issue_number: u64,
    delivery_id: &str,
    materializer_id: &str,
    window_id: &str,
    workspace_durable: bool,
    publication: Result<(), gwt::runtime_daemon_events::IssueMonitorControlPublishError>,
) -> Result<bool, gwt::runtime_daemon_events::IssueMonitorControlPublishError> {
    let unknown = match publication {
        Ok(()) => None,
        Err(
            error @ gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(_),
        ) => Some(error),
        Err(error) => return Err(error),
    };
    let prefs =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(project_root))
            .map_err(|error| {
            unknown.clone().unwrap_or_else(|| {
                gwt::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(
                    format!("launch delivery marker readback failed: {error}"),
                )
            })
        })?;
    let committed = prefs.pending_launch_deliveries.iter().any(|delivery| {
        let marker = if workspace_durable {
            delivery.workspace_durable_window_id.as_deref()
        } else {
            delivery.materialized_window_id.as_deref()
        };
        delivery.issue_number == issue_number
            && delivery.delivery_id == delivery_id
            && delivery.materializer_id.as_deref() == Some(materializer_id)
            && delivery.materializer_pid == Some(std::process::id())
            && delivery.materializer_window_id.as_deref() == Some(window_id)
            && delivery.materialized_window_id.as_deref() == Some(window_id)
            && marker == Some(window_id)
    });
    // Only exact committed ownership resolves an ambiguous response. Keep
    // an unproven outcome unknown so the ACK handler cannot close the pane.
    match (committed, unknown) {
        (false, Some(error)) => Err(error),
        _ => Ok(committed),
    }
}

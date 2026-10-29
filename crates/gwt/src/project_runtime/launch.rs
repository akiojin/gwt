//! Agent launch preparation owned by the project runtime.

use super::{
    launch_authority::{install_agent_capability_env, install_authenticated_active_resume_binding},
    launch_finalization::{
        rollback_materialized_fresh_execution_launch, terminalize_materialized_genesis_launch,
        FinalizedAgentCapabilityLaunch, MaterializedGenesisLaunch,
    },
    launch_output_mirror::DockerLaunchOutputMirror,
    launch_payload::{
        AgentLaunchResult, AgentLaunchRuntimeContext, LaunchFeedbackContext, ProcessLaunch,
    },
};
use crate as gwt;
use crate::{
    agent_capability::AgentCapabilityIssuer,
    docker_launch::{
        finalize_docker_agent_launch_config_with_binding, prepare_docker_runtime_for_launch,
        register_codex_managed_hook_trust_in_docker,
        register_codex_managed_project_trust_in_docker, resolve_docker_agent_program_with_binding,
        DockerLaunchBinding,
    },
    launch_runtime::{
        apply_windows_host_shell_wrapper, install_launch_gwt_bin_env, resolve_launch_worktree,
    },
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    session_launch::recovery::{
        persist_durable_launch_recovery_with_identity, DurableLaunchRecoveryKind,
    },
    session_launch::{initialize_launch_session, persist_finalized_launch_session},
    worktree_form::is_ephemeral_worktree_path,
};
use gwt_agent::resolve_host_runner_health_checked;
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Inputs captured by the project owner; prepared authority never crosses its wire boundary.
pub struct AgentLaunchInput {
    pub sessions_dir: PathBuf,
    pub project_root: String,
    pub window_id: String,
    pub config: gwt_agent::LaunchConfig,
    pub profile_config_path: PathBuf,
    pub agent_capability_issuer: Option<AgentCapabilityIssuer>,
    pub prepared_manual_launch_claim: Option<gwt_agent::SessionActiveLaunchHandshake>,
    pub issue_monitor_trust_candidate: Option<IssueMonitorTrustCandidate>,
    pub issue_monitor_launch: bool,
}

/// Public display updates emitted while owner-local launch preparation runs.
pub enum AgentLaunchEvent {
    Progress {
        message: String,
    },
    TerminalOutput {
        data: Vec<u8>,
    },
    Toast {
        level: String,
        message: String,
        issue_number: Option<u64>,
    },
}

fn configured_custom_agent_for_launch(
    config: &gwt_agent::LaunchConfig,
    profile_config_path: &Path,
) -> Result<Option<gwt_agent::CustomCodingAgent>, String> {
    if !matches!(&config.agent_id, gwt_agent::AgentId::Custom(_)) {
        return Ok(None);
    }
    Ok(
        gwt_agent::load_custom_agents_from_path(profile_config_path)?
            .into_iter()
            .find(|agent| config.agent_id == gwt_agent::AgentId::Custom(agent.id.clone())),
    )
}

/// Audit the immutable request without materializing any launch authority or pane.
pub(super) fn check_launch_permission(
    project_root: &Path,
    project_key: &str,
    operation_id: &str,
    config: &gwt_agent::LaunchConfig,
    current: &std::sync::atomic::AtomicBool,
) -> Result<Option<crate::cli::permission_readiness::PermissionReadinessRecord>, String> {
    use gwt_core::daemon::{RuntimeScope, RuntimeTarget};
    use std::sync::atomic::Ordering;
    if !current.load(Ordering::Acquire)
        || !RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .is_ok_and(|scope| scope.repo_hash == project_key)
    {
        return Err("permission audit project identity changed".into());
    }
    let worktree = dunce::canonicalize(config.working_dir.as_deref().unwrap_or(project_root))
        .map_err(|error| format!("permission audit worktree: {error}"))?;
    if !RuntimeScope::from_project_root(&worktree, RuntimeTarget::Host)
        .is_ok_and(|scope| scope.repo_hash == project_key)
        && !super::continuation::worktree_matches_project_state(&worktree, project_root)
    {
        return Err("permission audit worktree belongs to another project".into());
    }
    if config.is_ephemeral
        || config.suppress_execution_control
        || config.session_mode != gwt_agent::SessionMode::Normal
    {
        return Ok(None);
    }
    let Some(number) = config.linked_issue_number else {
        return Ok(None);
    };
    if number == 0 {
        return Err("permission audit owner is invalid".into());
    }
    let branch = crate::launch_runtime::current_git_branch(&worktree)?;
    if config
        .branch
        .as_deref()
        .is_some_and(|expected| crate::launch_runtime::normalize_branch_name(expected) != branch)
        || gwt_core::workspace_projection::work_owner_for_branch(&branch)
            .is_some_and(|owner| owner != format!("Issue #{number}"))
    {
        return Err("permission audit worktree owner changed".into());
    }
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: crate::cli::execution_state::detect_owner_kind(&worktree, number),
        number,
    };
    if producing_owner_after_delivery_check(&worktree, Some(owner), config.explicit_follow_up)
        .is_none()
    {
        return Ok(None);
    }
    let custom = configured_custom_agent_for_launch(config, &gwt_core::paths::gwt_config_path())?;
    let mut materialized = config.clone();
    gwt_agent::LaunchEnvironment::from_active_profile(
        &gwt_core::paths::gwt_config_path(),
        config.runtime_target,
    )?
    .with_project_root(&worktree)
    .apply_to_config(&mut materialized);
    let entrypoint =
        gwt_agent::permission_mode::entrypoint_from_launch_args(config.entrypoint_args(), false);
    let decision = gwt_agent::validate_materialized_launch(
        gwt_agent::decide_permission_mode(&gwt_agent::PermissionModeInputs {
            agent_id: &config.agent_id,
            custom_agent: custom.as_ref(),
            source: config.permission_decision.source,
            entrypoint: &entrypoint,
            launch_route: config.launch_route,
            requested_skip_permissions: Some(config.skip_permissions),
            producing_work: true,
        }),
        &materialized.args,
        &materialized.env_vars,
    );
    // This identity names an audit attempt, never an issued durable Session.
    let attempt = format!("permission-check:{operation_id}");
    let record = crate::cli::permission_readiness::pre_launch_block(
        owner.kind.as_str(),
        number,
        &attempt,
        &decision,
    );
    if !current.load(Ordering::Acquire) {
        return Err("permission audit cancelled".into());
    }
    if let Some(record) = record.as_ref() {
        crate::cli::permission_readiness::record_pre_launch_block(&worktree, record).map_err(
            |error| {
                format!(
                    "launch refused: {}; its permission record could not be confirmed: {error}",
                    record.describe(),
                )
            },
        )?;
    }
    if !current.load(Ordering::Acquire) {
        return Err("permission audit cancelled".into());
    }
    Ok(record)
}

/// Prepare the durable Session and process command on the owner's blocking worker.
pub fn prepare_agent_launch(
    input: AgentLaunchInput,
    emit: impl Fn(AgentLaunchEvent) + Send + Sync + 'static,
) -> AgentLaunchResult {
    let AgentLaunchInput {
        sessions_dir,
        project_root,
        window_id,
        mut config,
        profile_config_path,
        agent_capability_issuer,
        prepared_manual_launch_claim,
        issue_monitor_trust_candidate,
        issue_monitor_launch,
    } = input;
    // SPEC-2014 FR-139..142 — while a Docker launch prepares (preflight,
    // compose ps/up incl. image build, exec probes), mirror docker-kind
    // Process Console lines into the agent terminal. Host launches keep
    // their immediate-PTY behavior untouched (FR-142).
    let emit = Arc::new(emit);
    let docker_output_mirror = (config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker)
        .then(|| {
            let output = Arc::clone(&emit);
            DockerLaunchOutputMirror::start_with_hub(
                gwt_core::process_console::global(),
                move |data| {
                    output(AgentLaunchEvent::TerminalOutput { data });
                },
            )
        });
    let mut issued_capability_token = None;
    let member_reservation = config
        .env_vars
        .get("GWT_AGENT_MEMBER_ID")
        .zip(config.env_vars.get("GWT_AGENT_RESERVATION_ID"))
        .map(|(member, reservation)| (member.clone(), reservation.clone()));
    let mut active_launch_handshake_cleanup = prepared_manual_launch_claim;
    // Issue #4283 AC-5: attribute the pane-create route to its phases so
    // the next regression is read from the perf stream, not guessed.
    let mut phases = gwt::perf::RoutePhaseClock::start(gwt::perf::PerfRoute::PaneCreate);
    let result: AgentLaunchResult = (|| {
        emit(AgentLaunchEvent::Progress {
            message: "Preparing worktree...".to_string(),
        });
        resolve_launch_worktree(Path::new(&project_root), &mut config)?;
        let managed_codex_worktree = if issue_monitor_trust_candidate.is_some()
            && config.agent_id == gwt_agent::AgentId::Codex
        {
            let main_worktree_root =
                gwt_git::worktree::main_worktree_root(Path::new(&project_root))
                    .map_err(|error| error.to_string())?;
            let worktrees = gwt_git::WorktreeManager::new(&main_worktree_root)
                .list()
                .map_err(|error| error.to_string())?;
            validate_issue_monitor_managed_codex_worktree(
                issue_monitor_trust_candidate.as_ref(),
                Path::new(&project_root),
                &config,
                &worktrees,
            )?
        } else {
            None
        };
        phases.mark("worktree");

        emit(AgentLaunchEvent::Progress {
            message: "Starting Docker service...".to_string(),
        });
        let docker_launch_binding =
            prepare_docker_runtime_for_launch(Path::new(&project_root), &mut config)?;
        phases.mark("docker");

        emit(AgentLaunchEvent::Progress {
            message: "Configuring work...".to_string(),
        });
        let worktree_path = gwt_core::paths::normalize_windows_child_process_path(
            &config
                .working_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from(&project_root)),
        );
        if config.working_dir.is_some() {
            config.working_dir = Some(worktree_path.clone());
        }
        let initial_prompt_file =
            crate::launch_runtime::prepare_initial_prompt(&mut config, &worktree_path)?;
        gwt_agent::LaunchEnvironment::from_active_profile(
            &profile_config_path,
            config.runtime_target,
        )?
        .with_project_root(&worktree_path)
        .apply_to_config(&mut config);
        if let Some(managed_worktree) = managed_codex_worktree.as_ref() {
            let report = register_codex_managed_project_trust_for_resolved_launch(
                &profile_config_path,
                managed_worktree,
                &config,
                docker_launch_binding.as_ref(),
            )?;
            if report.is_some() {
                emit(AgentLaunchEvent::Progress {
                    message: "Trusted the gwt-managed Codex worktree.".to_string(),
                });
            }
        }
        resolve_docker_agent_program_with_binding(&mut config, docker_launch_binding.as_ref())?;
        phases.mark("environment");
        let runner_health_report = (config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host)
            .then(|| resolve_host_runner_health_checked(&mut config))
            .transpose()?;
        phases.mark("runner_health");
        let codex_hook_discovery_mode =
            codex_hook_discovery_mode_for_launch_config(&config, runner_health_report.as_ref());
        let managed_assets = refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
            &worktree_path,
            &config.agent_id,
            codex_hook_discovery_mode,
            config.is_ephemeral,
        )
        .map_err(|error| {
            // Attribute managed-asset failures to the worktree so the
            // operator sees which worktree's setup failed, not a bare
            // skill-writer error.
            format!(
                "managed asset setup failed for worktree {}: {error}",
                worktree_path.display()
            )
        })?;
        phases.mark("managed_assets");
        // Issue #4283 AC-5: a split of the phase above, not a sibling of
        // it. Materialization takes one lock per worktree and a short one
        // per repository; when this is most of `managed_assets` the fleet
        // is queueing rather than the work getting slower.
        gwt::perf::record_route_phase(
            gwt::perf::PerfRoute::PaneCreate,
            "asset_lock_wait",
            managed_assets.lock_wait,
        );
        if let Some(report) = maybe_register_codex_managed_hook_trust_for_launch(
            &profile_config_path,
            &worktree_path,
            &config,
            docker_launch_binding.as_ref(),
            codex_hook_discovery_mode,
            managed_assets.hook_bin.as_deref(),
        )? {
            if !report.trusted_entries.is_empty() {
                emit(AgentLaunchEvent::Progress {
                    message: format!(
                        "Trusted {} gwt-managed Codex hooks.",
                        report.trusted_entries.len()
                    ),
                });
            }
        }

        install_launch_gwt_bin_env(&mut config.env_vars, config.runtime_target)?;
        // SPEC #1921 Phase 86 (#3813): resolve the resource policy and
        // build parallelism before Docker materializes the exec environment
        // so container agents receive the same budget variables.
        let resource_policy = resolve_agent_resource_launch(
            &profile_config_path,
            Path::new(&project_root),
            &mut config.env_vars,
        )?;
        // SPEC-3248 P8a: derive the execution entrypoint from the raw
        // launch argv BEFORE the Windows host shell wrapper rewrites it
        // (the `$gwt-*` prompt token moves into an env var / embedded
        // script on wrapped launches).
        let execution_entrypoint = gwt::cli::execution_state::entrypoint_from_launch(
            config.entrypoint_args(),
            config.session_mode == gwt_agent::SessionMode::Resume,
        );
        apply_windows_host_shell_wrapper(&mut config)?;

        let branch_name = config.branch.clone().unwrap_or_else(|| "work".to_string());

        let agent_id = config.agent_id.clone();
        let mut session =
            initialize_launch_session(&worktree_path, Path::new(&project_root), &config);
        // Keep a separate observation identity if the resumed authority
        // cannot be installed. Never overwrite the coordinator's Session.
        let unbound_session_id = session.id.clone();
        // SPEC-3393 FR-012 (AC-12) / #3410: a Resume/Continue launch
        // recovers producing authority through the continuation
        // coordinator before spawn. Failure degrades to an unbound,
        // input-capable launch — a resume must degrade, never block.
        //
        // Issue #3423 / Issue #4207: the coordinator answers with two
        // outcomes and *neither* leaves a Prepared attempt behind.
        // `ReboundCurrent` re-validated the predecessor Session's
        // current-generation binding; `SuccessorCreated` minted the next
        // generation and activated it in the same lease
        // (`activate_successor_with_session_rebind`). Routing
        // `SuccessorCreated` through the Prepared install arm asked the
        // Prepared validator for an attempt that was already `Activated`,
        // so every resume of a Session that had settled its generation —
        // i.e. every Issue that had progressed far enough to push — died
        // before the PTY. Both outcomes carry Active authority.
        let mut rebound_continuation: Option<gwt_agent::SessionExecutionBinding> = None;
        if matches!(
            &config.execution_intent,
            gwt_agent::ExecutionLaunchIntent::Automatic
        ) && matches!(
            config.session_mode,
            gwt_agent::SessionMode::Resume | gwt_agent::SessionMode::Continue
        ) {
            if let Some(predecessor) = config.predecessor_session_id.clone() {
                // Issue #4783 AC-2: an automatic restore never reactivates
                // a terminal predecessor generation; see the coordinator.
                if let Some((receipt, binding)) = gwt::prepare_resume_producing_authority(
                    Path::new(&project_root),
                    &predecessor,
                    config.automatic_restore,
                ) {
                    match receipt.outcome {
                        gwt::AgentExecutionContinuationOutcome::SuccessorCreated
                        | gwt::AgentExecutionContinuationOutcome::ReboundCurrent => {
                            rebound_continuation = Some(binding);
                        }
                    }
                }
            }
        }
        let manual_successor_prepared = matches!(
            &config.execution_intent,
            gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(_)
        );
        let prepared_continuation = match &config.execution_intent {
            gwt_agent::ExecutionLaunchIntent::Automatic => None,
            gwt_agent::ExecutionLaunchIntent::PreparedContinuation(binding)
            | gwt_agent::ExecutionLaunchIntent::PreparedManualSuccessor(binding) => {
                session.id = binding.session_id.clone();
                session.set_execution_binding(Some(binding.clone()))?;
                if manual_successor_prepared {
                    let owner_kind = match binding.owner_kind.as_str() {
                        "spec" => gwt::cli::execution_state::ExecutionOwnerKind::Spec,
                        "issue" => gwt::cli::execution_state::ExecutionOwnerKind::Issue,
                        _ => {
                            return Err(("Prepared manual successor owner kind is not canonical"
                                .to_string())
                            .into())
                        }
                    };
                    let owner = gwt::cli::execution_state::ExecutionOwnerKey {
                        kind: owner_kind,
                        number: binding.owner_number,
                    };
                    let attempt =
                        gwt::cli::execution_state::prepared_fresh_linked_owner_launch_for_session(
                            &worktree_path,
                            owner,
                            &session.id,
                        )
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            "Prepared manual successor attempt is unavailable".to_string()
                        })?;
                    let session_identity =
                        gwt_agent::SessionExecutionIdentity::for_binding(&session, binding)?;
                    persist_durable_launch_recovery_with_identity(
                        &sessions_dir,
                        DurableLaunchRecoveryKind::FreshSuccessor {
                            operation_id: attempt.request.operation_id,
                        },
                        &session.id,
                        Path::new(&project_root),
                        &worktree_path,
                        owner,
                        Some(binding),
                        Some(&session.agent_id),
                        Some(&session_identity),
                    )?;
                }
                Some(binding.clone())
            }
            gwt_agent::ExecutionLaunchIntent::ManualSuccessor { .. } => {
                return Err(
                    ("Manual successor reached the launch worker before typed preparation"
                        .to_string())
                    .into(),
                )
            }
        };
        if let Some(binding) = rebound_continuation.as_ref() {
            install_authenticated_active_resume_binding(
                &mut session,
                &mut config.linked_issue_number,
                binding,
            )?;
        }

        let mut session_id = session.id.clone();
        let mut runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
        config.env_vars.insert(
            gwt_agent::GWT_SESSION_ID_ENV.to_string(),
            session_id.clone(),
        );
        config.env_vars.insert(
            gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
            runtime_path.display().to_string(),
        );
        let runtime_target = config.runtime_target;
        config
            .env_vars
            .entry("COLORTERM".to_string())
            .or_insert_with(|| "truecolor".to_string());
        let docker_runtime_worktree = docker_launch_binding
            .as_ref()
            .map(|binding| binding.container_cwd().to_string());
        let agent_project_root = docker_runtime_worktree.unwrap_or_else(|| {
            config
                .env_vars
                .get("GWT_PROJECT_ROOT")
                .cloned()
                .unwrap_or_else(|| worktree_path.display().to_string())
        });

        if runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            let project_state_root = session
                .project_state_root
                .as_deref()
                .filter(|root| !root.as_os_str().is_empty())
                .ok_or_else(|| {
                    "Docker launch is missing the host Project State root before Session persistence"
                        .to_string()
                })?
                .to_path_buf();
            session.bind_docker_runtime(&agent_project_root, &project_state_root)?;
        }

        // Genesis producing authority is minted only for a fresh linked
        // non-ephemeral launch that owns its execution lifecycle. Resume
        // and Continue launches recover producing authority exclusively
        // through the continuation coordinator; Continue work creates
        // successor generations through its coordinator instead of
        // falling through this genesis-only launch path.
        let session_mode = config.session_mode;
        let had_prepared_execution = prepared_continuation.is_some();
        let producing_owner = (!config.is_ephemeral
            && !config.suppress_execution_control
            && prepared_continuation.is_none()
            && session_mode == gwt_agent::SessionMode::Normal)
            .then(|| {
                config.linked_issue_number.map(|owner_number| {
                    gwt::cli::execution_state::ExecutionOwnerKey {
                        kind: gwt::cli::execution_state::detect_owner_kind(
                            &worktree_path,
                            owner_number,
                        ),
                        number: owner_number,
                    }
                })
            })
            .flatten();
        let producing_owner = producing_owner_after_delivery_check(
            &worktree_path,
            producing_owner,
            config.explicit_follow_up,
        );
        // A pre-row audit is no launch grant. Recheck the actual provider and
        // profile-materialized args/env before installing any authority.
        let custom = configured_custom_agent_for_launch(&config, &profile_config_path)?;
        let mut previous = config.permission_decision.clone();
        previous.provider_mapping =
            gwt_agent::provider_skip_mapping(&config.agent_id, custom.as_ref());
        previous.provider = previous.provider_mapping.provider.clone();
        if config.skip_permissions && !previous.skip_permissions_requested() {
            previous.requested_skip_permissions = Some(true);
        }
        let permission_decision = gwt_agent::redecide_for_materialized_launch(
            &previous,
            previous.source,
            false,
            &config.args,
            &config.env_vars,
        );
        // Keep the existing ECR decision parameter consistent with the final
        // materialization; Session.skip_permissions records the requested mode.
        config.permission_decision = permission_decision.clone();
        // Issue #4544 AC-1: the GUI screened its row separately. Conditions
        // that changed since that audit must still refuse here, before
        // capability/ECR installation or delivery of a prompt to a child.
        if let Some(owner) = producing_owner {
            gwt::cli::permission_readiness::block_launch_if_unready(
                &worktree_path,
                owner.kind.as_str(),
                owner.number,
                &session_id,
                &permission_decision,
            )?;
        }
        let capability_install = FinalizedAgentCapabilityLaunch {
            issuer: agent_capability_issuer.as_ref(),
            sessions_dir: &sessions_dir,
            session: &mut session,
            project_root: Path::new(&project_root),
            worktree: &worktree_path,
            producing_owner,
            prepared_continuation: prepared_continuation.as_ref(),
            rebound_continuation: rebound_continuation.as_ref(),
            execution_entrypoint: &execution_entrypoint,
            runtime_target,
            container_runtime: docker_launch_binding
                .as_ref()
                .map(DockerLaunchBinding::runtime),
            permission_decision: Some(&permission_decision),
        }
        .install_with_prepared_claim(
            &mut config.env_vars,
            active_launch_handshake_cleanup.as_ref(),
        );
        issued_capability_token = config
            .env_vars
            .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
            .cloned();
        active_launch_handshake_cleanup = match capability_install {
            Ok(handshake) => handshake,
            Err(error) if rebound_continuation.is_some() => {
                // Issue #3626: only automatic Resume/Continue recovered
                // through the coordinator may degrade. Fresh and explicitly
                // prepared launches retain their fail-closed contract.
                // ActiveContinuationInstall already released its own claim;
                // a competing Session's handshake must remain untouched.
                session.set_execution_binding(None)?;
                session.id = unbound_session_id;
                session_id = session.id.clone();
                runtime_path = gwt_agent::runtime_state_path(&sessions_dir, &session_id);
                config.env_vars.insert(
                    gwt_agent::GWT_SESSION_ID_ENV.to_string(),
                    session_id.clone(),
                );
                config.env_vars.insert(
                    gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                    runtime_path.display().to_string(),
                );
                // Also mask inherited credentials if issuing an inspection
                // capability fails. An input-capable PTY needs no bearer.
                let capability_env_keys = [
                    gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
                    gwt_agent::GWT_HOOK_FORWARD_URL_ENV,
                    gwt_agent::GWT_PANE_WS_URL_ENV,
                    gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV,
                ];
                for key in capability_env_keys {
                    config.env_vars.remove(key);
                    config.remove_env.push(key.to_string());
                }
                if let Err(unbound_error) = install_agent_capability_env(
                    &mut config.env_vars,
                    agent_capability_issuer.as_ref(),
                    Path::new(&project_root),
                    &session_id,
                    runtime_target,
                    docker_launch_binding
                        .as_ref()
                        .map(DockerLaunchBinding::runtime),
                ) {
                    tracing::warn!(%unbound_error, "resume inspection capability unavailable");
                }
                // Windows applies removals after explicit env assignments.
                // Keep only missing keys masked after inspection issuance.
                for key in capability_env_keys {
                    if config.env_vars.contains_key(key) {
                        config.remove_env.retain(|removed| removed != key);
                    }
                }
                issued_capability_token = config
                    .env_vars
                    .get(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
                    .cloned();
                let reason = gwt_core::error_ledger::sanitize_error_message(
                    &sanitize_launch_log_error(&error.to_string()),
                );
                let message = format!(
                    "Resume degraded to unbound (no execution authority). Reason: {reason}. \
                     Input remains available. Use Start Work / Launch to obtain execution \
                     authority before delivering changes."
                );
                emit(AgentLaunchEvent::TerminalOutput {
                    data: format!("\r\n[gwt] {message}\r\n").into_bytes(),
                });
                emit(AgentLaunchEvent::Toast {
                    level: "warn".to_string(),
                    message: message.clone(),
                    issue_number: config.linked_issue_number,
                });
                let _ = gwt::error_report::report_error_and_publish_with_context(
                    gwt_core::error_ledger::ErrorKind::LaunchFailure,
                    message,
                    gwt_core::error_ledger::ErrorTarget {
                        issue: config.linked_issue_number,
                        window_id: Some(window_id.clone()),
                        session_id: Some(session_id.clone()),
                        project_root: Some(project_root.clone()),
                    },
                    std::collections::BTreeMap::from([(
                        "disposition".to_string(),
                        "degraded_unbound".to_string(),
                    )]),
                );
                None
            }
            Err(error) => return Err(error),
        };
        if manual_successor_prepared {
            config.env_vars.insert(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV.to_string(),
                uuid::Uuid::new_v4().to_string(),
            );
        }
        // Docker fixes env keys into its argv: finalize only after the
        // capability and any degraded Session identity are final.
        finalize_docker_agent_launch_config_with_binding(
            &mut config,
            docker_launch_binding.as_ref(),
        )?;
        let carries_fresh_execution_readiness = config
            .env_vars
            .contains_key(gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV);
        let is_fresh_execution_install = carries_fresh_execution_readiness
            && ((producing_owner.is_some() && prepared_continuation.is_none())
                || manual_successor_prepared);
        let materialized_genesis_install = producing_owner
            .filter(|_| !is_fresh_execution_install)
            .zip(session.execution_binding.clone())
            .map(|(owner, binding)| {
                let session_identity =
                    gwt_agent::SessionExecutionIdentity::for_binding(&session, &binding)?;
                Ok::<_, String>(MaterializedGenesisLaunch {
                    worktree_path: worktree_path.clone(),
                    owner,
                    session_id: session_id.clone(),
                    binding,
                    session_identity,
                })
            })
            .transpose()?;
        // Recheck the proof against the actual child cwd and final environment.
        // A PM runtime may use a cwd different from the worktree.
        session.codex_auth_root = pm_provider_runtime_dir(&config)
            .as_deref()
            .or(config.working_dir.as_deref())
            .and_then(|cwd| config.validated_codex_auth_root_for_cwd(cwd));
        if let Err(error) = persist_finalized_launch_session(
            &sessions_dir,
            // Monitor's shared admission census must not mistake a
            // prepared no-child runtime for an unknown live worker. Its
            // physical spawn publishes exact identity under admission.
            (!issue_monitor_launch).then_some(runtime_path.as_path()),
            &mut session,
            None,
        ) {
            let rollback = if is_fresh_execution_install {
                Some(rollback_materialized_fresh_execution_launch(
                    &sessions_dir,
                    &session_id,
                    &worktree_path,
                    "fresh launch final Session persistence failed",
                    &session.agent_id,
                ))
            } else {
                materialized_genesis_install.as_ref().map(|genesis| {
                    terminalize_materialized_genesis_launch(
                        &sessions_dir,
                        Some(genesis),
                        "genesis final Session persistence failed",
                    )
                })
            };
            return Err((match rollback {
                None | Some(Ok(())) => error,
                Some(Err(rollback_error)) => {
                    format!("{error}; candidate rollback requires reconciliation: {rollback_error}")
                }
            })
            .into());
        }
        if let Some((member, reservation)) = member_reservation.as_ref() {
            gwt::agent_names::bind_project_member_session(
                Path::new(&project_root),
                member,
                reservation,
                &session.id,
            )?;
        }
        let expected_execution_identity =
            gwt_agent::SessionExecutionIdentity::from_session(&session)?;

        let process_launch = ProcessLaunch {
            initial_prompt_file,
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env_vars.clone(),
            remove_env: config.remove_env.clone(),
            cwd: pm_provider_runtime_dir(&config).or_else(|| config.working_dir.clone()),
            resource_policy,
        };
        // Issue #3490 AC-1: without a Backend Override profile every Codex
        // process shares the user's `~/.codex`, and two of them
        // initializing its SQLite state together lose the race with
        // `database is locked`. gwt fans launches out in parallel, so it
        // owns the mitigation: hold the last step before dispatch until
        // the previous shared-`~/.codex` spawn has cleared its
        // initialization window. This runs on the per-launch worker
        // thread, never on the UI thread, and a worktree-local CODEX_HOME
        // has no contention to pace.
        phases.mark("session");
        if gwt_agent::shares_user_codex_state(&agent_id, runtime_target, &process_launch.env) {
            let waited = gwt_agent::pace_shared_codex_spawn();
            if !waited.is_zero() {
                tracing::debug!(
                    waited_ms = waited.as_millis() as u64,
                    "paced Codex spawn to avoid shared ~/.codex state contention"
                );
            }
            phases.mark("codex_pace");
        }
        Ok((
            process_launch,
            session_id,
            branch_name,
            config.display_name,
            worktree_path,
            agent_id,
            config.linked_issue_number,
            config.base_branch.clone(),
            runtime_target,
            session_mode,
            had_prepared_execution,
            AgentLaunchRuntimeContext {
                agent_project_root,
                predecessor_session_id: config.predecessor_session_id.clone(),
                expected_execution_identity,
                active_launch_handshake: active_launch_handshake_cleanup.take(),
            },
        ))
    })();

    // Issue #4283 AC-5: everything above runs on this launch thread, while
    // `route:pane.create` is closed by the GUI event loop when it handles
    // the dispatch below. `route:pane.create` minus this sample is that
    // hand-back, which no mark on this thread can reach.
    phases.mark_total("launch_thread");

    // Drop (= final drain + join) BEFORE dispatching the result so the
    // tail of the mirrored docker output lands in the terminal ahead of
    // the success transition or the `[gwt] Launch failed` summary —
    // otherwise the failure summary gets buried mid-stream.
    drop(docker_output_mirror);

    if result.is_err() {
        if let Some((member, reservation)) = member_reservation.as_ref() {
            let _ = gwt::agent_names::release_project_member(
                Path::new(&project_root),
                &sessions_dir,
                member,
                reservation,
            );
        }
        if let Some(handshake) = active_launch_handshake_cleanup.as_ref() {
            let _ = gwt::cli::execution_state::finish_active_session_launch_handshake(
                &sessions_dir,
                handshake,
            );
        }
        if let (Some(issuer), Some(token)) = (
            agent_capability_issuer.as_ref(),
            issued_capability_token.as_deref(),
        ) {
            issuer.revoke_token(token);
        }
    }
    result
}

/// SPEC #1921 Phase 86 (#3813): resolve the AgentBootstrap resource policy
/// for a launch whose repository is `repo_path`, handing the build
/// parallelism to build tools through `env` where it is not already pinned. Settings come from
/// the global profile config; `max_active` from the repository's persisted
/// Issue Monitor preferences (missing preferences count as one agent).
pub fn resolve_agent_resource_launch(
    profile_config_path: &Path,
    repo_path: &Path,
    env: &mut HashMap<String, String>,
) -> Result<Option<gwt_terminal::pty::ProcessPolicy>, String> {
    let settings = if profile_config_path.exists() {
        gwt_config::Settings::load_from_path(profile_config_path).map_err(|error| {
            format!(
                "load agent resource policy from {}: {error}",
                profile_config_path.display()
            )
        })?
    } else {
        gwt_config::Settings::default()
    };
    let max_active =
        gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(repo_path))
            .map_or(1, |prefs| {
                let mut monitor =
                    gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
                monitor.refresh_agent_capacity(repo_path);
                monitor.effective_max_active_agents()
            });
    let logical_cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let resolved = gwt::agent_resource_policy::resolve_agent_resource_policy(
        &settings.agent.resource,
        max_active,
        logical_cores,
    )
    .map_err(|error| format!("agent resource policy: {error}"))?;
    gwt::agent_resource_policy::inject_build_parallelism_env(env, resolved.build_jobs);
    Ok(resolved.process_policy)
}

/// SPEC #3248 FR-239 / AS-216: drop the producing owner when this launch opens
/// a *delivered* one — closed, with its whole source state already in the
/// configured base.
///
/// Dropping it here is what makes Inspection zero-mutation: no Execution
/// Control Record, no Work, no action obligation, and therefore no
/// verification, commit, push, or PR is ever demanded of a session that has
/// nothing to deliver. That demand is the bug — an already shipped Issue could
/// only settle as terminally Blocked, because no new PR existed to point at.
///
/// The GitHub Issue is never touched either way. Only an explicit follow-up
/// intent produces against a delivered owner (FR-240); ambiguity about the
/// source surface falls to Inspection rather than to producing work.
fn producing_owner_after_delivery_check(
    worktree: &Path,
    producing_owner: Option<gwt::cli::execution_state::ExecutionOwnerKey>,
    explicit_follow_up: bool,
) -> Option<gwt::cli::execution_state::ExecutionOwnerKey> {
    producing_owner.filter(|owner| {
        let disposition = gwt::cli::delivered_owner::classify_launch_for_owner(
            worktree,
            worktree,
            owner.number,
            if explicit_follow_up {
                gwt::cli::delivered_owner::FollowUpIntent::ExplicitFollowUp
            } else {
                gwt::cli::delivered_owner::FollowUpIntent::Default
            },
        );
        if disposition.is_inspection() {
            tracing::info!(
                owner = owner.number,
                disposition = disposition.describe(),
                "delivered owner opens for inspection; no execution was materialized"
            );
        }
        !disposition.is_inspection()
    })
}

#[cfg(test)]
mod delivered_owner_launch_tests {
    use super::*;

    fn git(worktree: &Path, args: &[&str]) {
        let status = gwt_core::process::hidden_command("git")
            .arg("-C")
            .arg(worktree)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A worktree whose `origin/develop` already contains its whole source
    /// state — the shape a delivered owner leaves behind.
    fn delivered_worktree(worktree: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.com/t/delivered-owner.git",
            ],
            vec!["commit", "--allow-empty", "-qm", "init"],
            vec!["update-ref", "refs/remotes/origin/develop", "HEAD"],
            vec!["checkout", "-q", "-b", "work/issue-3290"],
        ] {
            git(worktree, &args);
        }
    }

    fn cache_owner_state(worktree: &Path, number: u64, state: &str) {
        let root = gwt::issue_cache::issue_cache_root_for_repo_path(worktree)
            .expect("the fixture repo resolves a repo hash");
        let dir = root.join(number.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("meta.json"),
            serde_json::json!({"number": number, "state": state, "labels": []}).to_string(),
        )
        .unwrap();
    }

    fn owner(number: u64) -> gwt::cli::execution_state::ExecutionOwnerKey {
        gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number,
        }
    }

    // AC-1 / AS-216: opening a closed, zero-diff owner drops the producing
    // owner, which is what keeps the launch zero-mutation.
    #[test]
    fn a_delivered_owner_opens_for_inspection() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        delivered_worktree(dir.path());
        cache_owner_state(dir.path(), 3290, "closed");

        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), Some(owner(3290)), false),
            None,
            "a delivered owner materializes no execution"
        );
    }

    // AC-1 / AS-217: an explicit follow-up is the one intent that produces
    // against a delivered owner.
    #[test]
    fn an_explicit_follow_up_produces_against_a_delivered_owner() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        delivered_worktree(dir.path());
        cache_owner_state(dir.path(), 3290, "closed");

        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), Some(owner(3290)), true),
            Some(owner(3290))
        );
    }

    // AC-1: an open owner, and a closed owner that still holds source work,
    // both keep the unchanged producing path. This is the regression that
    // matters most — the check must not quietly stop ordinary launches.
    #[test]
    fn open_owners_and_undelivered_source_keep_producing() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        delivered_worktree(dir.path());
        cache_owner_state(dir.path(), 4545, "open");
        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), Some(owner(4545)), false),
            Some(owner(4545)),
            "an open owner is never delivered"
        );

        // An owner the cache does not know is not evidence of delivery.
        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), Some(owner(9999)), false),
            Some(owner(9999))
        );

        cache_owner_state(dir.path(), 3290, "closed");
        std::fs::create_dir_all(dir.path().join("crates/gwt/src")).unwrap();
        std::fs::write(dir.path().join("crates/gwt/src/new.rs"), "pub fn x() {}\n").unwrap();
        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), Some(owner(3290)), false),
            Some(owner(3290)),
            "a closed owner with undelivered source still produces"
        );
    }

    // AC-1: a launch with no linked owner is untouched by the check.
    #[test]
    fn an_unlinked_launch_is_unaffected() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            producing_owner_after_delivery_check(dir.path(), None, false),
            None
        );
    }
}

/// Provenance carried only by Issue Monitor launches. Possessing this value is
/// not sufficient to trust a path: [`validate_issue_monitor_managed_codex_worktree`]
/// still reconciles it with the resolved launch and Git's authoritative
/// worktree inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorTrustCandidate {
    pub issue_number: u64,
    pub project_root: PathBuf,
    pub session_mode: gwt_agent::SessionMode,
    pub review_dispatch: bool,
}

/// A canonical worktree whose Issue Monitor ownership and Git inventory have
/// been checked together. Keep the path private so an arbitrary launch cwd
/// cannot be passed directly to the Codex trust writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCodexWorktree(PathBuf);

impl ManagedCodexWorktree {
    fn path(&self) -> &Path {
        &self.0
    }
}

pub fn issue_monitor_trust_candidate_from_feedback(
    agent_id: &gwt_agent::AgentId,
    context: Option<&LaunchFeedbackContext>,
) -> Result<Option<IssueMonitorTrustCandidate>, String> {
    if agent_id != &gwt_agent::AgentId::Codex {
        return Ok(None);
    }
    let Some((context, issue_number)) = context.and_then(|context| {
        context
            .issue_monitor_issue_number
            .map(|issue_number| (context, issue_number))
    }) else {
        return Ok(None);
    };
    let project_root = context.issue_monitor_project_root.clone().ok_or_else(|| {
        format!("Issue Monitor launch for Issue #{issue_number} has no project root provenance")
    })?;
    let session_mode = context.issue_monitor_session_mode.ok_or_else(|| {
        format!("Issue Monitor launch for Issue #{issue_number} has no session mode provenance")
    })?;
    if !matches!(
        session_mode,
        gwt_agent::SessionMode::Normal | gwt_agent::SessionMode::Resume
    ) {
        return Err(format!(
            "Issue Monitor managed Codex trust requires Normal or Resume session mode, got {session_mode:?}"
        ));
    }
    Ok(Some(IssueMonitorTrustCandidate {
        issue_number,
        project_root,
        session_mode,
        review_dispatch: context.issue_monitor_review_dispatch,
    }))
}

pub fn codex_hook_discovery_mode_for_launch_config(
    config: &gwt_agent::LaunchConfig,
    health_report: Option<&gwt_agent::HostRunnerHealthReport>,
) -> gwt_skills::CodexHookDiscoveryMode {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return gwt_skills::CodexHookDiscoveryMode::WorkspaceHome;
    }
    if let Some(mode) =
        codex_hook_discovery_mode_from_detected_codex_version(config.tool_version.as_deref())
    {
        return mode;
    }
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return gwt_skills::CodexHookDiscoveryMode::Both;
    }
    let Some(report) = health_report else {
        return gwt_skills::CodexHookDiscoveryMode::Both;
    };
    if let Some(mode) = report
        .version_output
        .as_deref()
        .and_then(codex_hook_discovery_mode_from_codex_version_output)
    {
        return mode;
    }
    gwt_skills::CodexHookDiscoveryMode::Both
}

pub fn codex_hook_discovery_mode_from_detected_codex_version(
    version: Option<&str>,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let version = version?.trim();
    codex_hook_discovery_mode_from_semver(version)
}

pub fn codex_hook_discovery_mode_from_codex_version_output(
    output: &str,
) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    output
        .split_whitespace()
        .find_map(codex_hook_discovery_mode_from_semver)
}

fn codex_hook_discovery_mode_from_semver(raw: &str) -> Option<gwt_skills::CodexHookDiscoveryMode> {
    let token = raw
        .trim()
        .trim_start_matches('v')
        .trim_matches(|c| c == ',' || c == ';');
    let version = semver::Version::parse(token).ok()?;
    let boundary =
        semver::Version::parse("0.131.0-alpha.21").expect("valid Codex hook discovery boundary");
    // Issue #5194: newer Codex releases do not reliably read the
    // workspace-home copy from a linked worktree (0.160 never ran SessionStart
    // when only that copy existed), so every Codex at or above the cutover
    // also gets the worktree-local file instead of trusting the cutover alone.
    Some(if version < boundary {
        gwt_skills::CodexHookDiscoveryMode::WorktreeLocal
    } else {
        gwt_skills::CodexHookDiscoveryMode::Both
    })
}

// Session and LaunchConfig keep the project checkout as their identity. Only
// the provider process discovers instructions from the isolated PM runtime.
fn pm_provider_runtime_dir(config: &gwt_agent::LaunchConfig) -> Option<PathBuf> {
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return None;
    }
    config
        .working_dir
        .as_deref()
        .and_then(gwt::pm_registry::pm_runtime_dir_for_pm_worktree)
}

/// `generated_hook_bin` is the fallback binary the materialization that just
/// ran pinned into the hook commands it wrote (#3967). Trust has to compare
/// against that exact value: the pin is released when materialization returns,
/// and re-deriving it here answered `target/debug/gwtd` — reduced to the bare
/// `gwtd` for a config outside that checkout — for every gwt started from a
/// development build, while the generated command carried the installed
/// absolute path. All five managed hooks then failed the exact-command match
/// and Codex stopped the launch on `Hooks need review`.
#[allow(clippy::too_many_arguments)]
pub fn maybe_register_codex_managed_hook_trust_for_launch(
    profile_config_path: &Path,
    worktree_path: &Path,
    config: &gwt_agent::LaunchConfig,
    docker_binding: Option<&DockerLaunchBinding>,
    codex_hook_discovery_mode: gwt_skills::CodexHookDiscoveryMode,
    generated_hook_bin: Option<&str>,
) -> Result<Option<gwt_skills::CodexHookTrustReport>, String> {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return Ok(None);
    }

    let settings = if profile_config_path.exists() {
        match gwt_config::Settings::load_from_path(profile_config_path) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(
                    profile_config = %profile_config_path.display(),
                    error = %error,
                    "failed to read gwt config while preparing Codex hook trust; continuing launch"
                );
                gwt_config::Settings::default()
            }
        }
    } else {
        gwt_config::Settings::default()
    };
    if settings.agent.codex_trust_managed_hooks == Some(false) {
        return Ok(None);
    }

    match config.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let pm_runtime = pm_provider_runtime_dir(config);
            let child_cwd = pm_runtime
                .as_deref()
                .or(config.working_dir.as_deref())
                .unwrap_or(worktree_path);
            let hooks_root = pm_runtime.as_deref().unwrap_or(worktree_path);
            let codex_config_path = match effective_host_codex_config_path(
                child_cwd,
                &config.env_vars,
                HostEnvKeySemantics::native(),
                dirs::home_dir().as_deref(),
            ) {
                Ok(path) => path,
                Err(error) => {
                    tracing::warn!(
                        profile_config = %profile_config_path.display(),
                        error = %error,
                        "cannot derive Codex config path while preparing Codex hook trust; continuing launch"
                    );
                    return Ok(None);
                }
            };
            // Issue #3967 AC-1: trust every managed hooks file that can exist
            // on disk, not just the one this launch would generate. The
            // `Both`-mode managed-asset writers (startup self-heal, hook front
            // door, health repair, SessionStart re-materialization) keep the
            // worktree-local copy refreshed even when generation targets the
            // workspace-home copy, and Codex discovers whichever it finds
            // first. Narrowing trust to the generation mode left the other copy
            // orphaned — present, gwt-generated, and untrusted. Trust state is
            // keyed by absolute hooks path, so entries for a file Codex does
            // not read are inert.
            let report = gwt_skills::register_codex_managed_hook_trust_for_mode_with_expected_bin(
                hooks_root,
                &codex_config_path,
                gwt::managed_assets::MANAGED_CODEX_HOOK_DISCOVERY_MODE,
                generated_hook_bin,
            )
            .map_err(|error| {
                format!(
                    "Codex hook trust registration failed for worktree {} (config {}): {error}",
                    worktree_path.display(),
                    codex_config_path.display()
                )
            })?;
            // Issue #4071 AC-2: the reason says whether any trust state was
            // written and what each hooks file was compared against, so a
            // skipped registration and a trusted_hash / path-form mismatch
            // are told apart in the launch failure record.
            if let Some(reason) = report.hooks_need_review_reason() {
                return Err(reason);
            }
            Ok(Some(report))
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            let Some(binding) = docker_binding else {
                tracing::warn!(
                    worktree = %worktree_path.display(),
                    "Docker launch has no resolved binding while preparing Codex hook trust; continuing launch"
                );
                return Ok(None);
            };
            let host_gwt_bin = std::env::current_exe()
                .map_err(|error| format!("current_exe: {error}"))
                .map(|current_exe| {
                    gwt_agent::resolve_public_gwt_bin_with_lookup(&current_exe, |command| {
                        which::which(command).ok()
                    })
                })
                .and_then(|path| {
                    path.into_os_string()
                        .into_string()
                        .map_err(|_| "host gwtd path is not valid UTF-8".to_string())
                });
            let result = host_gwt_bin.and_then(|host_gwt_bin| {
                register_codex_managed_hook_trust_in_docker(
                    binding,
                    &config.env_vars,
                    &host_gwt_bin,
                    codex_hook_discovery_mode,
                )
            });
            if let Err(error) = result {
                tracing::warn!(
                    worktree = %worktree_path.display(),
                    error = %error,
                    "failed to register gwt-managed Codex hook trust in Docker; continuing launch"
                );
            }
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEnvKeySemantics {
    CaseSensitive,
    WindowsCaseInsensitive,
}

impl HostEnvKeySemantics {
    fn native() -> Self {
        if cfg!(windows) {
            Self::WindowsCaseInsensitive
        } else {
            Self::CaseSensitive
        }
    }
}

fn effective_host_env_value<'a>(
    env_vars: &'a HashMap<String, String>,
    key: &str,
    semantics: HostEnvKeySemantics,
) -> Result<Option<&'a str>, String> {
    match semantics {
        HostEnvKeySemantics::CaseSensitive => Ok(env_vars.get(key).map(String::as_str)),
        HostEnvKeySemantics::WindowsCaseInsensitive => {
            let mut values = env_vars
                .iter()
                .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(key));
            let value = values.next().map(|(_, value)| value.as_str());
            if values.next().is_some() {
                return Err(format!(
                    "ambiguous {key}: the final Windows child environment contains multiple case-insensitive variants"
                ));
            }
            Ok(value)
        }
    }
}

/// Resolve the Codex config file exactly as Codex 0.148 resolves its home.
///
/// A configured CODEX_HOME is resolved from the final child cwd and must
/// already exist before Codex canonicalizes it. Without CODEX_HOME, Unix uses
/// an absolute HOME while Windows uses the OS known-folder home and ignores
/// HOME/USERPROFILE environment overrides.
pub fn effective_host_codex_config_path(
    child_cwd: &Path,
    env_vars: &HashMap<String, String>,
    semantics: HostEnvKeySemantics,
    os_user_home: Option<&Path>,
) -> Result<PathBuf, String> {
    let codex_home = effective_host_env_value(env_vars, "CODEX_HOME", semantics)?
        .filter(|value| !value.is_empty());
    if let Some(codex_home) = codex_home {
        let codex_home = Path::new(codex_home);
        let resolved = if codex_home.is_absolute() {
            codex_home.to_path_buf()
        } else {
            child_cwd.join(codex_home)
        };
        return canonical_launch_path(&resolved, "CODEX_HOME").map(|home| home.join("config.toml"));
    }

    let user_home = match semantics {
        HostEnvKeySemantics::CaseSensitive => {
            effective_host_env_value(env_vars, "HOME", semantics)?
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        }
        HostEnvKeySemantics::WindowsCaseInsensitive => os_user_home.map(Path::to_path_buf),
    }
    .ok_or_else(|| match semantics {
        HostEnvKeySemantics::CaseSensitive => {
            "cannot determine HOME for the Codex child environment".to_string()
        }
        HostEnvKeySemantics::WindowsCaseInsensitive => {
            "cannot determine the OS user home for Codex".to_string()
        }
    })?;
    if !user_home.is_absolute() {
        return Err(format!(
            "HOME must be absolute when CODEX_HOME is unset, got {}",
            user_home.display()
        ));
    }
    Ok(user_home.join(".codex").join("config.toml"))
}

fn canonical_launch_path(path: &Path, role: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|path| gwt_core::paths::normalize_windows_child_process_path(&path))
        .map_err(|error| format!("failed to canonicalize {role} {}: {error}", path.display()))
}

pub fn validate_issue_monitor_managed_codex_worktree(
    candidate: Option<&IssueMonitorTrustCandidate>,
    actual_project_root: &Path,
    config: &gwt_agent::LaunchConfig,
    worktrees: &[gwt_git::WorktreeInfo],
) -> Result<Option<ManagedCodexWorktree>, String> {
    if config.agent_id != gwt_agent::AgentId::Codex {
        return Ok(None);
    }
    let Some(candidate) = candidate else {
        return Ok(None);
    };

    let candidate_root =
        canonical_launch_path(&candidate.project_root, "Issue Monitor project root")?;
    let actual_root = canonical_launch_path(actual_project_root, "launch project root")?;
    if candidate_root != actual_root {
        return Err(format!(
            "Issue Monitor project root {} does not match launch project root {}",
            candidate_root.display(),
            actual_root.display()
        ));
    }
    if config.linked_issue_number != Some(candidate.issue_number) {
        return Err(format!(
            "Issue Monitor linked Issue #{} does not match launch metadata {:?}",
            candidate.issue_number, config.linked_issue_number
        ));
    }
    if !matches!(
        candidate.session_mode,
        gwt_agent::SessionMode::Normal | gwt_agent::SessionMode::Resume
    ) || config.session_mode != candidate.session_mode
    {
        return Err(format!(
            "Issue Monitor session mode {:?} does not match launch session mode {:?}; managed trust requires matching Normal or Resume provenance",
            candidate.session_mode, config.session_mode
        ));
    }

    let expected_branch = format!("work/issue-{}", candidate.issue_number);
    if !candidate.review_dispatch && config.branch.as_deref() != Some(expected_branch.as_str()) {
        return Err(format!(
            "Issue Monitor branch must be {expected_branch}, got {:?}",
            config.branch
        ));
    }

    let configured_worktree = config.working_dir.as_deref().ok_or_else(|| {
        "Issue Monitor Codex launch has no resolved working directory to validate".to_string()
    })?;
    let configured_worktree = canonical_launch_path(configured_worktree, "launch worktree")?;
    if candidate.review_dispatch {
        if !config.is_ephemeral
            || !config.suppress_execution_control
            || config
                .env_vars
                .get("GWT_REVIEW_DISPATCH")
                .map(String::as_str)
                != Some("1")
            || config.branch.is_some()
            || !is_ephemeral_worktree_path(&configured_worktree)
        {
            return Err(
                "Issue Monitor review trust requires an isolated read-only detached launch".into(),
            );
        }
        if !worktrees.iter().any(|worktree| {
            worktree.branch.is_none()
                && crate::launch_runtime::usable_worktree_entry(worktree)
                && canonical_launch_path(&worktree.path, "authoritative review worktree")
                    .is_ok_and(|path| path == configured_worktree)
        }) {
            return Err(
                "Issue Monitor review checkout is not an authoritative detached gwt worktree"
                    .into(),
            );
        }
        let reviewed_sha = config
            .ephemeral_base_ref
            .as_deref()
            .filter(|sha| !sha.is_empty())
            .ok_or_else(|| "Issue Monitor review trust requires its reviewed SHA".to_string())?;
        let head = gwt_core::process::run_git_logged(
            &["rev-parse", "--verify", "HEAD"],
            Some(&configured_worktree),
        )
        .map_err(|error| {
            format!("failed to inspect Issue Monitor review checkout HEAD: {error}")
        })?;
        if !head.status.success() || String::from_utf8_lossy(&head.stdout).trim() != reviewed_sha {
            return Err(format!(
                "Issue Monitor review checkout HEAD does not match reviewed SHA {reviewed_sha}"
            ));
        }
        return Ok(Some(ManagedCodexWorktree(configured_worktree)));
    }
    let authoritative_worktree =
        crate::launch_runtime::usable_worktree_path_for_branch(worktrees, &expected_branch)
            .ok_or_else(|| {
                format!("Issue Monitor branch {expected_branch} has no authoritative gwt worktree")
            })?;
    let authoritative_worktree =
        canonical_launch_path(&authoritative_worktree, "authoritative gwt worktree")?;
    if configured_worktree != authoritative_worktree {
        return Err(format!(
            "launch worktree {} is not the authoritative gwt worktree {} for {expected_branch}",
            configured_worktree.display(),
            authoritative_worktree.display()
        ));
    }

    Ok(Some(ManagedCodexWorktree(authoritative_worktree)))
}

pub fn register_codex_managed_project_trust_for_resolved_launch(
    profile_config_path: &Path,
    managed_worktree: &ManagedCodexWorktree,
    config: &gwt_agent::LaunchConfig,
    docker_binding: Option<&DockerLaunchBinding>,
) -> Result<Option<gwt_skills::CodexProjectTrustReport>, String> {
    register_codex_managed_project_trust_for_resolved_launch_with_host_context(
        profile_config_path,
        managed_worktree,
        config,
        docker_binding,
        std::env::var_os("CODEX_HOME").as_deref(),
        dirs::home_dir().as_deref(),
    )
}

pub fn register_codex_managed_project_trust_for_resolved_launch_with_host_context(
    _profile_config_path: &Path,
    managed_worktree: &ManagedCodexWorktree,
    config: &gwt_agent::LaunchConfig,
    docker_binding: Option<&DockerLaunchBinding>,
    process_codex_home: Option<&OsStr>,
    os_user_home: Option<&Path>,
) -> Result<Option<gwt_skills::CodexProjectTrustReport>, String> {
    match config.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            let child_cwd = config.working_dir.as_deref().ok_or_else(|| {
                format!(
                    "failed to trust gwt-managed Codex worktree {}: launch has no final child cwd",
                    managed_worktree.path().display()
                )
            })?;
            let child_codex_config_path = effective_host_codex_config_path(
                child_cwd,
                &config.env_vars,
                HostEnvKeySemantics::native(),
                os_user_home,
            )
            .map_err(|error| {
                format!(
                    "failed to trust gwt-managed Codex worktree {}: {error}",
                    managed_worktree.path().display()
                )
            })?;
            let Some(codex_config_path) =
                gwt::managed_assets::process_stable_codex_config_path_for_worktree_with(
                    managed_worktree.path(),
                    process_codex_home,
                    os_user_home,
                )
            else {
                return Ok(None);
            };
            if !gwt::managed_assets::codex_config_paths_equivalent(
                &child_codex_config_path,
                &codex_config_path,
            ) {
                tracing::info!(
                    worktree = %managed_worktree.path().display(),
                    "skipping project trust for a profile-owned Codex config"
                );
                return Ok(None);
            }
            gwt_skills::register_codex_managed_project_trust(
                managed_worktree.path(),
                &codex_config_path,
            )
            .map(Some)
            .map_err(|error| {
                format!(
                    "failed to trust gwt-managed Codex worktree {} using config {}: {error}",
                    managed_worktree.path().display(),
                    codex_config_path.display()
                )
            })
        }
        gwt_agent::LaunchRuntimeTarget::Docker => {
            let binding = docker_binding.ok_or_else(|| {
                format!(
                    "failed to trust gwt-managed Codex worktree {}: Docker launch has no resolved binding",
                    managed_worktree.path().display()
                )
            })?;
            register_codex_managed_project_trust_in_docker(binding, &config.env_vars)
                .map(|()| None)
                .map_err(|error| {
                    format!(
                        "failed to trust gwt-managed Codex worktree {}: {error}",
                        managed_worktree.path().display()
                    )
                })
        }
    }
}

pub fn sanitize_launch_log_error(error: &str) -> String {
    let sensitive_env_keys = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GWT_HOOK_TOKEN",
        "HOOK_TOKEN",
    ];
    let sensitive_flags = [
        "--api-key",
        "--apikey",
        "--token",
        "--auth-token",
        "--hook-token",
    ];

    let mut tokens = Vec::new();
    let mut redact_next = false;
    for token in error.split_whitespace() {
        if redact_next {
            tokens.push("[REDACTED]".to_string());
            redact_next = false;
            continue;
        }

        let normalized = token
            .trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | ';'))
            .to_ascii_lowercase();
        if sensitive_flags.iter().any(|flag| normalized == *flag) {
            tokens.push(token.to_string());
            redact_next = true;
            continue;
        }
        if let Some(flag) = sensitive_flags
            .iter()
            .find(|flag| normalized.starts_with(&format!("{flag}=")))
        {
            tokens.push(format!("{flag}=[REDACTED]"));
            continue;
        }
        if let Some((key, _value)) = token.split_once('=') {
            let normalized_key = key.trim_matches(|ch: char| matches!(ch, '"' | '\''));
            if sensitive_env_keys
                .iter()
                .any(|candidate| normalized_key.eq_ignore_ascii_case(candidate))
            {
                tokens.push(format!("{normalized_key}=[REDACTED]"));
                continue;
            }
        }

        tokens.push(token.to_string());
    }
    tokens.join(" ")
}

#[cfg(test)]
#[path = "launch_contract_tests.rs"]
mod launch_contract_tests;

pub type RuntimeIssueClient = Arc<dyn gwt_github::IssueClient>;
pub type RuntimeIssueClientFactory =
    Arc<dyn Fn(&str, &str) -> Result<RuntimeIssueClient, gwt_github::ApiError> + Send + Sync>;

pub fn default_issue_client_factory() -> RuntimeIssueClientFactory {
    Arc::new(|owner, repo| {
        let client =
            gwt_github::client::http::HttpIssueClient::from_runtime_environment(owner, repo)?;
        Ok(Arc::new(client) as RuntimeIssueClient)
    })
}

pub(crate) fn record_named_launch_assignment(
    session: &gwt_agent::Session,
    predecessor_session_id: Option<&str>,
    sessions_dir: &std::path::Path,
    factory: &RuntimeIssueClientFactory,
) -> Result<(), String> {
    if session.agent_name.is_none()
        || session.linked_issue_number.is_none()
        || session.review_dispatch
    {
        return Ok(());
    }
    let predecessor = predecessor_session_id.or(session.restore_source_session_id.as_deref());
    let previous_name = predecessor.and_then(|id| {
        gwt_agent::Session::load(&sessions_dir.join(format!("{id}.toml")))
            .ok()
            .filter(|previous| {
                previous.id == id && previous.linked_issue_number == session.linked_issue_number
            })
            .and_then(|previous| previous.agent_name)
    });
    let project_root = session
        .project_state_root
        .as_deref()
        .unwrap_or(&session.worktree_path);
    let (owner, repo) = crate::issue_monitor_worker::github_remote_owner_and_repo(project_root)
        .map_err(|error| error.to_string())?;
    let client = factory(&owner, &repo).map_err(|error| error.to_string())?;
    crate::session_launch::record_named_issue_assignment(
        client.as_ref(),
        session,
        previous_name.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
